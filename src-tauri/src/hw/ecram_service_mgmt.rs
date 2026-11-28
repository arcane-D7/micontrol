//! Management of the custom ecram_service (IoTService.exe replacement).
//!
//! This module ensures the custom IoTService.exe (built from `ecram_service.rs`)
//! is installed as a Windows service and running.  It provides ECRAM access
//! via named pipe `\\.\pipe\ecram_service` without requiring Xiaomi PC Manager.
//!
//! The service is installed as `IoTSvc` (matching the name the driver expects)
//! and runs as `NT AUTHORITY\SYSTEM` to pass the IoTDriver.sys security check.
//!
//! ## Verify, never re-install a healthy service
//!
//! Every caller of this module is a healer (the IoT watchdog, the health
//! supervisor, a UI tap), and a healer that "repairs" a working service turns
//! its own probe failure into an outage. So the contract is:
//!
//! 1. **Observe** the SCM, **ping** the pipe and compare the installed binary.
//!    This step has no side effects.
//! 2. **Decide** with the pure [`plan_repair`]: a healthy service is left alone,
//!    a running service with a dead pipe gets a controlled restart, and only a
//!    wrong binary or a wrong `binPath` may destroy the entry.
//! 3. **Spend budget**: at most one repair episode per backoff window, and stop
//!    entirely after [`MAX_REPAIR_EPISODES`] consecutive failures until a
//!    verification proves the service healthy again.
//! 4. **Read back** the effect (SCM query + pipe ping) and report what actually
//!    happened, so a caller can never mistake a throttled no-op for a repair.
//!
//! The previous shape of this module ran the destructive
//! stop -> `taskkill /F` -> copy -> delete -> create -> start ladder on a flat
//! 15-minute cooldown forever, which is what the SCM logged as 43 "terminated
//! unexpectedly" events (7034, `WIN32_EXIT_CODE 1067`) in 11 hours: the cooldown
//! capped the churn at one episode per 15 minutes, it did not stop it, and every
//! episode ended with a force kill instead of a stop.

use crate::errors::catalog::{AppError, ErrorCode};
use crate::util::win_service::{self, ServiceState};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant, UNIX_EPOCH};

const SERVICE_NAME: &str = "IoTSvc";
const SERVICE_EXE: &str = "ecram_service.exe";
/// The readback that proves the pipe server is serving, not merely present.
const PIPE_PING_REQUEST: &str = r#"{"op":"ping"}"#;

/// Bounded wait for a graceful stop before a repair escalates to a force kill.
const GRACEFUL_STOP_TIMEOUT: Duration = Duration::from_secs(10);
/// Bounded wait for RUNNING after a start request.
const START_TIMEOUT: Duration = Duration::from_secs(15);
/// Bounded wait for the pipe server to answer after the service is RUNNING.
const PIPE_WAIT_TIMEOUT: Duration = Duration::from_secs(10);
/// Poll interval while waiting for the pipe.
const PIPE_POLL: Duration = Duration::from_millis(500);
/// Settle delay before the second pipe probe (S66-005: one transient miss must
/// not read as a dead service).
const PIPE_SETTLE: Duration = Duration::from_millis(250);
/// Bounded wait for a marked-for-delete entry to settle.
const DELETE_SETTLE_TIMEOUT: Duration = Duration::from_secs(5);
/// The SCM resets its failure counter after this many seconds, so a long outage
/// is not remembered as one endless failure streak (`sc failure reset=`).
const FAILURE_RESET_SECS: u32 = 86_400;

/// Backoff ladder between DESTRUCTIVE repair episodes, in SECONDS, clamped at
/// the top rung.
const REPAIR_BACKOFF_SECS: [u64; 6] = [30, 60, 120, 240, 480, 900];
/// Consecutive destructive episodes after which the healer stops replacing or
/// restarting the service until a verification proves it healthy.
const MAX_REPAIR_EPISODES: u32 = 5;
/// Minimum gap between two plain `sc start` recoveries. A start is not
/// destructive (see [`Severity`]), so it is throttled but never capped: on a
/// machine where the SCM restart ladder cannot be installed, it is the only
/// thing that brings a dead service back.
const START_WINDOW: Duration = Duration::from_secs(900);

/// `taskkill /F /IM` inside an escalated stop. A force kill completes well under
/// a second; 10 s is headroom without wedging the repair (contract:
/// UNIVERSAL-CONTRACTS.md section 4, every child wait is bounded).
#[cfg(windows)]
const TASKKILL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Status of the ecram_service, reported to the elevated caller and the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub installed: bool,
    pub running: bool,
    pub pipe_available: bool,
    pub exe_path: Option<String>,
    pub message: String,
    /// What this verification cycle actually did: `none`, `start`, `restart` or
    /// `reinstall`. Callers must not infer a repair from `ok == true`; the
    /// healthy path and the throttled path both return `action = "none"`.
    pub action: String,
}

// ── Decision ────────────────────────────────────────────────────────────────

/// What one verification cycle is allowed to do to the service, ordered by blast
/// radius. Only [`Repair::Reinstall`] may destroy the SCM entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Repair {
    /// Leave it alone: healthy, or not provably broken.
    None,
    /// `sc start` only (entry exists, stopped, binary and `binPath` already right).
    Start,
    /// Controlled stop (waited) then start. Never deletes, never touches the
    /// DriverStore.
    Restart,
    /// Replace the entry: the only path that may stop, copy, delete and create.
    Reinstall,
}

impl Repair {
    /// Stable label for the JSON status and for tests.
    fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Start => "start",
            Self::Restart => "restart",
            Self::Reinstall => "reinstall",
        }
    }

    /// True when the plan touches the service at all.
    fn acts(self) -> bool {
        self != Self::None
    }

    /// True when the plan may remove the SCM entry. Everything else must be
    /// reachable without a delete, because a delete loses the recovery actions,
    /// the pipe security descriptor and a pending start of a live service.
    fn may_delete(self) -> bool {
        matches!(self, Self::Reinstall)
    }

    /// How much damage the plan can do, which is what the budget has to bound.
    fn severity(self) -> Severity {
        match self {
            Self::None => Severity::Verify,
            Self::Start => Severity::Start,
            Self::Restart | Self::Reinstall => Severity::Destructive,
        }
    }
}

/// How much damage a plan can do.
///
/// The distinction is load-bearing on this machine: the default service DACL
/// (`sc sdshow IoTSvc` -> `(A;;CCLCSWRPWPDTLOCRRC;;;SY)`) grants LocalSystem
/// `SERVICE_START` but NOT `SERVICE_CHANGE_CONFIG`, so our components cannot
/// install the SCM restart ladder at all (verified: `sc qfailure` reports no
/// ACTIONS for IoTSvc, MiControlBridge and MiControlFace alike). A plain
/// `sc start` is therefore the only recovery a service account has, and it is
/// also harmless: it must never be capped, while the destructive plans must.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Severity {
    /// Nothing happens: healthy, or not provably broken.
    Verify,
    /// `sc start` only: non-destructive recovery of a correctly installed entry.
    Start,
    /// Stops a running service, replaces the DriverStore binary, or recreates
    /// the SCM entry.
    Destructive,
}

/// Where the DriverStore `IoTService.exe` stands relative to our binary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BinaryPlacement {
    /// No DriverStore entry (the service runs from the app directory), or its
    /// metadata could not be read. Not evidence of anything.
    NotApplicable,
    /// The DriverStore binary is our `ecram_service.exe`.
    Ours,
    /// Something else is installed there (the stock Xiaomi service, which will
    /// reject our ioctls).
    NotOurs,
}

/// The facts a decision is made from, gathered without side effects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Observed {
    scm: ServiceState,
    pipe_ok: bool,
    bin_path_ok: bool,
    placement: BinaryPlacement,
}

/// Pure decision: which repair, if any, the observed facts justify.
///
/// Rules, in order:
/// * An absent entry must be created (nothing to lose, so it rides the
///   `Reinstall` plan with no destructive part).
/// * An unverifiable SCM answer (`Unknown`) or a transition state (`Pending`) is
///   not evidence of a fault, so it may not be evidence for a delete either
///   (P1: the SCM is the authority; "I could not ask" is not "it is broken").
/// * Only a wrong binary or a wrong `binPath` may destroy the entry.
/// * A service that answers its ping with the right binary and path is HEALTHY:
///   nothing may stop, delete or reinstall it. This is precisely the case the
///   old code kept "repairing" every 15 minutes.
/// * A running service with a dead pipe is restarted under control; a stopped
///   entry is started.
fn plan_repair(o: Observed) -> Repair {
    if o.scm == ServiceState::Absent {
        return Repair::Reinstall;
    }
    if matches!(o.scm, ServiceState::Unknown | ServiceState::Pending) {
        return Repair::None;
    }
    if !o.bin_path_ok || o.placement == BinaryPlacement::NotOurs {
        return Repair::Reinstall;
    }
    if o.pipe_ok {
        return Repair::None;
    }
    match o.scm {
        ServiceState::Running | ServiceState::Paused => Repair::Restart,
        ServiceState::Stopped => Repair::Start,
        _ => Repair::None,
    }
}

/// True when the stored `BINARY_PATH_NAME` is a form the IoT service starts
/// from: the quoted path plus the `service` token (empirically the only form
/// that reaches RUNNING; a bare path fails the start with error 2), or the bare
/// path an installer may have left behind.
fn bin_path_matches(stored: &str, target_exe: &str) -> bool {
    let quoted = format!("\"{target_exe}\" service");
    stored.eq_ignore_ascii_case(&quoted) || stored.eq_ignore_ascii_case(target_exe)
}

/// Does this pipe response prove a live server?
///
/// Pure half of [`pipe_answers_ping`], so the readback rule is unit-testable
/// without a service. An `{"ok":false}` answer is a refusal, not a success, and
/// an answer without `pong` proves nothing.
fn ping_ok(response: Option<&str>) -> bool {
    response
        .and_then(|r| serde_json::from_str::<serde_json::Value>(r).ok())
        .and_then(|v| v.get("pong").and_then(|p| p.as_bool()))
        .unwrap_or(false)
}

// ── Repair budget ───────────────────────────────────────────────────────────

/// Process-local repair budget. `Instant`s keep it monotonic across clock
/// changes; the bridge service is the only long-lived caller in practice.
///
/// Destructive episodes and plain starts are counted separately: the first are
/// capped and backed off, the second are only throttled (see [`Severity`]).
#[derive(Debug, Clone, Copy, Default)]
struct Budget {
    destructive: u32,
    last_destructive: Option<Instant>,
    last_start: Option<Instant>,
}

impl Budget {
    /// Window that must pass before the NEXT destructive episode may run.
    fn backoff(&self) -> Duration {
        let top = (REPAIR_BACKOFF_SECS.len() - 1) as u32;
        Duration::from_secs(REPAIR_BACKOFF_SECS[self.destructive.min(top) as usize])
    }

    /// True when `plan` is allowed to run now.
    ///
    /// The first destructive episode is always allowed (nothing has been spent
    /// yet); a plain start is allowed whenever [`START_WINDOW`] has passed since
    /// the previous one, for as long as the service stays dead.
    fn allowed_at(&self, plan: Repair, now: Instant) -> bool {
        match plan.severity() {
            Severity::Verify => false,
            Severity::Start => match self.last_start {
                None => true,
                Some(last) => now.saturating_duration_since(last) >= START_WINDOW,
            },
            Severity::Destructive => {
                if self.destructive >= MAX_REPAIR_EPISODES {
                    return false;
                }
                match self.last_destructive {
                    None => true,
                    Some(last) => now.saturating_duration_since(last) >= self.backoff(),
                }
            }
        }
    }

    /// Record that `plan` ran.
    fn note_plan(&mut self, plan: Repair, now: Instant) {
        match plan.severity() {
            Severity::Verify => {}
            Severity::Start => self.last_start = Some(now),
            Severity::Destructive => {
                self.destructive = self.destructive.saturating_add(1);
                self.last_destructive = Some(now);
            }
        }
    }

    /// A healthy verification resets the ladder, so a later genuine failure
    /// starts at the 30 s rung instead of inheriting an exhausted budget.
    fn note_healthy(&mut self) {
        *self = Budget::default();
    }
}

#[cfg(windows)]
static REPAIR_BUDGET: Mutex<Budget> = Mutex::new(Budget {
    destructive: 0,
    last_destructive: None,
    last_start: None,
});

/// Run `f` against the shared budget, recovering from a poison the way the rest
/// of this crate does: a panic elsewhere must not wedge the healer.
#[cfg(windows)]
fn with_budget<R>(f: impl FnOnce(&mut Budget) -> R) -> R {
    let mut guard = match REPAIR_BUDGET.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    f(&mut guard)
}

#[cfg(not(windows))]
fn with_budget<R: Default>(_f: impl FnOnce(&mut Budget) -> R) -> R {
    R::default()
}

// ── Observation ─────────────────────────────────────────────────────────────

/// (length, last-write-time) as a cheap content identity.
///
/// `std::fs::copy` preserves the last-write time, so our own install matches the
/// source exactly while the stock Xiaomi `IoTService.exe` cannot. Comparing
/// bytes instead (the previous behaviour) read two ~21 MB binaries on every
/// verification cycle, and the health supervisor verifies every 60 s.
fn file_identity(path: &str) -> Option<(u64, u128)> {
    let meta = std::fs::metadata(path).ok()?;
    let nanos = meta
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_nanos();
    Some((meta.len(), nanos))
}

fn binary_placement(driverstore_exe: Option<&str>, our_exe: Option<&str>) -> BinaryPlacement {
    let (Some(ds), Some(ours)) = (driverstore_exe, our_exe) else {
        return BinaryPlacement::NotApplicable;
    };
    match (file_identity(ds), file_identity(ours)) {
        (Some(a), Some(b)) if a == b => BinaryPlacement::Ours,
        (Some(_), Some(_)) => BinaryPlacement::NotOurs,
        // Unreadable metadata is not evidence that the binary is wrong, and
        // destroying a working service on an unverifiable premise is exactly the
        // bug this module exists to stop.
        _ => BinaryPlacement::NotApplicable,
    }
}

fn ping_now() -> bool {
    ping_ok(
        crate::hw::ecram::send_pipe_request(PIPE_PING_REQUEST)
            .ok()
            .as_deref(),
    )
}

/// Ask the pipe whether it is serving.
fn pipe_answers_ping() -> bool {
    if ping_now() {
        return true;
    }
    // S66-005: a single transient miss (service mid-start, all instances busy)
    // must not drop the caller into a repair. Settle and ask once more.
    std::thread::sleep(PIPE_SETTLE);
    ping_now()
}

/// Wait (bounded) for the pipe server to answer after the service is RUNNING.
fn wait_for_pipe() -> bool {
    let deadline = Instant::now() + PIPE_WAIT_TIMEOUT;
    loop {
        if ping_now() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(PIPE_POLL);
    }
}

/// Gather every fact the decision needs, touching nothing.
fn observe(target_exe: &str, driverstore_exe: Option<&str>, our_exe: Option<&str>) -> Observed {
    let (scm, stored_bin) = match win_service::query(SERVICE_NAME) {
        Ok(info) => (info.state, info.bin_path),
        Err(e) => {
            log::warn!("[ecram_service_mgmt] SCM query for {SERVICE_NAME} failed: {e}");
            (ServiceState::Unknown, None)
        }
    };
    let bin_path_ok = stored_bin
        .map(|stored| bin_path_matches(&stored, target_exe))
        .unwrap_or(false);
    Observed {
        scm,
        pipe_ok: pipe_answers_ping(),
        bin_path_ok,
        placement: binary_placement(driverstore_exe, our_exe),
    }
}

fn report(
    observed: Observed,
    action: Repair,
    exe_path: Option<String>,
    message: String,
) -> ServiceStatus {
    ServiceStatus {
        installed: observed.scm != ServiceState::Absent,
        running: observed.scm == ServiceState::Running && observed.pipe_ok,
        pipe_available: observed.pipe_ok,
        exe_path,
        message,
        action: action.label().to_string(),
    }
}

// ── Actions ─────────────────────────────────────────────────────────────────

/// Kill every process of one image name (best-effort `taskkill /F /IM`) through
/// the canonical spawn helper: bounded wait, tree kill on timeout,
/// CREATE_NO_WINDOW applied by the helper itself. The callers already treat a
/// missing process as fine, so failures stay best-effort but are logged.
#[cfg(windows)]
fn kill_process_image(image: &str) -> Result<(), crate::util::process_spawn::SpawnError> {
    let mut cmd = std::process::Command::new("taskkill");
    cmd.args(["/F", "/IM", image]);
    crate::util::process_spawn::run_capture(&mut cmd, TASKKILL_TIMEOUT).map(|_| ())
}

#[cfg(windows)]
fn report_kill(image: &str) {
    if let Err(e) = kill_process_image(image) {
        log::warn!("[ecram_service_mgmt] taskkill {image} failed: {e}");
    }
}

#[cfg(not(windows))]
fn report_kill(_image: &str) {}

/// Stop the service and WAIT for the SCM to report STOPPED, escalating to a
/// force kill only when the service ignores a bounded graceful stop.
///
/// An intentional stop must look like an intentional stop. The old code sent the
/// control, slept 2 s unconditionally and then `taskkill /F`-ed the process, so
/// every "repair" ended as `WIN32_EXIT_CODE 1067` in the event log with no
/// shutdown line in the service's own log.
fn stop_with_escalation(name: &str) -> &'static str {
    match win_service::stop(name, GRACEFUL_STOP_TIMEOUT) {
        Ok(()) => "stopped gracefully",
        Err(e) => {
            log::warn!(
                "[ecram_service_mgmt] {name} did not stop within {GRACEFUL_STOP_TIMEOUT:?} \
                 ({e}); force-killing the stale service process"
            );
            report_kill("IoTService.exe");
            report_kill("ecram_service.exe");
            let _ = win_service::wait_for_state(name, ServiceState::Stopped, GRACEFUL_STOP_TIMEOUT);
            "force-killed after a refused graceful stop"
        }
    }
}

/// Ask the SCM to start the service and wait (bounded) for RUNNING.
fn ensure_started(name: &str) -> Result<(), AppError> {
    win_service::start(name)?;
    win_service::wait_for_state(name, ServiceState::Running, START_TIMEOUT)
}

/// Poll (bounded) until a marked-for-delete entry is really gone. `DeleteService`
/// is asynchronous in the SCM; the previous fixed 2 s sleep was a guess.
fn wait_until_absent(name: &str) -> bool {
    let deadline = Instant::now() + DELETE_SETTLE_TIMEOUT;
    while Instant::now() < deadline {
        if win_service::query(name).is_ok_and(|i| i.state == ServiceState::Absent) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    log::warn!(
        "[ecram_service_mgmt] {name} still present after {DELETE_SETTLE_TIMEOUT:?} of delete settling"
    );
    false
}

/// The restart ladder the SCM applies when the service process really fails: a
/// crash must be recovered by the SCM, not by our healer.
fn failure_ladder() -> Vec<win_service::FailureAction> {
    vec![
        win_service::FailureAction::restart(5_000),
        win_service::FailureAction::restart(10_000),
        win_service::FailureAction::restart(30_000),
    ]
}

/// Outcome of the recovery-actions readback.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActionsState {
    /// Already exactly right: nothing was written.
    Matching,
    /// Installed and verified by readback.
    Installed,
    /// Could not be installed or verified (no service entry, or the SCM
    /// refused); loud, because a crash then has no recovery at all.
    Missing,
}

impl ActionsState {
    fn label(self) -> &'static str {
        match self {
            Self::Matching => "present",
            Self::Installed => "installed",
            Self::Missing => "absent",
        }
    }
}

/// Make sure the SCM holds the restart ladder for this service, verified by
/// readback instead of assumed.
///
/// This is a config write, never a restart, so it is safe against a healthy
/// service, and it is the only thing this module does on the healthy path. The
/// live machine had NO failure actions at all (`sc qfailure` reported
/// `RESET_PERIOD 0` with no `ACTIONS`), so a crash left `IoTSvc` stopped until a
/// healer episode happened to come round.
fn ensure_recovery_actions(name: &str) -> ActionsState {
    let wanted = failure_ladder();
    if let Ok(current) = win_service::failure_actions(name) {
        if current.actions == wanted && current.reset_period_secs == FAILURE_RESET_SECS {
            return ActionsState::Matching;
        }
        log::info!(
            "[ecram_service_mgmt] {name} recovery actions are {current:?}; \
             installing the restart ladder"
        );
    }
    if win_service::set_failure_actions(name, &wanted, FAILURE_RESET_SECS).is_err() {
        log::warn!(
            "[ecram_service_mgmt] could not install recovery actions for {name}; \
             a service crash will not be restarted by the SCM"
        );
        return ActionsState::Missing;
    }
    match win_service::failure_actions(name) {
        Ok(readback)
            if readback.actions == wanted && readback.reset_period_secs == FAILURE_RESET_SECS =>
        {
            ActionsState::Installed
        }
        Ok(readback) => {
            log::warn!("[ecram_service_mgmt] {name} recovery actions read back as {readback:?}");
            ActionsState::Missing
        }
        Err(e) => {
            log::warn!("[ecram_service_mgmt] {name} recovery actions unreadable after write: {e}");
            ActionsState::Missing
        }
    }
}

/// Replace the DriverStore `IoTService.exe` with our binary, backing the
/// original up once.
fn copy_into_driverstore(our_exe: &str, target: &str) -> Result<(), AppError> {
    if let (Ok(existing), Ok(ours)) = (std::fs::read(target), std::fs::read(our_exe)) {
        if existing == ours {
            return Ok(()); // already ours, nothing to do
        }
    }
    let backup = format!("{target}.bak");
    if std::fs::metadata(&backup).is_err() {
        let _ = std::fs::copy(target, &backup);
    }
    std::fs::copy(our_exe, target).map_err(|e| {
        AppError::new(
            ErrorCode::Io,
            format!("cannot replace {target} with {our_exe}: {e}"),
        )
    })?;
    log::info!("[ecram_service_mgmt] replaced {target} with our {SERVICE_EXE}");
    Ok(())
}

/// Create the service entry pointing at `exe_path`.
///
/// binPath MUST be the QUOTED path plus the `service` token: a bare path makes
/// the start fail with error 2 "cannot find the file specified" even when the
/// file exists (verified across installs and repairs on the live machine).
/// LocalSystem is the CreateServiceW default account; the old
/// `sc config obj= LocalSystem` re-statement was redundant and is gone.
#[cfg(windows)]
fn install_service(name: &str, exe_path: &str) -> Result<(), AppError> {
    use win_service::{CreateSpec, ServiceErrorControl, ServiceStartType};

    win_service::create(
        name,
        &CreateSpec {
            binary_path: format!("\"{exe_path}\" service"),
            display_name: "MiControl IoT Bridge Service".to_string(),
            start_type: ServiceStartType::Auto,
            error_control: ServiceErrorControl::Normal,
            deps: Vec::new(),
        },
    )?;
    log::info!("[ecram_service_mgmt] service '{name}' created at {exe_path}");
    Ok(())
}

#[cfg(not(windows))]
fn install_service(_name: &str, _exe_path: &str) -> Result<(), AppError> {
    Err(AppError::new(
        ErrorCode::NotSupported,
        "service installation is only supported on Windows",
    ))
}

// ── Executors ───────────────────────────────────────────────────────────────

fn pipe_word() -> &'static str {
    if wait_for_pipe() {
        "answers"
    } else {
        "still silent"
    }
}

/// Cheapest recovery: the entry is right, the service is only stopped.
fn run_start() -> String {
    match ensure_started(SERVICE_NAME) {
        Ok(()) => format!("started; pipe {}", pipe_word()),
        Err(e) => format!("start failed: {e}"),
    }
}

/// Controlled restart: graceful stop (waited), start, read back. No delete and
/// no DriverStore write.
fn run_restart() -> String {
    let stop_note = stop_with_escalation(SERVICE_NAME);
    match ensure_started(SERVICE_NAME) {
        Ok(()) => format!("{stop_note}; started; pipe {}", pipe_word()),
        Err(e) => format!("{stop_note}; start failed: {e}"),
    }
}

/// The only destructive path: wrong binary or wrong binPath.
///
/// Takes the plan as an argument so the delete cannot happen unless the decision
/// allowed it: `may_delete` is checked at run time, not just by convention.
fn run_reinstall(plan: Repair, our_exe: &str, target: &str) -> String {
    if !plan.may_delete() {
        return format!(
            "reinstall refused: plan '{}' may not delete the service entry",
            plan.label()
        );
    }
    let mut notes = Vec::new();
    // Stop (and, only if it refuses, force-kill) before replacing the image: a
    // running process keeps the old binary mapped, so a file swap alone changes
    // neither the running instance nor the DACL of its pipe.
    notes.push(stop_with_escalation(SERVICE_NAME).to_string());
    if std::path::Path::new(target) != std::path::Path::new(our_exe) {
        match copy_into_driverstore(our_exe, target) {
            Ok(()) => notes.push("DriverStore binary replaced".to_string()),
            Err(e) => return format!("DriverStore copy failed: {e}"),
        }
    }
    // Delete and recreate: `sc config binPath=` with quoted paths is unreliable.
    if win_service::delete(SERVICE_NAME).is_err() {
        notes.push("delete refused".to_string());
    }
    wait_until_absent(SERVICE_NAME);
    if let Err(e) = install_service(SERVICE_NAME, target) {
        return format!("{}; create failed: {e}", notes.join("; "));
    }
    notes.push("entry recreated".to_string());
    match ensure_started(SERVICE_NAME) {
        Ok(()) => {
            ensure_recovery_actions(SERVICE_NAME);
            notes.push(format!("started; pipe {}", pipe_word()));
        }
        Err(e) => notes.push(format!("start failed: {e}")),
    }
    notes.join("; ")
}

// ── Entry point ─────────────────────────────────────────────────────────────

/// Verify the ecram_service, and repair it only when the observation proves a
/// fault, within a bounded, escalating budget.
///
/// Steps: observe (SCM + real pipe ping + binary identity) -> ensure the SCM
/// recovery ladder -> decide -> spend budget -> act -> read back. A healthy
/// service costs one SCM query, one ping, two `stat`s and zero writes.
pub fn ensure_service_running() -> Result<ServiceStatus, String> {
    let our_exe = find_ecram_service_exe();
    let driverstore_exe = find_driverstore_iot_service_exe();
    // The service must point at a binary that exists: the DriverStore copy wins
    // because the IoT driver checks the caller's image path, and our own exe is
    // the fallback when no DriverStore entry is present.
    let Some(target_exe) = driverstore_exe.as_deref().or(our_exe.as_deref()) else {
        return Err("ecram_service.exe not found".to_string());
    };
    let target_exe = target_exe.to_string();

    let observed = observe(&target_exe, driverstore_exe.as_deref(), our_exe.as_deref());
    let plan = plan_repair(observed);
    let actions = ensure_recovery_actions(SERVICE_NAME);

    if !plan.acts() {
        if observed.pipe_ok {
            with_budget(|budget| budget.note_healthy());
            return Ok(report(
                observed,
                Repair::None,
                our_exe,
                format!(
                    "verified healthy: {SERVICE_NAME} {:?}, pipe answers, binary and binPath \
                     already correct (recovery actions: {}) - nothing was stopped, deleted or \
                     reinstalled",
                    observed.scm,
                    actions.label(),
                ),
            ));
        }
        return Ok(report(
            observed,
            Repair::None,
            our_exe,
            format!(
                "no action: {SERVICE_NAME} is {:?}, which is not evidence of a fault \
                 (recovery actions: {})",
                observed.scm,
                actions.label(),
            ),
        ));
    }

    let now = Instant::now();
    if !with_budget(|budget| budget.allowed_at(plan, now)) {
        let destructive = with_budget(|budget| budget.destructive);
        let observed_note = format!(
            "(observed: {:?}, pipe {})",
            observed.scm,
            if observed.pipe_ok {
                "answers"
            } else {
                "silent"
            }
        );
        let message = match plan.severity() {
            Severity::Start => format!(
                "start recovery throttled: at most one plain start per {START_WINDOW:?} {observed_note}"
            ),
            Severity::Destructive if destructive >= MAX_REPAIR_EPISODES => format!(
                "destructive repair budget exhausted after {destructive} episodes: the \
                 {SERVICE_NAME} entry stays untouched (no stop, no delete, no reinstall) \
                 while a plain start remains allowed every {START_WINDOW:?} {observed_note}"
            ),
            Severity::Destructive => format!(
                "destructive repair throttled: {destructive} episode(s) spent, next allowed in \
                 {:?} {observed_note}",
                with_budget(|budget| budget.backoff())
            ),
            Severity::Verify => unreachable!("a verify plan never reaches the repair branch"),
        };
        log::warn!("[ecram_service_mgmt] {message}");
        return Ok(report(observed, Repair::None, our_exe, message));
    }

    with_budget(|budget| budget.note_plan(plan, now));

    log::warn!(
        "[ecram_service_mgmt] repairing {SERVICE_NAME}: plan={} scm={:?} pipe={} binpath_ok={} \
         placement={:?}",
        plan.label(),
        observed.scm,
        observed.pipe_ok,
        observed.bin_path_ok,
        observed.placement,
    );

    let note = match plan {
        Repair::Start => run_start(),
        Repair::Restart => run_restart(),
        Repair::Reinstall => match our_exe.as_deref() {
            Some(our) => run_reinstall(plan, our, &target_exe),
            None => "reinstall refused: our ecram_service.exe is not on disk".to_string(),
        },
        Repair::None => unreachable!("the healthy branch returned above"),
    };

    // Read back the effect instead of assuming it: the caller must see the state
    // the SCM and the pipe actually report.
    let verified = observe(&target_exe, driverstore_exe.as_deref(), our_exe.as_deref());
    if verified.pipe_ok {
        with_budget(|budget| budget.note_healthy());
    }
    Ok(report(
        verified,
        plan,
        our_exe,
        format!(
            "{note}; readback: {:?}, pipe {}",
            verified.scm,
            if verified.pipe_ok {
                "answers"
            } else {
                "silent"
            }
        ),
    ))
}

/// Find the `ecram_service.exe` shipped with the app.
///
/// Looks in:
/// 1. Same directory as the current executable (installed mode)
/// 2. App install directory (C:\Program Files\miControl\)
/// 3. Development target directory
fn find_ecram_service_exe() -> Option<String> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(SERVICE_EXE);
            if candidate.exists() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }

    let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".to_string());
    let candidate = PathBuf::from(&pf).join("miControl").join(SERVICE_EXE);
    if candidate.exists() {
        return Some(candidate.to_string_lossy().into_owned());
    }

    // LOCALAPPDATA\miControl (dev mode)
    let la = std::env::var("LOCALAPPDATA").unwrap_or_default();
    if !la.is_empty() {
        let candidate = PathBuf::from(&la).join("miControl").join(SERVICE_EXE);
        if candidate.exists() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }

    None
}

/// Find the `IoTService.exe` path in the Windows DriverStore.
///
/// The IoTDriver.sys driver checks that the calling process is named
/// "IoTService.exe" and is located in the DriverStore FileRepository directory,
/// so our `ecram_service.exe` has to live there to pass the check.
///
/// Searches for `iotdriver.inf_amd64_*` directories in
/// `C:\WINDOWS\System32\DriverStore\FileRepository\`.
#[cfg(windows)]
fn find_driverstore_iot_service_exe() -> Option<String> {
    let driverstore = PathBuf::from(r"C:\WINDOWS\System32\DriverStore\FileRepository");
    if !driverstore.exists() {
        return None;
    }

    if let Ok(entries) = std::fs::read_dir(&driverstore) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("iotdriver.inf_amd64_") {
                let iot_exe = entry.path().join("IoTService.exe");
                if iot_exe.exists() {
                    return Some(iot_exe.to_string_lossy().into_owned());
                }
            }
        }
    }

    None
}

#[cfg(not(windows))]
fn find_driverstore_iot_service_exe() -> Option<String> {
    None
}

// ── Decision and budget tests ───────────────────────────────────────────────
//
// The symptom these pin: `IoTSvc` was stopped, force-killed and reinstalled on a
// flat 15-minute cooldown while it was perfectly healthy, 43 times in 11 hours.
// Every test mutates one rule of the decision or one rung of the budget and must
// fail.

#[cfg(test)]
mod repair_plan_tests {
    use super::*;

    const HEALTHY: Observed = Observed {
        scm: ServiceState::Running,
        pipe_ok: true,
        bin_path_ok: true,
        placement: BinaryPlacement::Ours,
    };

    /// (a) A healthy service verified many times must never be stopped, deleted
    /// or reinstalled. This is the case the old code kept "repairing".
    #[test]
    fn a_healthy_service_is_verified_and_left_alone_forever() {
        for cycle in 0..100 {
            let plan = plan_repair(HEALTHY);
            assert_eq!(
                plan,
                Repair::None,
                "cycle {cycle} must not touch a healthy service"
            );
            assert!(
                !plan.acts(),
                "cycle {cycle} reported an action on a healthy service"
            );
            assert!(
                !plan.may_delete(),
                "cycle {cycle} may delete a healthy service"
            );
        }
    }

    /// Only the reinstall plan may destroy the SCM entry, so a dead pipe alone
    /// can never reach `delete`.
    #[test]
    fn only_reinstall_may_delete_the_entry() {
        for plan in [Repair::None, Repair::Start, Repair::Restart] {
            assert!(
                !plan.may_delete(),
                "{plan:?} must never delete the service entry"
            );
        }
        assert!(Repair::Reinstall.may_delete());
    }

    /// (c) A running service with our binary and a silent pipe is restarted
    /// under control, not recreated.
    #[test]
    fn a_silent_pipe_on_a_running_service_restarts_it() {
        let o = Observed {
            pipe_ok: false,
            ..HEALTHY
        };
        assert_eq!(plan_repair(o), Repair::Restart);
        assert!(!plan_repair(o).may_delete());
    }

    /// A stopped entry with everything else right is started: the cheapest
    /// recovery, and the one the SCM's own restart ladder should have done.
    #[test]
    fn a_stopped_entry_with_the_right_binary_is_started() {
        let o = Observed {
            scm: ServiceState::Stopped,
            pipe_ok: false,
            ..HEALTHY
        };
        assert_eq!(plan_repair(o), Repair::Start);
        assert!(!plan_repair(o).may_delete());
    }

    /// The stock Xiaomi binary, a wrong binPath or no entry at all: the only
    /// situations that justify replacing the entry.
    #[test]
    fn a_wrong_binary_or_binpath_may_replace_the_entry() {
        for o in [
            Observed {
                placement: BinaryPlacement::NotOurs,
                ..HEALTHY
            },
            Observed {
                bin_path_ok: false,
                ..HEALTHY
            },
            Observed {
                scm: ServiceState::Stopped,
                pipe_ok: false,
                bin_path_ok: false,
                placement: BinaryPlacement::Ours,
            },
            Observed {
                scm: ServiceState::Absent,
                ..HEALTHY
            },
        ] {
            assert_eq!(plan_repair(o), Repair::Reinstall, "observation {o:?}");
        }
    }

    /// P1: "I could not ask the SCM" is not "it is broken", and a service caught
    /// mid-transition must not be fought. Neither may ever produce a delete,
    /// whatever the pipe and binPath look like.
    #[test]
    fn an_unverifiable_or_transitioning_scm_is_not_evidence_of_a_fault() {
        for state in [ServiceState::Unknown, ServiceState::Pending] {
            for pipe_ok in [true, false] {
                for bin_path_ok in [true, false] {
                    let o = Observed {
                        scm: state,
                        pipe_ok,
                        bin_path_ok,
                        placement: BinaryPlacement::NotOurs,
                    };
                    assert_eq!(
                        plan_repair(o),
                        Repair::None,
                        "{state:?} with pipe_ok={pipe_ok} bin_path_ok={bin_path_ok} must not be repaired"
                    );
                }
            }
        }
    }

    /// An unreadable DriverStore binary is `NotApplicable`, not "not ours": the
    /// service keeps running instead of being reinstalled on a guess.
    #[test]
    fn an_unknown_placement_never_forces_a_reinstall() {
        let healthy = Observed {
            placement: BinaryPlacement::NotApplicable,
            ..HEALTHY
        };
        assert_eq!(plan_repair(healthy), Repair::None);
        let silent = Observed {
            pipe_ok: false,
            placement: BinaryPlacement::NotApplicable,
            ..HEALTHY
        };
        assert_eq!(plan_repair(silent), Repair::Restart);
        assert!(!plan_repair(silent).may_delete());
    }

    /// The binPath contract: quoted path plus the `service` token, or the bare
    /// path an installer may have left behind. Nothing else.
    #[test]
    fn binpath_contract_accepts_both_canonical_forms_and_nothing_else() {
        let target =
            r"C:\WINDOWS\System32\DriverStore\FileRepository\iotdriver.inf_amd64_x\IoTService.exe";
        assert!(bin_path_matches(&format!("\"{target}\" service"), target));
        assert!(bin_path_matches(
            &format!("\"{}\" service", target.to_uppercase()),
            target
        ));
        assert!(bin_path_matches(target, target));
        assert!(!bin_path_matches(
            "\"C:\\Other\\IoTService.exe\" service",
            target
        ));
        assert!(
            !bin_path_matches(&format!("\"{target}\""), target),
            "a quoted path without the `service` token does not reach RUNNING"
        );
        assert!(!bin_path_matches("", target));
    }

    /// The readback is a real ping: an error answer, a partial answer or garbage
    /// is not health.
    #[test]
    fn ping_readback_requires_a_real_pong() {
        assert!(ping_ok(Some(r#"{"ok":true,"pong":true}"#)));
        assert!(!ping_ok(Some(r#"{"ok":false,"error":"busy"}"#)));
        assert!(
            !ping_ok(Some(r#"{"ok":true}"#)),
            "an answer without `pong` did not prove the server serves"
        );
        assert!(!ping_ok(Some("not json at all")));
        assert!(!ping_ok(Some("")));
        assert!(!ping_ok(None));
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    /// (b) One DESTRUCTIVE episode per backoff window, never two in one window.
    #[test]
    fn one_destructive_episode_per_backoff_window() {
        let t0 = Instant::now();
        let mut budget = Budget::default();
        assert!(
            budget.allowed_at(Repair::Reinstall, t0),
            "the first episode is always allowed"
        );
        budget.note_plan(Repair::Reinstall, t0);

        // One episode is banked, so the window that now applies is the SECOND rung.
        let window = Duration::from_secs(REPAIR_BACKOFF_SECS[1]);
        assert!(
            !budget.allowed_at(Repair::Reinstall, t0),
            "an episode may not repeat instantly"
        );
        assert!(
            !budget.allowed_at(Repair::Reinstall, t0 + window - Duration::from_millis(1)),
            "still inside the window"
        );
        assert!(
            budget.allowed_at(Repair::Reinstall, t0 + window),
            "the window has passed"
        );
    }

    /// The window grows rung by rung and clamps at the top, so a persistently
    /// broken install is probed less and less often, not every 15 minutes
    /// forever.
    #[test]
    fn backoff_grows_and_clamps() {
        let mut budget = Budget::default();
        let mut previous = Duration::ZERO;
        for rung in 0..REPAIR_BACKOFF_SECS.len() {
            let window = budget.backoff();
            assert!(
                window > previous,
                "rung {rung} must wait longer than the one before ({previous:?} -> {window:?})"
            );
            previous = window;
            budget.note_plan(Repair::Reinstall, Instant::now());
        }
        let top = budget.backoff();
        for _ in 0..10 {
            budget.note_plan(Repair::Reinstall, Instant::now());
        }
        assert_eq!(budget.backoff(), top, "the ladder clamps at the top rung");
        assert_eq!(
            top,
            Duration::from_secs(*REPAIR_BACKOFF_SECS.last().unwrap())
        );
    }

    /// (c) After the cap the healer stops replacing or restarting the service
    /// until a healthy verification re-arms the ladder.
    #[test]
    fn destructive_episodes_stop_at_the_cap_until_a_healthy_verification() {
        let t0 = Instant::now();
        let mut budget = Budget::default();
        let mut now = t0;
        let mut episodes = 0u32;
        // 24 h of watchdog/supervisor traffic against an install that never comes
        // back: 30 s ticks, and the budget decides how many DESTRUCTIVE repairs
        // actually run.
        for _ in 0..(24 * 60 * 2) {
            if budget.allowed_at(Repair::Reinstall, now) {
                budget.note_plan(Repair::Reinstall, now);
                episodes += 1;
            }
            now += Duration::from_secs(30);
        }
        assert_eq!(
            episodes, MAX_REPAIR_EPISODES,
            "destructive churn must be bounded, not looped (the old behaviour was one \
             stop/delete/reinstall every 15 min forever)"
        );
        assert!(!budget.allowed_at(Repair::Reinstall, now));
        assert!(!budget.allowed_at(Repair::Restart, now));
        budget.note_healthy();
        assert_eq!(budget.destructive, 0);
        assert!(
            budget.allowed_at(Repair::Reinstall, now),
            "a healthy verification re-arms the full ladder"
        );
    }

    /// A plain `sc start` is throttled but NEVER capped. This is what keeps the
    /// service recoverable on a machine where the SCM restart ladder cannot be
    /// installed at all (the default service DACL denies SERVICE_CHANGE_CONFIG to
    /// LocalSystem, verified with `sc sdshow IoTSvc`), and it never stops, deletes
    /// or reinstalls the entry on the way.
    #[test]
    fn a_plain_start_is_throttled_but_never_capped() {
        let t0 = Instant::now();
        let mut budget = Budget::default();
        let mut now = t0;
        let mut starts = 0u32;
        let ticks = 24 * 60 * 2; // 24 h of 30 s supervisor ticks
        for _ in 0..ticks {
            if budget.allowed_at(Repair::Start, now) {
                budget.note_plan(Repair::Start, now);
                starts += 1;
            }
            now += Duration::from_secs(30);
        }
        assert_eq!(
            starts as u64,
            (24 * 60 * 60) / START_WINDOW.as_secs(),
            "one plain start per {START_WINDOW:?}, for as long as the service stays dead"
        );
        assert_eq!(
            budget.destructive, 0,
            "a dead service with a correct entry must never be stopped, deleted or reinstalled"
        );
        // Plain starts must not consume the destructive budget: it stays whole
        // (still at the first rung) for a genuine install problem, and a start
        // never escalates into a restart or a reinstall on its own.
        assert_eq!(
            budget.backoff(),
            Duration::from_secs(REPAIR_BACKOFF_SECS[0]),
            "24 h of plain starts must not spend one rung of the destructive ladder"
        );
    }

    /// Only `Restart` and `Reinstall` count as destructive; `Start` is the cheap
    /// recovery and `None` spends nothing.
    #[test]
    fn severity_separates_the_cheap_recovery_from_the_destructive_one() {
        assert_eq!(Repair::None.severity(), Severity::Verify);
        assert_eq!(Repair::Start.severity(), Severity::Start);
        assert_eq!(Repair::Restart.severity(), Severity::Destructive);
        assert_eq!(Repair::Reinstall.severity(), Severity::Destructive);
        // A verify plan must never be "allowed" to run: it is not an action.
        assert!(!Budget::default().allowed_at(Repair::None, Instant::now()));
    }

    /// Poison safety: the shared budget must stay usable after a panic elsewhere
    /// in the process, or the healer wedges exactly when something is wrong.
    #[cfg(windows)]
    #[test]
    fn shared_budget_survives_a_poisoned_lock() {
        let joined = std::thread::spawn(|| {
            let _guard = REPAIR_BUDGET.lock().unwrap();
            panic!("simulated panic while holding the repair budget");
        })
        .join();
        assert!(
            joined.is_err(),
            "the helper thread must have panicked as designed"
        );
        let destructive = with_budget(|budget| budget.destructive);
        assert!(
            destructive <= MAX_REPAIR_EPISODES,
            "the budget is still readable after a poison: {destructive}"
        );
    }
}

// ── C2a: bounded spawn tests ────────────────────────────────────────────────

#[cfg(all(test, windows))]
mod ecram_service_mgmt_spawn_tests {
    use super::{failure_ladder, FAILURE_RESET_SECS};
    use crate::util::process_spawn::{self, SpawnError};
    use crate::util::win_service::FailureActionKind;
    use std::process::Command;
    use std::time::{Duration, Instant};

    /// A child sleeping past its budget answers SpawnError::Timeout and the
    /// process is really dead, not orphaned behind the timed-out wait. This pins
    /// the contract the escalated force kill in the repair path relies on.
    #[test]
    fn ecram_service_mgmt_timeout_kills_sleeping_child() {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "ping -n 10 127.0.0.1"]);
        let start = Instant::now();
        let err = process_spawn::run_capture(&mut cmd, Duration::from_millis(500))
            .expect_err("a sleeping child must hit the timeout");
        assert!(
            matches!(err, SpawnError::Timeout { .. }),
            "expected Timeout, got {err:?}"
        );
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "kill took too long: {:?}",
            start.elapsed()
        );

        std::thread::sleep(Duration::from_millis(300));
        let mut probe = Command::new("tasklist");
        probe.args(["/FI", "IMAGENAME eq ping.exe", "/NH"]);
        let out = process_spawn::run_capture(&mut probe, Duration::from_secs(15))
            .expect("tasklist probe failed");
        assert!(
            !out.stdout_str().contains("ping.exe"),
            "ping.exe still alive after timeout kill: {}",
            out.stdout_str()
        );
    }

    /// The kill helper stays best-effort for a missing image (taskkill's
    /// nonzero exit is a result, not a spawn error), and no direct child-wait
    /// calls remain in this file: every spawn goes through process_spawn.
    #[test]
    fn ecram_service_mgmt_no_direct_child_waits() {
        super::kill_process_image("mi_control_no_such_image_xyz")
            .expect("taskkill on a missing image is not a spawn failure");

        let src = include_str!("ecram_service_mgmt.rs");
        // Assembled from parts so this test's own source text cannot match
        // the tokens it forbids.
        let direct_waits = [
            format!(".{}{}{}", "out", "put", "()"),
            format!(".{}{}{}", "sta", "tus", "()"),
            format!(".{}{}", "wait", "()"),
        ];
        for token in &direct_waits {
            assert!(
                !src.contains(token.as_str()),
                "direct child wait present: {token}"
            );
        }
    }

    /// The recovery ladder is the restart trio with a daily reset, so a genuine
    /// crash is recovered by the SCM without the healer being involved.
    #[test]
    fn failure_ladder_is_three_restarts_with_a_daily_reset() {
        let ladder = failure_ladder();
        assert_eq!(ladder.len(), 3);
        assert!(
            ladder
                .iter()
                .all(|a| a.action == FailureActionKind::Restart),
            "every rung must be a restart"
        );
        assert_eq!(ladder[0].delay_ms, 5_000);
        assert_eq!(ladder[1].delay_ms, 10_000);
        assert_eq!(ladder[2].delay_ms, 30_000);
        assert_eq!(FAILURE_RESET_SECS, 86_400);
    }
}
