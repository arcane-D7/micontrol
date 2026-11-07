//! Crash recovery and application resilience.
//!
//! Implements Windows Restart Manager registration, WER crash dump collection,
//! and boot-persistence checks to detect abnormal termination and restore
//! runtime state on next launch.
//!
//! This mirrors XPM's `get_abnormal_restart_environment_recovery` and
//! `get_application_anomaly_monitoring_and_repair` features.

use serde::{Deserialize, Serialize};

use crate::hw::errors::HardwareResult;
use tauri::Manager;

/// Registry key for tracking clean vs abnormal shutdowns.
#[cfg(windows)]
const RECOVERY_REG_KEY: &str = r"SOFTWARE\MiControl\CrashRecovery";

/// Registry value name for the "last clean exit" timestamp (high 32 bits).
const LAST_CLEAN_EXIT_HI_VALUE: &str = "LastCleanExitHi";
/// Registry value name for the "last clean exit" timestamp (low 32 bits).
const LAST_CLEAN_EXIT_LO_VALUE: &str = "LastCleanExitLo";

/// Registry value name for the "abnormal restart detected" flag.
const ABNORMAL_RESTART_VALUE: &str = "AbnormalRestartDetected";

/// Path (under `%ProgramData%\MiControl`) of the user-quit sentinel file.
///
/// The bridge service runs as LOCAL SYSTEM and cannot reliably read the
/// interactive user's HKCU hive. Instead of registry, the app writes this
/// machine-wide sentinel on intentional quit; the watchdog treats its mere
/// presence as "user quit — do NOT restart" (indefinitely). The app removes
/// the sentinel at its own startup, re-arming the watchdog for that session.
#[cfg(windows)]
const USER_QUIT_SENTINEL_REL: &str = r"MiControl\watchdog_user_quit";

/// Heartbeat file (under `%ProgramData%\MiControl`) written by the app once
/// its UI/WebView2 setup has finished, then refreshed periodically.
///
/// S32-005: The SYSTEM bridge watchdog uses this to distinguish a HEALTHY
/// process from a "zombie" that is alive in the process table but never
/// completed (or lost) its UI — the exact symptom the user reported on the
/// watchdog-relaunched instance ("volta travada, não consigo abrir a UI").
/// If the process exists but the heartbeat is stale, the watchdog force-kills
/// it and relaunches a fresh one (bounded retries) instead of leaving a dead
/// UI running forever.
#[cfg(windows)]
const HEARTBEAT_REL: &str = r"MiControl\heartbeat";

/// Consecutive async-runtime probe failures after which the app escalates to
/// the bridge watchdog by writing [`STARVATION_MARKER`].
///
/// One probe takes up to 5 s and the heartbeat ticker sleeps 10 s, so 10
/// consecutive failures ≈ 2.5 minutes of a wedged tokio runtime — long enough
/// to rule out a brief thread-pool spike, short enough to matter (the 06-Nov
/// incident lasted 2 h 46 min because `ui_ok` had no consumer).
pub const STARVATION_ESCALATE_FAILS: u32 = 10;

/// Machine-readable escalation marker (under `%ProgramData%\MiControl`).
///
/// Written by this module when the async runtime has been starved for
/// [`STARVATION_ESCALATE_FAILS`] consecutive probes; read by the SYSTEM
/// bridge watchdog, which force-restarts the app. Format:
/// `"<epoch_ms>\n<pid>\n<fails>\n"` — same shape as the heartbeat, so a
/// human reading either file sees the same fields.
pub const STARVATION_MARKER: &str = "runtime_starved";

/// How long a starvation marker is trusted by the watchdog.
///
/// The writer refreshes it every [`STARVATION_ESCALATE_FAILS`] failures
/// (~2.5 min); anything older belongs to a previous session and must be
/// ignored (`pid` is checked as well).
pub const STARVATION_MARKER_TTL_MS: u64 = 180_000;

/// Snapshot of the escalation marker (producer: `write_starvation_marker`,
/// consumer: the bridge watchdog).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StarvationState {
    /// Wall-clock time the marker was written (epoch ms).
    pub ts_ms: u64,
    /// PID of the app that reported the starvation.
    pub pid: u32,
    /// Consecutive failed async-runtime probes at the time of writing.
    pub fails: u32,
}

/// Serialise a starvation marker. Pure, so the writer and the watchdog's
/// parser can be tested against the same bytes.
pub fn format_starvation_marker(state: StarvationState) -> String {
    format!("{}\n{}\n{}\n", state.ts_ms, state.pid, state.fails)
}

/// Parse a starvation marker. Strict: all three lines must be present and
/// valid, otherwise the watchdog ignores the file (never acts on garbage).
pub fn parse_starvation_marker(content: &str) -> Result<StarvationState, &'static str> {
    let mut lines = content.lines();
    let ts_ms = lines
        .next()
        .ok_or("missing timestamp")?
        .trim()
        .parse::<u64>()
        .map_err(|_| "invalid timestamp")?;
    let pid = lines
        .next()
        .ok_or("missing pid")?
        .trim()
        .parse::<u32>()
        .map_err(|_| "invalid pid")?;
    let fails = lines
        .next()
        .ok_or("missing fails")?
        .trim()
        .parse::<u32>()
        .map_err(|_| "invalid fails")?;
    if ts_ms == 0 {
        return Err("zero timestamp");
    }
    if pid == 0 {
        return Err("zero pid");
    }
    if fails == 0 {
        return Err("zero fails");
    }
    Ok(StarvationState { ts_ms, pid, fails })
}

/// Crash recovery status information.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CrashRecoveryStatus {
    /// True if the previous session ended abnormally (crash, power loss).
    pub abnormal_restart_detected: bool,
    /// Unix timestamp (ms) of the last clean exit, if any.
    pub last_clean_exit_ms: Option<u64>,
    /// Whether WER crash dump registration is active.
    pub wer_registered: bool,
    /// Whether Restart Manager registration is active.
    pub restart_manager_registered: bool,
}

/// Initialize crash recovery for the current session.
///
/// This should be called early during app startup. It:
/// 1. Checks if the previous session ended abnormally
/// 2. Registers with the Windows Restart Manager
/// 3. Registers WER crash dump collection
/// 4. Marks the current session as "running" (not yet cleanly exited)
pub fn init_crash_recovery() -> HardwareResult<CrashRecoveryStatus> {
    let abnormal_restart_detected = check_abnormal_restart();

    if abnormal_restart_detected {
        log::warn!(
            target: "hw::crash_recovery",
            "Abnormal restart detected — previous session did not exit cleanly"
        );
    }

    // Register with Windows Restart Manager
    let restart_manager_registered = register_restart_manager();

    // Register WER crash dump collection
    let wer_registered = register_wer();

    // Mark session as running (clear the clean-exit flag)
    mark_session_running();

    let last_clean_exit_ms = read_last_clean_exit();

    Ok(CrashRecoveryStatus {
        abnormal_restart_detected,
        last_clean_exit_ms,
        wer_registered,
        restart_manager_registered,
    })
}

/// Mark the current session as cleanly exited.
///
/// Call this on normal app shutdown (window close, tray quit, etc.)
/// to indicate that the next launch should NOT trigger crash recovery.
pub fn mark_clean_exit() {
    #[cfg(windows)]
    {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_CURRENT_USER;

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        if let Ok(key) = RegKeyGuard::create_write(HKEY_CURRENT_USER, RECOVERY_REG_KEY) {
            let hi = (now_ms >> 32) as u32;
            let lo = (now_ms & 0xFFFFFFFF) as u32;
            let _ = key.write_u32(LAST_CLEAN_EXIT_HI_VALUE, hi);
            let _ = key.write_u32(LAST_CLEAN_EXIT_LO_VALUE, lo);
            let _ = key.write_u32(ABNORMAL_RESTART_VALUE, 0);
        }
    }
}

/// Check if the previous session ended abnormally.
///
/// Returns true if the "session running" flag was set but no clean exit
/// was recorded (i.e., the process was killed or crashed).
fn check_abnormal_restart() -> bool {
    #[cfg(windows)]
    {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_CURRENT_USER;

        let key = match RegKeyGuard::open_read(HKEY_CURRENT_USER, RECOVERY_REG_KEY) {
            Ok(Some(k)) => k,
            _ => return false, // First run — no abnormal restart
        };

        // If AbnormalRestartDetected is 1, the previous session crashed
        match key.read_u32(ABNORMAL_RESTART_VALUE) {
            Ok(Some(val)) => val != 0,
            _ => false,
        }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Mark the current session as "running" — if the process dies without
/// calling `mark_clean_exit()`, the next launch will detect the abnormal restart.
fn mark_session_running() {
    #[cfg(windows)]
    {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_CURRENT_USER;

        if let Ok(key) = RegKeyGuard::create_write(HKEY_CURRENT_USER, RECOVERY_REG_KEY) {
            // Set abnormal flag to 1 — it will be cleared on clean exit
            let _ = key.write_u32(ABNORMAL_RESTART_VALUE, 1);
        }
    }
}

/// Read the last clean exit timestamp from registry.
fn read_last_clean_exit() -> Option<u64> {
    #[cfg(windows)]
    {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_CURRENT_USER;

        let key = RegKeyGuard::open_read(HKEY_CURRENT_USER, RECOVERY_REG_KEY).ok()??;
        let hi = key
            .read_u32(LAST_CLEAN_EXIT_HI_VALUE)
            .ok()
            .flatten()
            .unwrap_or(0);
        let lo = key
            .read_u32(LAST_CLEAN_EXIT_LO_VALUE)
            .ok()
            .flatten()
            .unwrap_or(0);
        if hi == 0 && lo == 0 {
            return None;
        }
        Some(((hi as u64) << 32) | (lo as u64))
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Register with the Windows Restart Manager.
///
/// This tells Windows to automatically restart the application if it
/// crashes or is terminated unexpectedly. The restart command is
/// `micontrol.exe --minimized`.
///
/// Fixed in v0.1.20:
/// - Flags are now `0` (restart on crash AND hang). The previous code passed
///   `RESTART_NO_CRASH | RESTART_NO_HANG`, which told Windows to NOT restart
///   on the exact failure modes we care about (a crash) — and, per MS docs,
///   passing `PCWSTR::null()` as the command line *removes* any previously
///   registered restart command. So before this fix the registration was
///   effectively a no-op for crash recovery.
/// - The command line is a real `"--minimized"` argument so a Windows-initiated
///   restart boots straight into the tray (no window popping onto the user's
///   screen), matching the autostart behavior.
///
/// NOTE: `RegisterApplicationRestart` alone is a *best-effort* mechanism —
/// Windows shows a WER-style dialog and asks the user for consent in some
/// configurations. The durable "never stop unless the user quit" guarantee is
/// provided by the MiControlBridge watchdog, which relaunches the app even if
/// WER's restart dialog is dismissed or suppressed.
fn register_restart_manager() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::System::Recovery::RegisterApplicationRestart;

        // Command line for the restarted instance: start minimized (tray-only).
        // Empty command line would unregister — we must pass a real value.
        let cmdline = windows::core::w!("--minimized");
        // flags = 0 → restart on crash, hang, or unexpected termination.
        // (RESTART_NO_CRASH would opt OUT of restarting after a crash.)
        let flags = windows::Win32::System::Recovery::REGISTER_APPLICATION_RESTART_FLAGS(0);

        let result = unsafe { RegisterApplicationRestart(cmdline, flags) };

        match result {
            Ok(()) => {
                log::info!(
                    target: "hw::crash_recovery",
                    "Restart Manager registered (cmdline=--minimized, flags=0 → auto-restart on crash)"
                );
                true
            }
            Err(e) => {
                log::warn!(
                    target: "hw::crash_recovery",
                    "Failed to register with Restart Manager: {e}"
                );
                false
            }
        }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Register Windows Error Reporting (WER) for crash dump collection.
///
/// This configures WER to collect minidumps when the application crashes,
/// storing them in `%LOCALAPPDATA%\MiControl\crashdumps`.
///
/// WER LocalDumps requires registry configuration under
/// `HKLM\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps\<exact
/// process image name>` with DumpFolder, DumpType, and DumpCount values.
///
/// S66-006: the key MUST match the process image name EXACTLY — the binary
/// is `micontrol.exe` (lowercase). The previous key `MiControl.exe` never
/// matched, so 9,718 crashes (2026-10-20..23) produced ZERO dumps. Both key
/// spellings are now written (the old one kept for compatibility with any
/// external tooling that reads it). The dump folder is also created with a
/// permissive ACL: when the elevated helper (SYSTEM) creates it first, the
/// inherited default ACL denied even directory-listing to the user, blocking
/// forensics.
fn register_wer() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::System::ErrorReporting::{
            WerSetFlags, WER_FAULT_REPORTING_FLAG_QUEUE, WER_FAULT_REPORTING_FLAG_QUEUE_UPLOAD,
        };

        // Set WER flags to queue crash reports
        let flags = WER_FAULT_REPORTING_FLAG_QUEUE | WER_FAULT_REPORTING_FLAG_QUEUE_UPLOAD;
        let _ = unsafe { WerSetFlags(flags) };

        // Configure WER LocalDumps via registry
        let dump_dir = std::env::var("LOCALAPPDATA")
            .map(|base| {
                std::path::PathBuf::from(base)
                    .join("MiControl")
                    .join("crashdumps")
            })
            .unwrap_or_else(|_| std::path::PathBuf::from("crashdumps"));

        let _ = std::fs::create_dir_all(&dump_dir);
        // S66-006: ensure the user can READ the dumps even when the folder
        // was first created by the SYSTEM helper (its default ACL denied
        // user listing). icacls grants the current user modify rights; best
        // effort — failure is logged, not fatal.
        {
            let whoami = std::process::Command::new("whoami.exe")
                .output()
                .ok()
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .unwrap_or_default();
            if !whoami.is_empty() {
                let dir = dump_dir.display().to_string();
                let out = std::process::Command::new("icacls.exe")
                    .args([&dir, "/grant", &format!("{whoami}:(OI)(CI)M"), "/Q"])
                    .output();
                if let Some(o) = out.ok().filter(|o| !o.status.success()) {
                    log::warn!(
                        target: "hw::crash_recovery",
                        "icacls grant on dump dir failed: {}",
                        String::from_utf8_lossy(&o.stderr).trim()
                    );
                }
            }
        }

        // Write WER LocalDumps registry settings — BOTH process-name
        // spellings (S66-006: exact-match requirement).
        let dump_dir_str = dump_dir.to_string_lossy().to_string();
        for key_name in ["micontrol.exe", "MiControl.exe"] {
            let wer_key_path = format!(
                r"SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps\{key_name}"
            );
            if let Ok(hklm) = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE)
                .create_subkey(&wer_key_path)
            {
                let (key, _) = hklm;
                let _ = key.set_value("DumpFolder", &dump_dir_str);
                let _ = key.set_value("DumpType", &2u32); // 2 = full dump
                let _ = key.set_value("DumpCount", &10u32); // keep last 10 dumps
            }
        }

        log::info!(
            target: "hw::crash_recovery",
            "WER crash dump collection registered (dumps → {})",
            dump_dir.display()
        );
        true
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Machine-wide sentinel path used to signal an intentional user quit to the
/// watchdog service (which runs as LOCAL SYSTEM and cannot read HKCU).
#[cfg(windows)]
fn user_quit_sentinel_path() -> Option<std::path::PathBuf> {
    let pd = std::env::var_os("ProgramData")?;
    let base = std::path::PathBuf::from(pd).join(USER_QUIT_SENTINEL_REL);
    Some(base)
}

/// Machine-wide marker path reflecting the user's autostart (and therefore
/// watchdog) preference. The SYSTEM bridge service cannot read HKCU, so the
/// app mirrors its autostart choice here: file present ⇒ watchdog armed.
#[cfg(windows)]
pub fn watchdog_enabled_path() -> Option<std::path::PathBuf> {
    let pd = std::env::var_os("ProgramData")?;
    Some(std::path::PathBuf::from(pd).join(r"MiControl\watchdog_enabled"))
}

/// Machine-wide escalation marker path (see [`STARVATION_MARKER`]).
#[cfg(windows)]
pub fn starvation_marker_path() -> Option<std::path::PathBuf> {
    let pd = std::env::var_os("ProgramData")?;
    Some(
        std::path::PathBuf::from(pd)
            .join("MiControl")
            .join(STARVATION_MARKER),
    )
}

/// Is this failure count an escalation boundary?
///
/// The first crossing at [`STARVATION_ESCALATE_FAILS`] writes the marker; every
/// further multiple refreshes its timestamp so the watchdog keeps trusting it
/// while the runtime stays wedged.
fn should_escalate(fails: u32) -> bool {
    fails >= STARVATION_ESCALATE_FAILS && fails.is_multiple_of(STARVATION_ESCALATE_FAILS)
}

/// Should this failure be logged?
///
/// The 06-Nov starvation produced 666 identical ERROR lines in 2 h 46 min —
/// the log became useless and nothing acted on it. Log the first two failures
/// (so a short spike is diagnosable) and then only at escalation boundaries.
fn should_log_failure(fails: u32) -> bool {
    fails <= 2 || should_escalate(fails)
}

/// Write/refresh the escalation marker. Best effort — a read-only ProgramData
/// only degrades to "no escalation", never a panic.
#[cfg(windows)]
fn write_starvation_marker(fails: u32) {
    let Some(path) = starvation_marker_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let ts_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let body = format_starvation_marker(StarvationState {
        ts_ms,
        pid: std::process::id(),
        fails,
    });
    match std::fs::write(&path, body) {
        Ok(()) => log::error!(
            target: "hw::crash_recovery",
            "async runtime starved {fails} consecutive probes — escalated to the bridge \
             watchdog ({}); expecting a force-restart",
            path.display()
        ),
        Err(e) => log::warn!(
            target: "hw::crash_recovery",
            "async runtime starved {fails} consecutive probes but the escalation \
             marker could not be written ({}): {e}",
            path.display()
        ),
    }
}

/// Remove the escalation marker once the runtime schedules tasks again.
#[cfg(windows)]
fn clear_starvation_marker(recovered_after: u32) {
    let Some(path) = starvation_marker_path() else {
        return;
    };
    if path.exists() {
        match std::fs::remove_file(&path) {
            Ok(()) => log::info!(
                target: "hw::crash_recovery",
                "async runtime recovered after {recovered_after} failed probes — \
                 escalation marker cleared"
            ),
            Err(e) => log::warn!(
                target: "hw::crash_recovery",
                "async runtime recovered but the escalation marker could not be \
                 removed ({}): {e}",
                path.display()
            ),
        }
    }
}

/// Enable or disable the MiControlBridge watchdog.
///
/// Called at every startup (mirroring the current autostart state) and by the
/// autostart toggle so the SYSTEM bridge service can decide whether to
/// auto-relaunch `micontrol.exe` after an unexpected death. A missing file
/// means "do not auto-relaunch".
pub fn set_watchdog_enabled(enabled: bool) {
    #[cfg(windows)]
    if let Some(path) = watchdog_enabled_path() {
        let parent = path.parent();
        let res = if enabled {
            if let Some(p) = parent {
                let _ = std::fs::create_dir_all(p);
            }
            std::fs::write(&path, b"1")
        } else {
            std::fs::remove_file(&path).or(Ok(()))
        };
        log::info!(
            target: "hw::crash_recovery",
            "Watchdog enabled marker {enabled} written ({}): {res:?}",
            path.display()
        );
    }
}

/// Mark an intentional quit so the MiControlBridge watchdog does NOT relaunch
/// the app.
///
/// Called by the tray Quit handler (and main-window close in dev builds) right
/// before the process exits. Writes a timestamp sentinel under `ProgramData`
/// (readable by the SYSTEM bridge service); the watchdog skips auto-restart for
/// a grace period (default 2 minutes) after a fresh sentinel.
pub fn mark_user_quit() {
    #[cfg(windows)]
    if let Some(path) = user_quit_sentinel_path() {
        let parent = path.parent();
        if let Some(p) = parent {
            let _ = std::fs::create_dir_all(p);
        }
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let ok = std::fs::write(&path, format!("{ts}")).is_ok();
        log::info!(
            target: "hw::crash_recovery",
            "User quit marked (sentinel={}, ok={ok}) — watchdog will not auto-restart",
            path.display()
        );
    }
}

/// Clear the user-quit sentinel. Called after a successful (non-restarted)
/// startup so the watchdog is armed again for the current session.
pub fn clear_user_quit_marker() {
    #[cfg(windows)]
    if let Some(path) = user_quit_sentinel_path() {
        let _ = std::fs::remove_file(&path);
    }
}

/// Was the app intentionally quit (user-quit sentinel present)?
///
/// Used by the app at startup for diagnostics only; the app ALWAYS clears the
/// sentinel on startup (a running MiControl means the user wants it running,
/// so the watchdog must be armed).
pub fn user_quit_pending() -> bool {
    #[cfg(windows)]
    {
        let Some(path) = user_quit_sentinel_path() else {
            return false;
        };
        path.exists()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Machine-wide heartbeat path.
#[cfg(windows)]
fn heartbeat_path() -> Option<std::path::PathBuf> {
    let pd = std::env::var_os("ProgramData")?;
    Some(std::path::PathBuf::from(pd).join(HEARTBEAT_REL))
}

/// App handle used by the UI-probe for heartbeat liveness.
static APP_HANDLE: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

/// Store the app handle so `write_heartbeat` can probe whether the UI
/// (main window + WebView2) is actually alive, not just the process.
#[cfg(windows)]
pub fn set_app_handle(app: tauri::AppHandle) {
    let _ = APP_HANDLE.set(app);
}

/// Result of the last async-runtime probe (set by the probe task itself).
static ASYNC_PROBE_DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Consecutive async-probe failures. ui_ok flips to 0 only after two probes
/// in a row fail (~20 s of starvation at the 10 s heartbeat cadence), so a
/// brief worker-thread spike does not trigger a watchdog restart.
///
/// The counter also drives escalation: at [`STARVATION_ESCALATE_FAILS`] the
/// app writes [`STARVATION_MARKER`] for the bridge watchdog. Before 2026-11-07
/// this counter only produced one ERROR log line per tick — 666 lines over
/// 2 h 46 min with no action.
static ASYNC_PROBE_FAILS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Is the tokio runtime (the one Tauri commands and poll loops run on) still
/// scheduling tasks? S56 FIX (2026-09-13 UI freeze): the async runtime can
/// starve — e.g. every worker parked behind a stuck IGCL mutex — while the
/// process, this heartbeat thread (plain OS thread) and even the main window
/// all look perfectly healthy. The watchdog then sees heartbeat=fresh,
/// ui_ok=1 and never restarts a UI that no longer responds to any invoke.
/// A trivial spawned task that must flip a flag proves the runtime still
/// schedules; two consecutive misses (~20 s) report the runtime as dead.
#[cfg(windows)]
fn async_runtime_alive() -> bool {
    ASYNC_PROBE_DONE.store(false, std::sync::atomic::Ordering::SeqCst);
    let spawned = tauri::async_runtime::spawn(async {
        ASYNC_PROBE_DONE.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    drop(spawned);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if ASYNC_PROBE_DONE.load(std::sync::atomic::Ordering::SeqCst) {
            let previous = ASYNC_PROBE_FAILS.swap(0, std::sync::atomic::Ordering::SeqCst);
            if previous > 0 {
                clear_starvation_marker(previous);
            }
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    let fails = ASYNC_PROBE_FAILS.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
    if should_escalate(fails) {
        // Crossing the threshold (and every refresh after it) is both the
        // log line and the machine-readable escalation to the watchdog.
        write_starvation_marker(fails);
    } else if should_log_failure(fails) {
        if fails == 1 {
            log::warn!(
                target: "hw::crash_recovery",
                "async runtime probe failed once (workers may be starved)"
            );
        } else {
            log::error!(
                target: "hw::crash_recovery",
                "async runtime probe failed {fails} consecutive times — runtime starved"
            );
        }
    }
    fails >= 2
}

/// Is the UI (main window) currently alive AND is the async runtime still
/// scheduling? Probing the WebviewWindow is cheap (a handle + visibility
/// check) and fails fast when the window was destroyed by a WebView2 runtime
/// crash — the exact "zombie" state that previously kept the process alive
/// with a fresh heartbeat but no UI. The runtime probe (S56) covers the other
/// freeze mode: window alive but every invoke stuck on starved workers.
#[cfg(windows)]
fn ui_is_alive() -> bool {
    let Some(app) = APP_HANDLE.get() else {
        return false;
    };
    if app.get_webview_window("main").is_none() {
        return false;
    }
    async_runtime_alive()
}

/// Mark the app's liveness for the watchdog. Called AFTER WebView2 setup
/// completes and on a 10 s ticker.
///
/// Format: "<unix_ms>\n<pid>\n<ui_ok>\n". The bridge watchdog reads the first
/// line (heartbeat freshness) AND the third line (UI liveness). A process
/// whose UI died (WebView2 crash) keeps refreshing line 1 via the tokio
/// ticker but reports `ui_ok=0` on line 3, so the watchdog can force-restart
/// it instead of leaving a dead-window zombie (the 0.1.23-style regression).
pub fn write_heartbeat() {
    #[cfg(windows)]
    if let Some(path) = heartbeat_path() {
        if let Some(p) = path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let ts_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let pid = std::process::id();
        // ui_ok=1 only while the main window (WebView2) exists; 0 once the
        // webview was destroyed (crash / unload) even though the process and
        // its tokio ticker are still running.
        let ui_ok = if ui_is_alive() { 1 } else { 0 };
        let contents = format!("{ts_ms}\n{pid}\n{ui_ok}\n");
        // Write-then-rename: the watchdog parses all three lines strictly, so
        // it must never observe a truncated file mid-write (a torn read would
        // look like "unreadable heartbeat" and could cost the app a restart).
        // `fs::rename` replaces the destination on Windows.
        let tmp = path.with_extension("tmp");
        let mut ok =
            std::fs::write(&tmp, &contents).is_ok() && std::fs::rename(&tmp, &path).is_ok();
        if !ok {
            // Fallback (e.g. AV holds the temp file): direct write. The
            // watchdog tolerates a torn read with a short retry.
            let _ = std::fs::remove_file(&tmp);
            ok = std::fs::write(&path, contents).is_ok();
        }
        if !ok {
            log::warn!(
                target: "hw::crash_recovery",
                "Failed to write heartbeat to {}",
                path.display()
            );
        }
    }
}

/// Periodically refresh the heartbeat.
///
/// S32-005b FIX: this used to be a tokio task (`tauri::async_runtime::spawn`)
/// with a 10 s sleep. Tokio tasks starve when any other task on the shared
/// multi-thread runtime blocks a worker for tens of seconds (e.g. an
/// elevated-bridge retry storm, WMI probes, synchronous file IO). When the
/// runtime starved, the heartbeat stopped being refreshed while the process
/// (and the UI!) stayed perfectly alive — the SYSTEM bridge watchdog then
/// read the stale heartbeat, declared the app a zombie and force-killed it,
/// producing the visible "app closes and reopens" loop every ~3.5 minutes.
/// A dedicated OS thread is immune to tokio starvation: it always wakes on
/// schedule no matter what the async runtime is doing.
pub fn start_heartbeat_ticker() {
    #[cfg(windows)]
    {
        // Spawn exactly once per process.
        static STARTED: std::sync::Once = std::sync::Once::new();
        STARTED.call_once(|| {
            std::thread::Builder::new()
                .name("heartbeat".into())
                .spawn(|| loop {
                    write_heartbeat();
                    std::thread::sleep(std::time::Duration::from_secs(10));
                })
                .expect("failed to spawn heartbeat thread");
        });
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starvation_marker_round_trips() {
        let state = StarvationState {
            ts_ms: 1_794_044_373_192,
            pid: 2944,
            fails: 10,
        };
        let body = format_starvation_marker(state);
        assert_eq!(body, "1794044373192\n2944\n10\n");
        assert_eq!(parse_starvation_marker(&body).expect("round trip"), state);
        // Trailing whitespace / extra lines must not break the consumer.
        assert_eq!(
            parse_starvation_marker("1794044373192\r\n2944\r\n10\r\n").expect("crlf"),
            state
        );
    }

    #[test]
    fn starvation_marker_rejects_truncated_and_garbage() {
        for bad in [
            "",
            "1794044373192",
            "1794044373192\n2944",
            "1794044373192\n2944\n",
            "garbage\n2944\n12\n",
            "1794044373192\nnope\n12\n",
            "1794044373192\n2944\nmany\n",
            "0\n2944\n12\n",
            "1794044373192\n0\n12\n",
            "1794044373192\n2944\n0\n",
        ] {
            assert!(parse_starvation_marker(bad).is_err(), "must reject {bad:?}");
        }
    }

    /// Escalation must happen at N consecutive failures (and refresh on every
    /// further multiple), never before.
    #[test]
    fn escalation_starts_at_ten_consecutive_failures() {
        assert_eq!(STARVATION_ESCALATE_FAILS, 10);
        for fails in 0..STARVATION_ESCALATE_FAILS {
            assert!(!should_escalate(fails), "fails={fails} must not escalate");
        }
        assert!(should_escalate(10));
        assert!(!should_escalate(11));
        assert!(should_escalate(20), "the marker must be refreshed");
        assert!(should_escalate(30));
    }

    /// The 06-Nov storm was one ERROR line per probe (666 lines). With
    /// throttling the same 30 failures produce five lines.
    #[test]
    fn failure_logs_are_throttled() {
        let logged: Vec<u32> = (1..=30).filter(|f| should_log_failure(*f)).collect();
        assert_eq!(
            logged,
            vec![1, 2, 10, 20, 30],
            "only the first spike, the second failure and escalation boundaries are logged"
        );
    }
}
