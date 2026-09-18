//! OS Turbo — system-level software optimization.
//!
//! Provides a functional equivalent to XPM's "OS Turbo" feature, which
//! optimizes system performance through:
//! - Background process throttling (EcoQoS)
//! - Foreground app prioritization
//! - Power plan switching
//! - Startup app management
//!
//! This is distinct from the hardware performance mode (EC 0x68) which
//! controls CPU TDP/fan curves. OS Turbo operates at the Windows scheduler
//! level without touching EC registers.

use crate::hw::errors::{HardwareError, HardwareResult};
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;

const HIGH_PERFORMANCE_SCHEME: u128 = 0x8c5e7fda_e8bf_4a96_9a85_a6e23a8c635c;
const SNAPSHOT_VERSION: u32 = 1;
const MAX_SNAPSHOT_PROCESSES: usize = 64;
/// Fixed interval between periodic worker sweeps (S39-003).
const WORKER_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);
/// Maximum candidates inspected per sweep. The recovery snapshot is already
/// bounded by `MAX_SNAPSHOT_PROCESSES`; this is a hard cap on per-sweep work
/// regardless of snapshot growth.
const WORKER_MAX_BATCH: usize = MAX_SNAPSHOT_PROCESSES;
/// Wall-clock budget per sweep; stops opening further candidates when spent.
const WORKER_MAX_SWEEP: std::time::Duration = std::time::Duration::from_secs(5);
/// Consecutive bridge/API failures before the worker opens a backoff window.
const WORKER_FAILURE_THRESHOLD: u32 = 3;
/// How long the worker skips sweeps after repeated failures.
const WORKER_BACKOFF: std::time::Duration = std::time::Duration::from_secs(300);
const THROTTLE_TARGETS: &[&str] = &["SearchIndexer.exe", "backgroundTaskHost.exe"];
const PROTECTED_PROCESS_NAMES: &[&str] = &[
    "MsMpEng.exe",
    "SecurityHealthService.exe",
    "TrustedInstaller.exe",
    "TiWorker.exe",
    "MoUsoCoreWorker.exe",
    "lsass.exe",
    "csrss.exe",
    "wininit.exe",
    "services.exe",
    "smss.exe",
    "winlogon.exe",
    "dwm.exe",
];

/// Registry key for persisting OS Turbo state.
///
/// S50 fix: this MUST be HKLM, not HKCU. `set_os_turbo` runs inside the
/// elevated bridge (SYSTEM context), so an HKCU write landed in the SYSTEM
/// account's hive while `get_os_turbo` read the logged-in user's HKCU — the
/// flag was therefore always "disabled" after an app restart. HKLM is
/// machine-wide and visible to both contexts (same approach as
/// performance.rs with `HKLM\SOFTWARE\MI\PerformanceMode`).
#[cfg(windows)]
const OS_TURBO_REG_KEY: &str = r"SOFTWARE\MiControl\OSTurbo";

#[cfg(windows)]
const OS_TURBO_SNAPSHOT_FILE: &str = "os_turbo_recovery.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ProcessIdentitySnapshot {
    pid: u32,
    creation_time: u64,
    process_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ProcessPowerStateSnapshot {
    version: u32,
    control_mask: u32,
    state_mask: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct ProcessRecoverySnapshot {
    identity: ProcessIdentitySnapshot,
    original_state: ProcessPowerStateSnapshot,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct OsTurboRecoverySnapshot {
    version: u32,
    original_power_scheme: u128,
    processes: Vec<ProcessRecoverySnapshot>,
}

fn is_throttle_candidate(process_name: &str) -> bool {
    THROTTLE_TARGETS
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(process_name))
        && !PROTECTED_PROCESS_NAMES
            .iter()
            .any(|protected| protected.eq_ignore_ascii_case(process_name))
}

fn snapshot_can_be_created(existing: Option<&OsTurboRecoverySnapshot>) -> bool {
    existing.is_none()
}

fn snapshot_is_valid(snapshot: &OsTurboRecoverySnapshot) -> bool {
    snapshot.version == SNAPSHOT_VERSION
        && snapshot.processes.len() <= MAX_SNAPSHOT_PROCESSES
        && snapshot.processes.iter().all(|process| {
            process.identity.pid != 0
                && process.identity.creation_time != 0
                && process.original_state.version == 1
                && is_throttle_candidate(&process.identity.process_name)
        })
}

fn process_identity_matches(
    expected: &ProcessIdentitySnapshot,
    observed_pid: u32,
    observed_creation_time: u64,
) -> bool {
    expected.pid == observed_pid && expected.creation_time == observed_creation_time
}

fn turbo_owns_power_scheme(active_scheme: u128) -> bool {
    active_scheme == HIGH_PERFORMANCE_SCHEME
}

fn should_retain_snapshot(process_restore_failed: bool, scheme_restore_failed: bool) -> bool {
    process_restore_failed || scheme_restore_failed
}

fn disable_has_recovery_state(enabled: bool, snapshot: Option<&OsTurboRecoverySnapshot>) -> bool {
    !enabled || snapshot.is_some()
}

/// OS Turbo status.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OsTurboStatus {
    pub enabled: bool,
    /// Number of background processes verified throttled by readback
    /// (from `GetProcessInformation`, never guessed).
    pub throttled_processes: u32,
    /// Current power plan GUID.
    pub power_plan: String,
    /// S39-003 measured sweep data — truthful readback counts.
    pub measured: OsTurboMeasuredStatus,
}

/// Measured results of the most recent OS Turbo sweep or user action.
///
/// S39-003: every count is derived from an actual `GetProcessInformation`
/// readback; no field is synthesized, extrapolated, or guessed.
/// `last_sweep_ms == 0` means no sweep has ever completed.
#[derive(Debug, Default, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct OsTurboMeasuredStatus {
    /// Allowlisted candidates examined during the last sweep.
    pub inspected: u32,
    /// Candidates verified in the throttled state (readback-verified).
    pub applied: u32,
    /// Candidates verified in their original (pre-enable) state.
    pub restored: u32,
    /// Candidates skipped: exited, PID reused (identity mismatch), or in a
    /// state OS Turbo does not own.
    pub skipped: u32,
    /// Open/read/write/bridge errors encountered during the last sweep.
    pub failures: u32,
    /// Wall-clock milliseconds (UNIX epoch) of the last completed sweep;
    /// 0 when no sweep has completed.
    pub last_sweep_ms: u64,
    /// Whether the periodic worker loop is currently alive.
    pub worker_active: bool,
}

/// Enable or disable OS Turbo.
pub fn set_os_turbo(enabled: bool) -> HardwareResult<OsTurboStatus> {
    if enabled {
        enable_os_turbo()
    } else {
        disable_os_turbo()
    }
}

/// Get the current OS Turbo state.
pub fn get_os_turbo() -> HardwareResult<OsTurboStatus> {
    #[cfg(windows)]
    {
        let enabled = read_enabled_compat();

        let power_plan = get_active_power_plan()?;
        let mut measured = last_measured_status();
        measured.worker_active = worker().started.load(Ordering::Acquire);
        // Prefer the worker's last truthful measured sweep; fall back to a
        // fresh readback when no sweep has completed yet. Never guessed.
        let throttled = if measured.last_sweep_ms != 0 {
            measured.applied
        } else {
            count_throttled_processes()?
        };

        Ok(OsTurboStatus {
            enabled,
            throttled_processes: throttled,
            power_plan,
            measured,
        })
    }
    #[cfg(not(windows))]
    {
        Ok(OsTurboStatus {
            enabled: false,
            throttled_processes: 0,
            power_plan: String::new(),
            measured: OsTurboMeasuredStatus::default(),
        })
    }
}

/// Enable OS Turbo optimizations.
fn enable_os_turbo() -> HardwareResult<OsTurboStatus> {
    #[cfg(windows)]
    {
        let existing_snapshot = read_recovery_snapshot()?;
        if read_enabled_compat() {
            if existing_snapshot.is_none() {
                return Err(HardwareError::Other(
                    "OS Turbo is enabled but its recovery state is missing".into(),
                ));
            }
            return get_os_turbo();
        }
        if !snapshot_can_be_created(existing_snapshot.as_ref()) {
            return Err(HardwareError::Other(
                "OS Turbo recovery is pending; disable before enabling again".into(),
            ));
        }

        let snapshot = capture_recovery_snapshot()?;
        save_recovery_snapshot(&snapshot)?;

        persist_os_turbo(true)?;
        let power_plan = set_power_plan_best_performance()?;
        let desired = throttling_state_snapshot(true);
        let mut write_failed = false;
        let mut counts = SweepCounts {
            inspected: snapshot.processes.len() as u32,
            ..SweepCounts::default()
        };

        for process in &snapshot.processes {
            let handle = match open_process_for_turbo(process.identity.pid) {
                Ok(handle) => handle,
                Err(_) => {
                    counts.failures = counts.failures.saturating_add(1);
                    continue;
                }
            };

            let identity_matches = match get_process_creation_time(handle) {
                Ok(creation_time) => {
                    process_identity_matches(&process.identity, process.identity.pid, creation_time)
                }
                Err(_) => {
                    counts.failures = counts.failures.saturating_add(1);
                    unsafe {
                        windows::Win32::Foundation::CloseHandle(handle).ok();
                    }
                    continue;
                }
            };
            if !identity_matches {
                counts.skipped = counts.skipped.saturating_add(1);
                unsafe {
                    windows::Win32::Foundation::CloseHandle(handle).ok();
                }
                continue;
            }
            match read_process_power_state(handle) {
                Ok(current) if current == desired => {
                    counts.applied = counts.applied.saturating_add(1);
                }
                Ok(_) => {
                    // set_process_power_state re-verifies by readback.
                    if set_process_power_state(handle, &desired).is_ok() {
                        counts.applied = counts.applied.saturating_add(1);
                    } else {
                        counts.failures = counts.failures.saturating_add(1);
                        write_failed = true;
                    }
                }
                Err(_) => counts.failures = counts.failures.saturating_add(1),
            }

            unsafe {
                windows::Win32::Foundation::CloseHandle(handle).ok();
            }
        }

        if write_failed {
            return Err(HardwareError::Other(
                "OS Turbo partially applied; recovery state was retained".into(),
            ));
        }

        Ok(OsTurboStatus {
            enabled: true,
            throttled_processes: counts.applied,
            power_plan,
            measured: OsTurboMeasuredStatus {
                inspected: counts.inspected,
                applied: counts.applied,
                restored: counts.restored,
                skipped: counts.skipped,
                failures: counts.failures,
                last_sweep_ms: sweep_timestamp_ms(),
                worker_active: false,
            },
        })
    }
    #[cfg(not(windows))]
    {
        Ok(OsTurboStatus {
            enabled: false,
            throttled_processes: 0,
            power_plan: String::new(),
            measured: OsTurboMeasuredStatus::default(),
        })
    }
}

/// Disable OS Turbo optimizations.
fn disable_os_turbo() -> HardwareResult<OsTurboStatus> {
    #[cfg(windows)]
    {
        let snapshot = read_recovery_snapshot()?;
        let enabled = read_enabled_compat();
        if !disable_has_recovery_state(enabled, snapshot.as_ref()) {
            return Err(HardwareError::Other(
                "OS Turbo cannot be disabled because recovery state is missing".into(),
            ));
        }
        let Some(snapshot) = snapshot else {
            return get_os_turbo();
        };

        let mut process_restore_failed = false;
        let mut counts = SweepCounts {
            inspected: snapshot.processes.len() as u32,
            ..SweepCounts::default()
        };
        for process in &snapshot.processes {
            let handle = match open_process_for_turbo(process.identity.pid) {
                Ok(handle) => handle,
                Err(_) => {
                    // Exited/unopenable: skipped, not a failure (S39-002
                    // restore contract: skip exited or unverifiable).
                    counts.skipped = counts.skipped.saturating_add(1);
                    continue;
                }
            };

            let identity_matches = match get_process_creation_time(handle) {
                Ok(creation_time) => {
                    process_identity_matches(&process.identity, process.identity.pid, creation_time)
                }
                Err(_) => {
                    // Cannot verify identity — treat as restore incomplete.
                    process_restore_failed = true;
                    counts.skipped = counts.skipped.saturating_add(1);
                    unsafe {
                        windows::Win32::Foundation::CloseHandle(handle).ok();
                    }
                    continue;
                }
            };
            if !identity_matches {
                counts.skipped = counts.skipped.saturating_add(1);
                unsafe {
                    windows::Win32::Foundation::CloseHandle(handle).ok();
                }
                continue;
            }
            match read_process_power_state(handle) {
                Ok(current) if current == process.original_state => {
                    counts.restored = counts.restored.saturating_add(1);
                }
                Ok(current) if current == throttling_state_snapshot(true) => {
                    if set_process_power_state(handle, &process.original_state).is_ok() {
                        counts.restored = counts.restored.saturating_add(1);
                    } else {
                        counts.failures = counts.failures.saturating_add(1);
                        process_restore_failed = true;
                    }
                }
                Ok(_) => counts.skipped = counts.skipped.saturating_add(1),
                Err(_) => {
                    counts.failures = counts.failures.saturating_add(1);
                    process_restore_failed = true;
                }
            }

            unsafe {
                windows::Win32::Foundation::CloseHandle(handle).ok();
            }
        }

        let mut scheme_restore_failed = false;
        let active_scheme = get_active_power_scheme_guid()?.to_u128();
        if turbo_owns_power_scheme(active_scheme)
            && snapshot.original_power_scheme != HIGH_PERFORMANCE_SCHEME
            && set_active_power_scheme(windows::core::GUID::from_u128(
                snapshot.original_power_scheme,
            ))
            .is_err()
        {
            scheme_restore_failed = true;
        }

        if should_retain_snapshot(process_restore_failed, scheme_restore_failed) {
            return Err(HardwareError::Other(
                "OS Turbo restore was incomplete; recovery state was retained".into(),
            ));
        }

        persist_os_turbo(false)?;
        remove_recovery_snapshot()?;
        let power_plan = get_active_power_plan()?;
        Ok(OsTurboStatus {
            enabled: false,
            throttled_processes: 0,
            power_plan,
            measured: OsTurboMeasuredStatus {
                inspected: counts.inspected,
                applied: 0,
                restored: counts.restored,
                skipped: counts.skipped,
                failures: counts.failures,
                last_sweep_ms: sweep_timestamp_ms(),
                worker_active: false,
            },
        })
    }
    #[cfg(not(windows))]
    {
        Ok(OsTurboStatus {
            enabled: false,
            throttled_processes: 0,
            power_plan: String::new(),
            measured: OsTurboMeasuredStatus::default(),
        })
    }
}

/// Set the active power plan to "Best Performance".
fn set_power_plan_best_performance() -> HardwareResult<String> {
    #[cfg(windows)]
    {
        set_active_power_scheme(windows::core::GUID::from_u128(HIGH_PERFORMANCE_SCHEME))?;
        get_active_power_plan()
    }
    #[cfg(not(windows))]
    {
        Ok(String::new())
    }
}

#[cfg(windows)]
fn set_active_power_scheme(target: windows::core::GUID) -> HardwareResult<windows::core::GUID> {
    use std::os::windows::process::CommandExt;
    use windows::Win32::System::Power::PowerSetActiveScheme;

    let result = unsafe { PowerSetActiveScheme(None, Some(&target)) };
    if result.0 != 0 {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let target_text = format!("{target:?}");
        let output = std::process::Command::new("powercfg")
            .args(["/setactive", target_text.as_str()])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|_| HardwareError::Other("Power scheme write failed".into()))?;
        if !output.status.success() {
            return Err(HardwareError::Other(format!(
                "Power scheme write failed: 0x{:08X}",
                result.0
            )));
        }
    }

    let actual = get_active_power_scheme_guid()?;
    if actual != target {
        return Err(HardwareError::Other(
            "Power scheme readback did not match the requested scheme".into(),
        ));
    }
    Ok(actual)
}

fn get_active_power_plan() -> HardwareResult<String> {
    #[cfg(windows)]
    {
        let guid = get_active_power_scheme_guid()?;
        let high_perf = windows::core::GUID::from_u128(HIGH_PERFORMANCE_SCHEME);
        let balanced = windows::core::GUID::from_u128(0x381b4222_f694_41f0_9685_ff5bb260df2e);
        let power_saver = windows::core::GUID::from_u128(0xa1841308_3541_4fab_bc81_f71556f20b4a);

        if guid == high_perf {
            Ok("Best Performance".to_string())
        } else if guid == balanced {
            Ok("Balanced".to_string())
        } else if guid == power_saver {
            Ok("Best Power Efficiency".to_string())
        } else {
            Ok(format!("{guid:?}"))
        }
    }
    #[cfg(not(windows))]
    {
        Ok(String::new())
    }
}

#[cfg(all(windows, test))]
fn execution_speed_throttling_state(
) -> windows::Win32::System::Threading::PROCESS_POWER_THROTTLING_STATE {
    use windows::Win32::System::Threading::{
        PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE,
    };

    PROCESS_POWER_THROTTLING_STATE {
        Version: 1,
        ControlMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
        StateMask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    }
}

#[cfg(all(windows, test))]
fn enable_execution_speed_throttling(
    handle: windows::Win32::Foundation::HANDLE,
) -> Result<(), i32> {
    set_process_power_state(handle, &throttling_state_snapshot(true))
}

#[cfg(windows)]
fn set_process_power_state(
    handle: windows::Win32::Foundation::HANDLE,
    state: &ProcessPowerStateSnapshot,
) -> Result<(), i32> {
    use windows::Win32::System::Threading::{
        ProcessPowerThrottling, SetProcessInformation, PROCESS_POWER_THROTTLING_STATE,
    };

    let native_state = PROCESS_POWER_THROTTLING_STATE {
        Version: state.version,
        ControlMask: state.control_mask,
        StateMask: state.state_mask,
    };
    unsafe {
        SetProcessInformation(
            handle,
            ProcessPowerThrottling,
            &native_state as *const _ as *const core::ffi::c_void,
            std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
        .map_err(|error| error.code().0)?;
    }

    let observed = read_process_power_state(handle)?;
    if &observed == state {
        Ok(())
    } else {
        Err(0x1)
    }
}

#[cfg(windows)]
fn read_process_power_state(
    handle: windows::Win32::Foundation::HANDLE,
) -> Result<ProcessPowerStateSnapshot, i32> {
    use windows::Win32::System::Threading::{
        GetProcessInformation, ProcessPowerThrottling, PROCESS_POWER_THROTTLING_STATE,
    };

    let mut observed = PROCESS_POWER_THROTTLING_STATE {
        Version: 1,
        ControlMask: 0,
        StateMask: 0,
    };
    unsafe {
        GetProcessInformation(
            handle,
            ProcessPowerThrottling,
            &mut observed as *mut _ as *mut core::ffi::c_void,
            std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
        )
        .map_err(|error| error.code().0)?;
    }
    Ok(ProcessPowerStateSnapshot {
        version: observed.Version,
        control_mask: observed.ControlMask,
        state_mask: observed.StateMask,
    })
}

fn throttling_state_snapshot(enabled: bool) -> ProcessPowerStateSnapshot {
    #[cfg(windows)]
    {
        use windows::Win32::System::Threading::PROCESS_POWER_THROTTLING_EXECUTION_SPEED;
        ProcessPowerStateSnapshot {
            version: 1,
            control_mask: PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
            state_mask: if enabled {
                PROCESS_POWER_THROTTLING_EXECUTION_SPEED
            } else {
                0
            },
        }
    }
    #[cfg(not(windows))]
    {
        ProcessPowerStateSnapshot {
            version: 1,
            control_mask: 1,
            state_mask: u32::from(enabled),
        }
    }
}

#[cfg(windows)]
fn open_process_for_turbo(pid: u32) -> Result<windows::Win32::Foundation::HANDLE, i32> {
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION,
    };

    unsafe {
        OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SET_INFORMATION,
            false,
            pid,
        )
        .map_err(|error| error.code().0)
    }
}

#[cfg(windows)]
fn get_process_creation_time(handle: windows::Win32::Foundation::HANDLE) -> Result<u64, i32> {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::Threading::GetProcessTimes;

    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe {
        GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user)
            .map_err(|error| error.code().0)?;
    }
    Ok((u64::from(creation.dwHighDateTime) << 32) | u64::from(creation.dwLowDateTime))
}

#[cfg(windows)]
fn get_active_power_scheme_guid() -> HardwareResult<windows::core::GUID> {
    use windows::Win32::System::Power::PowerGetActiveScheme;

    let mut guid_ptr = std::ptr::null_mut();
    let result = unsafe { PowerGetActiveScheme(None, &mut guid_ptr) };
    if result.0 != 0 || guid_ptr.is_null() {
        return Err(HardwareError::Other(format!(
            "Power scheme read failed: 0x{:08X}",
            result.0
        )));
    }

    let guid = unsafe { *guid_ptr };
    windows_targets::link!(
        "kernel32.dll"
        "system"
        fn LocalFree(hmem: *mut std::ffi::c_void) -> *mut std::ffi::c_void
    );
    let _ = unsafe { LocalFree(guid_ptr as *mut std::ffi::c_void) };
    Ok(guid)
}

/// Count currently throttled processes (informational).
fn count_throttled_processes() -> HardwareResult<u32> {
    #[cfg(windows)]
    {
        let processes = enumerate_processes()?;
        let mut count = 0u32;
        for (pid, name) in processes {
            if !is_throttle_candidate(&name) {
                continue;
            }
            let handle = match open_process_for_turbo(pid) {
                Ok(handle) => handle,
                Err(_) => continue,
            };
            if let Ok(state) = read_process_power_state(handle) {
                if state == throttling_state_snapshot(true) {
                    count = count.saturating_add(1);
                }
            }
            unsafe {
                windows::Win32::Foundation::CloseHandle(handle).ok();
            }
        }
        Ok(count)
    }
    #[cfg(not(windows))]
    {
        Ok(0)
    }
}

/// Enumerate running processes via WMI.
#[cfg(windows)]
fn enumerate_processes() -> anyhow::Result<Vec<(u32, String)>> {
    use crate::hw::wmi_cache;
    use crate::util::wmi_extract;
    use std::collections::HashMap;

    wmi_cache::with_cimv2(|wmi| {
        let processes: Vec<HashMap<String, wmi::Variant>> =
            wmi.raw_query("SELECT ProcessId, Name FROM Win32_Process")?;

        let result = processes
            .into_iter()
            .filter_map(|p| {
                let pid = wmi_extract::extract_u32(&p, "ProcessId")?;
                let name = wmi_extract::extract_string(&p, "Name")?;
                Some((pid, name))
            })
            .collect();

        Ok(result)
    })
    .map_err(|e| anyhow::anyhow!(e))
}

#[cfg(windows)]
fn capture_recovery_snapshot() -> HardwareResult<OsTurboRecoverySnapshot> {
    let original_power_scheme = get_active_power_scheme_guid()?.to_u128();
    let mut candidates: Vec<(u32, String)> = enumerate_processes()?
        .into_iter()
        .filter(|(_, name)| is_throttle_candidate(name))
        .collect();
    candidates.sort_by(|left, right| left.1.cmp(&right.1).then(left.0.cmp(&right.0)));

    let mut processes = Vec::with_capacity(candidates.len().min(MAX_SNAPSHOT_PROCESSES));
    for (pid, process_name) in candidates.into_iter().take(MAX_SNAPSHOT_PROCESSES) {
        let handle = match open_process_for_turbo(pid) {
            Ok(handle) => handle,
            Err(_) => continue,
        };
        let captured = get_process_creation_time(handle)
            .ok()
            .and_then(|creation_time| {
                read_process_power_state(handle)
                    .ok()
                    .map(|state| (creation_time, state))
            });
        unsafe {
            windows::Win32::Foundation::CloseHandle(handle).ok();
        }
        if let Some((creation_time, original_state)) = captured {
            processes.push(ProcessRecoverySnapshot {
                identity: ProcessIdentitySnapshot {
                    pid,
                    creation_time,
                    process_name,
                },
                original_state,
            });
        }
    }

    Ok(OsTurboRecoverySnapshot {
        version: SNAPSHOT_VERSION,
        original_power_scheme,
        processes,
    })
}

#[cfg(windows)]
fn recovery_snapshot_path() -> std::path::PathBuf {
    let base = std::env::var_os("ProgramData").unwrap_or_else(|| "C:\\ProgramData".into());
    std::path::PathBuf::from(base)
        .join("MiControl")
        .join(OS_TURBO_SNAPSHOT_FILE)
}

#[cfg(windows)]
fn read_recovery_snapshot() -> HardwareResult<Option<OsTurboRecoverySnapshot>> {
    let path = recovery_snapshot_path();
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(HardwareError::Other(
                "OS Turbo recovery snapshot cannot be read".into(),
            ));
        }
    };
    let snapshot: OsTurboRecoverySnapshot = serde_json::from_slice(&bytes)
        .map_err(|_| HardwareError::Other("OS Turbo recovery snapshot is invalid".into()))?;
    if !snapshot_is_valid(&snapshot) {
        return Err(HardwareError::Other(
            "OS Turbo recovery snapshot is unsupported or too large".into(),
        ));
    }
    Ok(Some(snapshot))
}

#[cfg(windows)]
fn save_recovery_snapshot(snapshot: &OsTurboRecoverySnapshot) -> HardwareResult<()> {
    if !snapshot_is_valid(snapshot) {
        return Err(HardwareError::Other(
            "OS Turbo recovery snapshot is unsupported or too large".into(),
        ));
    }
    let path = recovery_snapshot_path();
    if path.exists() {
        return Err(HardwareError::Other(
            "OS Turbo recovery snapshot already exists".into(),
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| HardwareError::Other("OS Turbo recovery snapshot path is invalid".into()))?;
    std::fs::create_dir_all(parent).map_err(|_| {
        HardwareError::Other("OS Turbo recovery directory cannot be created".into())
    })?;

    let temporary_path = parent.join(format!(
        "{}.{}.tmp",
        OS_TURBO_SNAPSHOT_FILE,
        std::process::id()
    ));
    let bytes = serde_json::to_vec(snapshot).map_err(|_| {
        HardwareError::Other("OS Turbo recovery snapshot cannot be serialized".into())
    })?;
    let write_result = (|| -> Result<(), String> {
        use std::io::Write;

        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
            .map_err(|_| "temporary snapshot cannot be created".to_string())?;
        file.write_all(&bytes)
            .map_err(|_| "temporary snapshot cannot be written".to_string())?;
        file.sync_all()
            .map_err(|_| "temporary snapshot cannot be flushed".to_string())?;
        crate::util::auth::restrict_file_acl(&temporary_path)?;
        std::fs::rename(&temporary_path, &path)
            .map_err(|_| "recovery snapshot cannot be committed".to_string())?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temporary_path);
        return Err(HardwareError::Other(
            "OS Turbo recovery snapshot cannot be committed".into(),
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn remove_recovery_snapshot() -> HardwareResult<()> {
    match std::fs::remove_file(recovery_snapshot_path()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(HardwareError::Other(
            "OS Turbo recovery snapshot cannot be removed".into(),
        )),
    }
}

/// Persist OS Turbo state to registry.
fn persist_os_turbo(enabled: bool) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;

        let key = RegKeyGuard::create_write(HKEY_LOCAL_MACHINE, OS_TURBO_REG_KEY)
            .map_err(|e| HardwareError::Registry(format!("Create OS Turbo key: {e}")))?;

        key.write_u32("Enabled", if enabled { 1 } else { 0 })
            .map_err(|e| HardwareError::Registry(format!("Write OS Turbo enabled: {e}")))?;

        Ok(())
    }
    #[cfg(not(windows))]
    {
        Ok(())
    }
}

pub fn restore_os_turbo() {}

/// Read the HKLM Enabled flag; `None` when the key/value does not exist.
#[cfg(windows)]
fn read_hklm_enabled() -> Option<bool> {
    use crate::util::registry::RegKeyGuard;
    use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
    RegKeyGuard::open_read(HKEY_LOCAL_MACHINE, OS_TURBO_REG_KEY)
        .ok()
        .flatten()
        .and_then(|k| k.read_u32("Enabled").ok().flatten())
        .map(|v| v != 0)
}

#[cfg(windows)]
fn read_legacy_hkcu_enabled() -> Option<bool> {
    use crate::util::registry::RegKeyGuard;
    use windows::Win32::System::Registry::HKEY_CURRENT_USER;
    RegKeyGuard::open_read(HKEY_CURRENT_USER, OS_TURBO_REG_KEY)
        .ok()
        .flatten()
        .and_then(|k| k.read_u32("Enabled").ok().flatten())
        .map(|v| v != 0)
}

#[cfg(windows)]
fn read_enabled_compat() -> bool {
    read_hklm_enabled()
        .or_else(read_legacy_hkcu_enabled)
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn read_enabled_compat() -> bool {
    false
}

// ── S39-003: measured status + bounded periodic worker ───────────────────────

/// Class of a per-process readback outcome during a sweep.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReadbackOutcome {
    /// Verified in the throttled state.
    Applied,
    /// Verified in the original (pre-enable) state.
    Restored,
    /// Valid identity but in a state Turbo does not own (foreign mask).
    Other,
    /// Exited or PID reused — cannot be attributed.
    Skipped,
    /// Open/read/set error.
    Failed,
}

/// Truthful, per-sweep aggregate of readback outcomes.
///
/// `inspected` equals `applied + restored + other + skipped + failures`
/// (saturated); no field is guessed or extrapolated, and a disabled Turbo
/// produces an all-default aggregate (no work, no fabricated counts).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct SweepCounts {
    inspected: u32,
    applied: u32,
    restored: u32,
    other: u32,
    skipped: u32,
    failures: u32,
}

impl SweepCounts {
    /// Identity-valid processes that should be throttled but are not.
    fn driftable(&self) -> u32 {
        self.restored.saturating_add(self.other)
    }

    fn drift_detected(&self) -> bool {
        self.driftable() > 0
    }
}

impl OsTurboMeasuredStatus {
    /// Collapse a measured status back into sweep counts (used by the worker
    /// after a bridge reapply response).
    #[cfg_attr(not(any(windows, test)), allow(dead_code))]
    fn as_counts(&self) -> SweepCounts {
        SweepCounts {
            inspected: self.inspected,
            applied: self.applied,
            restored: self.restored,
            other: 0,
            skipped: self.skipped,
            failures: self.failures,
        }
    }
}

/// Aggregate readback outcomes into counts, capped at `limit` items.
fn aggregate_counts(items: impl IntoIterator<Item = ReadbackOutcome>, limit: usize) -> SweepCounts {
    let mut counts = SweepCounts::default();
    for item in items.into_iter().take(limit) {
        counts.inspected = counts.inspected.saturating_add(1);
        match item {
            ReadbackOutcome::Applied => counts.applied = counts.applied.saturating_add(1),
            ReadbackOutcome::Restored => counts.restored = counts.restored.saturating_add(1),
            ReadbackOutcome::Other => counts.other = counts.other.saturating_add(1),
            ReadbackOutcome::Skipped => counts.skipped = counts.skipped.saturating_add(1),
            ReadbackOutcome::Failed => counts.failures = counts.failures.saturating_add(1),
        }
    }
    counts
}

/// Single-owner state for the periodic worker.
struct WorkerState {
    /// True while the worker loop task is alive.
    started: std::sync::atomic::AtomicBool,
    /// Set to cancel the loop (disable / shutdown).
    stop: std::sync::atomic::AtomicBool,
    /// Single-flight gate: at most one sweep at a time.
    busy: std::sync::atomic::AtomicBool,
    /// Consecutive bridge/API failures feeding the backoff decision.
    consecutive_failures: std::sync::atomic::AtomicU32,
    /// When set, sweeps are skipped (bounded backoff window).
    backoff_until: std::sync::Mutex<Option<std::time::Instant>>,
    /// Latest truthful measured status.
    measured: std::sync::Mutex<OsTurboMeasuredStatus>,
}

impl Default for WorkerState {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkerState {
    fn new() -> Self {
        Self {
            started: std::sync::atomic::AtomicBool::new(false),
            stop: std::sync::atomic::AtomicBool::new(false),
            busy: std::sync::atomic::AtomicBool::new(false),
            consecutive_failures: std::sync::atomic::AtomicU32::new(0),
            backoff_until: std::sync::Mutex::new(None),
            measured: std::sync::Mutex::new(OsTurboMeasuredStatus::default()),
        }
    }
    /// Claims the single-flight gate. `false` means a sweep is already running.
    fn try_begin_sweep(&self) -> bool {
        self.busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    fn end_sweep(&self) {
        self.busy.store(false, Ordering::Release);
    }

    /// Record a failure; repeated failures open a bounded backoff window.
    fn note_failure(&self) {
        let streak = self.consecutive_failures.fetch_add(1, Ordering::AcqRel) + 1;
        if streak >= WORKER_FAILURE_THRESHOLD {
            *self.backoff_until.lock().unwrap_or_else(|p| p.into_inner()) =
                Some(std::time::Instant::now() + WORKER_BACKOFF);
        }
    }

    fn note_success(&self) {
        self.consecutive_failures.store(0, Ordering::Release);
        *self.backoff_until.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    fn backoff_active(&self) -> bool {
        self.backoff_active_at(std::time::Instant::now())
    }

    fn backoff_active_at(&self, now: std::time::Instant) -> bool {
        self.backoff_until
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .map(|until| until > now)
            .unwrap_or(false)
    }

    fn record(&self, sweep: &SweepCounts, worker_active: bool) {
        *self.measured.lock().unwrap_or_else(|p| p.into_inner()) = OsTurboMeasuredStatus {
            inspected: sweep.inspected,
            applied: sweep.applied,
            restored: sweep.restored,
            skipped: sweep.skipped,
            failures: sweep.failures,
            last_sweep_ms: sweep_timestamp_ms(),
            worker_active,
        };
    }

    /// Adopt a status computed by an authoritative user action. Truthful
    /// because `set_os_turbo` (bridge) and `refresh_os_turbo` (bridge) both
    /// derive counts from real readback.
    fn record_user_action(&self, status: &OsTurboStatus, worker_active: bool) {
        *self.measured.lock().unwrap_or_else(|p| p.into_inner()) = OsTurboMeasuredStatus {
            inspected: status.measured.inspected,
            applied: status.measured.applied,
            restored: status.measured.restored,
            skipped: status.measured.skipped,
            failures: status.measured.failures,
            last_sweep_ms: status.measured.last_sweep_ms,
            worker_active,
        };
    }

    fn measured(&self) -> OsTurboMeasuredStatus {
        self.measured
            .lock()
            .map(|value| value.clone())
            .unwrap_or_default()
    }

    fn set_active(&self, active: bool) {
        if let Ok(mut value) = self.measured.lock() {
            value.worker_active = active;
        }
    }

    fn stop_requested(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }

    fn request_stop(&self) {
        self.stop.store(true, Ordering::Release);
    }
}

static WORKER: std::sync::OnceLock<WorkerState> = std::sync::OnceLock::new();

fn worker() -> &'static WorkerState {
    WORKER.get_or_init(WorkerState::new)
}

/// Whether the periodic worker loop is currently alive.
pub fn worker_active() -> bool {
    worker().started.load(Ordering::Acquire)
}

/// Latest truthful measured status from the worker (default when none yet).
pub fn last_measured_status() -> OsTurboMeasuredStatus {
    worker().measured()
}

/// Adopt a status returned by an authoritative user action (enable/disable
/// via the bridge) into the worker's measured slot. Truthful by construction:
/// `set_os_turbo` computes counts from real readback in the bridge.
pub fn note_user_action(status: &OsTurboStatus) {
    worker().record_user_action(status, worker().started.load(Ordering::Acquire));
}

/// Wall-clock milliseconds (UNIX epoch) for `last_sweep_ms`.
fn sweep_timestamp_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

/// Pure gate: may a sweep begin right now?
fn sweep_may_begin(disabled: bool, stop: bool, busy: bool, backoff_active: bool) -> bool {
    !disabled && !stop && !busy && !backoff_active
}

/// Start the bounded periodic worker. Idempotent; safe to call while OS Turbo
/// is disabled (the sweep self-gates on the persisted enabled flag).
pub fn start_worker() {
    let state = worker();
    state.stop.store(false, Ordering::Release);
    if state.started.swap(true, Ordering::AcqRel) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        // First sweep immediately so measured status exists right after enable.
        run_sweep().await;
        let mut interval = tokio::time::interval(WORKER_INTERVAL);
        interval.tick().await;
        loop {
            interval.tick().await;
            if worker().stop_requested() {
                worker().started.store(false, Ordering::Release);
                worker().set_active(false);
                return;
            }
            run_sweep().await;
        }
    });
}

/// Cancel the periodic worker (disable / shutdown). The loop observes the flag
/// at its next tick and exits within one interval; a running sweep finishes its
/// bounded work first and no thread leaks.
pub fn stop_worker() {
    worker().request_stop();
}

/// Run one bounded measurement sweep (plus a bounded bridge reapply on drift).
async fn run_sweep() {
    let state = worker();
    if !sweep_may_begin(
        !read_enabled_compat(),
        state.stop_requested(),
        state.busy.load(Ordering::Acquire),
        state.backoff_active(),
    ) {
        // Must not run while OS Turbo is disabled (no work, no new timestamp),
        // cancelled, already busy (single-flight), or in backoff.
        return;
    }
    // Atomic claim after the cheap gate — guarantees no overlapping sweeps.
    if !state.try_begin_sweep() {
        return;
    }

    let mut sweep = measure_sweep();
    #[cfg(windows)]
    {
        if sweep.drift_detected() && crate::elev_bridge::is_bridge_service_available() {
            // Privileged writes go through the authenticated bridge ONLY —
            // never UAC, never a scheduled-task fallback.
            let refreshed = crate::elev_bridge::run_elevated_bridge_only(
                "refresh_os_turbo",
                serde_json::json!({}),
            )
            .await
            .and_then(|data| {
                serde_json::from_value::<OsTurboStatus>(data).map_err(|e| e.to_string())
            });
            match refreshed {
                Ok(status) => {
                    state.note_success();
                    sweep = status.measured.as_counts();
                }
                Err(_) => {
                    // Repeated bridge failures open a bounded backoff window.
                    sweep.failures = sweep.failures.saturating_add(1);
                    state.note_failure();
                }
            }
        } else {
            state.note_success();
        }
    }
    #[cfg(not(windows))]
    {
        state.note_success();
    }

    state.end_sweep();
    state.record(&sweep, true);
}

/// Measure allowlisted candidates strictly by readback. Read-only — never
/// writes, never dispatches; bounded by batch and sweep duration.
#[cfg(windows)]
fn measure_sweep() -> SweepCounts {
    let Some(snapshot) = read_recovery_snapshot().ok().flatten() else {
        // Enabled without recovery state: nothing to measure truthfully.
        return SweepCounts::default();
    };
    let desired = throttling_state_snapshot(true);
    let deadline = std::time::Instant::now() + WORKER_MAX_SWEEP;
    let mut outcomes = Vec::with_capacity(snapshot.processes.len().min(WORKER_MAX_BATCH));
    for process in snapshot.processes.iter().take(WORKER_MAX_BATCH) {
        if std::time::Instant::now() >= deadline {
            break;
        }
        outcomes.push(readback_outcome(
            process.identity.pid,
            &process.identity,
            &process.original_state,
            &desired,
        ));
    }
    aggregate_counts(outcomes, WORKER_MAX_BATCH)
}

#[cfg(not(windows))]
fn measure_sweep() -> SweepCounts {
    SweepCounts::default()
}

/// Open a process with read-only rights (`PROCESS_QUERY_LIMITED_INFORMATION`).
/// The worker never requests write/set access — measurement only.
#[cfg(windows)]
fn open_process_for_read(pid: u32) -> Result<windows::Win32::Foundation::HANDLE, i32> {
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    unsafe {
        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).map_err(|error| error.code().0)
    }
}

/// Read-only per-process readback used by the measure sweep.
#[cfg(windows)]
fn readback_outcome(
    pid: u32,
    identity: &ProcessIdentitySnapshot,
    original: &ProcessPowerStateSnapshot,
    desired: &ProcessPowerStateSnapshot,
) -> ReadbackOutcome {
    let handle = match open_process_for_read(pid) {
        Ok(handle) => handle,
        Err(_) => return ReadbackOutcome::Failed,
    };
    let creation_time = match get_process_creation_time(handle) {
        Ok(creation_time) => creation_time,
        Err(_) => {
            unsafe {
                windows::Win32::Foundation::CloseHandle(handle).ok();
            }
            return ReadbackOutcome::Failed;
        }
    };
    if !process_identity_matches(identity, pid, creation_time) {
        unsafe {
            windows::Win32::Foundation::CloseHandle(handle).ok();
        }
        return ReadbackOutcome::Skipped;
    }
    let outcome = match read_process_power_state(handle) {
        Ok(state) if state == *desired => ReadbackOutcome::Applied,
        Ok(state) if state == *original => ReadbackOutcome::Restored,
        Ok(_) => ReadbackOutcome::Other,
        Err(_) => ReadbackOutcome::Failed,
    };
    unsafe {
        windows::Win32::Foundation::CloseHandle(handle).ok();
    }
    outcome
}

/// Re-apply throttling to recovery-snapshot processes. Runs only inside the
/// authenticated bridge (no UAC, no scheduled-task fallback). Bounded by the
/// snapshot size, the batch cap and the per-sweep duration budget. Only
/// touches allowlisted, identity-verified processes — never protected,
/// security or update processes, and never EC/fan/power-limit registers.
pub fn refresh_os_turbo() -> HardwareResult<OsTurboStatus> {
    #[cfg(windows)]
    {
        if !read_enabled_compat() {
            return get_os_turbo();
        }
        let snapshot = read_recovery_snapshot()?.ok_or_else(|| {
            HardwareError::Other("OS Turbo is enabled but its recovery state is missing".into())
        })?;
        let desired = throttling_state_snapshot(true);
        let deadline = std::time::Instant::now() + WORKER_MAX_SWEEP;
        let mut outcomes = Vec::with_capacity(snapshot.processes.len().min(WORKER_MAX_BATCH));
        for process in snapshot.processes.iter().take(WORKER_MAX_BATCH) {
            if std::time::Instant::now() >= deadline {
                break;
            }
            outcomes.push(throttle_readback(process, &desired));
        }
        let counts = aggregate_counts(outcomes, WORKER_MAX_BATCH);
        let power_plan = get_active_power_plan()?;
        Ok(OsTurboStatus {
            enabled: true,
            throttled_processes: counts.applied,
            power_plan,
            measured: OsTurboMeasuredStatus {
                inspected: counts.inspected,
                applied: counts.applied,
                restored: counts.restored,
                skipped: counts.skipped,
                failures: counts.failures,
                last_sweep_ms: sweep_timestamp_ms(),
                worker_active: false,
            },
        })
    }
    #[cfg(not(windows))]
    {
        get_os_turbo()
    }
}

/// Bridge-side per-process throttle-with-readback-verify helper.
#[cfg(windows)]
fn throttle_readback(
    process: &ProcessRecoverySnapshot,
    desired: &ProcessPowerStateSnapshot,
) -> ReadbackOutcome {
    let handle = match open_process_for_turbo(process.identity.pid) {
        Ok(handle) => handle,
        Err(_) => return ReadbackOutcome::Failed,
    };
    let creation_time = match get_process_creation_time(handle) {
        Ok(creation_time) => creation_time,
        Err(_) => {
            unsafe {
                windows::Win32::Foundation::CloseHandle(handle).ok();
            }
            return ReadbackOutcome::Failed;
        }
    };
    if !process_identity_matches(&process.identity, process.identity.pid, creation_time) {
        unsafe {
            windows::Win32::Foundation::CloseHandle(handle).ok();
        }
        return ReadbackOutcome::Skipped;
    }
    let outcome = match read_process_power_state(handle) {
        Ok(current) if current == *desired => ReadbackOutcome::Applied,
        Ok(_) => {
            // set_process_power_state re-verifies by readback.
            if set_process_power_state(handle, desired).is_ok() {
                ReadbackOutcome::Applied
            } else {
                ReadbackOutcome::Failed
            }
        }
        Err(_) => ReadbackOutcome::Failed,
    };
    unsafe {
        windows::Win32::Foundation::CloseHandle(handle).ok();
    }
    outcome
}

#[cfg(test)]
mod tests {
    fn sample_snapshot() -> super::OsTurboRecoverySnapshot {
        super::OsTurboRecoverySnapshot {
            version: super::SNAPSHOT_VERSION,
            original_power_scheme: 0x1234,
            processes: vec![super::ProcessRecoverySnapshot {
                identity: super::ProcessIdentitySnapshot {
                    pid: 42,
                    creation_time: 100,
                    process_name: "SearchIndexer.exe".to_string(),
                },
                original_state: super::ProcessPowerStateSnapshot {
                    version: 1,
                    control_mask: 1,
                    state_mask: 0,
                },
            }],
        }
    }

    #[test]
    fn snapshot_is_versioned_bounded_and_allowlisted() {
        let snapshot = sample_snapshot();
        assert!(super::snapshot_is_valid(&snapshot));
        assert!(!super::snapshot_is_valid(&super::OsTurboRecoverySnapshot {
            version: 2,
            ..snapshot
        }));

        let mut oversized = sample_snapshot();
        oversized.processes =
            vec![oversized.processes[0].clone(); super::MAX_SNAPSHOT_PROCESSES + 1];
        assert!(!super::snapshot_is_valid(&oversized));
        assert!(!super::is_throttle_candidate("MsMpEng.exe"));
        assert!(!super::is_throttle_candidate("TrustedInstaller.exe"));
    }

    #[test]
    fn process_identity_mismatch_is_not_restorable() {
        let snapshot = sample_snapshot();
        let identity = &snapshot.processes[0].identity;
        assert!(super::process_identity_matches(identity, 42, 100));
        assert!(!super::process_identity_matches(identity, 42, 101));
        assert!(!super::process_identity_matches(identity, 43, 100));
    }

    #[test]
    fn snapshot_is_not_replaced_after_first_capture() {
        let snapshot = sample_snapshot();
        assert!(super::snapshot_can_be_created(None));
        assert!(!super::snapshot_can_be_created(Some(&snapshot)));
    }

    #[test]
    fn partial_restore_failure_retains_reversible_state() {
        assert!(super::should_retain_snapshot(true, false));
        assert!(super::should_retain_snapshot(false, true));
        assert!(!super::should_retain_snapshot(false, false));
    }

    #[test]
    fn external_power_scheme_change_is_preserved() {
        assert!(super::turbo_owns_power_scheme(
            super::HIGH_PERFORMANCE_SCHEME
        ));
        assert!(!super::turbo_owns_power_scheme(0x1234));
    }

    #[test]
    fn missing_prior_state_fails_only_when_turbo_is_enabled() {
        assert!(!super::disable_has_recovery_state(true, None));
        assert!(super::disable_has_recovery_state(false, None));
        let snapshot = sample_snapshot();
        assert!(super::disable_has_recovery_state(true, Some(&snapshot)));
    }

    #[cfg(windows)]
    #[test]
    fn execution_speed_state_enables_only_execution_speed_throttling() {
        use windows::Win32::System::Threading::PROCESS_POWER_THROTTLING_EXECUTION_SPEED;

        let state = super::execution_speed_throttling_state();
        assert_eq!(state.Version, 1);
        assert_eq!(state.ControlMask, PROCESS_POWER_THROTTLING_EXECUTION_SPEED);
        assert_eq!(state.StateMask, PROCESS_POWER_THROTTLING_EXECUTION_SPEED);
    }

    #[cfg(windows)]
    #[test]
    fn public_api_updates_a_disposable_child_state() {
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{
            GetProcessInformation, OpenProcess, ProcessPowerThrottling,
            PROCESS_POWER_THROTTLING_EXECUTION_SPEED, PROCESS_POWER_THROTTLING_STATE,
            PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION,
        };

        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut child = Command::new("cmd.exe")
            .args(["/C", "ping", "127.0.0.1", "-n", "30"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .expect("spawn disposable Windows child");

        let result = (|| -> Result<(), String> {
            let handle = unsafe {
                OpenProcess(
                    PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SET_INFORMATION,
                    false,
                    child.id(),
                )
            }
            .map_err(|error| format!("OpenProcess failed: 0x{:08X}", error.code().0 as u32))?;

            let original_state = super::read_process_power_state(handle).map_err(|error_code| {
                format!("GetProcessInformation failed: 0x{:08X}", error_code as u32)
            })?;

            let set_result =
                super::enable_execution_speed_throttling(handle).map_err(|error_code| {
                    format!("SetProcessInformation failed: 0x{:08X}", error_code as u32)
                });

            let mut observed = PROCESS_POWER_THROTTLING_STATE {
                Version: 1,
                ControlMask: 0,
                StateMask: 0,
            };
            let query_result = unsafe {
                GetProcessInformation(
                    handle,
                    ProcessPowerThrottling,
                    &mut observed as *mut _ as *mut core::ffi::c_void,
                    std::mem::size_of::<PROCESS_POWER_THROTTLING_STATE>() as u32,
                )
            }
            .map_err(|error| {
                format!(
                    "GetProcessInformation failed: 0x{:08X}",
                    error.code().0 as u32
                )
            });

            let restore_result =
                super::set_process_power_state(handle, &original_state).map_err(|error_code| {
                    format!(
                        "Restore SetProcessInformation failed: 0x{:08X}",
                        error_code as u32
                    )
                });
            let restored_state = super::read_process_power_state(handle).map_err(|error_code| {
                format!(
                    "Restore GetProcessInformation failed: 0x{:08X}",
                    error_code as u32
                )
            });

            unsafe {
                CloseHandle(handle).ok();
            }

            set_result?;
            query_result?;
            assert_eq!(observed.Version, 1);
            assert_eq!(
                observed.ControlMask,
                PROCESS_POWER_THROTTLING_EXECUTION_SPEED
            );
            assert_eq!(observed.StateMask, PROCESS_POWER_THROTTLING_EXECUTION_SPEED);
            restore_result?;
            assert_eq!(restored_state?, original_state);
            Ok(())
        })();

        let _ = child.kill();
        let _ = child.wait();
        result.expect("public process-power API should update disposable child");
    }

    // ── S39-003: measured status + bounded periodic worker ───────────────────

    #[test]
    fn sweep_batch_is_bounded() {
        use super::{aggregate_counts, ReadbackOutcome as Outcome, WORKER_MAX_BATCH};

        let items = std::iter::repeat_n(Outcome::Applied, WORKER_MAX_BATCH + 512);
        let counts = aggregate_counts(items, WORKER_MAX_BATCH);
        assert_eq!(counts.inspected, WORKER_MAX_BATCH as u32);
        assert_eq!(counts.applied, WORKER_MAX_BATCH as u32);
        assert_eq!(counts.failures, 0);
    }

    #[test]
    fn counts_are_truthful_readback_aggregates() {
        use super::{aggregate_counts, ReadbackOutcome as Outcome, SweepCounts};

        let items = [
            Outcome::Applied,
            Outcome::Applied,
            Outcome::Restored,
            Outcome::Other,
            Outcome::Skipped,
            Outcome::Failed,
            Outcome::Restored,
        ];
        let counts = aggregate_counts(items, items.len());
        assert_eq!(counts.inspected, 7);
        assert_eq!(counts.applied, 2);
        assert_eq!(counts.restored, 2);
        assert_eq!(counts.skipped, 1);
        assert_eq!(counts.failures, 1);
        assert_eq!(counts.other, 1);
        // Every inspected candidate lands in exactly one bucket.
        assert_eq!(
            counts.applied + counts.restored + counts.other + counts.skipped + counts.failures,
            counts.inspected
        );
        // Identity-valid processes not in the throttled state → drift.
        assert!(counts.drift_detected());

        let no_drift = SweepCounts {
            applied: 5,
            ..SweepCounts::default()
        };
        assert!(!no_drift.drift_detected());
    }

    #[test]
    fn sweep_start_is_single_flight() {
        let state = super::WorkerState::new();
        assert!(state.try_begin_sweep());
        assert!(
            !state.try_begin_sweep(),
            "overlapping sweeps must be rejected"
        );
        state.end_sweep();
        assert!(state.try_begin_sweep());
        state.end_sweep();
    }

    #[test]
    fn worker_cancellation_stops_future_sweeps() {
        use super::sweep_may_begin;

        let state = super::WorkerState::new();
        assert!(!state.stop_requested());
        state.request_stop();
        assert!(state.stop_requested());

        assert!(!sweep_may_begin(true, false, false, false)); // disabled
        assert!(!sweep_may_begin(false, true, false, false)); // stop requested
        assert!(!sweep_may_begin(false, false, true, false)); // busy
        assert!(!sweep_may_begin(false, false, false, true)); // backoff
        assert!(sweep_may_begin(false, false, false, false));
    }

    #[test]
    fn disabled_state_performs_no_sweep_work() {
        use super::{aggregate_counts, SweepCounts};

        // A disabled Turbo must not begin a sweep and must not fabricate
        // counts: the no-work aggregate is all-default.
        assert!(!super::sweep_may_begin(true, false, false, false));
        assert!(!super::sweep_may_begin(true, true, true, true));
        let counts = aggregate_counts(std::iter::empty(), 64);
        assert_eq!(counts, SweepCounts::default());
    }

    #[test]
    fn repeated_failures_open_bounded_backoff() {
        use super::WORKER_FAILURE_THRESHOLD;

        let state = super::WorkerState::new();
        for _ in 0..(WORKER_FAILURE_THRESHOLD - 1) {
            state.note_failure();
        }
        assert!(!state.backoff_active(), "below threshold no backoff");
        state.note_failure();
        assert!(state.backoff_active(), "backoff opens at the threshold");
        state.note_success();
        assert!(!state.backoff_active(), "success closes the backoff");
    }

    #[test]
    fn backoff_expires_after_its_window() {
        use std::time::{Duration, Instant};

        let state = super::WorkerState::new();
        *state.backoff_until.lock().unwrap() = Some(Instant::now() + Duration::from_secs(60));
        assert!(state.backoff_active_at(Instant::now()));
        *state.backoff_until.lock().unwrap() = Some(Instant::now() - Duration::from_secs(1));
        assert!(!state.backoff_active_at(Instant::now()));
    }

    #[test]
    fn measured_status_round_trips_to_sweep_counts() {
        let status = super::OsTurboMeasuredStatus {
            inspected: 4,
            applied: 2,
            restored: 1,
            skipped: 0,
            failures: 1,
            last_sweep_ms: 123,
            worker_active: true,
        };
        let counts = status.as_counts();
        assert_eq!(counts.inspected, 4);
        assert_eq!(counts.applied, 2);
        assert_eq!(counts.restored, 1);
        assert_eq!(counts.failures, 1);
        assert!(counts.drift_detected());
    }
}
