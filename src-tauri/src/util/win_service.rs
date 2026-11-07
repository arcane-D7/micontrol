//! Minimal Windows Service Control Manager (SCM) helpers.
//!
//! Why this exists (audit 2026-11-07, P1): the app decided whether the
//! autonomous `MiControlBridge` service was available by opening its named
//! pipe (`CreateFileW`). When the bridge's accept loop wedged (C-01), the pipe
//! stayed permanently busy while the service itself was `RUNNING`, so the app
//! concluded "service not available" 597 times and fired 702 pointless
//! `install_bridge_service` calls (all no-ops). Service availability must be
//! answered by the SCM — the component that actually knows — and the pipe is
//! only evidence about the *data path*, not about whether the service exists.
//!
//! Everything here is read/start-only. Nothing stops, deletes or reconfigures
//! a service: those actions belong to the elevated installer path.

/// Coarse SCM state, collapsed to what the callers actually need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceState {
    /// `SERVICE_RUNNING`.
    Running,
    /// `SERVICE_STOPPED`.
    Stopped,
    /// `START_PENDING` / `STOP_PENDING` / `CONTINUE_PENDING` / `PAUSE_PENDING`.
    Pending,
    /// `SERVICE_PAUSED`.
    Paused,
    /// `OpenServiceW` returned `ERROR_SERVICE_DOES_NOT_EXIST` (1060).
    Absent,
    /// Could not determine (SCM unreachable, access denied, query failed).
    Unknown,
}

/// Map a raw `SERVICE_STATUS.dwCurrentState` value to [`ServiceState`].
///
/// Kept as a free function over `u32` (instead of taking the Windows newtype)
/// so the mapping is pure and unit-testable, and so non-Windows builds share
/// the same logic for tests.
pub fn classify_state(raw: u32) -> ServiceState {
    // Values from winsvc.h / SERVICE_STATUS.dwCurrentState.
    const SERVICE_STOPPED: u32 = 1;
    const SERVICE_START_PENDING: u32 = 2;
    const SERVICE_STOP_PENDING: u32 = 3;
    const SERVICE_RUNNING: u32 = 4;
    const SERVICE_CONTINUE_PENDING: u32 = 5;
    const SERVICE_PAUSE_PENDING: u32 = 6;
    const SERVICE_PAUSED: u32 = 7;

    match raw {
        SERVICE_STOPPED => ServiceState::Stopped,
        SERVICE_START_PENDING
        | SERVICE_STOP_PENDING
        | SERVICE_CONTINUE_PENDING
        | SERVICE_PAUSE_PENDING => ServiceState::Pending,
        SERVICE_RUNNING => ServiceState::Running,
        SERVICE_PAUSED => ServiceState::Paused,
        _ => ServiceState::Unknown,
    }
}

/// `ERROR_SERVICE_DOES_NOT_EXIST` as an HRESULT (0x80070424).
const HRESULT_SERVICE_DOES_NOT_EXIST: u32 = 0x8007_0424;
/// `ERROR_SERVICE_ALREADY_RUNNING` as an HRESULT (0x80070420).
const HRESULT_SERVICE_ALREADY_RUNNING: u32 = 0x8007_0420;

/// Query the SCM for the current state of `name` (e.g. `"MiControlBridge"`).
///
/// Never elevates: `SC_MANAGER_CONNECT` + `SERVICE_QUERY_STATUS` are granted to
/// standard users by the default service DACL. A failure is reported as
/// [`ServiceState::Unknown`] rather than as "absent" — treating "I could not
/// ask" as "it does not exist" is what would re-introduce install churn.
#[cfg(windows)]
pub fn query_service_state(name: &str) -> ServiceState {
    use windows::core::PCWSTR;
    use windows::Win32::System::Services::{
        CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceStatus, SC_MANAGER_CONNECT,
        SERVICE_QUERY_STATUS, SERVICE_STATUS,
    };

    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();

    // SAFETY: `wide` is a valid NUL-terminated UTF-16 string; handles returned
    // by the SCM are checked and closed before returning.
    unsafe {
        let scm = match OpenSCManagerW(None, None, SC_MANAGER_CONNECT) {
            Ok(handle) => handle,
            Err(_) => return ServiceState::Unknown,
        };

        let service = OpenServiceW(scm, PCWSTR(wide.as_ptr()), SERVICE_QUERY_STATUS);
        let _ = CloseServiceHandle(scm);

        match service {
            Ok(handle) => {
                let mut status = SERVICE_STATUS::default();
                let queried = QueryServiceStatus(handle, &mut status);
                let _ = CloseServiceHandle(handle);
                if queried.is_err() {
                    return ServiceState::Unknown;
                }
                classify_state(status.dwCurrentState.0)
            }
            Err(e) => {
                if e.code().0 as u32 == HRESULT_SERVICE_DOES_NOT_EXIST {
                    ServiceState::Absent
                } else {
                    ServiceState::Unknown
                }
            }
        }
    }
}

#[cfg(not(windows))]
pub fn query_service_state(_name: &str) -> ServiceState {
    ServiceState::Unknown
}

/// Ask the SCM to start `name`, then wait (bounded, ~5 s) for `RUNNING`.
///
/// `Ok(())` means the service is running. `Err` carries the reason, which the
/// caller must treat as "fall back to the elevated path" — a standard user may
/// legitimately lack `SERVICE_START` for a service installed with a restrictive
/// DACL, and that is not an error worth retrying in a loop.
#[cfg(windows)]
pub fn start_service(name: &str) -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Services::{
        CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceStatus, StartServiceW,
        SC_MANAGER_CONNECT, SERVICE_QUERY_STATUS, SERVICE_START, SERVICE_STATUS,
    };

    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();

    // SAFETY: valid NUL-terminated name; every handle is closed on each path.
    unsafe {
        let scm = OpenSCManagerW(None, None, SC_MANAGER_CONNECT)
            .map_err(|e| format!("OpenSCManagerW: {e}"))?;
        let service = match OpenServiceW(
            scm,
            PCWSTR(wide.as_ptr()),
            SERVICE_START | SERVICE_QUERY_STATUS,
        ) {
            Ok(handle) => handle,
            Err(e) => {
                let _ = CloseServiceHandle(scm);
                return Err(format!("OpenServiceW({name}): {e}"));
            }
        };

        let mut status = SERVICE_STATUS::default();
        let query_before = QueryServiceStatus(service, &mut status);
        let state_before = if query_before.is_ok() {
            classify_state(status.dwCurrentState.0)
        } else {
            ServiceState::Unknown
        };

        match state_before {
            ServiceState::Running => {
                let _ = CloseServiceHandle(service);
                let _ = CloseServiceHandle(scm);
                return Ok(());
            }
            // Already starting: do NOT issue StartServiceW (and never let the
            // caller conclude "stopped" and reinstall) — just wait for RUNNING.
            ServiceState::Pending => {}
            _ => match StartServiceW(service, None) {
                Ok(()) => {}
                // Another caller (the SCM, a failure action, the user) started
                // it first — that is success for our purpose.
                Err(e) if e.code().0 as u32 == HRESULT_SERVICE_ALREADY_RUNNING => {}
                Err(e) => {
                    let _ = CloseServiceHandle(service);
                    let _ = CloseServiceHandle(scm);
                    return Err(format!("StartServiceW({name}): {e}"));
                }
            },
        }

        // Bounded wait: 20 × 250 ms = 5 s. A cold boot can be slow, but the
        // caller runs this on the elevated health path, not in the UI thread.
        let mut last_state = ServiceState::Pending;
        for _ in 0..20 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let mut current = SERVICE_STATUS::default();
            if QueryServiceStatus(service, &mut current).is_ok() {
                last_state = classify_state(current.dwCurrentState.0);
                if last_state == ServiceState::Running {
                    let _ = CloseServiceHandle(service);
                    let _ = CloseServiceHandle(scm);
                    return Ok(());
                }
                if last_state == ServiceState::Stopped {
                    break;
                }
            }
        }

        let _ = CloseServiceHandle(service);
        let _ = CloseServiceHandle(scm);
        Err(format!(
            "service {name} did not reach RUNNING within 5 s (last state: {last_state:?})"
        ))
    }
}

#[cfg(not(windows))]
pub fn start_service(_name: &str) -> Result<(), String> {
    Err("service control is only available on Windows".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_every_documented_scm_state() {
        assert_eq!(classify_state(1), ServiceState::Stopped);
        assert_eq!(classify_state(2), ServiceState::Pending);
        assert_eq!(classify_state(3), ServiceState::Pending);
        assert_eq!(classify_state(4), ServiceState::Running);
        assert_eq!(classify_state(5), ServiceState::Pending);
        assert_eq!(classify_state(6), ServiceState::Pending);
        assert_eq!(classify_state(7), ServiceState::Paused);
    }

    #[test]
    fn unknown_values_never_claim_a_known_state() {
        assert_eq!(classify_state(0), ServiceState::Unknown);
        assert_eq!(classify_state(99), ServiceState::Unknown);
    }

    #[test]
    fn classify_matches_the_windows_constants() {
        #[cfg(windows)]
        {
            use windows::Win32::System::Services::{
                SERVICE_PAUSED, SERVICE_PAUSE_PENDING, SERVICE_RUNNING, SERVICE_START_PENDING,
                SERVICE_STOPPED, SERVICE_STOP_PENDING,
            };
            assert_eq!(classify_state(SERVICE_RUNNING.0), ServiceState::Running);
            assert_eq!(classify_state(SERVICE_STOPPED.0), ServiceState::Stopped);
            assert_eq!(
                classify_state(SERVICE_START_PENDING.0),
                ServiceState::Pending
            );
            assert_eq!(
                classify_state(SERVICE_STOP_PENDING.0),
                ServiceState::Pending
            );
            assert_eq!(
                classify_state(SERVICE_PAUSE_PENDING.0),
                ServiceState::Pending
            );
            assert_eq!(classify_state(SERVICE_PAUSED.0), ServiceState::Paused);
        }
    }

    /// The harness must not be able to pass the test suite while the binary
    /// mis-classifies a state: guard against accidental constant drift.
    #[test]
    fn hresult_constants_match_winerror_values() {
        assert_eq!(HRESULT_SERVICE_DOES_NOT_EXIST, 0x8007_0424);
        assert_eq!(HRESULT_SERVICE_ALREADY_RUNNING, 0x8007_0420);
    }
}
