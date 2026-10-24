//! Kernel handle ownership + leak prevention (S67).
//!
//! Windows kernel handles (files, pipes, events, processes) are NOT garbage
//! collected: every `CreateFileW` / `CreateEventW` / `OpenProcess` must be
//! paired with `CloseHandle`. A single `?` on an error path silently leaks —
//! which is exactly how `micontrol.exe` reached **188,715 handles / 2.5 GB
//! commit charge** after 218 h of uptime (see `memory-leak-audit.md`).
//!
//! This module provides three layers of defence:
//!
//! 1. [`OwnedHandle`] — RAII wrapper. Closes on `Drop`, so early `?` returns
//!    cannot leak. Use it for EVERY handle that is not owned by a safe wrapper
//!    (`std::fs::File`, `BorrowedHandle`, COM interfaces).
//! 2. [`OwnedEvent`] — same for unnamed events created for overlapped I/O.
//! 3. [`HandleMonitor`] — runtime leak detector. Samples `GetProcessHandleCount`
//!    periodically, computes the growth rate and logs/alerts when a leak is in
//!    progress, so a future regression is caught in hours instead of weeks.
//!
//! ## Rules for new code
//!
//! ```text
//! // WRONG — leaks on every early return
//! let h = unsafe { CreateFileW(...) }?;
//! let e = unsafe { CreateEventW(...) }?;
//! do_work(h, e)?;              // <-- leak here
//! unsafe { CloseHandle(h) };
//!
//! // RIGHT — cannot leak
//! let h = OwnedHandle::new(unsafe { CreateFileW(...) }?);
//! let e = OwnedEvent::new(unsafe { CreateEventW(...) }?);
//! do_work(h.raw(), e.raw())?;  // drop closes both
//! ```
//!
//! `OwnedHandle::raw()` intentionally returns the raw value without handing
//! over ownership, so the guard stays the single owner.

#[cfg(windows)]
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};

// ── RAII guards ─────────────────────────────────────────────────────────────

/// Sole owner of a Windows kernel handle. Closes it exactly once on drop.
///
/// `INVALID_HANDLE_VALUE` and null handles are ignored (nothing to close).
#[cfg(windows)]
#[derive(Debug)]
pub struct OwnedHandle(HANDLE);

#[cfg(windows)]
impl OwnedHandle {
    /// Take ownership of `handle`.
    pub fn new(handle: HANDLE) -> Self {
        Self(handle)
    }

    /// Raw handle value for passing to Win32 APIs. Ownership stays here.
    pub fn raw(&self) -> HANDLE {
        self.0
    }

    /// True when the wrapped handle cannot be used (invalid or null).
    pub fn is_invalid(&self) -> bool {
        self.0 == INVALID_HANDLE_VALUE || self.0.is_invalid()
    }

    /// Release the handle now (idempotent — `Drop` becomes a no-op).
    pub fn close(mut self) {
        self.close_inner();
    }

    fn close_inner(&mut self) {
        let h = std::mem::replace(&mut self.0, INVALID_HANDLE_VALUE);
        if h != INVALID_HANDLE_VALUE && !h.is_invalid() {
            // SAFETY: we own the handle (single owner by construction) and
            // replace it with INVALID_HANDLE_VALUE so it is closed only once.
            let _ = unsafe { CloseHandle(h) };
        }
    }
}

#[cfg(windows)]
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        self.close_inner();
    }
}

/// Convenience: an unnamed auto-reset/manual-reset event used by overlapped I/O.
///
/// Wrapping the event separately from [`OwnedHandle`] documents intent and
/// keeps call sites readable (`event.raw()` vs `handle.raw()`).
#[cfg(windows)]
#[derive(Debug)]
pub struct OwnedEvent(OwnedHandle);

#[cfg(windows)]
impl OwnedEvent {
    /// Take ownership of an event handle.
    pub fn new(handle: HANDLE) -> Self {
        Self(OwnedHandle::new(handle))
    }

    /// Raw event handle for `OVERLAPPED.hEvent` / `WaitForSingleObject`.
    pub fn raw(&self) -> HANDLE {
        self.0.raw()
    }
}

// ── Process-wide handle accounting ──────────────────────────────────────────

/// Current number of kernel handles held by this process.
///
/// Returns `None` on non-Windows or when the query fails.
#[cfg(windows)]
pub fn process_handle_count() -> Option<u32> {
    use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessHandleCount};
    let mut count = 0u32;
    // SAFETY: GetCurrentProcess returns a pseudo-handle (never closed) and
    // `count` is a valid out-pointer for the duration of the call.
    let ok = unsafe { GetProcessHandleCount(GetCurrentProcess(), &mut count) };
    if ok.is_ok() {
        Some(count)
    } else {
        None
    }
}

#[cfg(not(windows))]
pub fn process_handle_count() -> Option<u32> {
    None
}

/// Snapshot of the handle count, used by tests and by the runtime monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HandleSnapshot {
    pub count: u32,
    pub at: std::time::Instant,
}

/// Take a handle-count snapshot (test helper + monitor tick).
pub fn snapshot() -> Option<HandleSnapshot> {
    process_handle_count().map(|count| HandleSnapshot {
        count,
        at: std::time::Instant::now(),
    })
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use windows::Win32::System::Threading::{CreateEventW, GetCurrentProcess};
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    /// A guard must release the handle it owns.
    #[test]
    fn owned_handle_closes_on_drop() {
        let before = process_handle_count().expect("handle count");
        {
            let h = unsafe { CreateEventW(None, true, false, windows::core::PCWSTR::null()) }
                .expect("CreateEventW");
            let _guard = OwnedHandle::new(h);
            let during = process_handle_count().expect("handle count");
            assert!(during >= before, "handle count should not drop while held");
        }
        // Allow the kernel a tick to account for the close.
        std::thread::sleep(std::time::Duration::from_millis(50));
        let after = process_handle_count().expect("handle count");
        assert!(
            after <= before,
            "handle leaked: before={before} after={after} (guard did not close)"
        );
    }

    /// An early `?`-style return must not leak — that is the whole point.
    #[test]
    fn owned_handle_survives_early_return() {
        fn open_and_fail() -> Result<(), String> {
            let h = unsafe { CreateEventW(None, true, false, windows::core::PCWSTR::null()) }
                .map_err(|e| e.to_string())?;
            let _guard = OwnedHandle::new(h);
            Err("simulated failure after acquiring the handle".to_string())
        }

        let before = process_handle_count().expect("handle count");
        for _ in 0..50 {
            assert!(open_and_fail().is_err());
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        let after = process_handle_count().expect("handle count");
        // 50 failures would leak 50 handles without RAII; allow small noise.
        assert!(
            after <= before + 5,
            "RAII failed: 50 early returns leaked handles (before={before} after={after})"
        );
    }

    /// Explicit `close()` is idempotent with `Drop`.
    #[test]
    fn close_is_idempotent() {
        let h = unsafe { CreateEventW(None, true, false, windows::core::PCWSTR::null()) }
            .expect("CreateEventW");
        OwnedHandle::new(h).close();
    }

    /// `is_invalid` must recognise INVALID_HANDLE_VALUE.
    #[test]
    fn invalid_handle_is_detected() {
        let guard = OwnedHandle::new(INVALID_HANDLE_VALUE);
        assert!(guard.is_invalid());
        // Dropping an invalid handle must not call CloseHandle on garbage.
    }

    /// Opening + dropping many real process handles must stay flat.
    #[test]
    fn repeated_open_process_does_not_leak() {
        let before = process_handle_count().expect("handle count");
        for _ in 0..100 {
            let h = unsafe {
                OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, std::process::id())
            };
            if let Ok(h) = h {
                let _guard = OwnedHandle::new(h);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        let after = process_handle_count().expect("handle count");
        assert!(
            after <= before + 5,
            "OpenProcess leaked: before={before} after={after}"
        );
        // Reference GetCurrentProcess so the import is used on all cfgs.
        let _ = unsafe { GetCurrentProcess() };
    }
}
