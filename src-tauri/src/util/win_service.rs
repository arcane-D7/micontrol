//! Native Windows Service Control Manager (SCM) API.
//!
//! Why this exists (audit 2026-11-07, P1 + architecture governor D3 / C1a-U3):
//! the app decided whether the autonomous `MiControlBridge` service was
//! available by opening its named pipe (`CreateFileW`). When the bridge's
//! accept loop wedged (C-01), the pipe stayed permanently busy while the
//! service itself was `RUNNING`, so the app concluded "service not available"
//! 597 times and fired 702 pointless `install_bridge_service` calls. Service
//! state must be answered by the SCM — the component that actually knows.
//!
//! D3 additionally required retiring every production `sc.exe` spawn: the
//! query path parsed text (`STATE : 4 RUNNING`), the write path leaned on
//! `sc failure` / `sc config` argument quirks, and none of it carried real
//! error information. This module speaks to the SCM directly with
//! `OpenSCManagerW` / `OpenServiceW` / `CreateServiceW` / `ChangeServiceConfigW`
//! / `ChangeServiceConfig2W` / `ControlService` / `QueryServiceStatusEx` /
//! `QueryServiceConfigW` / `DeleteService` and returns catalogued
//! [`AppError`]s (never a bare String error).
//!
//! Every handle is owned by an RAII wrapper, no production path calls
//! `unwrap` / `expect`, and all operations are synchronous: callers on async
//! paths must wrap them in `spawn_blocking` with a timeout (see
//! `util::blocking`).
//!
//! Units note (matches `sc.exe`; do not "harmonize"): `reset=` in
//! `sc failure` is SECONDS, while the delay after `/` in `restart/<ms>` is
//! MILLISECONDS. [`FailureActions::reset_period_secs`] and
//! [`FailureAction::delay_ms`] keep those native units.

use crate::errors::catalog::{AppError, ErrorCode};

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

/// Service start type (`sc qc` -> `START_TYPE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceStartType {
    /// `SERVICE_BOOT_START` (0): loaded by the boot loader.
    Boot,
    /// `SERVICE_SYSTEM_START` (1): loaded during kernel init.
    System,
    /// `SERVICE_AUTO_START` (2): `sc create ... start= auto`.
    Auto,
    /// `SERVICE_DEMAND_START` (3): `start= demand` (manual).
    Demand,
    /// `SERVICE_DISABLED` (4): `start= disabled`.
    Disabled,
    /// Value outside the documented range.
    Unknown,
}

/// Map a raw `QUERY_SERVICE_CONFIGW.dwStartType` to [`ServiceStartType`].
pub fn classify_start_type(raw: u32) -> ServiceStartType {
    match raw {
        0 => ServiceStartType::Boot,
        1 => ServiceStartType::System,
        2 => ServiceStartType::Auto,
        3 => ServiceStartType::Demand,
        4 => ServiceStartType::Disabled,
        _ => ServiceStartType::Unknown,
    }
}

/// Service error control (`sc create ... error=`): what the SCM does when the
/// service process fails to start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceErrorControl {
    /// `SERVICE_ERROR_IGNORE` (0): log the error and keep booting.
    Ignore,
    /// `SERVICE_ERROR_NORMAL` (1): the default; notify the user, keep booting.
    Normal,
    /// `SERVICE_ERROR_SEVERE` (2): restart with last-known-good on boot.
    Severe,
    /// `SERVICE_ERROR_CRITICAL` (3): boot cannot proceed without the service.
    Critical,
}

/// What the SCM does when the service process exits uncleanly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureActionKind {
    /// `SC_ACTION_NONE` (0): take no action; also what an EMPTY action list
    /// reads back as (the `sc failure ... actions= ""` clear).
    None,
    /// `SC_ACTION_RESTART` (1): restart the process (`restart/<ms>`).
    Restart,
    /// `SC_ACTION_REBOOT` (2): reboot the machine (`reboot/<ms>`).
    Reboot,
    /// `SC_ACTION_RUN_COMMAND` (3): run a command (`<command>/<ms>`).
    RunCommand,
}

impl FailureActionKind {
    /// Raw `SC_ACTION.Type` value.
    pub fn to_raw(self) -> i32 {
        match self {
            Self::None => 0,
            Self::Restart => 1,
            Self::Reboot => 2,
            Self::RunCommand => 3,
        }
    }

    /// Map a raw `SC_ACTION.Type`; undocumented values degrade to
    /// [`FailureActionKind::None`] on the read path (the write path never
    /// emits them).
    pub fn classify(raw: i32) -> Self {
        match raw {
            1 => Self::Restart,
            2 => Self::Reboot,
            3 => Self::RunCommand,
            _ => Self::None,
        }
    }
}

/// One failure action. `delay_ms` is the SCM's millisecond delay (the value
/// after `/` in `sc failure`'s `restart/5000` syntax); `sc`'s `reset=` unit
/// (seconds) lives in [`FailureActions::reset_period_secs`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FailureAction {
    pub action: FailureActionKind,
    pub delay_ms: u32,
}

impl FailureAction {
    /// Convenience constructor for the restart actions every MiControl
    /// service installs (`restart/5000` and friends).
    pub const fn restart(delay_ms: u32) -> Self {
        Self {
            action: FailureActionKind::Restart,
            delay_ms,
        }
    }
}

/// The failure-actions block read back from the SCM
/// (`SERVICE_CONFIG_FAILURE_ACTIONS` via `QueryServiceConfig2W`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailureActions {
    /// Time after which the failure count resets, in SECONDS
    /// (`sc failure reset= 86400` -> 86400). 0 = never reset.
    pub reset_period_secs: u32,
    pub actions: Vec<FailureAction>,
}

/// Full SCM view of one service: `sc query` + `sc qc` plus the pid only
/// `sc queryex` used to expose, in two native calls.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceInfo {
    pub state: ServiceState,
    pub start_type: ServiceStartType,
    /// Raw `BINARY_PATH_NAME` exactly as stored (quoting and trailing
    /// arguments kept, e.g. `"C:\path\micontrol_bridge.exe" service`).
    pub bin_path: Option<String>,
    /// Process id while the service is running
    /// (`SERVICE_STATUS_PROCESS.dwProcessId`); `None` when stopped/unknown.
    pub pid: Option<u32>,
}

/// Everything `CreateServiceW` needs. The MiControl-installer equivalent is
/// `binary_path = "\"C:\...\micontrol_bridge.exe\" service"`, `start_type`
/// [`ServiceStartType::Auto`], `error_control` [`ServiceErrorControl::Normal`],
/// `deps` empty (the service runs as `LocalSystem`, like `sc create` does).
#[derive(Debug, Clone)]
pub struct CreateSpec {
    /// Stored as `BINARY_PATH_NAME` VERBATIM. Include quoting and trailing
    /// arguments here when required (the IoT services need the quoted path
    /// plus a ` service` token, see `hw::ecram_service_mgmt`).
    pub binary_path: String,
    pub display_name: String,
    pub start_type: ServiceStartType,
    pub error_control: ServiceErrorControl,
    /// Names of services / load-order groups this one depends on.
    pub deps: Vec<String>,
}

/// `ERROR_SERVICE_DOES_NOT_EXIST` (1060).
pub const WIN32_SERVICE_DOES_NOT_EXIST: u32 = 1060;
/// `ERROR_SERVICE_ALREADY_RUNNING` (1056).
pub const WIN32_SERVICE_ALREADY_RUNNING: u32 = 1056;
/// `ERROR_SERVICE_NOT_ACTIVE` (1062): stop requested on a stopped service.
pub const WIN32_SERVICE_NOT_ACTIVE: u32 = 1062;
/// `ERROR_SERVICE_MARKED_FOR_DELETE` (1072): asynchronous delete still
/// settling; the entry lingers until every open handle closes.
pub const WIN32_MARKED_FOR_DELETE: u32 = 1072;
/// `ERROR_SERVICE_EXISTS` (1073): an entry with this name already exists.
pub const WIN32_SERVICE_EXISTS: u32 = 1073;

/// `ERROR_SERVICE_DOES_NOT_EXIST` as an HRESULT (0x80070424).
const HRESULT_SERVICE_DOES_NOT_EXIST: u32 = 0x8007_0424;
/// `ERROR_SERVICE_ALREADY_RUNNING` as an HRESULT (0x80070420).
const HRESULT_SERVICE_ALREADY_RUNNING: u32 = 0x8007_0420;

/// Extract the Win32 error code (low word) from a Win32-facility HRESULT.
fn win32_code_from_hresult(hresult: u32) -> Option<u32> {
    // 0x8007xxxx = FACILITY_WIN32 with the original code in the low word.
    if (hresult >> 16) & 0x7 == 7 {
        Some(hresult & 0xFFFF)
    } else {
        None
    }
}

/// Win32 code carried by an [`AppError`] whose source is a
/// `windows::core::Error`. Callers may need to distinguish 1072 / 1073 from
/// anything else without string parsing.
#[cfg(windows)]
pub fn win32_code(err: &AppError) -> Option<u32> {
    let source = err.source.as_ref()?;
    source
        .downcast_ref::<windows::core::Error>()
        .and_then(|e| win32_code_from_hresult(e.code().0 as u32))
}

/// True when `err` wraps exactly this Win32 code.
#[cfg(windows)]
pub fn is_win32(err: &AppError, code: u32) -> bool {
    win32_code(err) == Some(code)
}

/// Build a catalogued error from a failed SCM call, elevating
/// `ERROR_ACCESS_DENIED` (5) to [`ErrorCode::ServiceAccessDenied`]: its user
/// action is "elevate", while the fallback code keeps the caller's class.
#[cfg(windows)]
fn scm_err(default: ErrorCode, op: &str, name: &str, e: windows::core::Error) -> AppError {
    let code = if win32_code_from_hresult(e.code().0 as u32) == Some(5) {
        ErrorCode::ServiceAccessDenied
    } else {
        default
    };
    AppError::with_source(code, format!("{op}({name}): {e}"), Box::new(e))
}

// ── RAII handles ─────────────────────────────────────────────────────────────

/// Owned SCM handle (`OpenSCManagerW`), closed on drop.
#[cfg(windows)]
struct ScmHandle {
    handle: windows::Win32::System::Services::SC_HANDLE,
}

#[cfg(windows)]
impl Drop for ScmHandle {
    fn drop(&mut self) {
        use windows::Win32::System::Services::CloseServiceHandle;
        if !self.handle.is_invalid() {
            // SAFETY: the handle came from a successful Open* call and is
            // closed exactly once; a failed close has no meaningful remedy.
            unsafe {
                let _ = CloseServiceHandle(self.handle);
            }
        }
    }
}

/// Owned service handle (`OpenServiceW` / `CreateServiceW`), closed on drop.
/// Holding the parent `ScmHandle` keeps the SCM connection alive for the
/// service handle's lifetime.
#[cfg(windows)]
struct ServiceHandle {
    handle: windows::Win32::System::Services::SC_HANDLE,
    // Keeps the SCM handle alive while the service handle exists.
    #[allow(dead_code)]
    scm: ScmHandle,
}

#[cfg(windows)]
impl Drop for ServiceHandle {
    fn drop(&mut self) {
        use windows::Win32::System::Services::CloseServiceHandle;
        if !self.handle.is_invalid() {
            unsafe {
                let _ = CloseServiceHandle(self.handle);
            }
        }
    }
}

/// NUL-terminated UTF-16 for SCM string parameters.
#[cfg(windows)]
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Read a NUL-terminated UTF-16 string the SCM stored in a config buffer.
#[cfg(windows)]
fn pwstr_to_string(p: windows::core::PWSTR) -> Option<String> {
    if p.0.is_null() {
        return None;
    }
    // SAFETY: `p` points at a NUL-terminated UTF-16 string inside the
    // caller-provided config buffer, which outlives this read.
    unsafe {
        let mut len = 0usize;
        while *p.0.add(len) != 0 {
            len += 1;
        }
        Some(String::from_utf16_lossy(std::slice::from_raw_parts(
            p.0, len,
        )))
    }
}

// ── Core Operations (freestanding, fully unit-callable) ──────────────────────
//
// Each takes an explicit SC_HANDLE/SCM handle so unit tests and callers can
// reuse an open handle; the API wrappers below open/close per call.

#[cfg(windows)]
mod ops {
    use super::{
        classify_start_type, classify_state, pwstr_to_string, scm_err, wide, AppError, ErrorCode,
        FailureAction, FailureActionKind, FailureActions, ScmHandle, ServiceErrorControl,
        ServiceHandle, ServiceStartType, ServiceState, HRESULT_SERVICE_ALREADY_RUNNING,
        HRESULT_SERVICE_DOES_NOT_EXIST,
    };
    use windows::core::PCWSTR;
    use windows::Win32::System::Services::{
        ChangeServiceConfig2W, ChangeServiceConfigW, ControlService, CreateServiceW, DeleteService,
        OpenSCManagerW, OpenServiceW, QueryServiceConfig2W, QueryServiceConfigW,
        QueryServiceStatusEx, StartServiceW, SC_ACTION, SC_ACTION_TYPE, SC_STATUS_PROCESS_INFO,
        SERVICE_CONFIG_FAILURE_ACTIONS, SERVICE_CONTROL_STOP, SERVICE_ERROR,
        SERVICE_ERROR_CRITICAL, SERVICE_ERROR_IGNORE, SERVICE_ERROR_NORMAL, SERVICE_ERROR_SEVERE,
        SERVICE_NO_CHANGE, SERVICE_START_TYPE, SERVICE_STATUS, SERVICE_STATUS_PROCESS,
        SERVICE_WIN32_OWN_PROCESS,
    };

    /// Open the local SCM with exactly the access the operation needs
    /// (`SC_MANAGER_CONNECT` is granted to standard users; `SC_MANAGER_CREATE`
    /// is the elevated install path); callers add it: see `super`.
    pub(super) fn open_scm(access: u32) -> Result<ScmHandle, AppError> {
        // SAFETY: both name parameters are None (local machine,
        // SERVICES_ACTIVE_DATABASE).
        unsafe {
            OpenSCManagerW(None, None, access)
                .map(|handle| ScmHandle { handle })
                .map_err(|e| scm_err(ErrorCode::ServiceQueryFailed, "OpenSCManagerW", "<scm>", e))
        }
    }

    /// Open a service handle; `Ok(None)` when the SCM says 1060 (does not
    /// exist). Every other failure is an error with an SCM error code.
    pub(super) fn open_service(
        scm: &ScmHandle,
        name: &str,
        access: u32,
    ) -> Result<Option<ServiceHandle>, AppError> {
        let wide: Vec<u16> = wide(name);
        // SAFETY: `wide` is a NUL-terminated UTF-16 buffer alive for the call;
        // the returned handle is wrapped and closed on drop.
        unsafe {
            match OpenServiceW(scm.handle, PCWSTR(wide.as_ptr()), access) {
                Ok(handle) => Ok(Some(ServiceHandle {
                    handle,
                    scm: ScmHandle { handle: scm.handle },
                })),
                Err(e) if e.code().0 as u32 == HRESULT_SERVICE_DOES_NOT_EXIST => Ok(None),
                Err(e) => Err(scm_err(
                    ErrorCode::ServiceQueryFailed,
                    "OpenServiceW",
                    name,
                    e,
                )),
            }
        }
    }

    /// `QueryServiceStatusEx(SC_STATUS_PROCESS_INFO)`: current state plus the
    /// pid of the running service process.
    pub(super) fn query_status(
        svc: &ServiceHandle,
        name: &str,
    ) -> Result<(ServiceState, Option<u32>), AppError> {
        let mut buf = vec![0u8; std::mem::size_of::<SERVICE_STATUS_PROCESS>()];
        let mut needed: u32 = 0;
        // SAFETY: the buffer is sized exactly for SERVICE_STATUS_PROCESS and
        // the SCM writes at most cbBufSize bytes into it.
        unsafe {
            QueryServiceStatusEx(
                svc.handle,
                SC_STATUS_PROCESS_INFO,
                Some(&mut buf),
                &mut needed,
            )
            .map_err(|e| {
                scm_err(
                    ErrorCode::ServiceQueryFailed,
                    "QueryServiceStatusEx",
                    name,
                    e,
                )
            })?;
            let status = *(buf.as_ptr() as *const SERVICE_STATUS_PROCESS);
            let pid = status.dwProcessId;
            Ok((
                classify_state(status.dwCurrentState.0),
                if pid == 0 { None } else { Some(pid) },
            ))
        }
    }

    /// `QueryServiceConfigW`: start type and the verbatim binary path.
    pub(super) fn query_config(
        svc: &ServiceHandle,
        name: &str,
    ) -> Result<(ServiceStartType, Option<String>), AppError> {
        // Two-call pattern: with a zero-size buffer the SCM reports the
        // required size in `needed` (ERROR_INSUFFICIENT_BUFFER).
        let mut needed: u32 = 0;
        // SAFETY: None config pointer + 0 size is the documented size probe.
        let probe = unsafe { QueryServiceConfigW(svc.handle, None, 0, &mut needed) };
        if probe.is_ok() && needed == 0 {
            // Theoretically unreachable for an existing service.
            return Ok((ServiceStartType::Unknown, None));
        }
        let mut buf = vec![0u8; needed as usize];
        // SAFETY: `buf` holds `needed` bytes; the SCM writes the
        // QUERY_SERVICE_CONFIGW header plus trailing strings inside it, and
        // the returned PWSTR fields point into this same buffer, read before
        // it is dropped at the end of the block.
        unsafe {
            QueryServiceConfigW(
                svc.handle,
                Some(buf.as_mut_ptr() as *mut _),
                needed,
                &mut needed,
            )
            .map_err(|e| {
                scm_err(
                    ErrorCode::ServiceQueryFailed,
                    "QueryServiceConfigW",
                    name,
                    e,
                )
            })?;
            let cfg =
                *(buf.as_ptr() as *const windows::Win32::System::Services::QUERY_SERVICE_CONFIGW);
            Ok((
                classify_start_type(cfg.dwStartType.0),
                pwstr_to_string(cfg.lpBinaryPathName),
            ))
        }
    }

    /// Issue `SERVICE_CONTROL_STOP` without waiting (the `sc stop`
    /// semantic). An already-stopped service (1062) is success.
    pub(super) fn request_stop(svc: &ServiceHandle, name: &str) -> Result<(), AppError> {
        let mut status = SERVICE_STATUS::default();
        // SAFETY: `status` is a valid out-struct for the final state snapshot.
        unsafe {
            match ControlService(svc.handle, SERVICE_CONTROL_STOP, &mut status) {
                Ok(()) => Ok(()),
                Err(e) if e.code().0 as u32 & 0xFFFF == super::WIN32_SERVICE_NOT_ACTIVE => Ok(()),
                Err(e) => Err(scm_err(
                    ErrorCode::ServiceControlFailed,
                    "ControlService(STOP)",
                    name,
                    e,
                )),
            }
        }
    }

    /// `StartServiceW` with default arguments; tolerates "already running"
    /// (1056) like every previous caller did.
    pub(super) fn start(svc: &ServiceHandle, name: &str) -> Result<(), AppError> {
        // SAFETY: no argument vector is passed.
        unsafe {
            match StartServiceW(svc.handle, None) {
                Ok(()) => Ok(()),
                Err(e) if e.code().0 as u32 == HRESULT_SERVICE_ALREADY_RUNNING => Ok(()),
                Err(e) => Err(scm_err(
                    ErrorCode::ServiceControlFailed,
                    "StartServiceW",
                    name,
                    e,
                )),
            }
        }
    }

    /// `CreateServiceW`: own-process Win32 service running as LocalSystem.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn create(
        scm: &ScmHandle,
        name: &str,
        binary_path: &str,
        display_name: &str,
        start_type: ServiceStartType,
        error_control: ServiceErrorControl,
        deps: &[String],
    ) -> Result<(), AppError> {
        let start_raw = match start_type {
            ServiceStartType::Boot => 0,
            ServiceStartType::System => 1,
            ServiceStartType::Auto => 2,
            ServiceStartType::Demand => 3,
            ServiceStartType::Disabled => 4,
            // A caller asking to CREATE an "Unknown" start type is a bug:
            // refuse instead of storing a garbage value in the SCM.
            ServiceStartType::Unknown => {
                return Err(AppError::new(
                    ErrorCode::ServiceConfigFailed,
                    format!("create({name}): unknown start type"),
                ));
            }
        };
        let error_raw = match error_control {
            ServiceErrorControl::Ignore => SERVICE_ERROR_IGNORE,
            ServiceErrorControl::Normal => SERVICE_ERROR_NORMAL,
            ServiceErrorControl::Severe => SERVICE_ERROR_SEVERE,
            ServiceErrorControl::Critical => SERVICE_ERROR_CRITICAL,
        };
        let name_w = wide(name);
        let display_w = wide(display_name);
        let bin_w = wide(binary_path);
        // Multi-sz dependency list: each entry NUL-terminated, the list
        // double-NUL terminated; empty -> null pointer.
        let deps_w: Vec<u16> = if deps.is_empty() {
            Vec::new()
        } else {
            let mut acc = Vec::new();
            for dep in deps {
                acc.extend(wide(dep));
            }
            acc.push(0);
            acc
        };
        // SAFETY: every PCWSTR points at a NUL-terminated buffer that outlives
        // the call; the returned handle is owned and registers no callback.
        unsafe {
            let _handle = CreateServiceW(
                scm.handle,
                PCWSTR(name_w.as_ptr()),
                PCWSTR(display_w.as_ptr()),
                windows::Win32::System::Services::SERVICE_ALL_ACCESS,
                SERVICE_WIN32_OWN_PROCESS,
                SERVICE_START_TYPE(start_raw),
                error_raw,
                PCWSTR(bin_w.as_ptr()),
                PCWSTR::null(),
                None,
                if deps_w.is_empty() {
                    PCWSTR::null()
                } else {
                    PCWSTR(deps_w.as_ptr())
                },
                PCWSTR::null(),
                PCWSTR::null(),
            )
            .map_err(|e| scm_err(ErrorCode::ServiceConfigFailed, "CreateServiceW", name, e))?;
            Ok(())
        }
    }

    /// `DeleteService`; the handle must have been opened with DELETE access.
    pub(super) fn delete(svc: &ServiceHandle, name: &str) -> Result<(), AppError> {
        // SAFETY: `svc` is an open handle with DELETE access.
        unsafe {
            DeleteService(svc.handle)
                .map_err(|e| scm_err(ErrorCode::ServiceControlFailed, "DeleteService", name, e))
        }
    }

    /// `ChangeServiceConfig2W(SERVICE_CONFIG_FAILURE_ACTIONS)`: replace the
    /// whole action list. An EMPTY list is the `sc failure reset= 0
    /// actions= ""` clear; `reset_period_secs` is `sc`'s `reset=` (SECONDS).
    pub(super) fn set_failure_actions(
        svc: &ServiceHandle,
        name: &str,
        actions: &[FailureAction],
        reset_period_secs: u32,
    ) -> Result<(), AppError> {
        let mut sc_actions: Vec<SC_ACTION> = actions
            .iter()
            .map(|a| SC_ACTION {
                Type: SC_ACTION_TYPE(a.action.to_raw()),
                Delay: a.delay_ms,
            })
            .collect();
        let block = windows::Win32::System::Services::SERVICE_FAILURE_ACTIONSW {
            dwResetPeriod: reset_period_secs,
            lpRebootMsg: windows::core::PWSTR::null(),
            lpCommand: windows::core::PWSTR::null(),
            cActions: sc_actions.len() as u32,
            lpsaActions: sc_actions.as_mut_ptr(),
        };
        // SAFETY: `block` and `sc_actions` outlive the call; the level and
        // pointer pairing matches SERVICE_CONFIG_FAILURE_ACTIONS.
        unsafe {
            ChangeServiceConfig2W(
                svc.handle,
                SERVICE_CONFIG_FAILURE_ACTIONS,
                Some(&block as *const _ as *const core::ffi::c_void),
            )
            .map_err(|e| {
                scm_err(
                    ErrorCode::ServiceConfigFailed,
                    "ChangeServiceConfig2W",
                    name,
                    e,
                )
            })
        }
    }

    /// Read the failure actions back (`QueryServiceConfig2W`) so callers can
    /// verify a write or a clear instead of assuming it.
    pub(super) fn query_failure_actions(
        svc: &ServiceHandle,
        name: &str,
    ) -> Result<FailureActions, AppError> {
        let mut buf: Vec<u8> = vec![0u8; 256];
        let mut needed: u32 = 0;
        // SAFETY: the SCM writes at most buf.len() bytes and reports the
        // required size in `needed` when the buffer is too small; on retry
        // the buffer is grown to exactly `needed`.
        unsafe {
            if let Err(e) = QueryServiceConfig2W(
                svc.handle,
                SERVICE_CONFIG_FAILURE_ACTIONS,
                Some(&mut buf),
                &mut needed,
            ) {
                if needed == 0 {
                    return Err(scm_err(
                        ErrorCode::ServiceQueryFailed,
                        "QueryServiceConfig2W",
                        name,
                        e,
                    ));
                }
                buf = vec![0u8; needed as usize];
                QueryServiceConfig2W(
                    svc.handle,
                    SERVICE_CONFIG_FAILURE_ACTIONS,
                    Some(&mut buf),
                    &mut needed,
                )
                .map_err(|e| {
                    scm_err(
                        ErrorCode::ServiceQueryFailed,
                        "QueryServiceConfig2W",
                        name,
                        e,
                    )
                })?;
            }
            let block = &*(buf.as_ptr()
                as *const windows::Win32::System::Services::SERVICE_FAILURE_ACTIONSW);
            let count = block.cActions as usize;
            let actions = if count == 0 || block.lpsaActions.is_null() {
                Vec::new()
            } else {
                std::slice::from_raw_parts(block.lpsaActions, count)
                    .iter()
                    .map(|a| FailureAction {
                        action: FailureActionKind::classify(a.Type.0),
                        delay_ms: a.Delay,
                    })
                    .collect()
            };
            Ok(FailureActions {
                reset_period_secs: block.dwResetPeriod,
                actions,
            })
        }
    }

    /// `ChangeServiceConfigW` changing ONLY the binary path (type, start mode
    /// and error control stay `SERVICE_NO_CHANGE`).
    pub(super) fn set_bin_path(
        svc: &ServiceHandle,
        name: &str,
        path: &str,
    ) -> Result<(), AppError> {
        let bin_w = wide(path);
        // SAFETY: every PCWSTR is NUL-terminated and outlives the call;
        // SERVICE_NO_CHANGE in every other slot keeps the existing config.
        unsafe {
            ChangeServiceConfigW(
                svc.handle,
                windows::Win32::System::Services::ENUM_SERVICE_TYPE(SERVICE_NO_CHANGE),
                SERVICE_START_TYPE(SERVICE_NO_CHANGE),
                SERVICE_ERROR(SERVICE_NO_CHANGE),
                PCWSTR(bin_w.as_ptr()),
                PCWSTR::null(),
                None,
                PCWSTR::null(),
                PCWSTR::null(),
                PCWSTR::null(),
                PCWSTR::null(),
            )
            .map_err(|e| {
                scm_err(
                    ErrorCode::ServiceConfigFailed,
                    "ChangeServiceConfigW",
                    name,
                    e,
                )
            })
        }
    }
}

// ── Public API ───────────────────────────────────────────────────────────────

#[cfg(windows)]
mod api {
    use super::ops;
    use super::{
        AppError, CreateSpec, ErrorCode, FailureAction, FailureActions, ScmHandle, ServiceHandle,
        ServiceInfo, ServiceStartType, ServiceState,
    };
    use windows::Win32::System::Services::{
        SC_MANAGER_CONNECT, SC_MANAGER_CREATE_SERVICE, SERVICE_CHANGE_CONFIG, SERVICE_QUERY_CONFIG,
        SERVICE_QUERY_STATUS, SERVICE_START, SERVICE_STOP,
    };

    /// Standard DELETE access right (0x00010000): windows-rs exposes the
    /// SCM-specific rights; the generic NT DELETE right is this bit.
    const DELETE_ACCESS: u32 = 0x0001_0000;

    /// Open a service with the given access; `Ok(None)` = absent (1060).
    fn open(scm: &ScmHandle, name: &str, access: u32) -> Result<Option<ServiceHandle>, AppError> {
        ops::open_service(scm, name, access)
    }

    /// Query the SCM for the full state of `name` (e.g. `"MiControlBridge"`).
    ///
    /// Never elevates: query access is granted to standard users by the
    /// default service DACL. An absent service is `Ok` with
    /// [`ServiceState::Absent`] — only the SCM's own 1060 produces `Absent`;
    /// every other failure is an `Err`. (Treating "I could not ask" as "it
    /// does not exist" is what re-introduced install churn in P1.)
    pub fn query(name: &str) -> Result<ServiceInfo, AppError> {
        let scm = ops::open_scm(SC_MANAGER_CONNECT)?;
        let Some(svc) = open(&scm, name, SERVICE_QUERY_STATUS | SERVICE_QUERY_CONFIG)? else {
            return Ok(ServiceInfo {
                state: ServiceState::Absent,
                start_type: ServiceStartType::Unknown,
                bin_path: None,
                pid: None,
            });
        };
        let (state, pid) = ops::query_status(&svc, name)?;
        let (start_type, bin_path) = ops::query_config(&svc, name)?;
        Ok(ServiceInfo {
            state,
            start_type,
            bin_path,
            pid,
        })
    }

    /// Issue `SERVICE_CONTROL_STOP` without waiting (the `sc stop`
    /// semantic). An already-stopped or already-absent service is success.
    pub fn stop_request(name: &str) -> Result<(), AppError> {
        let scm = ops::open_scm(SC_MANAGER_CONNECT)?;
        let Some(svc) = open(&scm, name, SERVICE_STOP | SERVICE_QUERY_STATUS)? else {
            return Ok(());
        };
        ops::request_stop(&svc, name)
    }

    /// Stop `name` and wait (bounded) until the SCM reports `STOPPED`.
    /// Absent counts as stopped. Composes `stop_request` + `wait_for_state`,
    /// the exact sequence the old `sc stop` + poll loops performed.
    pub fn stop(name: &str, timeout: std::time::Duration) -> Result<(), AppError> {
        stop_request(name)?;
        wait_for_state(name, ServiceState::Stopped, timeout)
    }

    /// Ask the SCM to start `name` (`sc start`). Tolerates
    /// `ERROR_SERVICE_ALREADY_RUNNING` (1056). Does NOT wait for RUNNING —
    /// compose with [`wait_for_state`] when the caller polled before.
    pub fn start(name: &str) -> Result<(), AppError> {
        let scm = ops::open_scm(SC_MANAGER_CONNECT)?;
        let Some(svc) = open(&scm, name, SERVICE_START | SERVICE_QUERY_STATUS)? else {
            return Err(AppError::new(
                ErrorCode::ServiceControlFailed,
                format!("{name} does not exist; cannot start"),
            ));
        };
        ops::start(&svc, name)
    }

    /// Create the service (`sc create`): own-process Win32 service running
    /// as LocalSystem with the given start type / error control / deps.
    /// While an old entry is still settling the SCM fails with Win32 1072
    /// (marked-for-delete) or 1073 (exists) — callers may retry those.
    pub fn create(name: &str, spec: &CreateSpec) -> Result<(), AppError> {
        let scm = ops::open_scm(SC_MANAGER_CONNECT | SC_MANAGER_CREATE_SERVICE)?;
        ops::create(
            &scm,
            name,
            &spec.binary_path,
            &spec.display_name,
            spec.start_type,
            spec.error_control,
            &spec.deps,
        )
    }

    /// Delete the service entry (`sc delete`). Absent is success (idempotent,
    /// the old "does not exist is fine" branch). The delete is asynchronous
    /// in the SCM: poll [`query`] afterwards when the entry must be provably
    /// gone.
    pub fn delete(name: &str) -> Result<(), AppError> {
        let scm = ops::open_scm(SC_MANAGER_CONNECT)?;
        let Some(svc) = open(&scm, name, DELETE_ACCESS)? else {
            return Ok(());
        };
        ops::delete(&svc, name)
    }

    /// Replace the whole failure-action list (`sc failure reset= <secs>
    /// actions= ...`). An EMPTY `actions` slice is the `reset= 0 actions= ""`
    /// clear. Verify with [`failure_actions`]. Unit note: `reset_period_secs`
    /// is SECONDS, `FailureAction::delay_ms` is MILLISECONDS.
    pub fn set_failure_actions(
        name: &str,
        actions: &[FailureAction],
        reset_period_secs: u32,
    ) -> Result<(), AppError> {
        let scm = ops::open_scm(SC_MANAGER_CONNECT)?;
        let Some(svc) = open(&scm, name, SERVICE_CHANGE_CONFIG | SERVICE_QUERY_CONFIG)? else {
            return Err(AppError::new(
                ErrorCode::ServiceConfigFailed,
                format!("{name} does not exist; cannot configure failure actions"),
            ));
        };
        ops::set_failure_actions(&svc, name, actions, reset_period_secs)
    }

    /// Read the failure actions back so writes and clears can be verified
    /// instead of assumed (`sc failure` view, via QueryServiceConfig2W).
    pub fn failure_actions(name: &str) -> Result<FailureActions, AppError> {
        let scm = ops::open_scm(SC_MANAGER_CONNECT)?;
        let Some(svc) = open(&scm, name, SERVICE_QUERY_CONFIG)? else {
            return Err(AppError::new(
                ErrorCode::ServiceQueryFailed,
                format!("{name} does not exist; no failure actions to read"),
            ));
        };
        ops::query_failure_actions(&svc, name)
    }

    /// Point the service at a new binary (`sc config binPath= `). Type, start
    /// mode and error control stay untouched (`SERVICE_NO_CHANGE`). NOTE: for
    /// the IoT services the delete-and-recreate path remains the proven
    /// mechanism for quoted binPaths (`hw::ecram_service_mgmt`); this API
    /// exists for plain single-token paths.
    pub fn set_bin_path(name: &str, path: &str) -> Result<(), AppError> {
        let scm = ops::open_scm(SC_MANAGER_CONNECT)?;
        let Some(svc) = open(&scm, name, SERVICE_CHANGE_CONFIG)? else {
            return Err(AppError::new(
                ErrorCode::ServiceConfigFailed,
                format!("{name} does not exist; cannot set binPath"),
            ));
        };
        ops::set_bin_path(&svc, name, path)
    }

    /// Poll (250 ms cadence) until the service reaches `target`, bounded by
    /// `timeout`. `Err(Timeout)` on the deadline. A service that stops (or
    /// vanishes — a deleted entry cannot come back) before reaching a
    /// non-stopped target fails fast.
    pub fn wait_for_state(
        name: &str,
        target: ServiceState,
        timeout: std::time::Duration,
    ) -> Result<(), AppError> {
        let deadline = std::time::Instant::now() + timeout;
        let scm = ops::open_scm(SC_MANAGER_CONNECT)?;
        let svc = match open(&scm, name, SERVICE_QUERY_STATUS)? {
            None => {
                return if target == ServiceState::Stopped {
                    Ok(())
                } else {
                    Err(AppError::new(
                        ErrorCode::ServiceControlFailed,
                        format!("{name} does not exist; cannot reach {target:?}"),
                    ))
                };
            }
            Some(s) => s,
        };
        loop {
            let (state, _) = ops::query_status(&svc, name)?;
            if state == target {
                return Ok(());
            }
            // A STOPPED service will not reach any other state without an
            // explicit start: fail fast instead of burning the whole budget.
            if state == ServiceState::Stopped && target != ServiceState::Stopped {
                return Err(AppError::new(
                    ErrorCode::ServiceControlFailed,
                    format!("{name} stopped before reaching {target:?}"),
                ));
            }
            if std::time::Instant::now() >= deadline {
                return Err(AppError::new(
                    ErrorCode::Timeout,
                    format!(
                        "{name} did not reach {target:?} within {timeout:?} (last state: {state:?})"
                    ),
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    }

    /// Legacy read-only helper (`elev_bridge`): state only, never fails.
    /// `Unknown` on any SCM failure; `Absent` only on the SCM's own 1060.
    pub fn query_service_state(name: &str) -> ServiceState {
        let Ok(scm) = ops::open_scm(SC_MANAGER_CONNECT) else {
            return ServiceState::Unknown;
        };
        match open(&scm, name, SERVICE_QUERY_STATUS) {
            Ok(Some(svc)) => ops::query_status(&svc, name)
                .map(|(state, _)| state)
                .unwrap_or(ServiceState::Unknown),
            // Distinguish "the SCM said 1060" from "open failed otherwise":
            // open() already avoids Err on 1060, so an Err here reports as
            // Unknown (never as Absent).
            Ok(None) => ServiceState::Absent,
            Err(_) => ServiceState::Unknown,
        }
    }

    /// Legacy start helper (`elev_bridge`): start, then bounded 5 s wait for
    /// RUNNING (the old 20 x 250 ms loop). The String signature is the
    /// pre-catalog boundary this function already had; no new String error
    /// surface is born here.
    pub fn start_service(name: &str) -> Result<(), String> {
        let scm = ops::open_scm(SC_MANAGER_CONNECT).map_err(|e| format!("OpenSCManagerW: {e}"))?;
        let svc = match open(&scm, name, SERVICE_START | SERVICE_QUERY_STATUS) {
            Ok(Some(s)) => s,
            Ok(None) => return Err(format!("OpenServiceW({name}): service does not exist")),
            Err(e) => return Err(format!("OpenServiceW({name}): {e}")),
        };
        let state_before = ops::query_status(&svc, name)
            .map(|(state, _)| state)
            .unwrap_or(ServiceState::Unknown);
        match state_before {
            ServiceState::Running => return Ok(()),
            // Already starting: do NOT issue StartServiceW (and never let the
            // caller conclude "stopped" and reinstall) — just wait for RUNNING.
            ServiceState::Pending => {}
            ServiceState::Stopped
            | ServiceState::Paused
            | ServiceState::Absent
            | ServiceState::Unknown => {
                ops::start(&svc, name).map_err(|e| e.to_string())?;
            }
        }
        wait_for_state(
            name,
            ServiceState::Running,
            std::time::Duration::from_secs(5),
        )
        .map_err(|e| format!("service {name}: {e}"))
    }
}

#[cfg(windows)]
pub use api::{
    create, delete, failure_actions, query, query_service_state, set_bin_path, set_failure_actions,
    start, start_service, stop, stop_request, wait_for_state,
};

// ── Public API (non-Windows fallbacks) ───────────────────────────────────────

#[cfg(not(windows))]
mod api_other {
    use super::{AppError, CreateSpec, FailureAction, FailureActions, ServiceInfo, ServiceState};
    use std::time::Duration;

    fn unsupported() -> AppError {
        AppError::new(
            crate::errors::catalog::ErrorCode::NotSupported,
            "service control is only available on Windows",
        )
    }

    pub fn query(_name: &str) -> Result<ServiceInfo, AppError> {
        Err(unsupported())
    }

    pub fn stop(_name: &str, _timeout: Duration) -> Result<(), AppError> {
        Err(unsupported())
    }

    pub fn stop_request(_name: &str) -> Result<(), AppError> {
        Err(unsupported())
    }

    pub fn start(_name: &str) -> Result<(), AppError> {
        Err(unsupported())
    }

    pub fn create(_name: &str, _spec: &CreateSpec) -> Result<(), AppError> {
        Err(unsupported())
    }

    pub fn delete(_name: &str) -> Result<(), AppError> {
        Err(unsupported())
    }

    pub fn set_failure_actions(
        _name: &str,
        _actions: &[FailureAction],
        _reset_period_secs: u32,
    ) -> Result<(), AppError> {
        Err(unsupported())
    }

    pub fn failure_actions(_name: &str) -> Result<FailureActions, AppError> {
        Err(unsupported())
    }

    pub fn set_bin_path(_name: &str, _path: &str) -> Result<(), AppError> {
        Err(unsupported())
    }

    pub fn wait_for_state(
        _name: &str,
        _target: ServiceState,
        _timeout: Duration,
    ) -> Result<(), AppError> {
        Err(unsupported())
    }

    pub fn query_service_state(_name: &str) -> ServiceState {
        ServiceState::Unknown
    }

    pub fn start_service(_name: &str) -> Result<(), String> {
        Err("service control is only available on Windows".to_string())
    }
}

#[cfg(not(windows))]
pub use api_other::*;

// ── Tests ────────────────────────────────────────────────────────────────────

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

    #[test]
    fn maps_every_documented_start_type() {
        assert_eq!(classify_start_type(0), ServiceStartType::Boot);
        assert_eq!(classify_start_type(1), ServiceStartType::System);
        assert_eq!(classify_start_type(2), ServiceStartType::Auto);
        assert_eq!(classify_start_type(3), ServiceStartType::Demand);
        assert_eq!(classify_start_type(4), ServiceStartType::Disabled);
        assert_eq!(classify_start_type(5), ServiceStartType::Unknown);
    }

    #[test]
    fn start_type_raw_values_match_winsvc_numbers() {
        // Guards against a renumbered local enum drifting from winsvc.h.
        assert_eq!(ServiceStartType::Boot as u32, 0);
        assert_eq!(ServiceStartType::System as u32, 1);
        assert_eq!(ServiceStartType::Auto as u32, 2);
        assert_eq!(ServiceStartType::Demand as u32, 3);
        assert_eq!(ServiceStartType::Disabled as u32, 4);
        assert_eq!(ServiceStartType::Unknown as u32, 5);
    }

    #[test]
    fn failure_action_kinds_match_sc_action_values() {
        assert_eq!(FailureActionKind::None.to_raw(), 0);
        assert_eq!(FailureActionKind::Restart.to_raw(), 1);
        assert_eq!(FailureActionKind::Reboot.to_raw(), 2);
        assert_eq!(FailureActionKind::RunCommand.to_raw(), 3);
        assert_eq!(FailureActionKind::classify(0), FailureActionKind::None);
        assert_eq!(FailureActionKind::classify(1), FailureActionKind::Restart);
        assert_eq!(FailureActionKind::classify(2), FailureActionKind::Reboot);
        assert_eq!(
            FailureActionKind::classify(3),
            FailureActionKind::RunCommand
        );
        // Undocumented values degrade to None (read path only).
        assert_eq!(FailureActionKind::classify(9), FailureActionKind::None);
    }

    #[test]
    fn error_control_values_match_winsvc() {
        assert_eq!(ServiceErrorControl::Ignore as u32, 0);
        assert_eq!(ServiceErrorControl::Normal as u32, 1);
        assert_eq!(ServiceErrorControl::Severe as u32, 2);
        assert_eq!(ServiceErrorControl::Critical as u32, 3);
    }

    /// The harness must not be able to pass the suite while an error constant
    /// drifts: these two HRESULTs are load-bearing in callers.
    #[test]
    fn hresult_constants_match_winerror_values() {
        assert_eq!(HRESULT_SERVICE_DOES_NOT_EXIST, 0x8007_0424);
        assert_eq!(HRESULT_SERVICE_ALREADY_RUNNING, 0x8007_0420);
    }

    #[test]
    fn win32_codes_are_the_documented_winerror_numbers() {
        assert_eq!(WIN32_SERVICE_DOES_NOT_EXIST, 1060);
        assert_eq!(WIN32_SERVICE_ALREADY_RUNNING, 1056);
        assert_eq!(WIN32_SERVICE_NOT_ACTIVE, 1062);
        assert_eq!(WIN32_MARKED_FOR_DELETE, 1072);
        assert_eq!(WIN32_SERVICE_EXISTS, 1073);
    }

    #[test]
    fn hresult_extraction_only_fires_on_win32_facility() {
        assert_eq!(win32_code_from_hresult(0x8007_0424), Some(0x0424));
        assert_eq!(win32_code_from_hresult(0x8007_0420), Some(0x0420));
        // Non-Win32 facilities carry no extractable code.
        assert_eq!(win32_code_from_hresult(0x8000_4005), None);
        assert_eq!(win32_code_from_hresult(0x0000_0000), None);
    }

    /// C1a/U3 gate: after the native migration NO production code in
    /// `src-tauri/src` may spawn `sc` / `sc.exe` again (queries, configs,
    /// failure actions and start/stop all go through this module). Test
    /// modules are excluded with the same brace heuristic the JS gates use,
    /// comments never match, and the pattern is assembled from fragments so
    /// this very test cannot match itself.
    #[test]
    fn no_sc_exe_spawns_anywhere_in_production() {
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let needle = concat!("Command::new", "(", '"', "sc", '"', ")");
        let mut offenders = Vec::new();
        let mut stack = vec![src];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).expect("src dir must be readable") {
                let path = entry.expect("dir entry readable").path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = std::fs::read_to_string(&path).expect("source readable");
                    for (idx, _) in text.match_indices(needle) {
                        // Skip #[cfg(test)] regions (same brace heuristic the
                        // JS gates use: test code is not production).
                        let before = &text[..idx];
                        if let Some(last) = before.rfind("#[cfg(test)]") {
                            let after = &before[last..];
                            if after.matches('{').count() > after.matches('}').count() {
                                continue;
                            }
                        }
                        let line = text[..idx].matches('\n').count() + 1;
                        offenders.push(format!("{}:{line}", path.display()));
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "sc.exe spawns must go through util::win_service; found:\n{}",
            offenders.join("\n")
        );
    }

    /// LIVE readback (#[ignore]d: touches the real SCM, read-only). Run with
    /// `cargo test --lib --features face win_service -- --ignored`. Asserts
    /// only that the SCM answers coherently; the state itself is printed,
    /// never asserted (services legitimately stop and start).
    #[test]
    #[ignore = "live SCM readback; run with --ignored"]
    fn live_query_reports_scm_truth_for_micontrol_services() {
        for name in ["IoTSvc", "MiControlBridge"] {
            let info = query(name).unwrap_or_else(|e| panic!("live query({name}) failed: {e}"));
            println!(
                "{name}: state={state:?} start_type={start:?} pid={pid:?} bin_path={bin:?}",
                state = info.state,
                start = info.start_type,
                pid = info.pid,
                bin = info.bin_path,
            );
            assert_ne!(
                info.state,
                ServiceState::Unknown,
                "{name}: the SCM could not be asked; query must not degrade to Unknown"
            );
        }
    }
}
