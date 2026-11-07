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
/// Background/indexing processes eligible for EcoQoS throttling.
///
/// S68: the original list had only TWO entries (SearchIndexer,
/// backgroundTaskHost) — on a typical machine ZERO of them were running, so
/// enabling OS Turbo throttled nothing (the "does absolutely nothing in
/// practice" report). The expanded allowlist covers the common background
/// noise on a Windows 11 laptop while staying strictly opt-in per process:
/// indexers, updaters, telemetry and helper hosts. Everything NOT on this
/// list is never touched, and PROTECTED_PROCESS_NAMES below is a second
/// safety net.
const THROTTLE_TARGETS: &[&str] = &[
    // Windows search + indexing
    "SearchIndexer.exe",
    "SearchApp.exe",
    "SearchProtocolHost.exe",
    "SearchFilterHost.exe",
    // Update/telemetry hosts
    "backgroundTaskHost.exe",
    "MoUsoCoreWorker.exe",
    "TiWorker.exe",
    "WaaSMedicSvc.exe", // host process name for the servicing stack
    "CompatTelRunner.exe",
    "OfficeClickToRun.exe",
    "UpdateHealthTools.exe",
    // OneDrive sync (background by definition)
    "OneDrive.exe",
    // Phone Link background sync
    "PhoneExperienceHost.exe",
    // Widget feed host (pure background churn)
    "Widgets.exe",
    "WidgetService.exe",
];
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

impl ProcessPowerStateSnapshot {
    /// Windows' per-process default: no throttling policy applied. Writing
    /// this state back is what "restore a process" means.
    const DEFAULT: Self = Self {
        version: 1,
        control_mask: 0,
        state_mask: 0,
    };
}

/// Health of the recovery snapshot, derived from the liveness of the
/// processes it recorded.
///
/// Since F2 the snapshot only records processes whose **original** power
/// state was not the Windows default, so on a typical machine it is empty and
/// cannot rot. When it does carry entries they must still be backed by the
/// same live process, or the record no longer describes the machine — that is
/// exactly the 2026-10-26 failure (PIDs 9400/20704 exited, the snapshot kept
/// being treated as valid for 12 days).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SnapshotHealth {
    /// No snapshot file at all.
    Missing,
    /// At least one recorded process is still alive, or nothing was recorded.
    Current,
    /// Every recorded process has exited: the snapshot must be recaptured.
    Stale,
}

/// Startup reconciliation decision. Pure so it is unit-tested directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReconcileAction {
    /// Flag and snapshot agree: do not touch the machine.
    Nothing,
    /// Flag is on but the snapshot is missing/stale: recapture and re-apply.
    Reapply,
    /// Flag is off but recovery state is still on disk: finish the disable so
    /// the power plan and the process masks return to their captured values.
    CompleteDisable,
}

fn reconcile_action(enabled: bool, health: SnapshotHealth) -> ReconcileAction {
    match (enabled, health) {
        (true, SnapshotHealth::Missing | SnapshotHealth::Stale) => ReconcileAction::Reapply,
        (false, SnapshotHealth::Current | SnapshotHealth::Stale) => {
            ReconcileAction::CompleteDisable
        }
        _ => ReconcileAction::Nothing,
    }
}

/// Count recorded entries whose process is still the same live process.
/// Returns `(alive, expired)`.
fn count_live_entries(
    entries: &[ProcessRecoverySnapshot],
    mut is_alive: impl FnMut(&ProcessIdentitySnapshot) -> bool,
) -> (u32, u32) {
    let mut alive = 0u32;
    let mut expired = 0u32;
    for entry in entries {
        if is_alive(&entry.identity) {
            alive = alive.saturating_add(1);
        } else {
            expired = expired.saturating_add(1);
        }
    }
    (alive, expired)
}

fn snapshot_health(alive: u32, expired: u32) -> SnapshotHealth {
    if alive > 0 || expired == 0 {
        SnapshotHealth::Current
    } else {
        SnapshotHealth::Stale
    }
}

/// Snapshot entry for exactly this process incarnation, if one was recorded.
fn find_snapshot_entry<'a>(
    snapshot: Option<&'a OsTurboRecoverySnapshot>,
    pid: u32,
    creation_time: u64,
) -> Option<&'a ProcessRecoverySnapshot> {
    snapshot?
        .processes
        .iter()
        .find(|entry| process_identity_matches(&entry.identity, pid, creation_time))
}

/// State that restoring this process must write back. Processes Turbo never
/// recorded were in the Windows default state before Turbo touched them.
fn original_state_for(
    snapshot: Option<&OsTurboRecoverySnapshot>,
    pid: u32,
    creation_time: u64,
) -> ProcessPowerStateSnapshot {
    let recorded = find_snapshot_entry(snapshot, pid, creation_time)
        .map(|entry| entry.original_state.clone())
        .unwrap_or(ProcessPowerStateSnapshot::DEFAULT);
    // Defensive: never restore INTO the throttled state. A legacy snapshot
    // that recorded EcoQoS as the "original" would otherwise keep the process
    // throttled after disable — the opposite of what disable promises.
    if recorded == throttling_state_snapshot(true) {
        ProcessPowerStateSnapshot::DEFAULT
    } else {
        recorded
    }
}

/// Drop snapshot entries whose process is no longer in the live candidate set.
/// Returns `true` when the snapshot changed.
fn prune_snapshot_entries(snapshot: &mut OsTurboRecoverySnapshot, live_pids: &[u32]) -> bool {
    let before = snapshot.processes.len();
    snapshot
        .processes
        .retain(|entry| live_pids.contains(&entry.identity.pid));
    snapshot.processes.len() != before
}

/// `OpenProcess` reports a PID that no longer exists as
/// `ERROR_INVALID_PARAMETER` (87), either bare or wrapped as
/// `HRESULT_FROM_WIN32(87)`. Anything else (access denied, timeout) means the
/// process exists but could not be opened — a real failure, not a termination.
fn open_error_means_process_gone(code: i32) -> bool {
    const ERROR_INVALID_PARAMETER: u32 = 87;
    const HRESULT_FROM_WIN32_BASE: u32 = 0x8007_0000;
    let raw = code as u32;
    (raw & 0xFFFF) == ERROR_INVALID_PARAMETER
        && (raw == ERROR_INVALID_PARAMETER || (raw & 0xFFFF_0000) == HRESULT_FROM_WIN32_BASE)
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

/// Restoring is required whenever the feature is (or was) active: an enabled
/// flag with no snapshot still means processes may have been throttled by
/// discovery, and a snapshot with the flag off is an interrupted disable.
fn disable_requires_restore(enabled: bool, has_snapshot: bool) -> bool {
    enabled || has_snapshot
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
    /// The recovery snapshot records processes that no longer exist and a
    /// recapture is due. `#[serde(default)]` so a status produced by an older
    /// bridge binary still deserializes.
    #[serde(default)]
    pub snapshot_stale: bool,
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
    /// Live candidates found in the Windows default state — discovered but
    /// not throttled yet. Reset on every restart of the process; unlike
    /// `restored` this is not a regression, it is work still to do.
    #[serde(default)]
    pub pending: u32,
    /// Candidates verified in their original (pre-enable) state.
    pub restored: u32,
    /// Candidates skipped: exited, PID reused (identity mismatch), or in a
    /// state OS Turbo does not own.
    pub skipped: u32,
    /// Open/read/write/bridge errors encountered during the last sweep. A
    /// process that no longer exists is `skipped`, never a failure.
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
        let mut enabled = read_enabled_compat();
        let mut snapshot_stale = false;

        // S68/F1 (OS Turbo "does nothing" bug): the flag said ENABLED while the
        // recovery snapshot was missing OR — the 2026-10-26 failure — while
        // every process it recorded had already exited. Both states make the
        // feature a no-op that still claims to be on. Auto-repair captures a
        // FRESH snapshot from the current process state. The original
        // power-scheme record is preserved when a stale snapshot is replaced:
        // dropping it would lose the only remaining undo information.
        if enabled {
            let (health, previous) = match read_recovery_snapshot() {
                Ok(Some(snapshot)) => {
                    let (alive, expired) =
                        count_live_entries(&snapshot.processes, identity_is_alive);
                    (snapshot_health(alive, expired), Some(snapshot))
                }
                Ok(None) => (SnapshotHealth::Missing, None),
                Err(error) => {
                    log::warn!(
                        "[os_turbo] recovery snapshot unreadable ({error}) — treating as missing"
                    );
                    (SnapshotHealth::Missing, None)
                }
            };
            if health == SnapshotHealth::Stale {
                log::warn!(
                    "[os_turbo] recovery snapshot records only exited processes — recapturing (auto-repair)"
                );
                snapshot_stale = true;
            }
            if health != SnapshotHealth::Current {
                match capture_recovery_snapshot(previous.as_ref()) {
                    Ok(snapshot) => {
                        if let Err(e) = write_recovery_snapshot(&snapshot) {
                            log::warn!("[os_turbo] auto-repair snapshot save failed: {e}");
                            // The record could not be refreshed: the flag is on
                            // but there is no usable snapshot to describe it.
                            snapshot_stale = true;
                        } else {
                            log::info!(
                                "[os_turbo] auto-repair captured a fresh recovery snapshot ({} recorded state(s))",
                                snapshot.processes.len()
                            );
                            snapshot_stale = false;
                        }
                    }
                    Err(e) => {
                        // Cannot capture (e.g. process enumeration failed): the
                        // honest state is DISABLED — clear the stale flag rather
                        // than keep advertising a no-op.
                        log::warn!(
                            "[os_turbo] auto-repair capture failed ({e}) — clearing stale enabled flag"
                        );
                        let _ = persist_os_turbo(false);
                        enabled = false;
                        snapshot_stale = false;
                    }
                }
            }
        }

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
            snapshot_stale,
            measured,
        })
    }
    #[cfg(not(windows))]
    {
        Ok(OsTurboStatus {
            enabled: false,
            throttled_processes: 0,
            power_plan: String::new(),
            snapshot_stale: false,
            measured: OsTurboMeasuredStatus::default(),
        })
    }
}

/// Enable OS Turbo optimizations.
///
/// F2+F4: "enable" and "already enabled, apply now" are the same operation —
/// throttle every live allowlisted candidate, not just the PIDs that happened
/// to exist when the snapshot was first captured.
fn enable_os_turbo() -> HardwareResult<OsTurboStatus> {
    #[cfg(windows)]
    {
        if read_enabled_compat() {
            // F4: touching the toggle while it is already on must APPLY, not
            // return the previous status (the old code was a pure no-op, so a
            // user had to switch it off and on again to get a re-apply).
            log::info!(
                "[os_turbo] enable requested while already enabled — re-applying to live candidates"
            );
            return refresh_os_turbo();
        }

        // Leftover recovery state with the flag off means an interrupted run
        // (crash between the snapshot and the flag, or a disable that failed).
        // Finish that restore before capturing a new baseline, otherwise the
        // processes it still throttles would never be freed. Its recorded
        // power scheme is carried over: it is the only evidence of what the
        // plan was before OS Turbo changed it.
        let previous = match read_recovery_snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                log::warn!(
                    "[os_turbo] existing recovery snapshot is unusable ({error}) — replacing it"
                );
                None
            }
        };
        if previous.is_some() {
            log::warn!(
                "[os_turbo] recovery state present while disabled — completing the restore before enabling"
            );
            let _ = disable_os_turbo();
        }

        let mut snapshot = capture_recovery_snapshot(previous.as_ref())?;
        // Write the recovery record BEFORE touching a single process: if the
        // app dies mid-apply, the next start still knows what to undo. The
        // `Enabled` flag is written LAST, for the same reason.
        write_recovery_snapshot(&snapshot)?;

        let power_plan = set_power_plan_best_performance()?;
        let (counts, changed) = apply_to_live_candidates(&mut snapshot);
        if changed {
            write_recovery_snapshot(&snapshot)?;
        }

        persist_os_turbo(true)?;
        log::info!(
            "[os_turbo] enabled: inspected={} applied={} pending={} skipped={} failures={} plan={power_plan}",
            counts.inspected,
            counts.applied,
            counts.pending,
            counts.skipped,
            counts.failures
        );

        Ok(OsTurboStatus {
            enabled: true,
            throttled_processes: counts.applied,
            power_plan,
            snapshot_stale: false,
            measured: OsTurboMeasuredStatus {
                inspected: counts.inspected,
                applied: counts.applied,
                pending: counts.pending,
                restored: counts.restored,
                skipped: counts.skipped,
                failures: counts.failures,
                last_sweep_ms: sweep_timestamp_ms(),
                worker_active: worker_active(),
            },
        })
    }
    #[cfg(not(windows))]
    {
        Ok(OsTurboStatus {
            enabled: false,
            throttled_processes: 0,
            power_plan: String::new(),
            snapshot_stale: false,
            measured: OsTurboMeasuredStatus::default(),
        })
    }
}

/// Disable OS Turbo optimizations.
///
/// F2: restore by **discovery**, not by replaying a stale PID list — every
/// live allowlisted candidate is checked, so processes that were throttled
/// after the snapshot was captured are freed too (they are not in the
/// snapshot by design).
fn disable_os_turbo() -> HardwareResult<OsTurboStatus> {
    #[cfg(windows)]
    {
        let snapshot = match read_recovery_snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                log::warn!(
                    "[os_turbo] recovery snapshot unreadable ({error}) — restoring by discovery only"
                );
                None
            }
        };
        let enabled = read_enabled_compat();
        if !disable_requires_restore(enabled, snapshot.is_some()) {
            // Nothing was applied and nothing is recorded: make the persisted
            // flag agree without touching the machine.
            persist_os_turbo(false)?;
            log::info!("[os_turbo] disable: no recovery state to restore");
            let power_plan = get_active_power_plan()?;
            return Ok(OsTurboStatus {
                enabled: false,
                throttled_processes: 0,
                power_plan,
                snapshot_stale: false,
                measured: OsTurboMeasuredStatus {
                    last_sweep_ms: sweep_timestamp_ms(),
                    worker_active: worker_active(),
                    ..OsTurboMeasuredStatus::default()
                },
            });
        }

        let (counts, process_restore_failed) = restore_all(snapshot.as_ref());

        let mut scheme_restore_failed = false;
        let active_scheme = get_active_power_scheme_guid()?.to_u128();
        if turbo_owns_power_scheme(active_scheme) {
            if let Some(original) = snapshot.as_ref().map(|s| s.original_power_scheme) {
                if original != HIGH_PERFORMANCE_SCHEME
                    && set_active_power_scheme(windows::core::GUID::from_u128(original)).is_err()
                {
                    scheme_restore_failed = true;
                }
            }
        }

        if should_retain_snapshot(process_restore_failed, scheme_restore_failed) {
            log::warn!(
                "[os_turbo] disable incomplete (restore_failed={process_restore_failed} scheme_failed={scheme_restore_failed}) — recovery state retained"
            );
            return Err(HardwareError::Other(
                "OS Turbo restore was incomplete; recovery state was retained".into(),
            ));
        }

        persist_os_turbo(false)?;
        remove_recovery_snapshot()?;
        log::info!(
            "[os_turbo] disabled: inspected={} restored={} skipped={} failures={}",
            counts.inspected,
            counts.restored,
            counts.skipped,
            counts.failures
        );
        let power_plan = get_active_power_plan()?;
        Ok(OsTurboStatus {
            enabled: false,
            throttled_processes: 0,
            power_plan,
            snapshot_stale: false,
            measured: OsTurboMeasuredStatus {
                inspected: counts.inspected,
                applied: 0,
                pending: 0,
                restored: counts.restored,
                skipped: counts.skipped,
                failures: counts.failures,
                last_sweep_ms: sweep_timestamp_ms(),
                worker_active: worker_active(),
            },
        })
    }
    #[cfg(not(windows))]
    {
        Ok(OsTurboStatus {
            enabled: false,
            throttled_processes: 0,
            power_plan: String::new(),
            snapshot_stale: false,
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
// leak-check: every caller (7 sites) closes this handle explicitly on ALL paths;
// the function cannot use RAII without changing 7 signatures. A future refactor
// should return `OwnedHandle` and drop the manual CloseHandle calls.
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

/// Live allowlisted background candidates, deterministically ordered.
#[cfg(windows)]
fn discover_candidates() -> HardwareResult<Vec<(u32, String)>> {
    let mut candidates: Vec<(u32, String)> = enumerate_processes()?
        .into_iter()
        .filter(|(_, name)| is_throttle_candidate(name))
        .collect();
    candidates.sort_by(|left, right| left.1.cmp(&right.1).then(left.0.cmp(&right.0)));
    Ok(candidates)
}

/// `true` when this exact process incarnation is still running.
#[cfg(windows)]
fn identity_is_alive(identity: &ProcessIdentitySnapshot) -> bool {
    // leak-check: audited — the handle is closed on both exit paths (S67).
    let Ok(handle) = open_process_for_read(identity.pid) else {
        return false;
    };
    let alive = get_process_creation_time(handle)
        .map(|creation_time| process_identity_matches(identity, identity.pid, creation_time))
        .unwrap_or(false);
    unsafe {
        windows::Win32::Foundation::CloseHandle(handle).ok();
    }
    alive
}

/// Capture the minimum state OS Turbo must be able to undo.
///
/// F2: only the **power plan** and processes whose original state is not the
/// Windows default are recorded. A process sitting in the default state needs
/// no record — restoring it means writing the default back, which is always
/// valid. This is what makes "PIDs in the snapshot are dead" structurally
/// impossible: there are usually no per-process entries at all.
///
/// `previous` supplies the power scheme recorded by an earlier snapshot, so
/// replacing a stale snapshot cannot lose the undo information for a plan
/// change Turbo made days ago.
#[cfg(windows)]
fn capture_recovery_snapshot(
    previous: Option<&OsTurboRecoverySnapshot>,
) -> HardwareResult<OsTurboRecoverySnapshot> {
    let active_scheme = get_active_power_scheme_guid()?.to_u128();
    let original_power_scheme = power_scheme_to_record(previous, active_scheme);
    let desired = throttling_state_snapshot(true);

    let mut processes = Vec::new();
    for (pid, process_name) in discover_candidates()?
        .into_iter()
        .take(MAX_SNAPSHOT_PROCESSES)
    {
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
            // States Turbo itself wrote (or the Windows default) are not
            // "original": recording them would make disable re-apply EcoQoS.
            if !should_record_original(&original_state, &desired) {
                continue;
            }
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

/// Atomically write the recovery snapshot, replacing any existing file.
///
/// Replacement is intentional: a stale snapshot (every recorded process
/// exited) is recaptured rather than treated as valid forever.
#[cfg(windows)]
fn write_recovery_snapshot(snapshot: &OsTurboRecoverySnapshot) -> HardwareResult<()> {
    if !snapshot_is_valid(snapshot) {
        return Err(HardwareError::Other(
            "OS Turbo recovery snapshot is unsupported or too large".into(),
        ));
    }
    let path = recovery_snapshot_path();
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
            .create(true)
            .truncate(true)
            .open(&temporary_path)
            .map_err(|_| "temporary snapshot cannot be created".to_string())?;
        file.write_all(&bytes)
            .map_err(|_| "temporary snapshot cannot be written".to_string())?;
        file.sync_all()
            .map_err(|_| "temporary snapshot cannot be flushed".to_string())?;
        crate::util::auth::restrict_file_acl(&temporary_path)?;
        // `fs::rename` replaces an existing destination on Windows
        // (MoveFileExW + MOVEFILE_REPLACE_EXISTING): recapturing a stale
        // snapshot is exactly the intended behaviour.
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

/// `%LOCALAPPDATA%\MiControl` — where the anonymous action log lives.
fn action_log_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|base| std::path::PathBuf::from(base).join("MiControl"))
}

/// Coarse bucket for the current hour (UTC). The log deliberately carries no
/// exact timestamp; see `hw::os_turbo_log`.
fn current_time_bucket() -> crate::hw::os_turbo_log::TimeBucket {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    crate::hw::os_turbo_log::TimeBucket::from_hour(((seconds % 86_400) / 3_600) as u32)
}

/// Append one record to the anonymous, bounded action log and persist it.
///
/// F6: the 0.2.20 changelog promised this file and nothing ever instantiated
/// `ActionLog`, which is why "OS Turbo has no logs" was literally true. It is
/// now enabled by default and written from the user-context app (the bridge
/// runs as SYSTEM, whose `LOCALAPPDATA` is not the user's).
fn record_os_turbo_action(build: impl FnOnce() -> crate::hw::os_turbo_log::ActionRecord) {
    use crate::hw::os_turbo_log::{load_action_log, save_action_log, ActionLog};

    let Some(dir) = action_log_dir() else {
        return;
    };
    static ACTION_LOG: std::sync::OnceLock<std::sync::Mutex<ActionLog>> =
        std::sync::OnceLock::new();
    let log = ACTION_LOG.get_or_init(|| std::sync::Mutex::new(load_action_log(&dir, true)));
    let mut guard = log.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.push(build());
    if let Err(error) = save_action_log(&dir, &guard) {
        log::warn!("[os_turbo] action log write failed: {error}");
    }
}

/// Startup reconciliation (F5).
///
/// The old `restore_os_turbo()` was an empty body with no caller, so nothing
/// ever reconciled the persisted flag with the machine: a crash (or a disable
/// that failed halfway) left throttled processes and a `High performance`
/// plan behind with no code path to undo them.
///
/// Runs in the user-context app, so every privileged write goes through the
/// authenticated bridge. Never blocks startup: a failure is logged and the
/// 30 s worker sweep retries the repair path.
pub fn restore_os_turbo() {
    #[cfg(windows)]
    {
        let enabled = read_enabled_compat();
        let health = match read_recovery_snapshot() {
            Ok(None) => SnapshotHealth::Missing,
            Ok(Some(snapshot)) => {
                let (alive, expired) = count_live_entries(&snapshot.processes, identity_is_alive);
                snapshot_health(alive, expired)
            }
            Err(error) => {
                log::warn!("[os_turbo] startup reconcile: recovery snapshot unreadable ({error})");
                SnapshotHealth::Missing
            }
        };

        match reconcile_action(enabled, health) {
            ReconcileAction::Nothing => {
                log::info!(
                    "[os_turbo] startup reconcile: nothing to do (enabled={enabled}, snapshot={health:?})"
                );
            }
            ReconcileAction::Reapply => {
                log::warn!(
                    "[os_turbo] startup reconcile: enabled with a {health:?} recovery snapshot — re-applying through the bridge"
                );
                tauri::async_runtime::spawn(async move {
                    match crate::elev_bridge::run_elevated_bridge_only(
                        "refresh_os_turbo",
                        serde_json::json!({}),
                    )
                    .await
                    {
                        Ok(_) => log::info!("[os_turbo] startup reconcile: re-apply completed"),
                        Err(error) => log::warn!(
                            "[os_turbo] startup reconcile: re-apply failed ({error}); the worker will retry"
                        ),
                    }
                });
            }
            ReconcileAction::CompleteDisable => {
                log::warn!(
                    "[os_turbo] startup reconcile: flag is off but recovery state is still on disk — completing the disable"
                );
                tauri::async_runtime::spawn(async move {
                    match crate::elev_bridge::run_elevated_bridge_only(
                        "set_os_turbo",
                        serde_json::json!({ "enabled": false }),
                    )
                    .await
                    {
                        Ok(_) => log::info!("[os_turbo] startup reconcile: disable completed"),
                        Err(error) => log::warn!(
                            "[os_turbo] startup reconcile: disable failed ({error}); recovery state retained"
                        ),
                    }
                });
            }
        }
    }
}

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
    /// Alive and in the Windows default state: discovered but not throttled
    /// yet. This is work to do, not a regression.
    Pending,
    /// Valid identity but verified back in its captured (non-default) state.
    Restored,
    /// Valid identity but in a state Turbo does not own (foreign mask).
    Other,
    /// Exited or PID reused — cannot be attributed (counted as terminated,
    /// never as a failure).
    Skipped,
    /// Open/read/set error on a process that still exists.
    Failed,
}

/// Truthful, per-sweep aggregate of readback outcomes.
///
/// `inspected` equals `applied + pending + restored + other + skipped +
/// failures` (saturated); no field is guessed or extrapolated, and a disabled
/// Turbo produces an all-default aggregate (no work, no fabricated counts).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct SweepCounts {
    inspected: u32,
    applied: u32,
    pending: u32,
    restored: u32,
    other: u32,
    skipped: u32,
    failures: u32,
}

impl SweepCounts {
    /// Candidates that are alive and should be throttled but are not.
    fn driftable(&self) -> u32 {
        self.restored
            .saturating_add(self.other)
            .saturating_add(self.pending)
    }

    /// F3: drift is "there is work to do", and a live process that could not
    /// be opened or read is work to do as well — the bridge runs as SYSTEM and
    /// may well succeed where the unprivileged sweep failed. A process that no
    /// longer exists is `Skipped`, so it never reaches this branch.
    fn drift_detected(&self) -> bool {
        self.driftable() > 0 || self.failures > 0
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
            pending: self.pending,
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
            ReadbackOutcome::Pending => counts.pending = counts.pending.saturating_add(1),
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
    /// Last aggregate written to the log. A steady state must not produce a
    /// line every 30 s, so logging and action-log records are edge-triggered
    /// on a change of these counts.
    last_logged: std::sync::Mutex<Option<SweepCounts>>,
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
            last_logged: std::sync::Mutex::new(None),
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
            pending: sweep.pending,
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
            pending: status.measured.pending,
            restored: status.measured.restored,
            skipped: status.measured.skipped,
            failures: status.measured.failures,
            last_sweep_ms: status.measured.last_sweep_ms,
            worker_active,
        };
    }

    /// F6: one `info!` line per material change, never per 30 s tick. The
    /// counts (not process names) are the whole message, matching the
    /// anonymity policy of `os_turbo_log`.
    fn log_sweep(&self, sweep: &SweepCounts) {
        let mut last = self.last_logged.lock().unwrap_or_else(|p| p.into_inner());
        let previous = *last;
        if previous == Some(*sweep) {
            log::debug!(
                "[os_turbo] sweep unchanged: applied={} pending={} failures={}",
                sweep.applied,
                sweep.pending,
                sweep.failures
            );
            return;
        }
        *last = Some(*sweep);
        drop(last);

        log::info!(
            "[os_turbo] sweep: inspected={} applied={} pending={} restored={} skipped={} failures={}",
            sweep.inspected,
            sweep.applied,
            sweep.pending,
            sweep.restored,
            sweep.skipped,
            sweep.failures
        );

        let before = previous.unwrap_or_default();
        if sweep.applied > before.applied {
            let affected = sweep.applied.saturating_sub(before.applied);
            record_os_turbo_action(|| {
                crate::hw::os_turbo_log::ActionRecord::applied(
                    current_time_bucket(),
                    crate::hw::os_turbo_policy::PolicyAction::ThrottleAllowlistedBackground,
                )
                .with_affected(affected)
            });
        }
        if sweep.restored > before.restored {
            let affected = sweep.restored.saturating_sub(before.restored);
            record_os_turbo_action(|| {
                crate::hw::os_turbo_log::ActionRecord::rolled_back(
                    current_time_bucket(),
                    crate::hw::os_turbo_policy::RollbackReason::VerificationFailed,
                )
                .with_affected(affected)
            });
        }
        if sweep.failures > before.failures {
            let affected = sweep.failures.saturating_sub(before.failures);
            record_os_turbo_action(|| {
                crate::hw::os_turbo_log::ActionRecord::failed(
                    current_time_bucket(),
                    crate::hw::os_turbo_log::SafeErrorCode::ProcessWriteFailed,
                )
                .with_affected(affected)
            });
        }
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
    // The bridge has no user-context logger or data dir (it runs as SYSTEM),
    // so the app records both the human-readable line and the anonymous
    // action-log entry for the operation the user just performed.
    if status.enabled {
        log::info!(
            "[os_turbo] user enable: inspected={} applied={} pending={} skipped={} failures={}",
            status.measured.inspected,
            status.measured.applied,
            status.measured.pending,
            status.measured.skipped,
            status.measured.failures
        );
        let affected = status.measured.applied;
        record_os_turbo_action(|| {
            crate::hw::os_turbo_log::ActionRecord::applied(
                current_time_bucket(),
                crate::hw::os_turbo_policy::PolicyAction::ThrottleAllowlistedBackground,
            )
            .with_affected(affected)
        });
    } else {
        log::info!(
            "[os_turbo] user disable: inspected={} restored={} skipped={} failures={}",
            status.measured.inspected,
            status.measured.restored,
            status.measured.skipped,
            status.measured.failures
        );
        let affected = status.measured.restored;
        record_os_turbo_action(|| {
            crate::hw::os_turbo_log::ActionRecord::rolled_back(
                current_time_bucket(),
                crate::hw::os_turbo_policy::RollbackReason::UserDisabled,
            )
            .with_affected(affected)
        });
    }
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
            log::debug!(
                "[os_turbo] drift: pending={} restored={} failed={} — re-applying through the bridge",
                sweep.pending,
                sweep.restored,
                sweep.failures
            );
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
                Err(error) => {
                    // Repeated bridge failures open a bounded backoff window.
                    log::warn!("[os_turbo] drift re-apply failed: {error}");
                    sweep.failures = sweep.failures.saturating_add(1);
                    state.note_failure();
                }
            }
        } else if sweep.drift_detected() {
            // Drift while the bridge is down: counted, no write attempted.
            log::warn!("[os_turbo] drift detected but the elevation bridge is unavailable");
            state.note_failure();
        } else {
            state.note_success();
        }
    }
    #[cfg(not(windows))]
    {
        state.note_success();
    }

    state.end_sweep();
    state.log_sweep(&sweep);
    state.record(&sweep, true);
}

/// Measure allowlisted candidates strictly by readback. Read-only — never
/// writes, never dispatches; bounded by batch and sweep duration.
///
/// F2: candidates come from live process enumeration, not from the recovery
/// snapshot, so a background process that started after enable is inspected
/// (and then throttled by the refresh pass) on the very next sweep.
#[cfg(windows)]
fn measure_sweep() -> SweepCounts {
    if !read_enabled_compat() {
        return SweepCounts::default();
    }
    let snapshot = read_recovery_snapshot().ok().flatten();
    let desired = throttling_state_snapshot(true);
    let deadline = std::time::Instant::now() + WORKER_MAX_SWEEP;
    let mut outcomes = Vec::with_capacity(WORKER_MAX_BATCH);
    for (pid, _name) in discover_candidates()
        .unwrap_or_default()
        .into_iter()
        .take(WORKER_MAX_BATCH)
    {
        if std::time::Instant::now() >= deadline {
            break;
        }
        outcomes.push(measure_candidate(pid, snapshot.as_ref(), &desired));
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
    // leak-check: audited — every exit path closes the handle explicitly (S67).
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    unsafe {
        OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).map_err(|error| error.code().0)
    }
}

/// Map an `OpenProcess` failure to an outcome: a process that no longer
/// exists is "terminated" (skipped), never a failure (F3).
#[cfg(windows)]
fn open_failure_outcome(code: i32) -> ReadbackOutcome {
    if open_error_means_process_gone(code) {
        ReadbackOutcome::Skipped
    } else {
        ReadbackOutcome::Failed
    }
}

/// Classify a readback against the states Turbo knows about.
///
/// Order matters: the throttled state is Turbo's own, the Windows default on
/// a discovered candidate means "not throttled yet", and only a state that
/// equals the recorded original is a genuine `Restored`.
fn classify_readback(
    state: &ProcessPowerStateSnapshot,
    original: &ProcessPowerStateSnapshot,
    desired: &ProcessPowerStateSnapshot,
) -> ReadbackOutcome {
    if state == desired {
        ReadbackOutcome::Applied
    } else if state == &ProcessPowerStateSnapshot::DEFAULT {
        ReadbackOutcome::Pending
    } else if state == original {
        ReadbackOutcome::Restored
    } else {
        ReadbackOutcome::Other
    }
}

/// `true` when this pre-enable state must be recorded so disable can undo it:
/// it is neither the Windows default nor a state Turbo itself wrote.
fn should_record_original(
    state: &ProcessPowerStateSnapshot,
    desired: &ProcessPowerStateSnapshot,
) -> bool {
    state != &ProcessPowerStateSnapshot::DEFAULT && state != desired
}

/// Power scheme to record in a snapshot. An earlier snapshot wins: replacing a
/// stale snapshot must not overwrite the scheme Turbo changed days ago with
/// the scheme Turbo itself activated.
fn power_scheme_to_record(previous: Option<&OsTurboRecoverySnapshot>, active: u128) -> u128 {
    previous
        .map(|snapshot| snapshot.original_power_scheme)
        .unwrap_or(active)
}

/// Read-only per-process readback used by the measure sweep.
#[cfg(windows)]
fn measure_candidate(
    pid: u32,
    snapshot: Option<&OsTurboRecoverySnapshot>,
    desired: &ProcessPowerStateSnapshot,
) -> ReadbackOutcome {
    let handle = match open_process_for_read(pid) {
        Ok(handle) => handle,
        Err(code) => return open_failure_outcome(code),
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
    let original = original_state_for(snapshot, pid, creation_time);
    let outcome = match read_process_power_state(handle) {
        Ok(state) => classify_readback(&state, &original, desired),
        Err(_) => ReadbackOutcome::Failed,
    };
    unsafe {
        windows::Win32::Foundation::CloseHandle(handle).ok();
    }
    outcome
}

/// Re-apply throttling to every live allowlisted candidate. Runs only inside
/// the authenticated bridge (no UAC, no scheduled-task fallback). Bounded by
/// the batch cap and the per-sweep duration budget. Only touches allowlisted,
/// identity-verified processes — never protected, security or update
/// processes, and never EC/fan/power-limit registers.
///
/// F1/F2/F4: repairs a missing or stale recovery snapshot (preserving its
/// recorded power scheme) and then throttles by discovery, so this is also
/// the "apply now" path behind the toggle.
pub fn refresh_os_turbo() -> HardwareResult<OsTurboStatus> {
    #[cfg(windows)]
    {
        if !read_enabled_compat() {
            return get_os_turbo();
        }

        let previous = match read_recovery_snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                log::warn!("[os_turbo] recovery snapshot unreadable ({error}) — recapturing");
                None
            }
        };
        let (mut snapshot, recaptured) = match previous {
            Some(snapshot) => {
                let (alive, expired) = count_live_entries(&snapshot.processes, identity_is_alive);
                if snapshot_health(alive, expired) == SnapshotHealth::Stale {
                    log::warn!(
                        "[os_turbo] recovery snapshot is stale ({expired} recorded process(es) already exited) — recapturing"
                    );
                    (capture_recovery_snapshot(Some(&snapshot))?, true)
                } else {
                    (snapshot, false)
                }
            }
            None => {
                log::warn!(
                    "[os_turbo] enabled but recovery snapshot is missing — capturing a fresh one"
                );
                (capture_recovery_snapshot(None)?, true)
            }
        };

        let (counts, changed) = apply_to_live_candidates(&mut snapshot);
        if recaptured || changed {
            write_recovery_snapshot(&snapshot)?;
        }

        let power_plan = get_active_power_plan()?;
        log::info!(
            "[os_turbo] refresh: inspected={} applied={} pending={} restored={} skipped={} failures={} recaptured={recaptured}",
            counts.inspected,
            counts.applied,
            counts.pending,
            counts.restored,
            counts.skipped,
            counts.failures
        );
        Ok(OsTurboStatus {
            enabled: true,
            throttled_processes: counts.applied,
            power_plan,
            snapshot_stale: false,
            measured: OsTurboMeasuredStatus {
                inspected: counts.inspected,
                applied: counts.applied,
                pending: counts.pending,
                restored: counts.restored,
                skipped: counts.skipped,
                failures: counts.failures,
                last_sweep_ms: sweep_timestamp_ms(),
                worker_active: worker_active(),
            },
        })
    }
    #[cfg(not(windows))]
    {
        get_os_turbo()
    }
}

/// Throttle every live allowlisted candidate, readback-verified, recording any
/// non-default original state in `snapshot` so disable can restore it.
///
/// Returns the counts and whether `snapshot` gained/lost entries.
#[cfg(windows)]
fn apply_to_live_candidates(snapshot: &mut OsTurboRecoverySnapshot) -> (SweepCounts, bool) {
    let desired = throttling_state_snapshot(true);
    let deadline = std::time::Instant::now() + WORKER_MAX_SWEEP;
    let candidates = discover_candidates().unwrap_or_default();
    let live_pids: Vec<u32> = candidates.iter().map(|(pid, _)| *pid).collect();
    let mut outcomes = Vec::with_capacity(candidates.len().min(WORKER_MAX_BATCH));
    let mut changed = false;

    for (pid, process_name) in candidates.into_iter().take(WORKER_MAX_BATCH) {
        if std::time::Instant::now() >= deadline {
            break;
        }
        let handle = match open_process_for_turbo(pid) {
            Ok(handle) => handle,
            Err(code) => {
                outcomes.push(open_failure_outcome(code));
                continue;
            }
        };
        let creation_time = match get_process_creation_time(handle) {
            Ok(creation_time) => creation_time,
            Err(_) => {
                unsafe {
                    windows::Win32::Foundation::CloseHandle(handle).ok();
                }
                outcomes.push(ReadbackOutcome::Failed);
                continue;
            }
        };
        let current = match read_process_power_state(handle) {
            Ok(state) => state,
            Err(_) => {
                unsafe {
                    windows::Win32::Foundation::CloseHandle(handle).ok();
                }
                outcomes.push(ReadbackOutcome::Failed);
                continue;
            }
        };
        if current == desired {
            outcomes.push(ReadbackOutcome::Applied);
            unsafe {
                windows::Win32::Foundation::CloseHandle(handle).ok();
            }
            continue;
        }

        // Record the pre-enable state only when it is something Turbo must undo
        // later: not the Windows default, and not a state Turbo itself wrote.
        if should_record_original(&current, &desired)
            && find_snapshot_entry(Some(snapshot), pid, creation_time).is_none()
            && snapshot.processes.len() < MAX_SNAPSHOT_PROCESSES
        {
            snapshot.processes.push(ProcessRecoverySnapshot {
                identity: ProcessIdentitySnapshot {
                    pid,
                    creation_time,
                    process_name,
                },
                original_state: current,
            });
            changed = true;
        }

        // set_process_power_state re-verifies by readback.
        if set_process_power_state(handle, &desired).is_ok() {
            outcomes.push(ReadbackOutcome::Applied);
        } else {
            outcomes.push(ReadbackOutcome::Failed);
        }
        unsafe {
            windows::Win32::Foundation::CloseHandle(handle).ok();
        }
    }

    changed |= prune_snapshot_entries(snapshot, &live_pids);
    (aggregate_counts(outcomes, WORKER_MAX_BATCH), changed)
}

/// Clear throttling on every live allowlisted candidate, restoring the state
/// the snapshot recorded (the Windows default when it recorded nothing).
///
/// Returns the counts and whether any restore failed (the snapshot must then
/// be retained so the caller can retry).
#[cfg(windows)]
fn restore_all(snapshot: Option<&OsTurboRecoverySnapshot>) -> (SweepCounts, bool) {
    let desired = throttling_state_snapshot(true);
    let deadline = std::time::Instant::now() + WORKER_MAX_SWEEP;
    let candidates = discover_candidates().unwrap_or_default();
    let live_pids: Vec<u32> = candidates.iter().map(|(pid, _)| *pid).collect();
    let mut outcomes = Vec::new();
    let mut restore_failed = false;

    // Recorded processes that already exited have nothing to restore: count
    // them as terminated (skipped), which is what they are.
    if let Some(snapshot) = snapshot {
        for entry in &snapshot.processes {
            if !live_pids.contains(&entry.identity.pid) {
                outcomes.push(ReadbackOutcome::Skipped);
            }
        }
    }

    for (pid, _name) in candidates.into_iter().take(WORKER_MAX_BATCH) {
        if std::time::Instant::now() >= deadline {
            break;
        }
        let handle = match open_process_for_turbo(pid) {
            Ok(handle) => handle,
            Err(code) => {
                outcomes.push(open_failure_outcome(code));
                continue;
            }
        };
        let creation_time = match get_process_creation_time(handle) {
            Ok(creation_time) => creation_time,
            Err(_) => {
                unsafe {
                    windows::Win32::Foundation::CloseHandle(handle).ok();
                }
                restore_failed = true;
                outcomes.push(ReadbackOutcome::Failed);
                continue;
            }
        };
        let original = original_state_for(snapshot, pid, creation_time);
        let outcome = match read_process_power_state(handle) {
            Ok(state) if state == original => ReadbackOutcome::Restored,
            Ok(state) if state == desired => {
                if set_process_power_state(handle, &original).is_ok() {
                    ReadbackOutcome::Restored
                } else {
                    restore_failed = true;
                    ReadbackOutcome::Failed
                }
            }
            // A foreign mask (set by something else) is not ours to clear.
            Ok(_) => ReadbackOutcome::Skipped,
            Err(_) => {
                restore_failed = true;
                ReadbackOutcome::Failed
            }
        };
        unsafe {
            windows::Win32::Foundation::CloseHandle(handle).ok();
        }
        outcomes.push(outcome);
    }

    (
        aggregate_counts(outcomes, WORKER_MAX_BATCH + MAX_SNAPSHOT_PROCESSES),
        restore_failed,
    )
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
    fn snapshot_liveness_distinguishes_live_from_exited_processes() {
        let snapshot = sample_snapshot();
        let entries = &snapshot.processes;

        // The 2026-10-26 case: the recorded PID 42 no longer exists.
        let (alive, expired) = super::count_live_entries(entries, |_| false);
        assert_eq!((alive, expired), (0, 1));
        assert_eq!(
            super::snapshot_health(alive, expired),
            super::SnapshotHealth::Stale,
            "a snapshot whose processes all exited is stale"
        );

        // Same PID, different creation time → PID reuse, still stale.
        let (alive, expired) = super::count_live_entries(entries, |identity| {
            super::process_identity_matches(identity, identity.pid, identity.creation_time + 1)
        });
        assert_eq!((alive, expired), (0, 1));
        assert_eq!(
            super::snapshot_health(alive, expired),
            super::SnapshotHealth::Stale
        );

        // Same PID and creation time → alive.
        let (alive, expired) = super::count_live_entries(entries, |_| true);
        assert_eq!((alive, expired), (1, 0));
        assert_eq!(
            super::snapshot_health(alive, expired),
            super::SnapshotHealth::Current
        );

        // A partially dead snapshot still describes something real: keep it.
        assert_eq!(super::snapshot_health(1, 7), super::SnapshotHealth::Current);
        // No recorded processes (the normal case since F2) cannot rot.
        assert_eq!(super::snapshot_health(0, 0), super::SnapshotHealth::Current);
        assert_eq!(
            super::count_live_entries(&[], |_| false),
            (0, 0),
            "an empty snapshot has nothing to check"
        );
    }

    #[test]
    fn startup_reconcile_matches_flag_with_snapshot_health() {
        use super::{reconcile_action, ReconcileAction, SnapshotHealth};

        assert_eq!(
            reconcile_action(true, SnapshotHealth::Stale),
            ReconcileAction::Reapply,
            "enabled + dead snapshot must recapture and re-apply"
        );
        assert_eq!(
            reconcile_action(true, SnapshotHealth::Missing),
            ReconcileAction::Reapply
        );
        assert_eq!(
            reconcile_action(false, SnapshotHealth::Stale),
            ReconcileAction::CompleteDisable,
            "a stale snapshot still holds the original power plan"
        );
        assert_eq!(
            reconcile_action(false, SnapshotHealth::Current),
            ReconcileAction::CompleteDisable,
            "flag off with state on disk is an interrupted disable"
        );
        assert_eq!(
            reconcile_action(false, SnapshotHealth::Missing),
            ReconcileAction::Nothing
        );
        assert_eq!(
            reconcile_action(true, SnapshotHealth::Current),
            ReconcileAction::Nothing
        );
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
    fn disable_restores_whenever_the_feature_was_active() {
        assert!(
            super::disable_requires_restore(true, false),
            "enabled without a snapshot may still have throttled processes"
        );
        assert!(super::disable_requires_restore(true, true));
        assert!(
            super::disable_requires_restore(false, true),
            "a snapshot with the flag off is an interrupted disable"
        );
        assert!(
            !super::disable_requires_restore(false, false),
            "nothing was applied and nothing is recorded"
        );
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
        // leak-check: audited — every exit path closes the handle explicitly (S67).
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
            Outcome::Pending,
            Outcome::Restored,
            Outcome::Other,
            Outcome::Skipped,
            Outcome::Failed,
            Outcome::Restored,
        ];
        let counts = aggregate_counts(items, items.len());
        assert_eq!(counts.inspected, 8);
        assert_eq!(counts.applied, 2);
        assert_eq!(counts.pending, 1);
        assert_eq!(counts.restored, 2);
        assert_eq!(counts.skipped, 1);
        assert_eq!(counts.failures, 1);
        assert_eq!(counts.other, 1);
        // Every inspected candidate lands in exactly one bucket.
        assert_eq!(
            counts.applied
                + counts.pending
                + counts.restored
                + counts.other
                + counts.skipped
                + counts.failures,
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

    /// F3: failures are drift (work the SYSTEM bridge may still complete), but
    /// a process that simply exited is not — it is `Skipped`.
    #[test]
    fn drift_counts_pending_and_live_failures_but_not_terminations() {
        use super::SweepCounts;

        let discovered = SweepCounts {
            inspected: 3,
            pending: 3,
            ..SweepCounts::default()
        };
        assert!(
            discovered.drift_detected(),
            "new candidates need a re-apply"
        );

        let unreadable = SweepCounts {
            inspected: 1,
            failures: 1,
            ..SweepCounts::default()
        };
        assert!(
            unreadable.drift_detected(),
            "a live unreadable process is drift"
        );

        let all_gone = SweepCounts {
            inspected: 2,
            skipped: 2,
            ..SweepCounts::default()
        };
        assert!(
            !all_gone.drift_detected(),
            "terminated processes must never trigger a re-apply"
        );

        let healthy = SweepCounts {
            inspected: 2,
            applied: 2,
            ..SweepCounts::default()
        };
        assert!(!healthy.drift_detected());
    }

    #[test]
    fn discovered_default_state_is_pending_not_restored() {
        use super::{classify_readback, ProcessPowerStateSnapshot as State, ReadbackOutcome as O};

        let desired = State {
            version: 1,
            control_mask: 1,
            state_mask: 1,
        };
        let foreign = State {
            version: 1,
            control_mask: 1,
            state_mask: 0,
        };

        assert_eq!(
            classify_readback(&desired, &State::DEFAULT, &desired),
            O::Applied
        );
        // A live candidate Turbo never touched: work to do, not a regression.
        assert_eq!(
            classify_readback(&State::DEFAULT, &State::DEFAULT, &desired),
            O::Pending
        );
        // A recorded non-default original that came back: drift.
        assert_eq!(classify_readback(&foreign, &foreign, &desired), O::Restored);
        // Same state, but nothing of ours recorded: foreign mask.
        assert_eq!(
            classify_readback(&foreign, &State::DEFAULT, &desired),
            O::Other
        );
    }

    #[test]
    fn only_non_default_and_non_turbo_states_are_recorded() {
        use super::{should_record_original, ProcessPowerStateSnapshot as State};

        let desired = State {
            version: 1,
            control_mask: 1,
            state_mask: 1,
        };
        let foreign = State {
            version: 1,
            control_mask: 1,
            state_mask: 0,
        };

        assert!(!should_record_original(&State::DEFAULT, &desired));
        assert!(
            !should_record_original(&desired, &desired),
            "Turbo's own state must never be stored as the original"
        );
        assert!(should_record_original(&foreign, &desired));
    }

    #[test]
    fn recapture_keeps_the_previously_recorded_power_scheme() {
        use super::{power_scheme_to_record, OsTurboRecoverySnapshot};

        let original = 0x381b_4222_f694_41f0_9685_ff5b_b260_df2e;
        let active = super::HIGH_PERFORMANCE_SCHEME;
        let previous = OsTurboRecoverySnapshot {
            version: super::SNAPSHOT_VERSION,
            original_power_scheme: original,
            processes: Vec::new(),
        };

        assert_eq!(
            power_scheme_to_record(Some(&previous), active),
            original,
            "a stale snapshot's scheme is the only record of what Turbo changed"
        );
        assert_eq!(power_scheme_to_record(None, active), active);
    }

    #[test]
    fn a_legacy_snapshot_never_restores_into_the_throttled_state() {
        use super::{original_state_for, ProcessPowerStateSnapshot as State};

        let mut snapshot = sample_snapshot();
        assert_eq!(
            original_state_for(Some(&snapshot), 42, 100),
            snapshot.processes[0].original_state,
            "a recorded non-default original is restored as recorded"
        );

        // A snapshot that (wrongly) recorded EcoQoS as the pre-Turbo state must
        // not leave the process throttled after disable.
        snapshot.processes[0].original_state = super::throttling_state_snapshot(true);
        assert_eq!(original_state_for(Some(&snapshot), 42, 100), State::DEFAULT);

        // An identity mismatch must never inherit another process's state.
        assert_eq!(original_state_for(Some(&snapshot), 42, 101), State::DEFAULT);
        assert_eq!(original_state_for(None, 42, 100), State::DEFAULT);
    }

    #[test]
    fn terminated_pid_is_reported_as_gone_not_as_a_failure() {
        use super::open_error_means_process_gone;

        // ERROR_INVALID_PARAMETER: the PID does not exist.
        assert!(open_error_means_process_gone(87));
        // HRESULT_FROM_WIN32(87), what OpenProcess actually returns.
        assert!(open_error_means_process_gone(0x8007_0057u32 as i32));
        // ERROR_ACCESS_DENIED and HRESULT_FROM_WIN32(5): the process exists.
        assert!(!open_error_means_process_gone(5));
        assert!(!open_error_means_process_gone(0x8007_0005u32 as i32));
        assert!(!open_error_means_process_gone(0));
    }

    #[test]
    fn pruning_drops_entries_for_processes_that_are_gone() {
        use super::{prune_snapshot_entries, ProcessRecoverySnapshot};

        let mut snapshot = sample_snapshot();
        snapshot.processes.push(ProcessRecoverySnapshot {
            identity: super::ProcessIdentitySnapshot {
                pid: 77,
                creation_time: 900,
                process_name: "OneDrive.exe".to_string(),
            },
            original_state: super::ProcessPowerStateSnapshot {
                version: 1,
                control_mask: 1,
                state_mask: 0,
            },
        });

        assert!(
            !prune_snapshot_entries(&mut snapshot, &[42, 77]),
            "nothing changed when every recorded process is still enumerated"
        );
        assert_eq!(snapshot.processes.len(), 2);

        assert!(prune_snapshot_entries(&mut snapshot, &[77]));
        assert_eq!(snapshot.processes.len(), 1);
        assert_eq!(snapshot.processes[0].identity.pid, 77);
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
    fn a_sweep_with_discovery_counts_processes_the_snapshot_never_recorded() {
        use super::{aggregate_counts, classify_readback, ProcessPowerStateSnapshot as State};

        // F2: the snapshot is empty (it only stores non-default originals), so
        // a snapshot-driven sweep would inspect exactly zero processes. The
        // sweep must instead inspect what is alive right now.
        let empty_snapshot: Vec<(u32, State)> = Vec::new();
        let desired = State {
            version: 1,
            control_mask: 1,
            state_mask: 1,
        };
        let live = [
            (9001u32, State::DEFAULT),
            (9002u32, State::DEFAULT),
            (9003u32, State::DEFAULT),
        ];

        let outcomes = live.iter().map(|(pid, state)| {
            let original = empty_snapshot
                .iter()
                .find(|(snapshot_pid, _)| snapshot_pid == pid)
                .map(|(_, recorded)| recorded.clone())
                .unwrap_or(State::DEFAULT);
            classify_readback(state, &original, &desired)
        });
        let counts = aggregate_counts(outcomes, super::WORKER_MAX_BATCH);

        assert_eq!(counts.inspected, 3, "every live candidate is inspected");
        assert_eq!(counts.pending, 3, "none of them is throttled yet");
        assert_eq!(counts.applied, 0);
        assert!(
            counts.drift_detected(),
            "discovered processes must schedule a re-apply"
        );
    }

    #[test]
    fn measured_status_round_trips_to_sweep_counts() {
        let status = super::OsTurboMeasuredStatus {
            inspected: 5,
            applied: 2,
            pending: 1,
            restored: 1,
            skipped: 0,
            failures: 1,
            last_sweep_ms: 123,
            worker_active: true,
        };
        let counts = status.as_counts();
        assert_eq!(counts.inspected, 5);
        assert_eq!(counts.applied, 2);
        assert_eq!(counts.pending, 1);
        assert_eq!(counts.restored, 1);
        assert_eq!(counts.failures, 1);
        assert!(counts.drift_detected());
    }
}
