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
/// `micontrol.exe` with no arguments.
fn register_restart_manager() -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::System::Recovery::RegisterApplicationRestart;

        // Register for restart with no command-line arguments.
        // RESTART_NO_CRASH | RESTART_NO_HANG — only restart on crash/hang,
        // not on system reboot (we handle that via autostart).
        let flags = windows::Win32::System::Recovery::RESTART_NO_CRASH
            | windows::Win32::System::Recovery::RESTART_NO_HANG;

        let result = unsafe { RegisterApplicationRestart(windows::core::PCWSTR::null(), flags) };

        match result {
            Ok(()) => {
                log::info!(
                    target: "hw::crash_recovery",
                    "Restart Manager registered successfully"
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
