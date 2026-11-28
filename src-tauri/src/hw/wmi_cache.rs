//! WMI Connection Cache — S4-001
//!
//! Caches WMI connections per-thread to eliminate per-query connection churn.
//! WMI connections are COM objects (thread-affine), so we use `thread_local!`
//! storage. All access must happen inside `spawn_blocking` (per S3-004).
//! Two namespaces are cached:
//! - `ROOT\CIMV2` — system info, display, discovery
//! - `ROOT\WMI` — battery, brightness

use crate::hw::errors::{HardwareError, HardwareResult};
use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicU64, Ordering};
use wmi::WMIConnection;

/// Namespace for system-level WMI queries (Win32_* classes).
pub const NS_CIMV2: &str = "ROOT\\CIMV2";
/// Namespace for WMI-level queries (BatteryStatus, WmiMonitorBrightness, etc.).
pub const NS_WMI: &str = "ROOT\\WMI";
/// Namespace for Windows Defender status (MSFT_MPComputerStatus).
pub const NS_DEFENDER: &str = "ROOT\\Microsoft\\Windows\\Defender";

thread_local! {
    /// Per-thread cached WMI connections.
    /// Each `WMIConnection` internally owns its `COMLibrary`, so COM stays
    /// initialised for the lifetime of the cache entry.
    static WMI_CACHE: RefCell<Option<WmiThreadCache>> = const { RefCell::new(None) };
    static WMI_CACHE_GENERATION: Cell<u64> = const { Cell::new(0) };
}

static WMI_INVALIDATION_GENERATION: AtomicU64 = AtomicU64::new(0);

/// What to do with cached COM pointers when a thread tears down.
///
/// Split out as a pure decision so the ordering rule is unit-testable
/// without a live WMI service or a second logon.
#[cfg(windows)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Teardown {
    /// COM is still initialised on this thread: the provider vtables are
    /// mapped, so `Release()` is safe and the connections must be released.
    Release,
    /// The apartment is already gone. The interface vtables live in the WMI
    /// provider DLLs (`fastprox.dll` / `wbemsvc.dll`), which COM unloads when
    /// the apartment dies. Calling `Release()` then reads a vtable slot inside
    /// unmapped memory. Leak the pointers instead: a leaked pointer is inert,
    /// a released one is an access violation.
    Leak,
}

/// Pure half of [`com_alive`]: the teardown decision for a given apartment
/// state. Kept separate so tests can pin the rule by mutation.
#[cfg(windows)]
fn teardown_action(com_alive: bool) -> Teardown {
    if com_alive {
        Teardown::Release
    } else {
        Teardown::Leak
    }
}

/// Is COM still initialised on the calling thread?
///
/// `CoGetApartmentType` answers `CO_E_NOTINITIALIZED` once the apartment has
/// been torn down. This matters because at thread exit Windows runs the DLL
/// thread-detach callbacks (where COM tears the apartment down and unloads the
/// provider libraries it no longer references) BEFORE the Rust `thread_local!`
/// destructors. A cached `IWbemServices` released from a TLS destructor
/// therefore calls into an unmapped vtable.
///
/// Live evidence (`C:\ProgramData\MI\IoTService\IoTService.exe.45864.dmp`,
/// 2026-11-27): exception `0xc0000005` reading `0x7ffd4452f2f0`, fault
/// instruction `call qword ptr [rax+0x10]` (vtable slot 2 = `Release`) at a
/// constant offset in `IoTService.exe`; `rax` = `0x7ffd4452f2e0` is inside NO
/// loaded module, and 279 further pointers in the dump fall in the dead band
/// `0x7ffd44480000..0x7ffd44575000` (~980 KB, the size of the WMI providers).
/// `fastprox`/`wbemsvc`/`IWbemServices` strings are present in the heap, so the
/// providers were loaded and then unloaded. 43 such crashes in 11 hours took
/// the whole `IoTSvc` service down with them (SCM 7034, `WIN32_EXIT_CODE 1067`).
#[cfg(windows)]
fn com_alive() -> bool {
    use windows::Win32::System::Com::{CoGetApartmentType, APTTYPE, APTTYPEQUALIFIER};

    let mut apt = APTTYPE::default();
    let mut qualifier = APTTYPEQUALIFIER::default();
    // SAFETY: both out-parameters are valid, initialised, writable locals.
    unsafe { CoGetApartmentType(&mut apt, &mut qualifier).is_ok() }
}

/// Per-thread COM initialisation for WMI.
///
/// P3: `COMLibrary::new()` (the `wmi` crate) calls
/// `CoInitializeEx(None, COINIT_MULTITHREADED)` and turns
/// `RPC_E_CHANGED_MODE` (0x80010106) into an error. At startup Tauri/WebView2
/// (and the tray/WinRT code) may already have put the calling thread in a
/// single-threaded apartment, so the first WMI query failed — 4 times in
/// 689 ms, once per `util::retry` attempt, each logging a WARN.
///
/// This module initialises COM exactly ONCE per thread and accepts either
/// outcome:
/// * MTA (`S_OK` / `S_FALSE`) — the model the rest of the hardware layer uses.
/// * STA (`RPC_E_CHANGED_MODE`) — the thread was already initialised by
///   another component; WMI works from an STA too, so we reuse it instead of
///   failing. Re-initialising is impossible by design (COM rejects changing
///   the apartment after it is set).
///
/// `WmiThreadCache::init` then creates all its `WMIConnection`s from this one
/// `COMLibrary` instead of calling `COMLibrary::new()` three times.
#[cfg(windows)]
mod com_init {
    use std::cell::Cell;
    use std::sync::OnceLock;

    use windows::Win32::Foundation::{RPC_E_CHANGED_MODE, RPC_E_TOO_LATE};
    use windows::Win32::System::Com::{
        CoInitializeEx, CoInitializeSecurity, COINIT_MULTITHREADED, EOAC_NONE,
        RPC_C_AUTHN_LEVEL_DEFAULT, RPC_C_IMP_LEVEL_IMPERSONATE,
    };
    use wmi::COMLibrary;

    /// Apartment model observed (or established) on the current thread.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum Apartment {
        Uninitialised,
        MultiThreaded,
        SingleThreaded,
    }

    thread_local! {
        static APARTMENT: Cell<Apartment> = const { Cell::new(Apartment::Uninitialised) };
    }

    // Counts real `CoInitializeEx` calls on THIS thread, so tests can prove
    // idempotency without racing other test threads.
    #[cfg(test)]
    thread_local! {
        static COM_INIT_CALLS: Cell<u32> = const { Cell::new(0) };
    }

    #[cfg(test)]
    pub(super) fn init_calls() -> u32 {
        COM_INIT_CALLS.with(|c| c.get())
    }

    /// Process-wide `CoInitializeSecurity` (first call wins; `RPC_E_TOO_LATE`
    /// means another component — WebView2, the `wmi` crate — set it already).
    fn init_security_once() {
        static SECURITY: OnceLock<()> = OnceLock::new();
        SECURITY.get_or_init(|| {
            // SAFETY: standard COM process initialisation; all pointer
            // arguments are NULL, which tells COM to use its defaults.
            let result = unsafe {
                CoInitializeSecurity(
                    None,
                    -1,
                    None,
                    None,
                    RPC_C_AUTHN_LEVEL_DEFAULT,
                    RPC_C_IMP_LEVEL_IMPERSONATE,
                    None,
                    EOAC_NONE,
                    None,
                )
            };
            if let Err(e) = result {
                if e.code() != RPC_E_TOO_LATE {
                    log::warn!(
                        "WMI cache: CoInitializeSecurity failed: 0x{:08X} \
                         (continuing with process defaults)",
                        e.code().0 as u32
                    );
                }
            }
        });
    }

    /// Initialise COM for the calling thread (idempotent) and return a
    /// `COMLibrary` marker bound to it.
    ///
    /// The returned `COMLibrary` is a `Copy` zero-sized marker: it performs no
    /// `CoUninitialize`, so the apartment stays valid for the thread's life —
    /// exactly what the per-thread WMI cache needs.
    pub(super) fn ensure_com() -> anyhow::Result<COMLibrary> {
        if APARTMENT.with(|state| state.get()) == Apartment::Uninitialised {
            #[cfg(test)]
            COM_INIT_CALLS.with(|c| c.set(c.get() + 1));

            // SAFETY: `CoInitializeEx` initialises COM for this thread; the
            // first caller decides the apartment model.
            let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
            if hr.is_ok() {
                APARTMENT.with(|state| state.set(Apartment::MultiThreaded));
                log::debug!("WMI cache: COM initialised (MTA) for this thread");
            } else if hr == RPC_E_CHANGED_MODE {
                // 0x80010106: another component already initialised this
                // thread (typically STA for Tauri/WebView2/WinRT). WMI works
                // in an STA, so reuse the existing apartment instead of
                // failing every query on this thread.
                APARTMENT.with(|state| state.set(Apartment::SingleThreaded));
                log::info!(
                    "WMI cache: thread already COM-initialised in STA \
                     (RPC_E_CHANGED_MODE) — reusing the existing apartment"
                );
            } else {
                return Err(anyhow::Error::from(
                    crate::hw::errors::HardwareError::WmiConnection(format!(
                        "CoInitializeEx failed: 0x{:08X}",
                        hr.0 as u32
                    )),
                ));
            }
        }
        // Process-wide and idempotent: the wmi crate used to do this on every
        // `COMLibrary::new()`. `RPC_E_TOO_LATE` means another component
        // (WebView2, the wmi crate) already set it — fine.
        init_security_once();

        // SAFETY: `ensure_com` has just verified that COM is initialised on
        // this thread (or found it already initialised), and it never calls
        // `CoUninitialize`.
        Ok(unsafe { COMLibrary::assume_initialized() })
    }

    /// Current apartment for the calling thread (test/diagnostic only).
    #[cfg(test)]
    pub(super) fn apartment() -> Apartment {
        APARTMENT.with(|state| state.get())
    }
}

#[cfg(windows)]
use com_init::ensure_com;

/// The three cached connections of one thread.
///
/// Every slot is an `Option` so the whole cache can be released by *taking*
/// the values, which is what makes the teardown order controllable: see the
/// `Drop` implementation below.
struct WmiThreadCache {
    cimv2: Option<WMIConnection>,
    wmi: Option<WMIConnection>,
    defender: Option<WMIConnection>,
}

#[cfg(windows)]
impl Drop for WmiThreadCache {
    /// Release the cached COM pointers, but never after the apartment died.
    ///
    /// Rust runs `thread_local!` destructors when the thread exits, which is
    /// *after* COM has torn the apartment down and unloaded the WMI provider
    /// libraries. Releasing then reads a vtable slot in unmapped memory and
    /// takes the whole process with it (43 crashes/11 h in `IoTSvc`, see
    /// [`com_alive`]). So the apartment state decides the teardown: release
    /// while COM lives, leak otherwise.
    fn drop(&mut self) {
        let connections = [self.cimv2.take(), self.wmi.take(), self.defender.take()];
        match teardown_action(com_alive()) {
            Teardown::Release => drop(connections),
            Teardown::Leak => {
                log::warn!(
                    "WMI cache: COM apartment already gone on this thread \
                     (thread teardown); leaking the cached connections instead \
                     of releasing them, because releasing would call into the \
                     unloaded WMI provider DLLs"
                );
                std::mem::forget(connections);
            }
        }
    }
}

impl WmiThreadCache {
    fn init() -> anyhow::Result<Self> {
        // P3: ONE COM init per thread, tolerant of an existing STA. The
        // resulting `COMLibrary` is a Copy marker and is reused for every
        // namespace connection (previously three separate `COMLibrary::new()`
        // calls, each able to fail with RPC_E_CHANGED_MODE).
        let com = ensure_com()?;
        let cimv2 = WMIConnection::with_namespace_path(NS_CIMV2, com).map_err(|e| {
            HardwareError::WmiQuery {
                query: format!("WMIConnection cimv2 namespace={NS_CIMV2}"),
                source: Box::new(e),
            }
        })?;
        let wmi = WMIConnection::with_namespace_path(NS_WMI, com).map_err(|e| {
            HardwareError::WmiQuery {
                query: format!("WMIConnection wmi namespace={NS_WMI}"),
                source: Box::new(e),
            }
        })?;
        // Defender namespace may not exist on systems without Defender — do not fail.
        let defender = WMIConnection::with_namespace_path(NS_DEFENDER, com).ok();
        Ok(Self {
            cimv2: Some(cimv2),
            wmi: Some(wmi),
            defender,
        })
    }
}

/// Drop this thread's cached WMI connections right now, while COM is still
/// alive on it.
///
/// Short-lived worker threads (the IoT pipe serves one request per thread) must
/// not leave the release to the TLS destructor: by then the apartment is gone
/// and the WMI provider DLLs are unmapped. Calling this at the end of the
/// worker body makes the teardown ordered and safe. Idempotent, and cheap when
/// the thread never touched WMI.
pub fn release_current_thread() {
    WMI_CACHE.with(|cell| {
        let taken = cell.borrow_mut().take();
        if let Some(cache) = taken {
            // Runs the Drop impl above, which sees COM still initialised.
            drop(cache);
        }
    });
}

/// Execute a closure with the cached `ROOT\CIMV2` connection, with one retry.
///
/// On first call (or after [`invalidate`]), initialises COM and creates the
/// connection. If the closure returns an error, the cache is ONLY invalidated
/// for connection-level errors (COM init failure, namespace binding failure),
/// NOT for transient query errors.
pub fn with_cimv2<F, T>(f: F) -> HardwareResult<T>
where
    F: Fn(&WMIConnection) -> anyhow::Result<T>,
{
    let result: anyhow::Result<T> = crate::util::retry::with_retry("WMI cimv2 query", || {
        reset_if_invalidated();
        WMI_CACHE.with(|cell| {
            let mut cache_ref = cell.borrow_mut();
            if cache_ref.is_none() {
                match WmiThreadCache::init() {
                    Ok(cache) => *cache_ref = Some(cache),
                    Err(e) => {
                        log::warn!("WMI cache: cimv2 cache initialization failed: {e}");
                        return Err(e);
                    }
                }
            }
            match cache_ref.as_ref().and_then(|c| c.cimv2.as_ref()) {
                Some(conn) => f(conn),
                None => {
                    log::error!("WMI cache: cimv2 cache unavailable after init");
                    Err(anyhow::anyhow!(HardwareError::WmiConnection(
                        "cimv2 cache unavailable after initialization".to_string(),
                    )))
                }
            }
        })
    });
    match &result {
        Err(e) if is_connection_error(e) => {
            log::info!("WMI cache: cimv2 connection error, invalidating: {e}");
            WMI_CACHE.with(|cell| {
                *cell.borrow_mut() = None;
            });
        }
        Err(e) => {
            log::debug!("WMI cache: cimv2 transient query error (cache preserved): {e}");
        }
        Ok(_) => {}
    }
    result.map_err(HardwareError::from)
}

/// Execute a closure with the cached `ROOT\WMI` connection, with one retry.
///
/// Same semantics as [`with_cimv2`].
///
/// WORKING FORM — DO NOT MODIFY: This function uses `cell.borrow_mut()` which
/// holds a RefCell borrow for the entire duration of the closure. If any code
/// inside the closure calls `with_wmi()` or `with_cimv2()` again (nested call),
/// the second `borrow_mut()` will panic with "RefCell already borrowed".
///
/// Callers MUST NOT nest WMI calls. If you need data from multiple WMI sources,
/// collect all data from one source into a struct, exit the closure, then call
/// the second source separately. See `battery.rs::get_battery_info()` for the
/// correct pattern (BatterySnapshot struct).
pub fn with_wmi<F, T>(f: F) -> HardwareResult<T>
where
    F: Fn(&WMIConnection) -> anyhow::Result<T>,
{
    let result: anyhow::Result<T> = crate::util::retry::with_retry("WMI wmi query", || {
        reset_if_invalidated();
        WMI_CACHE.with(|cell| {
            let mut cache_ref = cell.borrow_mut();
            if cache_ref.is_none() {
                match WmiThreadCache::init() {
                    Ok(cache) => *cache_ref = Some(cache),
                    Err(e) => {
                        log::warn!("WMI cache: wmi cache initialization failed: {e}");
                        return Err(e);
                    }
                }
            }
            match cache_ref.as_ref().and_then(|c| c.wmi.as_ref()) {
                Some(conn) => f(conn),
                None => {
                    log::error!("WMI cache: wmi cache unavailable after init");
                    Err(anyhow::anyhow!(HardwareError::WmiConnection(
                        "wmi cache unavailable after initialization".to_string(),
                    )))
                }
            }
        })
    });
    match &result {
        Err(e) if is_connection_error(e) => {
            log::info!("WMI cache: wmi connection error, invalidating: {e}");
            WMI_CACHE.with(|cell| {
                *cell.borrow_mut() = None;
            });
        }
        Err(e) => {
            // 0x80041002 from wmi_ec.rs is expected intermittently.
            // Log at trace level to avoid log spam.
            let is_expected = e
                .downcast_ref::<windows::core::Error>()
                .map(|win_err| win_err.code().0 as u32 == 0x80041002)
                .unwrap_or(false);
            if is_expected {
                log::trace!("WMI cache: wmi transient query error (cache preserved): {e}");
            } else {
                log::debug!("WMI cache: wmi transient query error (cache preserved): {e}");
            }
        }
        Ok(_) => {}
    }
    result.map_err(HardwareError::from)
}

/// Execute a closure with the cached `ROOT\Microsoft\Windows\Defender`
/// connection (MSFT_MPComputerStatus).
///
/// Returns `HardwareError::WmiConnection` when the Defender WMI namespace is
/// unavailable (e.g. Defender replaced by a third-party AV). Callers should
/// fall back to the registry-based status in that case.
pub fn with_defender<F, T>(f: F) -> HardwareResult<T>
where
    F: Fn(&WMIConnection) -> anyhow::Result<T>,
{
    let result: anyhow::Result<T> = WMI_CACHE.with(|cell| {
        let mut cache_ref = cell.borrow_mut();
        if cache_ref.is_none() {
            match WmiThreadCache::init() {
                Ok(cache) => *cache_ref = Some(cache),
                Err(e) => {
                    log::warn!("WMI cache: cache initialization failed: {e}");
                    return Err(e);
                }
            }
        }
        match cache_ref.as_ref().and_then(|c| c.defender.as_ref()) {
            Some(conn) => f(conn),
            None => {
                log::debug!("WMI cache: Defender namespace unavailable");
                Err(anyhow::anyhow!(HardwareError::WmiConnection(
                    "Defender WMI namespace not available".to_string(),
                )))
            }
        }
    });
    result.map_err(HardwareError::from)
}

/// `WBEM_E_ACCESS_DENIED` (0x80041003): the class exists but the caller lacks
/// the privilege. Permanent by design for the ESIF/ACPI thermal classes when
/// queried from the unprivileged UI process — such failures must be logged
/// once at INFO and retried on a slow schedule, never treated as a bug/new
/// WARN on every poll (P11).
pub const WBEM_E_ACCESS_DENIED: u32 = 0x80041003;

/// HRESULT carried by a `wmi::WMIError`, when it has one.
pub fn wmi_error_hresult(err: &wmi::WMIError) -> Option<u32> {
    match err {
        wmi::WMIError::HResultError { hres } => Some(*hres as u32),
        _ => None,
    }
}

/// True when the query failed with `WBEM_E_ACCESS_DENIED`.
pub fn is_access_denied(err: &wmi::WMIError) -> bool {
    wmi_error_hresult(err) == Some(WBEM_E_ACCESS_DENIED)
}

/// Grow a failure retry window (doubling, clamped).
///
/// Used by the thermal sources whose WMI classes deny unprivileged callers:
/// retrying every 60 s forever produced ~2 predictable WARN/min of noise, so
/// the window grows 60 → 120 → 240 → 480 → 900 s (cap) and resets on success.
pub fn grow_failure_backoff(current_secs: u64, start_secs: u64, max_secs: u64) -> u64 {
    current_secs.saturating_mul(2).clamp(start_secs, max_secs)
}

/// Returns `true` if the error indicates a connection-level failure
/// (COM init, namespace binding, or WMI infrastructure) vs. a permanent
/// or transient query error (class not found, access denied, invalid query).
///
/// Classifies by HRESULT code, NOT by error message strings (which are
/// localized on non-English Windows and would break silently).
///
/// ## Transient / connection-level HRESULTs (invalidate cache + retry)
/// These indicate the WMI infrastructure itself is unavailable, and a
/// COM re-init + reconnect may help:
/// - `0x80041015` WBEM_E_TRANSPORT_FAILURE
/// - `0x8004101D` WBEM_E_UNEXPECTED
/// - `0x80041033` WBEM_E_SHUTTING_DOWN
/// - `0x80041045` WBEM_E_SERVER_TOO_BUSY
/// - `0x80041069` WBEM_E_TIMED_OUT
/// - `0x800706xx` RPC/DCOM family (server unavailable, etc.)
///
/// ## Permanent HRESULTs (do NOT invalidate, do NOT retry)
/// These are deterministic — retrying the identical query will always fail:
/// - `0x80041003` WBEM_E_ACCESS_DENIED
/// - `0x8004100E` WBEM_E_INVALID_NAMESPACE
/// - `0x80041010` WBEM_E_INVALID_CLASS
/// - `0x80041017` WBEM_E_INVALID_QUERY
/// - `0x80041002` WBEM_E_NOT_FOUND
///
/// Unknown HRESULTs default to **transient** (conservative) so genuine
/// transient failures still recover.
fn is_connection_error(e: &anyhow::Error) -> bool {
    // First, try downcasting to our structured HardwareError
    if let Some(hw_err) = e.downcast_ref::<HardwareError>() {
        return matches!(hw_err, HardwareError::WmiConnection(_));
    }

    // Check for WMIError with HRESULT — classify by specific code
    if let Some(wmi_err) = e.downcast_ref::<wmi::WMIError>() {
        return match wmi_err {
            wmi::WMIError::HResultError { hres } => {
                let hres_u32 = *hres as u32;

                // Check if it's in the PERMANENT set — these should NOT
                // invalidate the cache or trigger retries.
                let is_permanent = matches!(
                    hres_u32,
                    0x80041003 | 0x8004100E | 0x80041010 | 0x80041017 | 0x80041002
                );

                if is_permanent {
                    log::debug!(
                        "WMI HRESULT 0x{:08X} classified as permanent (NOT a connection error)",
                        hres_u32
                    );
                    false
                } else {
                    // Unknown HRESULTs default to transient (conservative)
                    log::debug!(
                        "WMI HRESULT 0x{:08X} classified as transient (connection error)",
                        hres_u32
                    );
                    true
                }
            }
            // Other WMIError variants (connection issues, COM init, etc.)
            // are treated as connection errors
            _ => true,
        };
    }

    // Check for windows::core::Error (from direct COM calls in wmi_ec.rs, hq_wmi.rs)
    // These come from GetObject/ExecQuery/ExecMethod calls that use the `?`
    // operator to convert windows::core::Error into anyhow::Error.
    #[cfg(windows)]
    if let Some(win_err) = e.downcast_ref::<windows::core::Error>() {
        let hres_u32 = win_err.code().0 as u32;
        let is_permanent = matches!(
            hres_u32,
            0x80041003 | 0x8004100E | 0x80041010 | 0x80041017 | 0x80041002
        );
        if is_permanent {
            // 0x80041002 (WBEM_E_NOT_FOUND) is expected intermittently from
            // the MICommonInterface WMI provider when it's in a degraded state.
            // Log at trace level to avoid log spam.
            log::trace!(
                "COM HRESULT 0x{:08X} classified as permanent (NOT a connection error)",
                hres_u32
            );
            return false;
        }
        // Unknown COM HRESULTs default to transient (conservative)
        log::debug!(
            "COM HRESULT 0x{:08X} classified as transient (connection error)",
            hres_u32
        );
        return true;
    }

    // Fallback: check the error chain for connection-related causes using
    // structured iteration rather than blind substring matching
    let mut source: Option<&dyn std::error::Error> = Some(e.as_ref());
    while let Some(err) = source {
        let type_name = std::any::type_name_of_val(err);
        if type_name.contains("WMIError")
            || type_name.contains("COMError")
            || type_name.contains("HResult")
        {
            return true;
        }
        source = err.source();
    }

    false
}

#[allow(dead_code)]
/// Invalidate the cached connections on the current thread.
///
/// The next call to [`with_cimv2`] or [`with_wmi`] will recreate them.
/// Call this from error-recovery paths when a WMI query fails with a
/// connection-level error.
pub fn invalidate() {
    WMI_INVALIDATION_GENERATION.fetch_add(1, Ordering::AcqRel);
    reset_if_invalidated();
}

/// Drop the current thread's COM connections when another thread invalidated
/// the cache, such as the power listener after resume from sleep.
fn reset_if_invalidated() {
    let generation = WMI_INVALIDATION_GENERATION.load(Ordering::Acquire);
    WMI_CACHE_GENERATION.with(|local_generation| {
        if local_generation.get() == generation {
            return;
        }
        WMI_CACHE.with(|cell| {
            *cell.borrow_mut() = None;
        });
        local_generation.set(generation);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_connection_error_hardware_error_wmi_connection() {
        let err: anyhow::Error =
            anyhow::Error::from(HardwareError::WmiConnection("COM init failed".to_string()));
        assert!(is_connection_error(&err));
    }

    #[test]
    fn test_is_connection_error_hardware_error_wmi_query_is_not_connection() {
        // WmiQuery is a transient query error, NOT a connection error
        let err: anyhow::Error = anyhow::Error::from(HardwareError::WmiQuery {
            query: "SELECT * FROM Win32_Processor".to_string(),
            source: Box::new(std::io::Error::other("query failed")),
        });
        assert!(!is_connection_error(&err));
    }

    #[test]
    fn test_is_connection_error_wmi_error_transient_hresult() {
        // A wmi::WMIError with a transient HRESULT (transport failure)
        // should be detected as a connection error
        let wmi_err = wmi::WMIError::HResultError {
            hres: 0x80041015u32 as i32, // WBEM_E_TRANSPORT_FAILURE
        };
        let err: anyhow::Error = anyhow::Error::from(wmi_err);
        assert!(is_connection_error(&err));
    }

    #[test]
    fn test_is_connection_error_wmi_error_access_denied_is_not_connection() {
        // WBEM_E_ACCESS_DENIED (0x80041003) is a PERMANENT error, NOT a
        // connection error — retrying the same query will always fail.
        let wmi_err = wmi::WMIError::HResultError {
            hres: 0x80041003u32 as i32, // WBEM_E_ACCESS_DENIED
        };
        let err: anyhow::Error = anyhow::Error::from(wmi_err);
        assert!(!is_connection_error(&err));
    }

    #[test]
    fn test_is_connection_error_wmi_error_invalid_class_is_not_connection() {
        // WBEM_E_INVALID_CLASS (0x80041010) is a permanent query error
        let wmi_err = wmi::WMIError::HResultError {
            hres: 0x80041010u32 as i32, // WBEM_E_INVALID_CLASS
        };
        let err: anyhow::Error = anyhow::Error::from(wmi_err);
        assert!(!is_connection_error(&err));
    }

    #[test]
    fn test_is_connection_error_wmi_error_unknown_hresult_defaults_transient() {
        // Unknown HRESULTs should default to transient (conservative)
        let wmi_err = wmi::WMIError::HResultError {
            hres: 0x80040154u32 as i32, // REGDB_E_CLASSNOTREG — not in permanent list
        };
        let err: anyhow::Error = anyhow::Error::from(wmi_err);
        assert!(is_connection_error(&err));
    }

    #[test]
    fn test_is_connection_error_generic_string_is_not_connection() {
        // A generic string error should NOT be detected as a connection error
        let err: anyhow::Error = anyhow::Error::msg("some random transient error");
        assert!(!is_connection_error(&err));
    }

    #[test]
    fn test_is_connection_error_io_error_is_not_connection() {
        // A plain IO error should NOT be detected as a connection error
        let err: anyhow::Error = anyhow::Error::from(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "file not found",
        ));
        assert!(!is_connection_error(&err));
    }

    // ── S19-07: Cache lifecycle and thread-local tests ───────────────────────

    // ── IoTSvc crash regression: COM teardown ordering at thread exit ──────
    //
    // `IoTSvc` died 43 times in 11 hours with `0xc0000005` inside a
    // `thread_local!` destructor: at thread exit COM tears the apartment down
    // and unloads the WMI providers BEFORE Rust runs the TLS destructors, so
    // releasing a cached `IWbemServices` calls a vtable slot in unmapped
    // memory. These tests pin the rule that keeps that from coming back.

    /// The teardown rule is decided by the apartment state and by nothing else.
    #[cfg(windows)]
    #[test]
    fn teardown_releases_only_while_com_is_alive() {
        assert_eq!(teardown_action(true), Teardown::Release);
        assert_eq!(teardown_action(false), Teardown::Leak);
        assert_ne!(
            teardown_action(false),
            teardown_action(true),
            "the two apartment states must not produce the same teardown"
        );
    }

    /// A thread that initialised COM must report a live apartment, so its cached
    /// connections are released normally instead of leaked.
    ///
    /// The state BEFORE `ensure_com` is deliberately not asserted: in a process
    /// that already used COM (the full suite does), a brand-new thread can be
    /// handed an implicit MTA (`CoGetApartmentType` -> `APTTYPE_MNA`), and
    /// `com_alive` reporting "live" is the conservative direction: it only ever
    /// enables a `Release` that COM itself supports. The dangerous direction
    /// ("live" while the apartment is really gone) is what the guard removes, and
    /// the rule itself is pinned by `teardown_releases_only_while_com_is_alive`.
    #[cfg(windows)]
    #[test]
    fn com_alive_reports_live_after_ensure_com() {
        std::thread::spawn(|| {
            let _implicit_apartment_before_init = com_alive();
            ensure_com().expect("COM init on the test thread");
            assert!(
                com_alive(),
                "after ensure_com the apartment is live, so cached connections \
                 may be released normally"
            );
        })
        .join()
        .expect("COM-alive test thread panicked");
    }

    /// `release_current_thread` empties the cache and is idempotent, so a worker
    /// thread that never touched WMI pays nothing for calling it.
    #[cfg(windows)]
    #[test]
    fn release_current_thread_empties_the_cache_idempotently() {
        std::thread::spawn(|| {
            release_current_thread();
            WMI_CACHE.with(|cell| assert!(cell.borrow().is_none()));
            release_current_thread();
            release_current_thread();
            WMI_CACHE.with(|cell| assert!(cell.borrow().is_none()));
        })
        .join()
        .expect("release_current_thread test thread panicked");
    }

    /// The scenario, run in a FRESH process by the test below: one worker thread
    /// queries WMI and then exits, optionally releasing its cache first.
    ///
    /// Isolation is load-bearing. Inside a shared test process the WMI provider
    /// DLLs stay mapped because other threads hold their own connections, so the
    /// unload that killed `IoTSvc` cannot happen and the loop would never go red.
    /// In a process whose only WMI user is this one thread, the provider refcount
    /// drops to zero at thread exit, exactly as in the pipe service.
    #[cfg(windows)]
    #[test]
    #[ignore = "helper scenario: only runs as the child of the fresh-process test"]
    fn wmi_worker_tls_teardown_scenario() {
        let release_explicitly = std::env::var("MICONTROL_WMI_TEST_RELEASE").as_deref() == Ok("1");
        std::thread::spawn(move || {
            let rows: Vec<std::collections::HashMap<String, wmi::Variant>> =
                with_cimv2(|conn| Ok(conn.raw_query("SELECT Caption FROM Win32_OperatingSystem")?))
                    .expect("WMI query on a short-lived worker thread must succeed");
            assert!(!rows.is_empty(), "Win32_OperatingSystem returned no rows");
            if release_explicitly {
                // What the pipe service now does at the end of every request
                // thread: release while COM is still alive.
                release_current_thread();
            }
            // Otherwise the thread simply returns: COM tears the apartment down
            // and unloads the providers, then Rust runs the `thread_local!`
            // destructor, which must leak instead of releasing into an unmapped
            // vtable.
        })
        .join()
        .expect("the worker thread must not panic");
        println!("scenario survived (explicit_release={release_explicitly})");
    }

    /// Red-capable regression loop for the `IoTSvc` crash: the exact sequence
    /// (one WMI-using worker thread per pipe request) in a fresh process, for
    /// both teardown layers.
    ///
    /// With the `com_alive` guard removed the child dies with `0xc0000005` and
    /// this test fails; with the fix both children must exit clean. Run it with
    /// `cargo test --lib -- --ignored worker_threads_that_used_wmi_survive_teardown_in_a_fresh_process`.
    #[cfg(windows)]
    #[test]
    #[ignore = "spawns helper processes and needs a live WMI service"]
    fn worker_threads_that_used_wmi_survive_teardown_in_a_fresh_process() {
        use crate::util::process_spawn;
        use std::process::Command;
        use std::time::Duration;

        let exe = std::env::current_exe().expect("the test binary must be runnable");
        for release in ["0", "1"] {
            let mut cmd = Command::new(&exe);
            cmd.args([
                "--exact",
                "hw::wmi_cache::tests::wmi_worker_tls_teardown_scenario",
                "--ignored",
                "--test-threads=1",
                "--nocapture",
            ]);
            cmd.env("MICONTROL_WMI_TEST_RELEASE", release);
            let out = process_spawn::run_capture(&mut cmd, Duration::from_secs(120))
                .expect("the helper process must start and finish within its budget");
            assert!(
                out.success(),
                "child process died (release={release}, exit={:?}): {}",
                out.status.code(),
                out.stdout_str()
            );
        }
    }

    #[test]
    fn test_invalidate_clears_thread_local_cache() {
        // invalidate() should set the thread_local cache to None
        invalidate();
        WMI_CACHE.with(|cell| {
            assert!(
                cell.borrow().is_none(),
                "Cache should be None after invalidate()"
            );
        });
    }

    #[test]
    fn test_invalidate_is_idempotent() {
        // Calling invalidate() multiple times should not panic
        invalidate();
        invalidate();
        invalidate();
        WMI_CACHE.with(|cell| {
            assert!(cell.borrow().is_none());
        });
    }

    #[test]
    fn test_thread_local_cache_isolation() {
        // Invalidate on the main thread
        invalidate();

        // Spawn a thread that also invalidates its own cache
        let handle = std::thread::spawn(|| {
            invalidate();
            WMI_CACHE.with(|cell| {
                assert!(
                    cell.borrow().is_none(),
                    "Cache should be None on spawned thread"
                );
            });
        });
        handle.join().unwrap();

        // The main thread's cache should still be None (unchanged by the other thread)
        WMI_CACHE.with(|cell| {
            assert!(
                cell.borrow().is_none(),
                "Main thread cache should be unaffected"
            );
        });
    }

    #[cfg(windows)]
    #[test]
    fn test_with_wmi_transient_error_preserves_cache() {
        // First, try to initialize the cache with a successful query
        let init_result = with_wmi(|_conn| Ok::<(), anyhow::Error>(()));
        if init_result.is_err() {
            eprintln!("WMI not available in this environment, skipping test");
            return;
        }

        // Verify cache is initialized
        WMI_CACHE.with(|cell| {
            assert!(
                cell.borrow().is_some(),
                "Cache should be initialized after success"
            );
        });

        // Run a query that returns a transient error
        let result =
            with_wmi(|_conn| Err::<(), anyhow::Error>(anyhow::anyhow!("transient query error")));
        assert!(result.is_err(), "Transient error should be returned");

        // The cache should still be initialized (not invalidated for transient errors)
        WMI_CACHE.with(|cell| {
            assert!(
                cell.borrow().is_some(),
                "Cache should be preserved after transient error"
            );
        });
    }

    #[cfg(windows)]
    #[test]
    fn test_with_wmi_connection_error_invalidates_cache() {
        // First, try to initialize the cache
        let init_result = with_wmi(|_conn| Ok::<(), anyhow::Error>(()));
        if init_result.is_err() {
            eprintln!("WMI not available in this environment, skipping test");
            return;
        }

        // Run a query that returns a connection-level error
        let result = with_wmi(|_conn| {
            Err::<(), anyhow::Error>(anyhow::Error::from(HardwareError::WmiConnection(
                "forced connection error".to_string(),
            )))
        });
        assert!(result.is_err(), "Connection error should be returned");

        // The cache should be invalidated after a connection error
        WMI_CACHE.with(|cell| {
            assert!(
                cell.borrow().is_none(),
                "Cache should be invalidated after connection error"
            );
        });
    }

    // ── P3: COM apartment handling ──────────────────────────────────────────

    /// A thread that already has COM in MTA must not pay a second
    /// `CoInitializeEx` call: the guard is per-thread and idempotent.
    #[cfg(windows)]
    #[test]
    fn ensure_com_is_idempotent_per_thread() {
        std::thread::spawn(|| {
            assert_eq!(com_init::init_calls(), 0);
            assert!(ensure_com().is_ok(), "first COM init must succeed");
            assert_eq!(
                com_init::init_calls(),
                1,
                "first call must invoke CoInitializeEx exactly once"
            );
            assert!(ensure_com().is_ok(), "second COM init must succeed");
            assert_eq!(
                com_init::init_calls(),
                1,
                "second call must NOT invoke CoInitializeEx again"
            );
            assert_eq!(
                com_init::apartment(),
                com_init::Apartment::MultiThreaded,
                "a fresh thread gets the MTA the rest of the hardware layer uses"
            );
        })
        .join()
        .expect("COM init test thread panicked");
    }

    /// P3 regression: the startup thread was already STA (Tauri/WebView2), so
    /// `COMLibrary::new()` failed with RPC_E_CHANGED_MODE four times in a row.
    /// `ensure_com` must accept the existing apartment instead of failing.
    #[cfg(windows)]
    #[test]
    fn ensure_com_reuses_an_existing_sta_apartment() {
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};

        std::thread::spawn(|| {
            // SAFETY: this thread is dedicated to the test; COM is initialised
            // in STA before `ensure_com` runs.
            let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            assert!(hr.is_ok(), "pre-initialising the test thread as STA failed");

            let com = ensure_com();
            assert!(
                com.is_ok(),
                "ensure_com must not fail with RPC_E_CHANGED_MODE on an STA thread: {:?}",
                com.err()
            );
            assert_eq!(
                com_init::apartment(),
                com_init::Apartment::SingleThreaded,
                "the existing STA apartment must be detected, not replaced"
            );
        })
        .join()
        .expect("STA reuse test thread panicked");
    }

    /// `ensure_com` must keep returning the same working marker on repeated
    /// calls in the same thread (callers invoke it for every namespace).
    #[cfg(windows)]
    #[test]
    fn ensure_com_returns_usable_com_library_twice() {
        std::thread::spawn(|| {
            let a = ensure_com().expect("first ensure_com");
            let b = ensure_com().expect("second ensure_com");
            // COMLibrary is a Copy marker; both values refer to the same
            // thread apartment and can be passed to WMIConnection.
            let _ = (a, b);
        })
        .join()
        .expect("ensure_com test thread panicked");
    }

    /// P3 end-to-end check of the fix: a thread that was already STA (the
    /// startup case) must be able to run a real WMI query through the cache.
    /// Ignored by default (needs a live WMI service) — run explicitly with
    /// `cargo test --lib -- --ignored sta_thread_can_query_wmi`.
    #[cfg(windows)]
    #[test]
    #[ignore = "requires a live WMI service (Windows host)"]
    fn sta_thread_can_query_wmi() {
        use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED};

        std::thread::spawn(|| {
            // SAFETY: dedicated test thread; STA before the cache initialises.
            let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
            assert!(hr.is_ok(), "pre-initialising the thread as STA failed");
            assert!(ensure_com().is_ok(), "ensure_com must reuse the STA");

            let rows = with_cimv2(|conn| {
                let rows: Vec<std::collections::HashMap<String, wmi::Variant>> =
                    conn.raw_query("SELECT Caption FROM Win32_OperatingSystem")?;
                Ok(rows)
            })
            .expect("WMI query on an STA thread must succeed");
            assert!(!rows.is_empty(), "Win32_OperatingSystem returned no rows");
        })
        .join()
        .expect("STA WMI test thread panicked");
    }

    // ── P11: expected-ACCESS_DENIED classification and slow retry ──────────

    #[test]
    fn access_denied_classifies_only_0x80041003() {
        let denied = wmi::WMIError::HResultError {
            hres: 0x80041003u32 as i32,
        };
        assert!(is_access_denied(&denied));
        assert_eq!(wmi_error_hresult(&denied), Some(WBEM_E_ACCESS_DENIED));

        let not_found = wmi::WMIError::HResultError {
            hres: 0x80041002u32 as i32,
        };
        assert!(!is_access_denied(&not_found));

        let other = wmi::WMIError::SerdeError("not an HRESULT".into());
        assert!(!is_access_denied(&other));
        assert_eq!(wmi_error_hresult(&other), None);
    }

    #[test]
    fn failure_backoff_doubles_and_caps() {
        assert_eq!(grow_failure_backoff(60, 60, 900), 120);
        assert_eq!(grow_failure_backoff(120, 60, 900), 240);
        assert_eq!(grow_failure_backoff(240, 60, 900), 480);
        assert_eq!(grow_failure_backoff(480, 60, 900), 900);
        assert_eq!(grow_failure_backoff(900, 60, 900), 900, "capped");
        assert_eq!(
            grow_failure_backoff(10, 60, 900),
            60,
            "never below the start window"
        );
    }
}
