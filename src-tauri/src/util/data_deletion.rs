//! Data deletion utilities for GDPR Art.17 (right to erasure).
//!
//! Deletes AI performance logs, credential store entries, and other
//! user data when requested.

use serde::Serialize;
use std::path::PathBuf;
use tauri::Manager;

/// Returns the local data directory: `%LOCALAPPDATA%\MiControl`.
///
/// This is where security-sensitive files (HMAC key, nonces, AI usage, etc.)
/// are stored, as opposed to Tauri's `app_data_dir()` which resolves to
/// `%APPDATA%\com.micontrol.app`.
///
/// S29-002: Previously `delete_all_user_data()` only looked in `app_data_dir()`,
/// missing files that are written to `%LOCALAPPDATA%\MiControl` via `elev_dir()`.
fn local_data_dir() -> Result<PathBuf, String> {
    let base = std::env::var("LOCALAPPDATA").map_err(|e| format!("LOCALAPPDATA not set: {e}"))?;
    Ok(PathBuf::from(base).join("MiControl"))
}

/// Try to delete a file, returning `true` if it was deleted (or didn't exist).
/// On error, returns `false` and pushes the error message to `errors`.
fn try_delete_file(path: &std::path::Path, errors: &mut Vec<String>, label: &str) -> bool {
    if !path.exists() {
        return true; // Nothing to delete — success.
    }
    match std::fs::remove_file(path) {
        Ok(()) => true,
        Err(e) => {
            errors.push(format!("Failed to delete {label}: {e}"));
            false
        }
    }
}

/// Delete all user data stored by the application.
/// This includes:
/// - AI performance logs (JSONL files)
/// - Credential store entries
/// - Schedule data
/// - localStorage is cleared by the frontend
///
/// S29-002: Now deletes from BOTH `app_data_dir()` (Tauri's `%APPDATA%\com.micontrol.app`)
/// AND `local_data_dir()` (`%LOCALAPPDATA%\MiControl`), because security-sensitive
/// files (HMAC key, nonces, AI usage, hotkey consent, etc.) are stored in the latter.
pub fn delete_all_user_data(app: &tauri::AppHandle) -> Result<DeleteDataReport, String> {
    let mut report = DeleteDataReport::default();

    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("AppData dir unavailable: {e}"))?;

    // S29-002: Also get the local data directory where security-sensitive files live.
    let local_dir = local_data_dir()?;

    // 1. Delete AI performance logs (from app_data_dir)
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

    // 3. Delete schedule data (from app_data_dir)
    let schedule_path = app_data.join("schedule.json");
    if schedule_path.exists() {
        match std::fs::remove_file(&schedule_path) {
            Ok(()) => report.schedule_deleted = true,
            Err(e) => report
                .errors
                .push(format!("Failed to delete schedule: {e}")),
        }
    }

    // 4. Delete consent records (consent.json) — from both locations
    let consent_app = app_data.join("consent.json");
    try_delete_file(&consent_app, &mut report.errors, "consent.json (app_data)");
    let consent_local = local_dir.join("consent.json");
    try_delete_file(&consent_local, &mut report.errors, "consent.json (local)");
    report.consent_deleted = true;

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

    // 6. Purge consent audit log (also delete the file from local_data_dir)
    crate::util::consent_audit::purge_audit_log();
    let audit_log_local = local_dir.join("consent_audit.log");
    try_delete_file(&audit_log_local, &mut report.errors, "consent_audit.log");
    report.audit_log_deleted = true;

    // 7. Delete hardware profile — from both locations
    let hw_app = app_data.join("hardware_profile.json");
    try_delete_file(
        &hw_app,
        &mut report.errors,
        "hardware_profile.json (app_data)",
    );
    let hw_local = local_dir.join("hardware_profile.json");
    try_delete_file(
        &hw_local,
        &mut report.errors,
        "hardware_profile.json (local)",
    );
    report.hardware_profile_deleted = true;

    // 8. Delete hotkeys config — from both locations
    let hotkeys_app = app_data.join("hotkeys.json");
    try_delete_file(&hotkeys_app, &mut report.errors, "hotkeys.json (app_data)");
    let hotkeys_local = local_dir.join("hotkeys.json");
    if try_delete_file(&hotkeys_local, &mut report.errors, "hotkeys.json (local)") {
        report.hotkeys_deleted = true;
    }

    // 9. Delete nonces file — from local_data_dir (where it's actually written)
    let nonces_local = local_dir.join("nonces.json");
    // Also check app_data_dir for older installations
    let nonces_app = app_data.join("nonces.json");
    try_delete_file(&nonces_app, &mut report.errors, "nonces.json (app_data)");
    if try_delete_file(&nonces_local, &mut report.errors, "nonces.json (local)") {
        report.nonces_deleted = true;
    }

    // 10. Delete elevated bridge HMAC key — from local_data_dir (where it's actually written)
    let elev_key_local = local_dir.join("elev_key.bin");
    let elev_key_app = app_data.join("elev_key.bin");
    try_delete_file(&elev_key_app, &mut report.errors, "elev_key.bin (app_data)");
    if try_delete_file(&elev_key_local, &mut report.errors, "elev_key.bin (local)") {
        report.elev_key_deleted = true;
    }

    // 11. Delete old elevated bridge key backup — from local_data_dir
    let elev_key_old_local = local_dir.join("elev_key.bin.old");
    let elev_key_old_app = app_data.join("elev_key.bin.old");
    try_delete_file(
        &elev_key_old_app,
        &mut report.errors,
        "elev_key.bin.old (app_data)",
    );
    try_delete_file(
        &elev_key_old_local,
        &mut report.errors,
        "elev_key.bin.old (local)",
    );

    // 12. Delete AI config — from both locations
    let ai_config_app = app_data.join("ai_config.json");
    try_delete_file(
        &ai_config_app,
        &mut report.errors,
        "ai_config.json (app_data)",
    );
    let ai_config_local = local_dir.join("ai_config.json");
    try_delete_file(
        &ai_config_local,
        &mut report.errors,
        "ai_config.json (local)",
    );

    // S29-002: Delete ai_usage.json — from local_data_dir (where it's actually written)
    let ai_usage_local = local_dir.join("ai_usage.json");
    if try_delete_file(&ai_usage_local, &mut report.errors, "ai_usage.json") {
        report.ai_usage_deleted = true;
    }

    // S29-002: Delete hotkey_consent.json — from local_data_dir (where it's actually written)
    let hotkey_consent_local = local_dir.join("hotkey_consent.json");
    if try_delete_file(
        &hotkey_consent_local,
        &mut report.errors,
        "hotkey_consent.json",
    ) {
        report.hotkey_consent_deleted = true;
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
    /// S29-002: AI usage stats file deleted from `%LOCALAPPDATA%\MiControl`.
    pub ai_usage_deleted: bool,
    /// S29-002: Hotkey consent file deleted from `%LOCALAPPDATA%\MiControl`.
    pub hotkey_consent_deleted: bool,
    pub errors: Vec<String>,
}
