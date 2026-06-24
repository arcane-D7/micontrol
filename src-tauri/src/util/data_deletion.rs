//! Data deletion utilities for GDPR Art.17 (right to erasure).
//!
//! Deletes AI performance logs, credential store entries, and other
//! user data when requested.

use serde::Serialize;
use tauri::Manager;

/// Delete all user data stored by the application.
/// This includes:
/// - AI performance logs (JSONL files)
/// - Credential store entries
/// - Schedule data
/// - localStorage is cleared by the frontend
pub fn delete_all_user_data(app: &tauri::AppHandle) -> Result<DeleteDataReport, String> {
    let mut report = DeleteDataReport::default();

    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("AppData dir unavailable: {e}"))?;

    // 1. Delete AI performance logs
    let log_dir = app_data.join("ai_perf_logs");
    if log_dir.exists() {
        match std::fs::remove_dir_all(&log_dir) {
            Ok(()) => report.logs_deleted = true,
            Err(e) => report.errors.push(format!("Failed to delete logs: {e}")),
        }
    }

    // 2. Delete credential store entries
    match keyring::Entry::new("com.mipc.micontrol", "openai_api_key")
        .and_then(|e| e.delete_credential())
    {
        Ok(()) => report.credentials_deleted = true,
        Err(keyring::Error::NoEntry) => report.credentials_deleted = true,
        Err(e) => report
            .errors
            .push(format!("Failed to delete credential: {e}")),
    }

    // 3. Delete schedule data
    let schedule_path = app_data.join("schedule.json");
    if schedule_path.exists() {
        match std::fs::remove_file(&schedule_path) {
            Ok(()) => report.schedule_deleted = true,
            Err(e) => report
                .errors
                .push(format!("Failed to delete schedule: {e}")),
        }
    }

    // 4. Delete consent records (consent.json)
    let consent_path = app_data.join("consent.json");
    if consent_path.exists() {
        match std::fs::remove_file(&consent_path) {
            Ok(()) => report.consent_deleted = true,
            Err(e) => report.errors.push(format!("Failed to delete consent: {e}")),
        }
    }

    // 5. Delete telemetry consent keyring entry
    match keyring::Entry::new("com.mipc.micontrol", "telemetry_consent")
        .and_then(|e| e.delete_credential())
    {
        Ok(()) => report.credentials_deleted = true,
        Err(keyring::Error::NoEntry) => {}
        Err(e) => report
            .errors
            .push(format!("Failed to delete telemetry consent key: {e}")),
    }

    // 6. Purge consent audit log
    crate::util::consent_audit::purge_audit_log();
    report.audit_log_deleted = true;

    // 7. Delete hardware profile
    let hw_profile_path = app_data.join("hardware_profile.json");
    if hw_profile_path.exists() {
        match std::fs::remove_file(&hw_profile_path) {
            Ok(()) => report.hardware_profile_deleted = true,
            Err(e) => report
                .errors
                .push(format!("Failed to delete hardware profile: {e}")),
        }
    }

    // 8. Delete hotkeys config
    let hotkeys_path = app_data.join("hotkeys.json");
    if hotkeys_path.exists() {
        match std::fs::remove_file(&hotkeys_path) {
            Ok(()) => report.hotkeys_deleted = true,
            Err(e) => report.errors.push(format!("Failed to delete hotkeys: {e}")),
        }
    }

    // 9. Delete nonces file
    let nonces_path = app_data.join("nonces.json");
    if nonces_path.exists() {
        match std::fs::remove_file(&nonces_path) {
            Ok(()) => report.nonces_deleted = true,
            Err(e) => report.errors.push(format!("Failed to delete nonces: {e}")),
        }
    }

    // 10. Delete elevated bridge HMAC key
    let elev_key_path = app_data.join("elev_key.bin");
    if elev_key_path.exists() {
        match std::fs::remove_file(&elev_key_path) {
            Ok(()) => report.elev_key_deleted = true,
            Err(e) => report
                .errors
                .push(format!("Failed to delete elev key: {e}")),
        }
    }

    // 11. Delete old elevated bridge key backup
    let elev_key_old_path = app_data.join("elev_key.bin.old");
    if elev_key_old_path.exists() {
        let _ = std::fs::remove_file(&elev_key_old_path);
    }

    // 12. Delete AI config if it exists
    let ai_config_path = app_data.join("ai_config.json");
    if ai_config_path.exists() {
        let _ = std::fs::remove_file(&ai_config_path);
    }

    Ok(report)
}

/// Rotate AI performance logs — delete entries older than 30 days.
pub fn rotate_logs(app: &tauri::AppHandle) -> Result<u32, String> {
    let mut deleted_count = 0u32;

    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("AppData dir unavailable: {e}"))?;

    let log_dir = app_data.join("ai_perf_logs");
    if !log_dir.exists() {
        return Ok(0);
    }

    let cutoff = std::time::SystemTime::now() - std::time::Duration::from_secs(30 * 24 * 60 * 60);

    if let Ok(entries) = std::fs::read_dir(&log_dir) {
        for entry in entries.flatten() {
            if let Ok(metadata) = entry.metadata() {
                if let Ok(modified) = metadata.modified() {
                    if modified < cutoff && std::fs::remove_file(entry.path()).is_ok() {
                        deleted_count += 1;
                    }
                }
            }
        }
    }

    Ok(deleted_count)
}

#[derive(Default, Serialize)]
pub struct DeleteDataReport {
    pub logs_deleted: bool,
    pub credentials_deleted: bool,
    pub schedule_deleted: bool,
    pub consent_deleted: bool,
    pub audit_log_deleted: bool,
    pub hardware_profile_deleted: bool,
    pub hotkeys_deleted: bool,
    pub nonces_deleted: bool,
    pub elev_key_deleted: bool,
    pub errors: Vec<String>,
}
