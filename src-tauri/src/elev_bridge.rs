//! Bridge from the main (unprivileged) process to the elevated helper task.
//!
//! Every privileged hardware operation is dispatched through here:
//!   0. Try the autonomous `MiControlBridge` Windows service via named pipe
//!      `\\.\pipe\micontrol_bridge` (installed at install time, runs as
//!      `NT AUTHORITY\SYSTEM` — NO UAC prompt ever, even after reboot).
//!   1. If the service is not installed/running, fall back to the
//!      `MiControlElevated` scheduled task: write a JSON command to
//!      `%LOCALAPPDATA%\MiControl\elev_cmd_<request_id>.json`, trigger the task
//!      via `schtasks /run`, poll the result file.
//!   2. As a last resort (dev mode / no installer), fall back to a UAC prompt.
//!
//! The service pipe is the preferred, fully-autonomous path: after
//! installation the app NEVER prompts for elevation. The scheduled task and
//! UAC paths remain only for development and for self-healing a broken install.
//!
//! # P1 (audit 2026-11-07): availability is decided by the SCM, not the pipe
//!
//! Until this was fixed, "is the bridge service available?" was answered with a
//! single `CreateFileW` on the pipe. When the bridge's accept loop deadlocked
//! (C-01), the service stayed `RUNNING` while the pipe stayed permanently busy
//! (`ERROR_PIPE_BUSY`), so the app concluded "not available" **597×** and fired
//! **702** `install_bridge_service` calls — every one of them a no-op because
//! the service was, in fact, running.
//!
//! The decision now uses [`util::win_service`] (`OpenServiceW` +
//! `QueryServiceStatusEx` semantics) and is expressed as a pure function,
//! [`decide_bridge_action`], over an injected [`BridgeProbe`]:
//!
//! | SCM state | pipe | action |
//! |-----------|------|--------|
//! | `Running` | (ignored) | [`BridgeAction::AlreadyRunning`] — never install |
//! | `Stopped` / pending | (ignored) | [`BridgeAction::StartService`] |
//! | `Absent` | (ignored) | [`BridgeAction::Install`] |
//! | unknown | reachable | [`BridgeAction::AlreadyRunning`] |
//! | unknown | unreachable | [`BridgeAction::Install`] |
//!
//! The pipe is still probed — but as *data-path health*, not as an existence
//! check, and only when the SCM could not answer.
//!
//! # P8 (audit 2026-11-07): why background commands never prompt for UAC
//!
//! `disable_copilot_key`, `set_scancode_map`, `ensure_bridge_service` and
//! `ensure_face_service` run during the first seconds after boot, several of
//! them from background pollers. Letting those fall back to a `runas` prompt is
//! what produced the MIOT-38 "UAC storm": a consent dialog every few seconds
//! when the scheduled task was missing. Keeping the fallback disabled for
//! **background** commands is therefore intentional, not an oversight.
//!
//! The trade-off used to be silent: a timeout logged a one-line WARN from the
//! caller and nothing else. Now every failure of a no-prompt command emits a
//! single actionable WARN (`cmd`, elapsed, reason, policy) and a one-time boot
//! summary ([`spawn_boot_summary`]). For debugging a machine where the
//! scheduled task is broken, `MICONTROL_ELEV_ALLOW_UAC=1` opts that session into
//! the UAC fallback; it is off by default and never applies to the two service
//! `ensure_*` commands, which must never prompt at boot.

use crate::util::auth;
use crate::util::rate_limit::MessageRateLimiter;
use crate::util::win_service::{self, ServiceState};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Name of the scheduled task registered by the NSIS installer.
const TASK_NAME: &str = "MiControlElevated";
/// SCM name of the autonomous elevated service installed at install time.
const BRIDGE_SERVICE_NAME: &str = "MiControlBridge";
/// Named pipe of the autonomous elevated service installed at install time.
const BRIDGE_PIPE_NAME: &str = r"\\.\pipe\micontrol_bridge";
const POLL_INTERVAL_MS: u64 = 150;
const ELEV_TIMEOUT_SECS: u64 = 15;
const STALE_FILE_MAX_AGE_SECS: u64 = 120;
static ELEV_REQUEST_LOCK: Mutex<()> = Mutex::const_new(());

/// UAC prompts from background elevated dispatch are one-shot by default:
/// once the user cancels a consent dialog (or the elevated helper fails to
/// launch), every subsequent elevated call in this process skips the prompt
/// and returns an error instead. This prevents the MIOT-38 UAC storm — the
/// `MCElev_heal.bat` consent popup firing every few seconds — when the
/// scheduled task is broken AND the bridge service is absent.
static UAC_DECLINED: AtomicBool = AtomicBool::new(false);

/// SCM state of the bridge service, collapsed for the availability decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeServiceState {
    Running,
    Stopped,
    Absent,
    Unknown,
}

impl BridgeServiceState {
    /// Fold pending/paused states into the closest actionable bucket.
    ///
    /// `Pending` is treated as `Stopped` on purpose: asking the SCM to start a
    /// service that is already starting returns `ERROR_SERVICE_ALREADY_RUNNING`
    /// (handled as success), whereas installing would be the heavy, wrong move.
    fn from_scm(state: ServiceState) -> Self {
        match state {
            ServiceState::Running => Self::Running,
            ServiceState::Stopped | ServiceState::Pending | ServiceState::Paused => Self::Stopped,
            ServiceState::Absent => Self::Absent,
            ServiceState::Unknown => Self::Unknown,
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Stopped => "stopped",
            Self::Absent => "absent",
            Self::Unknown => "unknown",
        }
    }
}

/// Inputs of the availability decision (injected so the rule is testable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeProbe {
    pub service: BridgeServiceState,
    /// Result of a `CreateFileW` probe. Only consulted when the SCM could not
    /// answer; a wedged pipe must never make a running service look missing.
    pub pipe_reachable: bool,
}

/// What [`decide_bridge_action`] wants the caller to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeAction {
    /// Service is (or is believed to be) already running — do nothing.
    AlreadyRunning,
    /// Service exists but is stopped — try `StartServiceW`, no install.
    StartService,
    /// Service does not exist (or cannot be identified) — dispatch the
    /// elevated `ensure_bridge_service` installer.
    Install,
}

/// P1: decide what to do about the bridge service from its SCM state.
///
/// Pure and total: every combination has a defined outcome. The key property —
/// the one that fixes the 597-warning loop — is that a `Running` service is
/// never re-installed, no matter how busy its pipe is.
pub fn decide_bridge_action(probe: BridgeProbe) -> BridgeAction {
    match probe.service {
        BridgeServiceState::Running => BridgeAction::AlreadyRunning,
        BridgeServiceState::Stopped => BridgeAction::StartService,
        BridgeServiceState::Absent => BridgeAction::Install,
        BridgeServiceState::Unknown => {
            if probe.pipe_reachable {
                BridgeAction::AlreadyRunning
            } else {
                BridgeAction::Install
            }
        }
    }
}

/// Global rate limiter for the diagnostics that used to repeat thousands of
/// times (P1). One line per message every 5 minutes; the next emitted line
/// reports how many identical messages were suppressed.
static ELEV_LOG_LIMITER: MessageRateLimiter =
    MessageRateLimiter::new(MessageRateLimiter::DEFAULT_WINDOW);

/// Emit `render(suppressed)` at most once per key per 5 minutes.
fn log_rate_limited(warn: bool, key: &'static str, render: impl FnOnce(u32) -> String) {
    if let Some(suppressed) = ELEV_LOG_LIMITER.allow(key) {
        let suffix = if suppressed > 0 {
            format!(" [{suppressed} identical message(s) suppressed in the last 5 min]")
        } else {
            String::new()
        };
        let line = format!("{}{suffix}", render(suppressed));
        if warn {
            log::warn!(target: "elev_bridge", "{line}");
        } else {
            log::info!(target: "elev_bridge", "{line}");
        }
    }
}

/// Query the SCM for the bridge service state. Cheap (no pipe, no UAC).
pub fn bridge_service_state() -> BridgeServiceState {
    BridgeServiceState::from_scm(win_service::query_service_state(BRIDGE_SERVICE_NAME))
}

/// Cheap `CreateFileW` probe of the bridge pipe.
///
/// This answers "can I talk to the bridge right now?", NOT "is the service
/// installed?". P1 exists because those two questions used to be conflated.
pub fn is_bridge_pipe_reachable() -> bool {
    // leak-check: audited — every exit path closes the handle explicitly (S67).
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::{
            CloseHandle, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE,
        };
        use windows::Win32::Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        };

        let path_w: Vec<u16> = std::ffi::OsStr::new(BRIDGE_PIPE_NAME)
            .encode_wide()
            .chain(Some(0))
            .collect();
        // SAFETY: path_w is a valid null-terminated wide string.
        let handle = unsafe {
            CreateFileW(
                PCWSTR(path_w.as_ptr()),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                windows::Win32::Foundation::HANDLE::default(),
            )
        };
        match handle {
            Ok(h) if h != INVALID_HANDLE_VALUE => {
                unsafe {
                    CloseHandle(h).ok();
                }
                true
            }
            _ => false,
        }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Data-path health of the bridge: service state plus pipe reachability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeHealth {
    /// Service `RUNNING` and the pipe accepted a connection.
    Healthy,
    /// Service `RUNNING` but the pipe is wedged/busy (the C-01 symptom).
    ServiceRunningPipeUnavailable,
    /// Service installed, not running.
    ServiceStopped,
    /// Service not installed.
    ServiceAbsent,
    /// SCM could not be queried.
    ServiceUnknown,
}

impl BridgeHealth {
    pub fn is_healthy(self) -> bool {
        matches!(self, Self::Healthy)
    }

    /// The service itself exists and is running (the pipe may still be wedged).
    pub fn service_running(self) -> bool {
        matches!(self, Self::Healthy | Self::ServiceRunningPipeUnavailable)
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::Healthy => "service running, pipe reachable",
            Self::ServiceRunningPipeUnavailable => "service running but pipe unreachable",
            Self::ServiceStopped => "service stopped",
            Self::ServiceAbsent => "service absent",
            Self::ServiceUnknown => "service state unknown",
        }
    }
}

/// Probe both layers. The pipe is only touched when the service is running —
/// there is nothing to talk to otherwise, and the probe costs a handle.
pub fn bridge_health() -> BridgeHealth {
    match bridge_service_state() {
        BridgeServiceState::Running => {
            if is_bridge_pipe_reachable() {
                BridgeHealth::Healthy
            } else {
                BridgeHealth::ServiceRunningPipeUnavailable
            }
        }
        BridgeServiceState::Stopped => BridgeHealth::ServiceStopped,
        BridgeServiceState::Absent => BridgeHealth::ServiceAbsent,
        BridgeServiceState::Unknown => BridgeHealth::ServiceUnknown,
    }
}

/// Returns `true` when the autonomous `MiControlBridge` service exists and is
/// `RUNNING` **according to the SCM**.
///
/// P1: this used to be a pipe probe. The pipe says nothing about whether the
/// service is installed, and a wedged accept loop (C-01) made this return
/// `false` forever against a perfectly running service.
pub fn is_bridge_service_available() -> bool {
    bridge_service_state() == BridgeServiceState::Running
}

/// Ensure the autonomous `MiControlBridge` service is installed and running.
///
/// Decision (P1): SCM state first.
///   * `RUNNING` → return immediately (`already_running`). **No install.**
///   * `STOPPED` → try `StartServiceW` directly (cheap, usually permitted);
///     only if that is refused, dispatch the elevated helper.
///   * absent/unknown → dispatch `ensure_bridge_service` through the elevated
///     path (scheduled task, already elevated — no UAC).
///
/// Never shows a UAC prompt (see the P8 note in the module docs): a consent
/// dialog at boot is exactly what the autonomous-bridge design prevents.
pub async fn ensure_bridge_service() -> Result<Value, String> {
    let service = bridge_service_state();
    let probe = BridgeProbe {
        service,
        // Only spend a handle probing the pipe when the SCM could not answer.
        pipe_reachable: service == BridgeServiceState::Unknown && is_bridge_pipe_reachable(),
    };

    match decide_bridge_action(probe) {
        BridgeAction::AlreadyRunning => Ok(serde_json::json!({
            "status": "already_running",
            "decided_by": "scm",
            "service_state": service.describe(),
        })),
        BridgeAction::StartService => {
            // `StartServiceW` + the bounded RUNNING wait sleep on the calling
            // thread — run it on the fixed blocking pool so the async runtime
            // is never stalled (S69: fixed pool, no thread churn).
            let start_result: Result<(), String> =
                match crate::util::fixed_pool::run_blocking_timeout(Duration::from_secs(12), || {
                    win_service::start_service(BRIDGE_SERVICE_NAME)
                })
                .await
                {
                    Ok(inner) => inner,
                    Err(e) => Err(format!("start_service task failed: {e}")),
                };

            match start_result {
                Ok(()) => {
                    log::info!(
                        target: "elev_bridge",
                        "MiControlBridge was stopped — started via SCM (no install needed)"
                    );
                    Ok(serde_json::json!({
                        "status": "started",
                        "decided_by": "scm",
                        "service_state": "stopped",
                    }))
                }
                Err(e) => {
                    // A standard user may lack SERVICE_START. Not a failure of
                    // the app: the elevated helper can start it with SYSTEM
                    // rights, and that path needs no consent dialog.
                    log_rate_limited(false, "bridge_start_direct", |_| {
                        format!(
                            "MiControlBridge is stopped and could not be started directly ({e}) — \
                             dispatching the elevated start"
                        )
                    });
                    run_elevated_strict("ensure_bridge_service", serde_json::json!({})).await
                }
            }
        }
        BridgeAction::Install => {
            log_rate_limited(false, "bridge_install", |_| {
                "MiControlBridge service not installed (SCM: absent/unknown) — \
                 installing via elevated path"
                    .to_string()
            });
            run_elevated_strict("ensure_bridge_service", serde_json::json!({})).await
        }
    }
}

/// Post-reboot self-heal for the `MiControlFace` auth service.
///
/// The face service crashes ~60 min after boot (0xc0000005 in
/// FrameServerClient.dll_unloaded) and — having no SCM failure actions — is
/// left STOPPED-1067 forever, breaking Face Unlock after every reboot.
///
/// This walks the same launch-ordering ladder as `ensure_bridge_service`:
///   0. If the MiControlBridge service pipe is already available, dispatch
///      `ensure_face_service` through it (SYSTEM, no UAC, works even when the
///      face service is dead).
///   1. Otherwise fall back to the MiControlElevated scheduled task.
///
/// Never prompts UAC — strictly, even when `MICONTROL_ELEV_ALLOW_UAC=1`, since
/// this runs unattended at boot. Callers invoke this once at app startup and
/// on the Face Unlock tab when `service_running` is false.
pub async fn ensure_face_service() -> Result<Value, String> {
    run_elevated_strict("ensure_face_service", serde_json::json!({})).await
}
static NEXT_REQ: AtomicU64 = AtomicU64::new(1);

/// Outcome of a task-path self-healing attempt.
enum TaskHealResult {
    /// Task already exists and points to the current exe — no action needed.
    AlreadyCorrect,
    /// Task was missing or mis-pointed and has been re-registered successfully.
    Healed,
    /// Healing was attempted but failed (not admin, UAC declined, etc.).
    Failed,
}

/// Timeout for slow commands that do WMI/IOCTL probes or driver installs.
const ELEV_TIMEOUT_SLOW_SECS: u64 = 90;
/// Timeout for medium commands (WMI queries on cold start).
const ELEV_TIMEOUT_MEDIUM_SECS: u64 = 45;

/// Returns the timeout for a given elevated command.
///
/// Slow commands (hardware discovery, driver install) do WMI + IOCTL probes
/// or run `pnputil`, which can take 30–60 s on a cold system. The task
/// scheduler's `ExecutionTimeLimit` is PT120S, so the bridge timeout must be
/// shorter than that to avoid waiting for a killed helper.
fn timeout_for_cmd(cmd: &str) -> Duration {
    match cmd {
        "run_hardware_discovery" | "install_driver" | "trigger_driver_scan" => {
            Duration::from_secs(ELEV_TIMEOUT_SLOW_SECS)
        }
        "wmi_ec_read_sensor_data"
        | "wmi_ec_read_battery_health"
        | "wmi_ec_read_adapter_power"
        | "wmi_ec_get_performance_mode"
        | "diag_wmi_query"
        | "set_battery_care"
        | "set_eye_protection"
        | "set_os_turbo"
        | "refresh_os_turbo"
        | "set_function_key" => Duration::from_secs(ELEV_TIMEOUT_MEDIUM_SECS),

        // Audit F2.1: the elevated half of a cleanup is now `clean_system_junk`
        // (Windows Update cache + Windows logs). It walks two trees, so it gets
        // the slow budget like any other multi-step elevated command.
        "clean_system_junk" | "set_sys_opt_tweak" => {
            // S55 FIX 7: set_sys_opt_tweak can run heavy operations — OneDrive
            // uninstall (PowerShell + OneDriveSetup wait), services sweep
            // (multiple sc stop), Appx removals. The default 15 s timeout
            // killed the pipe round-trip mid-operation: the UI showed an
            // error while the bridge kept working, leaving the toggle state
            // inconsistent ("clico num e o outro some"). Give it the slow
            // budget (90 s).
            Duration::from_secs(ELEV_TIMEOUT_SLOW_SECS)
        }
        // SCM query + start is quick (~1-2 s), but `sc failure` may hit a
        // busy SCM on cold boot — give it the medium timeout.
        "ensure_face_service" => Duration::from_secs(ELEV_TIMEOUT_MEDIUM_SECS),
        // `archive_event_log` shells out twice (`wevtutil epl` of a full
        // 20 MB channel, then `wevtutil cl`). Both are single-digit seconds on
        // a healthy disk, but the export rewrites the whole `.evtx` and the
        // EventLog service serializes it, so the 15 s default is too tight.
        // Note: when this runs through the bridge PIPE, the service's own
        // 30 s dispatch deadline still caps it; the scheduled-task path gets
        // the full medium budget under the task's PT120S ExecutionTimeLimit.
        "archive_event_log" => Duration::from_secs(ELEV_TIMEOUT_MEDIUM_SECS),
        // S64: `ensure_ecram_service` legitimately takes ~15.2 s (it probes the
        // pipe, compares the DriverStore binary, may copy ~500 KB and run
        // `sc stop/delete/create/failure`, then waits up to 10 s for the pipe).
        // The 15 s default made the client give up while the bridge was still
        // working: bridge.log shows `ok=true total_elapsed_ms=15174` for a call
        // the client reported as failed, so every watchdog tick logged
        // "service recovery failed" and the recovery never registered as
        // successful. Give it the slow budget.
        "ensure_ecram_service" => Duration::from_secs(ELEV_TIMEOUT_SLOW_SECS),
        _ => Duration::from_secs(ELEV_TIMEOUT_SECS),
    }
}

/// Dispatch a privileged command through the autonomous elevated service,
/// the scheduled elevated task, or (last resort) a UAC prompt.
///
/// `cmd` must match one of the branches in `elevated::dispatch()`.
/// `args` is the JSON arguments object (use `serde_json::json!({...})`).
pub async fn run_elevated(cmd: &'static str, args: Value) -> Result<Value, String> {
    run_elevated_impl(cmd, args, true).await
}

/// P8: opt-in for the UAC fallback on **background** commands.
///
/// Default `false`. The four boot-time commands that timed out in the audit
/// (`disable_copilot_key`, `set_scancode_map`, `ensure_bridge_service`,
/// `ensure_face_service`) run unattended; letting them pop `runas` dialogs is
/// the MIOT-38 UAC storm. The flag exists so a developer/field engineer can
/// diagnose a machine with a broken scheduled task without rebuilding:
///
/// ```text
/// set MICONTROL_ELEV_ALLOW_UAC=1 && micontrol.exe
/// ```
///
/// When it is set, the very first background failure that reaches the UAC
/// fallback logs a WARN so the behaviour is never silent.
pub fn background_uac_fallback_enabled() -> bool {
    std::env::var("MICONTROL_ELEV_ALLOW_UAC")
        .map(|value| parse_bool_flag(&value))
        .unwrap_or(false)
}

/// Pure parser for boolean env flags (`1`, `true`, `yes`, `on`; case-insensitive).
pub fn parse_bool_flag(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Dispatch a privileged command without prompting UAC **by default**.
///
/// Uses the same bridge-first / scheduled-task fallback chain as
/// [`run_elevated`], but if no path succeeds it returns an error instead of
/// popping the elevation consent dialog. Use this for non-critical reads and
/// background work (thermal/temperature polling, boot-time policy writes) where
/// a UAC prompt at boot would be a terrible user experience — the worst case is
/// a missing value.
///
/// P8: the fallback can be enabled for a debugging session with
/// `MICONTROL_ELEV_ALLOW_UAC=1` (see [`background_uac_fallback_enabled`]). It is
/// off by default; call [`run_elevated_strict`] behaviour via the service
/// `ensure_*` entry points, which never prompt.
pub async fn run_elevated_no_prompt(cmd: &'static str, args: Value) -> Result<Value, String> {
    run_elevated_impl(cmd, args, background_uac_fallback_enabled()).await
}

/// Dispatch with the UAC fallback forcibly disabled, ignoring the env opt-in.
///
/// Used by the boot-critical service self-heal (`ensure_bridge_service`,
/// `ensure_face_service`): a consent dialog raised at startup by the app itself
/// is never acceptable, even in a debugging session.
async fn run_elevated_strict(cmd: &'static str, args: Value) -> Result<Value, String> {
    run_elevated_impl(cmd, args, false).await
}

/// Dispatch a command exclusively through the authenticated bridge service.
///
/// This deliberately does not use direct elevated dispatch, the scheduled
/// task, or the UAC fallback. Feature-specific administrator writes that need
/// the autonomous service boundary must call this helper instead of the
/// generic elevation ladder.
pub async fn run_elevated_bridge_only(cmd: &'static str, args: Value) -> Result<Value, String> {
    let result = run_via_service_pipe(cmd, args).await;
    if result.is_ok() {
        note_pipe_success();
    }
    result
}

// ── S51: session circuit breaker for a degraded elevated infrastructure ─────
//
// When neither the bridge service NOR the scheduled task is available (e.g.
// the service was deleted by a previous update and the task is missing), every
// elevated call runs the full failure chain: pipe attempt → schtasks spawn →
// heal attempt → schtasks retry → 3-4 WARN logs. At boot ~12 features fire
// elevated commands simultaneously, producing ~90 WARNs and a dozen pointless
// process spawns in a single second.
//
// The breaker tracks consecutive "no elevated path" failures. After
// BREAKER_TRIP_THRESHOLD consecutive failures it short-circuits with the
// cached error (no schtasks spawn, single log line) until
// `note_elevated_recovery()` is called — either by a successful call or by the
// periodic `ensure_bridge_service` health check succeeding.
const BREAKER_TRIP_THRESHOLD: u32 = 3;

static ELEVATED_BROKEN_STREAK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static ELEVATED_BROKEN_SINCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// True when the elevated infrastructure is known-degraded this session and
/// further attempts would just repeat the same failure chain.
fn elevated_broken() -> bool {
    ELEVATED_BROKEN_STREAK.load(std::sync::atomic::Ordering::Relaxed) >= BREAKER_TRIP_THRESHOLD
}

/// Record an elevated-path failure (pipe + task both unavailable).
fn note_elevated_failure() {
    let streak = ELEVATED_BROKEN_STREAK.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    if streak == BREAKER_TRIP_THRESHOLD {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        ELEVATED_BROKEN_SINCE.store(now, std::sync::atomic::Ordering::Relaxed);
        log::warn!(
            target: "elev_bridge",
            "Elevated infrastructure unavailable (bridge service missing, scheduled task \
             missing) — circuit open, elevated commands will fail fast this session. \
             Reinstall MiControl or run 'micontrol_bridge.exe install' (as admin) to repair."
        );
    }
}

/// Record an elevated-path success — closes the circuit.
fn note_elevated_success() {
    let was_broken = ELEVATED_BROKEN_STREAK.swap(0, std::sync::atomic::Ordering::Relaxed);
    if was_broken >= BREAKER_TRIP_THRESHOLD {
        log::info!(target: "elev_bridge", "Elevated infrastructure recovered — circuit closed");
    }
    ELEVATED_BROKEN_SINCE.store(0, std::sync::atomic::Ordering::Relaxed);
}

/// Manually close the circuit breaker (e.g. after a successful bridge-service
/// health check that did not go through `run_elevated`).
pub fn note_elevated_recovery() {
    note_elevated_success();
}

// ── P8: pipe-health cooldown ────────────────────────────────────────────────
//
// C-01 wedges the bridge's accept loop: the service is RUNNING, the pipe exists
// but never answers. Without a cooldown every elevated command first spends its
// whole 15 s pipe budget (twice, because of the S66-002 retry) before falling
// back to the scheduled task — which is why the four boot-time commands in the
// audit timed out at 15 s/15 s/15 s/45 s instead of reaching the task path
// immediately. After two consecutive pipe timeouts the pipe is skipped for
// 60 s; a single successful round-trip resets the streak.
const PIPE_TIMEOUT_TRIP: u32 = 2;
const PIPE_COOLDOWN: Duration = Duration::from_secs(60);

/// Streak + cooldown state for the bridge pipe.
///
/// Pure and clock-injected so the transitions are unit-testable; the process
/// global below guards one instance.
#[derive(Debug)]
struct PipeCooldown {
    streak: u32,
    skip_until_ms: u64,
}

impl PipeCooldown {
    const fn new() -> Self {
        Self {
            streak: 0,
            skip_until_ms: 0,
        }
    }

    /// `true` when the pipe may be attempted at `now_ms`.
    fn allowed(&self, now_ms: u64) -> bool {
        now_ms >= self.skip_until_ms
    }

    /// A working round-trip clears the streak.
    fn note_success(&mut self) {
        self.streak = 0;
    }

    /// Record a timeout. Returns `Some(streak)` **only** when this timeout
    /// opened a new cooldown window (so the caller logs it once).
    fn note_timeout(&mut self, now_ms: u64) -> Option<u32> {
        self.streak = self.streak.saturating_add(1);
        if self.streak >= PIPE_TIMEOUT_TRIP && self.allowed(now_ms) {
            self.skip_until_ms = now_ms.saturating_add(PIPE_COOLDOWN.as_millis() as u64);
            Some(self.streak)
        } else {
            None
        }
    }
}

static PIPE_COOLDOWN_STATE: std::sync::Mutex<PipeCooldown> =
    std::sync::Mutex::new(PipeCooldown::new());

/// Milliseconds since process start (monotonic; immune to clock changes).
fn process_elapsed_ms() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START
        .get_or_init(Instant::now)
        .elapsed()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

/// `false` while the pipe is in its post-timeout cooldown.
fn pipe_attempt_allowed() -> bool {
    let state = match PIPE_COOLDOWN_STATE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    state.allowed(process_elapsed_ms())
}

/// Record a pipe timeout / empty response and open the cooldown on the
/// transition. `class` names the failure shape for the log line.
fn note_pipe_timeout(class: &'static str) {
    let opened = {
        let mut state = match PIPE_COOLDOWN_STATE.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        state.note_timeout(process_elapsed_ms())
    };

    if let Some(streak) = opened {
        log_rate_limited(true, "pipe_cooldown", |_| {
            format!(
                "Bridge pipe did not answer {streak}× in a row ({class}) — skipping it for {} s so \
                 elevated commands reach the scheduled task without burning their 15 s pipe budget",
                PIPE_COOLDOWN.as_secs()
            )
        });
    }
}

/// A working round-trip proves the pipe is fine again.
fn note_pipe_success() {
    let mut state = match PIPE_COOLDOWN_STATE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    state.note_success();
}

// ── P8: boot-time elevated-command report ───────────────────────────────────
//
// The audit found four elevated commands failing silently during the first
// minute after boot, each with a one-line WARN from its caller. Diagnostics are
// now centralised: every no-prompt failure is recorded, and a one-time summary
// is emitted 90 s after startup listing command, elapsed and reason.
const BOOT_WINDOW: Duration = Duration::from_secs(120);
const BOOT_SUMMARY_DELAY: Duration = Duration::from_secs(90);

/// One failed elevated command inside the boot window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootFailure {
    pub cmd: &'static str,
    pub elapsed_ms: u64,
    pub attempts: u32,
    pub reason: String,
}

#[derive(Default)]
struct BootReport {
    failures: Vec<BootFailure>,
    successes: u32,
    summarised: bool,
}

static BOOT_REPORT: std::sync::OnceLock<std::sync::Mutex<BootReport>> = std::sync::OnceLock::new();

fn boot_report() -> &'static std::sync::Mutex<BootReport> {
    BOOT_REPORT.get_or_init(|| std::sync::Mutex::new(BootReport::default()))
}

/// Record the outcome of an elevated call made during the boot window.
///
/// `&'static str` cmd keeps the registry allocation-free for the common
/// success path; failures store the reason string (bounded, see
/// [`format_boot_summary`]).
fn note_boot_outcome(cmd: &'static str, elapsed: Duration, result: &Result<Value, String>) {
    if process_elapsed_ms() > BOOT_WINDOW.as_millis() as u64 {
        return;
    }
    let mut report = match boot_report().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if report.summarised {
        return;
    }
    match result {
        Ok(_) => report.successes = report.successes.saturating_add(1),
        Err(reason) => {
            let elapsed_ms = elapsed.as_millis().min(u64::MAX as u128) as u64;
            if let Some(existing) = report.failures.iter_mut().find(|f| f.cmd == cmd) {
                existing.attempts = existing.attempts.saturating_add(1);
                existing.elapsed_ms = elapsed_ms;
                existing.reason = reason.clone();
            } else {
                report.failures.push(BootFailure {
                    cmd,
                    elapsed_ms,
                    attempts: 1,
                    reason: reason.clone(),
                });
            }
        }
    }
}

/// Render the one-time boot summary. Pure so the format is unit-tested.
///
/// Long reasons are truncated: the summary is a pointer to the per-command WARN
/// (which carries the full text), not a replacement for it.
pub fn format_boot_summary(failures: &[BootFailure], successes: u32) -> String {
    const MAX_REASON: usize = 180;
    if failures.is_empty() {
        return format!(
            "Elevated bootstrap summary (first {} s): {successes} command(s) succeeded, none failed",
            BOOT_WINDOW.as_secs()
        );
    }

    let mut line = format!(
        "Elevated bootstrap summary (first {} s): {successes} succeeded, {} failed — \
         UAC fallback is disabled by design for background commands (MIOT-38); \
         set MICONTROL_ELEV_ALLOW_UAC=1 to enable it for a debugging session",
        BOOT_WINDOW.as_secs(),
        failures.len()
    );

    for failure in failures {
        let flattened = failure.reason.replace(['\r', '\n'], " ");
        let mut reason: String = flattened.chars().take(MAX_REASON).collect();
        if reason.len() < flattened.len() {
            reason.push('…');
        }
        line.push_str(&format!(
            "; {} (attempt {}, {} ms): {}",
            failure.cmd, failure.attempts, failure.elapsed_ms, reason
        ));
    }
    line
}

/// Spawn the one-time boot summary (90 s after startup).
///
/// Called once from `lib.rs` setup. If the app is a short-lived dev run the task
/// dies with the runtime — acceptable: the per-command WARNs are already in the
/// log.
pub fn spawn_boot_summary() {
    if background_uac_fallback_enabled() {
        log::warn!(
            target: "elev_bridge",
            "MICONTROL_ELEV_ALLOW_UAC is set — background elevated failures may now raise a \
             UAC consent dialog (debug mode)"
        );
    }

    tauri::async_runtime::spawn(async {
        tokio::time::sleep(BOOT_SUMMARY_DELAY).await;
        let (failures, successes) = {
            let mut report = match boot_report().lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            if report.summarised {
                return;
            }
            report.summarised = true;
            (report.failures.clone(), report.successes)
        };

        if failures.is_empty() {
            log::info!(target: "elev_bridge", "{}", format_boot_summary(&failures, successes));
        } else {
            log::warn!(target: "elev_bridge", "{}", format_boot_summary(&failures, successes));
        }
    });
}

/// Measure, report and record an elevated call.
///
/// Wrapper around [`run_elevated_inner`] so every exit path (including the
/// circuit-breaker short-circuit) is accounted for exactly once.
async fn run_elevated_impl(
    cmd: &'static str,
    args: Value,
    allow_uac: bool,
) -> Result<Value, String> {
    let started = Instant::now();
    let result = run_elevated_inner(cmd, args, allow_uac).await;
    let elapsed = started.elapsed();

    note_boot_outcome(cmd, elapsed, &result);

    if let Err(reason) = &result {
        // P8: one actionable line per command per 5 min — command, elapsed,
        // reason and policy. Callers may log their own context too; this is the
        // canonical line for triage.
        log_rate_limited(true, cmd, |_| {
            format!(
                "Elevated command '{cmd}' failed after {} ms (uac_fallback={}): {reason}",
                elapsed.as_millis(),
                if allow_uac {
                    "allowed"
                } else {
                    "disabled by design for background commands (MIOT-38)"
                }
            )
        });
    }

    result
}

/// Shared implementation; `allow_uac` controls whether the function may
/// escalate via a `ShellExecuteExW("runas")` UAC prompt as a last resort.
async fn run_elevated_inner(
    cmd: &'static str,
    args: Value,
    allow_uac: bool,
) -> Result<Value, String> {
    // Serialise elevated calls. The scheduled-task path has no request-id argv,
    // so the elevated helper discovers the newest pending file. Running one at a
    // time prevents cross-request mixups.
    let _guard = ELEV_REQUEST_LOCK.lock().await;

    // ── Fast path: already elevated ──────────────────────────────────────────
    // When this process is running as an administrator (dev mode or installed
    // with admin manifest), dispatch the privileged operation directly in a
    // blocking thread.  This eliminates the ~15 s scheduled-task round-trip.
    // S69: FIXED pool (no tokio blocking-pool churn).
    #[cfg(windows)]
    if is_admin() {
        let args2 = args.clone();
        let cmd_static: &'static str = cmd;
        return match crate::util::fixed_pool::run_blocking_timeout(
            Duration::from_secs(timeout_for_cmd(cmd).as_secs()),
            move || {
                let result = crate::elevated::dispatch_cmd(cmd_static, args2);
                if result["ok"].as_bool().unwrap_or(false) {
                    Ok(result["data"].clone())
                } else {
                    Err(result["error"]
                        .as_str()
                        .unwrap_or("elevated dispatch failed")
                        .to_string())
                }
            },
        )
        .await
        {
            Ok(v) => v,
            Err(e) => Err(format!("blocking task failed: {e}")),
        };
    }

    // ── Preferred path: autonomous MiControlBridge service pipe ─────────────
    // The service runs as SYSTEM since installation — no UAC prompt ever.
    //
    // P8: honour the pipe-health cooldown. When the service answers but its
    // pipe is wedged (C-01), attempting it costs the full 15 s budget per try.
    // After two consecutive timeouts the pipe is skipped for 60 s so the
    // command reaches the scheduled task immediately.
    let mut pipe_was_skipped = false;
    if pipe_attempt_allowed() {
        if let Ok(response) = run_via_service_pipe(cmd, args.clone()).await {
            note_pipe_success();
            note_elevated_success();
            return Ok(response);
        }
    } else {
        pipe_was_skipped = true;
    }

    // ── S66-002: retry the pipe once before escalating to the task path ────
    // The bridge answers routine polls in ~14 ms average (47k ok responses
    // over 4 days), yet transient pipe hiccups (ERROR_PIPE_BUSY, a busy
    // worker, a slow HMAC read) dropped ~90-150 calls/hour into the
    // scheduled-task path — each one spawning a full helper process that
    // used to crash at exit (S66-001) and still costs a process spawn +
    // WER report. A single short retry absorbs the transient failures
    // without measurably delaying the genuine-failure fallback.
    if pipe_attempt_allowed() {
        if let Ok(response) = run_via_service_pipe(cmd, args.clone()).await {
            note_pipe_success();
            note_elevated_success();
            log::debug!(
                target: "elev_bridge",
                "Elevated command '{cmd}' succeeded on pipe retry (transient failure absorbed)"
            );
            return Ok(response);
        }
    } else if !pipe_was_skipped {
        // The first attempt tripped the cooldown mid-call; do not retry.
        pipe_was_skipped = true;
    }
    // Service pipe unavailable after retry — fall through to the scheduled task.
    if pipe_was_skipped {
        log::debug!(
            target: "elev_bridge",
            "Pipe attempt skipped for '{cmd}' (post-timeout cooldown active) — using the scheduled task"
        );
    }

    // ── S51 circuit breaker: short-circuit when the infra is known-degraded ─
    // Only for no-prompt calls: critical user-initiated writes (UAC allowed)
    // always get the full attempt chain, since the user explicitly asked.
    if !allow_uac && elevated_broken() {
        return Err(format!(
            "Elevated command '{cmd}' skipped: elevated infrastructure is degraded \
             (bridge service and scheduled task both unavailable) — circuit open"
        ));
    }

    let dir = crate::elevated::elev_dir();
    // S26-006: run on a blocking thread — cleanup_stale_elev_files() uses
    // std::fs::read_dir. S69: FIXED pool (no tokio blocking-pool churn).
    let dir_clone = dir.clone();
    crate::util::fixed_pool::run_blocking_timeout(Duration::from_secs(10), move || {
        cleanup_stale_elev_files(&dir_clone)
    })
    .await
    .map_err(|e| format!("cleanup_stale_elev_files task failed: {e}"))?;

    let request_id = make_request_id();
    let cmd_path = dir.join(cmd_file_name(&request_id));
    let result_path = dir.join(result_file_name(&request_id));
    let nonce = auth::generate_nonce();
    let mut payload = serde_json::json!({
        "protocol_version": 2,
        "request_id": request_id,
        "created_at_ms": auth::now_ms(),
        "nonce": nonce,
        "caller_pid": std::process::id(),
        "cmd": cmd,
        "args": args,
    });

    // Sign the payload with HMAC-SHA256 using the shared key.
    // S22-002: get_or_create_key() does sync file I/O with a 5-second polling
    // loop, which would block the async runtime. S69: FIXED pool (no tokio
    // blocking-pool churn).
    let key = crate::util::fixed_pool::run_blocking_timeout(
        Duration::from_secs(10),
        auth::get_or_create_key,
    )
    .await
    .map_err(|e| format!("HMAC key task failed: {e}"))?
    .map_err(|e| format!("Cannot obtain HMAC key: {e}"))?;
    auth::sign_payload(&mut payload, &key);

    // Remove any stale result from a previous run for this request id.
    let _ = tokio::fs::remove_file(&result_path).await;

    // Write the command payload atomically: write to a temp file, then rename.
    // This eliminates the TOCTOU race — the elevated helper never sees a
    // partially-written file.
    let tmp_path = dir.join(format!("elev_cmd_{request_id}.tmp"));
    tokio::fs::write(&tmp_path, payload.to_string())
        .await
        .map_err(|e| format!("Cannot write elevated command: {e}"))?;
    tokio::fs::rename(&tmp_path, &cmd_path)
        .await
        .map_err(|e| format!("Cannot rename elevated command file: {e}"))?;
    if let Err(e) = auth::restrict_file_acl(&cmd_path) {
        log::warn!("Failed to restrict ACL on command file: {e}");
    }

    // Launch the scheduled task (returns immediately; task runs asynchronously).
    // CREATE_NO_WINDOW prevents the flash of a console window on every call.
    let task_ok = run_schtasks_run().await;

    // S37-001: Release the serialization lock BEFORE the result-poll loop.
    // Holding ELEV_REQUEST_LOCK across the 15 s poll serializes every
    // elevated call behind a single slot: while the thermal poll (which runs
    // every ~15 s) holds the lock, a set_performance_mode invoke would starve
    // indefinitely. The lock only needs to guard the write+dispatch critical
    // section above — the request_id is unique, so concurrent poll loops are
    // safe.
    drop(_guard);

    if !task_ok {
        // Self-healing: try to re-register the scheduled task with the correct
        // path. This fixes the case where the task was registered during
        // `cargo tauri dev` and points to the debug exe.
        // S69: FIXED pool (no tokio blocking-pool churn).
        let healed = crate::util::fixed_pool::run_blocking_timeout(
            Duration::from_secs(30),
            ensure_task_correct_path,
        )
        .await
        .map_err(|e| format!("task heal task failed: {e}"))?;

        let no_uac = || async {
            log::warn!(
                "Scheduled task unavailable and UAC prompts are disabled for this \
                 command — returning error instead"
            );
            let _ = tokio::fs::remove_file(&cmd_path).await;
            Err(format!(
                "Elevated command not executed: scheduled task '{}' is unavailable \
                 and UAC fallback is disabled for this operation.",
                TASK_NAME
            ))
        };

        match healed {
            TaskHealResult::AlreadyCorrect => {
                // Task was fine — the original /run failure was transient.
                // Retry once before falling back.
                let retry_ok = run_schtasks_run().await;
                if !retry_ok {
                    if allow_uac {
                        log::warn!(
                            "Scheduled task still failing after AlreadyCorrect, falling back to UAC"
                        );
                        launch_uac_fallback(&request_id, &cmd_path).await?;
                    } else {
                        no_uac().await?;
                    }
                }
            }
            TaskHealResult::Healed => {
                let retry_ok = run_schtasks_run().await;
                if !retry_ok {
                    if allow_uac {
                        log::warn!(
                            "Scheduled task still failed after self-healing, falling back to UAC"
                        );
                        launch_uac_fallback(&request_id, &cmd_path).await?;
                    } else {
                        no_uac().await?;
                    }
                }
            }
            TaskHealResult::Failed => {
                if allow_uac {
                    launch_uac_fallback(&request_id, &cmd_path).await?;
                } else {
                    no_uac().await?;
                }
            }
        }
        // S51: pipe + task both failed without UAC — the infrastructure is
        // degraded this session. Count it so later no-prompt calls fail fast
        // instead of repeating the whole spawn/heal chain.
        note_elevated_failure();
    }

    // Poll for the result file (check every 150 ms, timeout per-command)
    let timeout = timeout_for_cmd(cmd);
    let start = Instant::now();
    loop {
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;

        if result_path.exists() {
            let content = tokio::fs::read_to_string(&result_path)
                .await
                .map_err(|e| format!("Cannot read elevated result: {e}"))?;
            let _ = tokio::fs::remove_file(&result_path).await;
            let _ = tokio::fs::remove_file(&cmd_path).await;

            let mut v: Value =
                serde_json::from_str(&content).map_err(|e| format!("Invalid result JSON: {e}"))?;

            // Verify the response HMAC to detect tampering or spoofing.
            if let Err(e) = auth::verify_payload(&mut v, &key) {
                log::warn!("Elevated response HMAC verification failed: {e}");
                return Err(format!("Elevated response authentication failed: {e}"));
            }

            let result_req = v["request_id"].as_str().unwrap_or_default();
            if result_req != request_id {
                return Err(format!(
                    "Elevated result request_id mismatch (expected {}, got {})",
                    request_id, result_req
                ));
            }

            return if v["ok"].as_bool().unwrap_or(false) {
                note_elevated_success();
                Ok(v["data"].clone())
            } else {
                Err(v["error"]
                    .as_str()
                    .unwrap_or("elevated process failed")
                    .to_string())
            };
        }

        if start.elapsed() > timeout {
            // The scheduled task ran but produced no result.  This usually means
            // the task is registered without the `--elevated` argument (so the
            // full GUI launched instead of the helper).
            #[cfg(windows)]
            {
                if !allow_uac {
                    let _ = tokio::fs::remove_file(&cmd_path).await;
                    // P8: name the command, the real elapsed time and the
                    // policy that suppressed the fallback — this line is what
                    // makes the failure actionable instead of silent.
                    log::warn!(
                        target: "elev_bridge",
                        "Elevated command '{}' produced no result within {} ms (limit {} s); \
                         UAC fallback disabled by design for background commands (MIOT-38) — \
                         scheduled task '{}' is broken or not running the helper",
                        cmd,
                        start.elapsed().as_millis(),
                        timeout.as_secs(),
                        TASK_NAME
                    );
                    return Err(format!(
                        "Elevated command '{}' timed out after {} ms (limit {} s) and UAC fallback \
                         is disabled for this operation (background command, MIOT-38 policy).",
                        cmd,
                        start.elapsed().as_millis(),
                        timeout.as_secs()
                    ));
                }
                // Re-write the command file in case the bad task process
                // consumed or deleted it.
                let _ = tokio::fs::write(&cmd_path, payload.to_string()).await;
                // S26-005: launch_elevated_via_uac() blocks for up to 30 s via
                // WaitForSingleObject. S69: FIXED pool (no tokio blocking-pool
                // churn).
                let req_id_owned = request_id.clone();
                let uac_result = crate::util::fixed_pool::run_blocking_timeout(
                    Duration::from_secs(35),
                    move || launch_elevated_via_uac(&req_id_owned),
                )
                .await
                .map_err(|e| format!("UAC timeout fallback task failed: {e}"))?;
                if let Err(e) = uac_result {
                    let _ = tokio::fs::remove_file(&cmd_path).await;
                    return Err(format!(
                        "Elevated process timed out after {} s and UAC fallback \
                         failed: {e}. Reinstall MiControl to fix the scheduled task.",
                        timeout.as_secs()
                    ));
                }
                // UAC helper ran synchronously; result should be present now.
                if result_path.exists() {
                    let content = tokio::fs::read_to_string(&result_path)
                        .await
                        .map_err(|e| format!("Cannot read elevated result: {e}"))?;
                    let _ = tokio::fs::remove_file(&result_path).await;
                    let _ = tokio::fs::remove_file(&cmd_path).await;
                    let mut v: Value = serde_json::from_str(&content)
                        .map_err(|e| format!("Invalid result JSON: {e}"))?;

                    // Verify the response HMAC.
                    if let Err(e) = auth::verify_payload(&mut v, &key) {
                        log::warn!("Elevated response HMAC verification failed: {e}");
                        return Err(format!("Elevated response authentication failed: {e}"));
                    }

                    let result_req = v["request_id"].as_str().unwrap_or_default();
                    if result_req != request_id {
                        return Err(format!(
                            "Elevated result request_id mismatch (expected {}, got {})",
                            request_id, result_req
                        ));
                    }
                    return if v["ok"].as_bool().unwrap_or(false) {
                        Ok(v["data"].clone())
                    } else {
                        Err(v["error"]
                            .as_str()
                            .unwrap_or("elevated process failed")
                            .to_string())
                    };
                }
                return Err(format!(
                    "Elevated process timed out after {} s. UAC fallback ran \
                     but produced no result.",
                    timeout.as_secs()
                ));
            }
            #[cfg(not(windows))]
            {
                let _ = tokio::fs::remove_file(&cmd_path).await;
                return Err(format!(
                    "Elevated process timed out after {} s. \
                     Ensure the '{}' scheduled task is registered.",
                    timeout.as_secs(),
                    TASK_NAME
                ));
            }
        }
    }
}

/// Send a privileged command to the autonomous `MiControlBridge` service over
/// the named pipe `\\.\pipe\micontrol_bridge`.
///
/// The bridge service runs as `NT AUTHORITY\SYSTEM` since installation, so
/// this path NEVER triggers a UAC prompt. Authentication uses the same
/// HMAC-SHA256 shared key as the scheduled-task path, plus a freshness window.
///
/// Returns `Err` when the service is not installed/running or the command
/// fails; the caller falls back to the scheduled task in that case.
#[cfg(windows)]
async fn run_via_service_pipe(cmd: &str, args: Value) -> Result<Value, String> {
    let request_id = make_request_id();
    let nonce = auth::generate_nonce();
    let mut payload = serde_json::json!({
        "protocol_version": 3,
        "request_id": request_id,
        "created_at_ms": auth::now_ms(),
        "nonce": nonce,
        "caller_pid": std::process::id(),
        "cmd": cmd,
        "args": args,
    });

    // Sign the payload (same shared key as the scheduled-task bridge).
    // S69: run on the FIXED pool — `tokio::task::spawn_blocking` pool threads
    // expire after ~10 s idle and churn (each new thread initializes COM and
    // the TLS teardown retains handles: the residual ~15 handles/min leak).
    let key = crate::util::fixed_pool::run_blocking_timeout(
        Duration::from_secs(10),
        auth::get_or_create_key,
    )
    .await
    .map_err(|e| format!("HMAC key task failed: {e}"))?
    .map_err(|e| format!("Cannot obtain HMAC key: {e}"))?;
    auth::sign_payload(&mut payload, &key);

    // Open the pipe (blocking — do it on a blocking thread). Bound the whole
    // round-trip with a timeout so a wedged bridge service can never hold the
    // serialized ELEV_REQUEST_LOCK forever (a hung pipe previously stalled
    // every subsequent elevated command and froze the UI).
    // S69: FIXED pool instead of `tokio::task::spawn_blocking` (thread churn
    // → COM init + TLS teardown → the residual handle leak).
    let body = payload.to_string();
    let result = tokio::time::timeout(
        Duration::from_secs(ELEV_TIMEOUT_SECS),
        crate::util::fixed_pool::run_blocking_timeout(
            Duration::from_secs(ELEV_TIMEOUT_SECS),
            move || pipe_request(&body),
        ),
    )
    .await
    .map_err(|_| {
        // MIOT-38: this silent failure previously hid WHY the app fell back
        // to the scheduled-task/UAC path — log it so the root cause is
        // visible in tauri-app.log.
        // P8: rate-limited (P1 storm: 4.298 identical lines in 49 h) and
        // counted by the pipe-health cooldown.
        note_pipe_timeout("round-trip timeout");
        log_rate_limited(true, "pipe_request_failed", |_| {
            format!(
                "Bridge service pipe round-trip timed out after {ELEV_TIMEOUT_SECS}s \
                 (service did not respond) — falling back to scheduled task path"
            )
        });
        format!(
            "Bridge service pipe round-trip timed out after {ELEV_TIMEOUT_SECS}s \
             (service did not respond)"
        )
    })?
    .map_err(|e| {
        log::warn!("pipe request task failed: {e}");
        format!("pipe request task failed: {e}")
    })?;

    let content = match result {
        Ok(c) => c,
        Err(e) => {
            if e.starts_with("No response from bridge service") {
                note_pipe_timeout("empty read");
            }
            // Rate-limited: with a wedged pipe (C-01) this line repeated
            // 4.298 times in 49 h and buried every other signal.
            log_rate_limited(true, "pipe_request_failed", |_| {
                format!(
                    "Bridge service pipe request failed: {e} — falling back to scheduled task path"
                )
            });
            return Err(e);
        }
    };

    // Parse + verify response.
    // S68: these error paths were SILENT — the app fell back to the scheduled
    // task (spawning a helper process per poll) with no log line explaining
    // why the healthy pipe was skipped. Every failure now logs its reason.
    let mut v: Value = serde_json::from_str(&content).map_err(|e| {
        log::warn!("Bridge response not valid JSON ({e}) — falling back to task path");
        format!("Invalid bridge response JSON: {e}")
    })?;
    if let Err(e) = auth::verify_payload(&mut v, &key) {
        log::warn!("Bridge response HMAC verification failed: {e} — falling back to task path");
        return Err(format!("Bridge response authentication failed: {e}"));
    }

    let resp_req = v["request_id"].as_str().unwrap_or_default();
    if resp_req != request_id {
        log::warn!(
            "Bridge response request_id mismatch (expected {request_id}, got {resp_req}) — \
             falling back to task path"
        );
        return Err(format!(
            "Bridge response request_id mismatch (expected {}, got {})",
            request_id, resp_req
        ));
    }

    if v["ok"].as_bool().unwrap_or(false) {
        Ok(v["data"].clone())
    } else {
        Err(v["error"]
            .as_str()
            .unwrap_or("bridge service failed")
            .to_string())
    }
}

#[cfg(not(windows))]
async fn run_via_service_pipe(_cmd: &str, _args: Value) -> Result<Value, String> {
    Err("Bridge service pipe only available on Windows".to_string())
}

/// Perform a synchronous request/response round-trip on the bridge named pipe.
///
/// Uses OVERLAPPED I/O with a bounded wait so a wedged bridge service can
/// never block the calling thread indefinitely (a previous synchronous
/// `ReadFile` could hang forever and stall the whole elevated-command chain).
#[cfg(windows)]
fn pipe_request(body: &str) -> Result<String, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{
        GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_OVERLAPPED,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
    use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult};

    // S55 FIX 25c: was 8s. `ensure_ecram_service` legitimately takes ~10.4s
    // (pipe probe + DriverStore check + service restart retries — measured in
    // bridge.log), which exceeded the old per-op wait: the app cancelled the
    // read, logged "No response from bridge service" and fell back to the
    // missing scheduled task every ~2.5 min, even though the service answered
    // fine 2s later. 30s still bounds a wedged service well below the
    // scheduled-task round-trip (~15s+), and slow commands keep their own
    // larger budgets in timeout_for_cmd.
    const PIPE_OP_TIMEOUT_MS: u32 = 30_000; // per read/write wait
    const MAX_RESPONSE_BYTES: usize = 16_384;
    // S32-005c: ERROR_PIPE_BUSY (0xE7 / 231) means the service's listener had
    // no free instance at this instant. Previously this immediately fell
    // through to the scheduled-task path (which spawns 3-4 failing schtasks
    // processes and 4 WARN log lines). A short bounded retry is correct:
    // the service accepts the next client within milliseconds.
    //
    // S55 FIX 22: ERROR_FILE_NOT_FOUND (0x2) is equally transient. The
    // accept loop hands a connected instance to a client thread and then
    // recreates the listener; in that sub-millisecond window NO instance
    // exists and CreateFileW returns 0x2. Retrying is correct — the next
    // listener appears immediately. Without this, the first elevated call
    // after boot periodically fell back to the (missing) scheduled task.
    const PIPE_BUSY_RETRIES: u32 = 20;
    const PIPE_BUSY_BACKOFF_MS: u32 = 50;

    let path_w: Vec<u16> = std::ffi::OsStr::new(BRIDGE_PIPE_NAME)
        .encode_wide()
        .chain(Some(0))
        .collect();

    let mut handle = INVALID_HANDLE_VALUE;
    for attempt in 0..=PIPE_BUSY_RETRIES {
        let attempt_result = unsafe {
            CreateFileW(
                PCWSTR(path_w.as_ptr()),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OVERLAPPED,
                HANDLE::default(),
            )
        };
        match attempt_result {
            Ok(h) => {
                handle = h;
                break;
            }
            Err(e) => {
                let code = e.code();
                let busy = code == windows::core::HRESULT(0x8007_00E7_u32 as i32);
                let transient_absent = code == windows::core::HRESULT(0x8007_0002_u32 as i32);
                if (busy || transient_absent) && attempt < PIPE_BUSY_RETRIES {
                    std::thread::sleep(std::time::Duration::from_millis(
                        PIPE_BUSY_BACKOFF_MS as u64,
                    ));
                    continue;
                }
                return Err(format!("Open bridge pipe: {e}"));
            }
        }
    }

    if handle == INVALID_HANDLE_VALUE {
        return Err("INVALID_HANDLE_VALUE opening bridge pipe".to_string());
    }

    // S67: RAII for the pipe handle as well. Before this, every early return
    // below (`write_result.map_err(..)?` and the read loop's break paths that
    // bail through errors) skipped the two manual CloseHandle calls at the end,
    // leaking 2 kernel handles per failed elevated round-trip — the audited
    // ~11 handles/min that reached 188,715 handles / 2.5 GB commit in 218 h.
    let _pipe_guard = crate::util::win_handle::OwnedHandle::new(handle);

    // Event used for both the write and read overlapped waits.
    // S67: RAII — the event is closed on EVERY exit path (including the
    // `write_result.map_err(...)?` below, which previously leaked it).
    let _event_guard = crate::util::win_handle::OwnedEvent::new(
        unsafe { CreateEventW(None, true, false, PCWSTR::null()) }
            .map_err(|e| format!("CreateEventW bridge pipe: {e}"))?,
    );
    let event = _event_guard.raw();

    let mut write_result: Result<(), windows::core::Error> = Ok(());
    let mut written = 0u32;

    // Write the request via overlapped I/O with a bounded wait.
    let req_bytes = body.as_bytes();
    let mut write_ov: windows::Win32::System::IO::OVERLAPPED = unsafe { std::mem::zeroed() };
    write_ov.hEvent = event;
    let mut write_pending = false;
    unsafe {
        // SAFETY: handle/event valid, req_bytes valid for the operation duration.
        let op = WriteFile(
            handle,
            Some(req_bytes),
            Some(&mut written),
            Some(&mut write_ov),
        );
        if op.is_err() {
            let last_err = std::io::Error::last_os_error();
            let code = last_err.raw_os_error().unwrap_or(0);
            if code == 997
            /* ERROR_IO_PENDING */
            {
                write_pending = true;
            } else {
                write_result = Err(windows::core::Error::from_win32());
            }
        }
        if write_pending {
            // Wait for completion with timeout, then collect the result.
            let wait = WaitForSingleObject(event, PIPE_OP_TIMEOUT_MS);
            if wait == WAIT_TIMEOUT {
                let _ = CancelIoEx(handle, Some(&write_ov));
                note_pipe_timeout("write timeout");
                write_result = Err(windows::core::Error::from_win32());
            } else if wait != WAIT_OBJECT_0 {
                write_result = Err(windows::core::Error::from_win32());
            } else {
                let mut transferred = 0u32;
                if GetOverlappedResult(handle, &write_ov, &mut transferred, false).is_err() {
                    write_result = Err(windows::core::Error::from_win32());
                }
                // transferred bytes are not needed — a successful write of the
                // full request body is implied by GetOverlappedResult OK.
            }
        }
    }
    write_result.map_err(|e| format!("WriteFile bridge pipe: {e}"))?;

    // Read the response via overlapped I/O with per-read bounded waits until
    // the full JSON object is received (ends with '}').
    let mut response_buf = [0u8; MAX_RESPONSE_BYTES];
    let mut total_read = 0usize;
    loop {
        if total_read >= response_buf.len() {
            break; // Cap the response size; bridge responses are bounded.
        }
        let mut bytes_read = 0u32;
        let mut read_ov: windows::Win32::System::IO::OVERLAPPED = unsafe { std::mem::zeroed() };
        read_ov.hEvent = event;
        unsafe {
            // SAFETY: buffers valid, handle valid.
            let op = ReadFile(
                handle,
                Some(&mut response_buf[total_read..]),
                Some(&mut bytes_read),
                Some(&mut read_ov),
            );
            if op.is_err() {
                let last_err = std::io::Error::last_os_error();
                let code = last_err.raw_os_error().unwrap_or(0);
                if code != 997
                /* ERROR_IO_PENDING */
                {
                    break; // EOF or real error — stop reading.
                }
                // Pending: wait with timeout, then collect the result.
                let wait = WaitForSingleObject(event, PIPE_OP_TIMEOUT_MS);
                if wait == WAIT_TIMEOUT {
                    let _ = CancelIoEx(handle, Some(&read_ov));
                    note_pipe_timeout("read timeout");
                    break;
                }
                if wait != WAIT_OBJECT_0 {
                    break;
                }
                let mut transferred = 0u32;
                if GetOverlappedResult(handle, &read_ov, &mut transferred, false).is_err() {
                    break;
                }
                bytes_read = transferred;
            }
        }
        if bytes_read == 0 {
            break; // EOF.
        }
        total_read += bytes_read as usize;
        if total_read > 0 && response_buf[total_read - 1] == b'}' {
            break;
        }
    }

    // S67: the pipe handle and overlapped event are owned by `_pipe_guard` /
    // `_event_guard` and closed on drop — including every early return above.
    // Manual CloseHandle calls were removed (they double-closed on the happy
    // path, and leaked on every error path).

    if total_read == 0 {
        return Err("No response from bridge service (timed out or empty)".to_string());
    }
    Ok(String::from_utf8_lossy(&response_buf[..total_read]).to_string())
}

/// Run `schtasks /run /tn MiControlElevated` with CREATE_NO_WINDOW to avoid
/// flashing a console window on every elevated operation.
async fn run_schtasks_run() -> bool {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let status = tokio::process::Command::new("schtasks")
            .args(["/run", "/tn", TASK_NAME])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .await;
        match status {
            Ok(s) if s.success() => true,
            Ok(s) => {
                log::warn!("schtasks /run MiControlElevated failed with exit {s}");
                false
            }
            Err(e) => {
                log::warn!("schtasks /run MiControlElevated failed to spawn: {e}");
                false
            }
        }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// UAC fallback: launch the current binary as administrator via ShellExecuteExW
/// "runas" so that a single UAC prompt lets us run `micontrol.exe --elevated`.
async fn launch_uac_fallback(request_id: &str, cmd_path: &std::path::Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        let req_id_owned = request_id.to_string();
        // S69: FIXED pool (no tokio blocking-pool churn).
        let uac_result =
            crate::util::fixed_pool::run_blocking_timeout(Duration::from_secs(35), move || {
                launch_elevated_via_uac(&req_id_owned)
            })
            .await
            .map_err(|e| format!("UAC launch task failed: {e}"))?;

        if let Err(e) = uac_result {
            let _ = tokio::fs::remove_file(cmd_path).await;
            return Err(format!(
                "Scheduled task '{}' not found AND UAC fallback failed: {e}. \
                 Reinstall MiControl to register the scheduled task.",
                TASK_NAME
            ));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = tokio::fs::remove_file(cmd_path).await;
        Err(format!("Scheduled task '{TASK_NAME}' not found."))
    }
}

/// Launch the current executable as administrator using `ShellExecuteExW`
/// with verb `"runas"` and argument `"--elevated --request-id <id>"`.
///
/// Blocks until the spawned process exits (max 30 s).
/// Returns `Ok(())` if the process was launched successfully; the caller
/// must still poll for `elev_result.json`.
///
/// MIOT-38: this is the ONLY remaining UAC prompt in the elevated chain. It
/// is gated by [`UAC_DECLINED`] — once the user dismisses the dialog, every
/// later call in this process skips the prompt (no more prompt storms).
#[cfg(windows)]
fn launch_elevated_via_uac(request_id: &str) -> Result<(), String> {
    // Session memory: the user already declined a UAC prompt — do NOT ask
    // again (this is what turned into the MCElev_heal.bat prompt storm).
    if UAC_DECLINED.load(Ordering::SeqCst) {
        return Err(
            "UAC fallback was declined earlier in this session — skipping prompt".to_string(),
        );
    }

    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HWND};
    use windows::Win32::System::Threading::WaitForSingleObject;
    use windows::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };

    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let exe_str = exe.to_string_lossy().into_owned();

    let verb: Vec<u16> = OsStr::new("runas").encode_wide().chain(Some(0)).collect();
    let file: Vec<u16> = OsStr::new(&exe_str).encode_wide().chain(Some(0)).collect();
    let params_text = format!("--elevated --request-id {}", request_id);
    let params: Vec<u16> = OsStr::new(&params_text)
        .encode_wide()
        .chain(Some(0))
        .collect();

    unsafe {
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
            hwnd: HWND(std::ptr::null_mut()),
            lpVerb: PCWSTR(verb.as_ptr()),
            lpFile: PCWSTR(file.as_ptr()),
            lpParameters: PCWSTR(params.as_ptr()),
            nShow: 0, // SW_HIDE — no visible window
            ..std::mem::zeroed()
        };

        // SAFETY: ShellExecuteExW with SEE_MASK_NOCLOSEPROCESS launches the executable and returns a process handle. The verb ("runas"), file, and parameters are all valid null-terminated wide strings. hProcess is checked for validity before WaitForSingleObject/CloseHandle. zeroed() is safe for the remaining fields as cbSize is explicitly set and Windows ignores unspecified fields.
        if let Err(e) = ShellExecuteExW(&mut info) {
            // ERROR_CANCELLED (0x800704C7) — the user closed/dismissed the
            // UAC consent dialog. Remember it and NEVER re-prompt in this
            // session (MIOT-38 prompt storm). NOTE: HRESULT is i32, so the
            // high bit makes this negative (−2147023673).
            const ERROR_CANCELLED_HRESULT: i32 = 0x8007_04C7u32 as i32;
            if e.code().0 == ERROR_CANCELLED_HRESULT {
                UAC_DECLINED.store(true, Ordering::SeqCst);
                log::warn!(
                    "UAC prompt declined by the user — suppressing further UAC \
                     prompts for this session"
                );
            }
            return Err(format!("ShellExecuteExW: {e}"));
        }

        if !info.hProcess.is_invalid() {
            // Wait up to 30 s for the elevated helper to finish writing its result
            WaitForSingleObject(info.hProcess, 30_000);
            let _ = CloseHandle(info.hProcess);
        }
    }
    Ok(())
}

/// Re-launch the current executable as administrator using `ShellExecuteExW` "runas".
///
/// Unlike [`launch_elevated_via_uac`] this function:
/// - does NOT pass `--elevated` to the new instance (normal startup)
/// - shows the new window (`SW_SHOWNORMAL`)
/// - does NOT wait for the new process to finish
///
/// After this returns the caller should call `app.exit(0)` to shut down the
/// current (non-elevated) instance and let the elevated instance take over.
#[cfg(windows)]
pub fn relaunch_self_as_admin() -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW};

    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let exe_str = exe.to_string_lossy().into_owned();

    let verb: Vec<u16> = OsStr::new("runas").encode_wide().chain(Some(0)).collect();
    let file: Vec<u16> = OsStr::new(&exe_str).encode_wide().chain(Some(0)).collect();

    unsafe {
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOASYNC,
            hwnd: HWND(std::ptr::null_mut()),
            lpVerb: PCWSTR(verb.as_ptr()),
            lpFile: PCWSTR(file.as_ptr()),
            lpParameters: PCWSTR::null(),
            nShow: 1, // SW_SHOWNORMAL
            ..std::mem::zeroed()
        };

        // SAFETY: ShellExecuteExW with "runas" verb launches the process with elevation request. The verb, file, and parameters are valid null-terminated wide strings. zeroed() is safe for remaining fields since cbSize is explicitly set.
        ShellExecuteExW(&mut info).map_err(|e| format!("ShellExecuteExW: {e}"))?;
    }

    Ok(())
}

/// Returns true if the current process token has the Administrators group enabled
/// (i.e. the process is running elevated / as administrator).
#[cfg(windows)]
fn is_admin() -> bool {
    use windows::Win32::UI::Shell::IsUserAnAdmin;
    // SAFETY: IsUserAnAdmin() is a simple Win32 check with no safety invariants — it always succeeds and returns a BOOL.
    unsafe { IsUserAnAdmin().as_bool() }
}

fn make_request_id() -> String {
    let seq = NEXT_REQ.fetch_add(1, Ordering::Relaxed);
    format!("{:08x}-{:016x}-{:08x}", std::process::id(), now_ms(), seq)
}

fn cmd_file_name(request_id: &str) -> String {
    format!("elev_cmd_{request_id}.json")
}

fn result_file_name(request_id: &str) -> String {
    format!("elev_result_{request_id}.json")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn cleanup_stale_elev_files(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let looks_like_elev_file = (name.starts_with("elev_cmd_")
            || name.starts_with("elev_result_"))
            && name.ends_with(".json");
        if !looks_like_elev_file {
            continue;
        }
        let is_stale = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|ts| now.duration_since(ts).ok())
            .map(|age| age.as_secs() >= STALE_FILE_MAX_AGE_SECS)
            .unwrap_or(false);
        if is_stale {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Check if the scheduled task exists and points to the current executable.
/// If the task is missing or points to a different path (e.g. debug exe from
/// `cargo tauri dev`), re-register it with the correct path.
///
/// Tries non-elevated `schtasks` only. If that fails (Access Denied on a
/// standard user session) it does NOT elevate — the scheduled task is merely
/// a fallback behind the MiControlBridge SYSTEM service (the primary
/// elevated path, installed post-install, never prompts). MIOT-38: elevating
/// here (ShellExecuteExW "runas" → MCElev_heal.bat) is what produced the
/// UAC prompt storm every few seconds when the task was missing.
///
/// Returns the outcome of the self-healing attempt.
#[cfg(windows)]
fn ensure_task_correct_path() -> TaskHealResult {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    // S64: throttle the self-heal. This runs on the elevated dispatch chain,
    // which background polls (thermal/battery) trigger every few seconds. When
    // the task is genuinely missing and the non-elevated re-register keeps
    // failing (standard user session), every call re-ran `schtasks /query`
    // plus a failed `/create` and emitted 4 WARN lines — observed 2823
    // repetitions in one log. A cooldown keeps the diagnostic value (we retry
    // eventually, e.g. after a reinstall) without spamming.
    const HEAL_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(900);
    static LAST_HEAL_ATTEMPT: std::sync::OnceLock<std::sync::Mutex<Option<std::time::Instant>>> =
        std::sync::OnceLock::new();
    {
        let lock = LAST_HEAL_ATTEMPT.get_or_init(|| std::sync::Mutex::new(None));
        let mut guard = match lock.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        match *guard {
            Some(last) if last.elapsed() < HEAL_COOLDOWN => {
                return TaskHealResult::AlreadyCorrect;
            }
            _ => *guard = Some(std::time::Instant::now()),
        }
    }

    // Get the current executable path.
    let current_exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            log::warn!("Cannot get current exe path for task healing: {e}");
            return TaskHealResult::Failed;
        }
    };
    let current_path = current_exe.to_string_lossy().to_string();

    // Query the existing task's action path.
    let output = std::process::Command::new("schtasks")
        .args(["/query", "/tn", TASK_NAME, "/xml"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    let need_reregister = match output {
        Ok(out) => {
            let xml = String::from_utf8_lossy(&out.stdout);
            // Check if the task points to the current exe. The task XML
            // stores the <Command> WITH quotes around the path
            // ("C:\...\micontrol.exe") and may use forward slashes — match
            // on all forms. MIOT-38: comparing against the UNQUOTED path made
            // a correct task always look "wrong", so the self-heal
            // re-registered an identical task on every elevated call and
            // looped UAC prompts.
            let path_quoted = format!("\"{current_path}\"");
            let path_quoted_fwd = format!("\"{}\"", current_path.replace('\\', "/"));
            let path_matches = xml.contains(&current_path)
                || xml.contains(&current_path.replace('\\', "/"))
                || xml.contains(&path_quoted)
                || xml.contains(&path_quoted_fwd);
            if path_matches {
                false
            } else {
                log::info!(
                    "Scheduled task points to wrong path, re-registering with: {current_path}"
                );
                true
            }
        }
        Err(_) => {
            log::info!("Scheduled task not found, registering with: {current_path}");
            true
        }
    };

    if !need_reregister {
        return TaskHealResult::AlreadyCorrect;
    }

    // Build the task XML with the correct path.
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task"><Triggers><TimeTrigger><StartBoundary>2000-01-01T00:00:00</StartBoundary><Enabled>false</Enabled></TimeTrigger></Triggers><Principals><Principal id="Author"><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals><Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT120S</ExecutionTimeLimit><Enabled>true</Enabled></Settings><Actions Context="Author"><Exec><Command>"{current_path}"</Command><Arguments>--elevated</Arguments></Exec></Actions></Task>"#
    );

    let temp_dir = std::env::temp_dir();
    let xml_path = temp_dir.join("MCElev_heal.xml");
    if let Err(e) = std::fs::write(&xml_path, &xml) {
        log::warn!("Cannot write task XML for healing: {e}");
        return TaskHealResult::Failed;
    }
    let xml_str = xml_path.to_string_lossy().to_string();

    // Try 1: non-elevated schtasks (works if user has rights or is already admin)
    let _ = std::process::Command::new("schtasks")
        .args(["/delete", "/tn", TASK_NAME, "/f"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status();

    let create_ok = std::process::Command::new("schtasks")
        .args(["/create", "/tn", TASK_NAME, "/xml", &xml_str, "/f"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    let success = match create_ok {
        Ok(out) if out.status.success() => true,
        _ => {
            // MIOT-38: NO UAC prompt here. The non-elevated create failed
            // (access denied on a standard user session) and the previous
            // code elevated via ShellExecuteExW "runas" → wrote
            // MCElev_heal.bat → popped a consent dialog from a BACKGROUND
            // dispatch chain (thermal/battery polls fire this loop every few
            // seconds) — the prompt storm the user reported. The scheduled
            // task is only a fallback: the MiControlBridge service (installed
            // post-install as SYSTEM, no UAC) is the primary elevated path.
            // Task missing → return a clean failure; reinstall repairs it
            // (or the service path is used instead).
            log::warn!(
                "Non-elevated schtasks re-register of '{TASK_NAME}' failed — NOT \
                 prompting UAC (scheduled task is a fallback; the MiControlBridge \
                 service is the primary elevated path). Reinstall MiControl or run \
                 'micontrol_bridge.exe install' to repair."
            );
            false
        }
    };

    let _ = std::fs::remove_file(&xml_path);

    if success {
        log::info!("Scheduled task re-registered successfully with correct path");
        TaskHealResult::Healed
    } else {
        log::warn!("Failed to re-register scheduled task (UAC may have been declined)");
        TaskHealResult::Failed
    }
}

#[cfg(not(windows))]
fn ensure_task_correct_path() -> TaskHealResult {
    TaskHealResult::Failed
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── P1: SCM-based availability decision ────────────────────────────────

    /// The regression that caused 597 warnings + 702 no-op installs: a running
    /// service whose pipe is wedged must never be re-installed.
    #[test]
    fn running_service_is_available_even_when_the_pipe_is_wedged() {
        let action = decide_bridge_action(BridgeProbe {
            service: BridgeServiceState::Running,
            pipe_reachable: false,
        });
        assert_eq!(action, BridgeAction::AlreadyRunning);
        assert_eq!(
            action,
            decide_bridge_action(BridgeProbe {
                service: BridgeServiceState::Running,
                pipe_reachable: true,
            }),
            "pipe state must not influence a known-running service"
        );
    }

    #[test]
    fn stopped_service_is_started_not_reinstalled() {
        assert_eq!(
            decide_bridge_action(BridgeProbe {
                service: BridgeServiceState::Stopped,
                pipe_reachable: false,
            }),
            BridgeAction::StartService
        );
        assert_eq!(
            decide_bridge_action(BridgeProbe {
                service: BridgeServiceState::Stopped,
                pipe_reachable: true,
            }),
            BridgeAction::StartService,
            "a reachable pipe with a stopped service still needs the service started"
        );
    }

    #[test]
    fn absent_service_is_installed() {
        assert_eq!(
            decide_bridge_action(BridgeProbe {
                service: BridgeServiceState::Absent,
                pipe_reachable: false,
            }),
            BridgeAction::Install
        );
    }

    #[test]
    fn unknown_scm_state_falls_back_to_the_pipe_probe() {
        assert_eq!(
            decide_bridge_action(BridgeProbe {
                service: BridgeServiceState::Unknown,
                pipe_reachable: true,
            }),
            BridgeAction::AlreadyRunning
        );
        assert_eq!(
            decide_bridge_action(BridgeProbe {
                service: BridgeServiceState::Unknown,
                pipe_reachable: false,
            }),
            BridgeAction::Install
        );
    }

    #[test]
    fn scm_states_collapse_to_actionable_buckets() {
        assert_eq!(
            BridgeServiceState::from_scm(ServiceState::Running),
            BridgeServiceState::Running
        );
        assert_eq!(
            BridgeServiceState::from_scm(ServiceState::Pending),
            BridgeServiceState::Stopped,
            "a start-pending service must be asked to start, never reinstalled"
        );
        assert_eq!(
            BridgeServiceState::from_scm(ServiceState::Paused),
            BridgeServiceState::Stopped
        );
        assert_eq!(
            BridgeServiceState::from_scm(ServiceState::Absent),
            BridgeServiceState::Absent
        );
        assert_eq!(
            BridgeServiceState::from_scm(ServiceState::Unknown),
            BridgeServiceState::Unknown
        );
    }

    // ── P8: UAC opt-in flag ────────────────────────────────────────────────

    #[test]
    fn parses_uac_opt_in_flag() {
        for truthy in ["1", "true", "TRUE", "yes", "on", " 1 "] {
            assert!(parse_bool_flag(truthy), "{truthy:?} should enable");
        }
        for falsy in ["0", "false", "no", "off", "", "2", "yes please"] {
            assert!(!parse_bool_flag(falsy), "{falsy:?} should not enable");
        }
    }

    // ── P8: boot summary ───────────────────────────────────────────────────

    #[test]
    fn boot_summary_reports_success_without_noise() {
        let line = format_boot_summary(&[], 7);
        assert!(line.contains("7 command(s) succeeded"), "{line}");
        assert!(line.contains("none failed"), "{line}");
    }

    #[test]
    fn boot_summary_lists_command_elapsed_and_reason() {
        let failures = vec![
            BootFailure {
                cmd: "disable_copilot_key",
                elapsed_ms: 15_002,
                attempts: 1,
                reason: "timed out after 15002 ms and UAC fallback is disabled".into(),
            },
            BootFailure {
                cmd: "ensure_face_service",
                elapsed_ms: 45_120,
                attempts: 2,
                reason: "scheduled task unavailable".into(),
            },
        ];
        let line = format_boot_summary(&failures, 3);
        assert!(line.contains("disable_copilot_key"), "{line}");
        assert!(line.contains("15002 ms"), "{line}");
        assert!(line.contains("ensure_face_service"), "{line}");
        assert!(line.contains("attempt 2"), "{line}");
        assert!(line.contains("MICONTROL_ELEV_ALLOW_UAC=1"), "{line}");
        assert!(!line.contains('\n'), "summary must stay a single log line");
    }

    #[test]
    fn boot_summary_sanitises_and_truncates_reasons() {
        let long_reason = format!("line one\nline two\r\n{}", "x".repeat(400));
        let failures = vec![BootFailure {
            cmd: "set_scancode_map",
            elapsed_ms: 1,
            attempts: 1,
            reason: long_reason,
        }];
        let line = format_boot_summary(&failures, 0);
        assert!(!line.contains('\n'), "newlines must be flattened: {line}");
        assert!(line.ends_with('…'), "reason must be truncated: {line}");
        assert!(
            line.len() < 600,
            "summary must stay bounded: {}",
            line.len()
        );
    }

    // ── P8: pipe cooldown ──────────────────────────────────────────────────

    #[test]
    fn pipe_cooldown_opens_after_two_timeouts_and_expires() {
        let mut cooldown = PipeCooldown::new();
        assert!(cooldown.allowed(0));
        assert_eq!(
            cooldown.note_timeout(100),
            None,
            "one timeout is not enough to skip the pipe"
        );
        assert!(cooldown.allowed(100));

        assert_eq!(cooldown.note_timeout(200), Some(2));
        assert!(!cooldown.allowed(201));
        assert!(!cooldown.allowed(200 + 59_999));
        assert!(cooldown.allowed(200 + 60_000));
    }

    #[test]
    fn pipe_cooldown_does_not_reopen_or_relog_while_active() {
        let mut cooldown = PipeCooldown::new();
        cooldown.note_timeout(0);
        assert_eq!(cooldown.note_timeout(10), Some(2));
        // Further timeouts inside the window must not log again.
        assert_eq!(cooldown.note_timeout(20), None);
        assert_eq!(cooldown.note_timeout(30), None);
        assert!(!cooldown.allowed(30));
    }

    #[test]
    fn pipe_success_resets_the_streak() {
        let mut cooldown = PipeCooldown::new();
        cooldown.note_timeout(0);
        assert_eq!(cooldown.note_timeout(10), Some(2));
        cooldown.note_success();
        // After recovery, two new timeouts are needed to skip again.
        assert_eq!(cooldown.note_timeout(70_000), None);
        assert_eq!(cooldown.note_timeout(70_100), Some(2));
    }
}
