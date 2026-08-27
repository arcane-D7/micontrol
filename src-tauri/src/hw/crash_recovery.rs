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
/// `HKLM\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps\MiControl.exe`
/// with DumpFolder, DumpType, and DumpCount values.
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

        // Write WER LocalDumps registry settings
        // HKLM\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps\MiControl.exe
        let dump_dir_str = dump_dir.to_string_lossy().to_string();
        let wer_key_path =
            r"SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps\MiControl.exe";
        if let Ok(hklm) =
            winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE).create_subkey(wer_key_path)
        {
            let (key, _) = hklm;
            let _ = key.set_value("DumpFolder", &dump_dir_str);
            let _ = key.set_value("DumpType", &2u32); // 2 = full dump
            let _ = key.set_value("DumpCount", &10u32); // keep last 10 dumps
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

/// Mark the app as UI-healthy. Called AFTER WebView2 setup completes and the
/// tray/main window exist (not just after the process spawns).
///
/// Format: "<unix_ms>\n<pid>\n". The bridge watchdog reads the first line; if
/// the timestamp is stale while a micontrol.exe process is alive, that
/// instance is a zombie (UI dead) and gets force-restarted.
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
        let contents = format!("{ts_ms}\n{pid}\n");
        let ok = std::fs::write(&path, contents).is_ok();
        if !ok {
            log::warn!(
                target: "hw::crash_recovery",
                "Failed to write heartbeat to {}",
                path.display()
            );
        }
    }
}

/// Periodically refresh the heartbeat from the adaptive-brightness loop's
/// existing tokio runtime (a cheap, already-running 2 s ticker).
/// The write itself is tiny; we only do it every HEARTBEAT_INTERVAL_MS.
pub fn start_heartbeat_ticker() {
    #[cfg(windows)]
    {
        use std::time::Duration;
        tauri::async_runtime::spawn(async move {
            loop {
                write_heartbeat();
                tokio::time::sleep(Duration::from_secs(10)).await;
            }
        });
    }
}
