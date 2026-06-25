//! Privacy commands — GDPR Art.20 data portability.
//!
//! Exports all user data as a ZIP archive so the user can obtain a copy
//! of their data in a machine-readable format.

use std::io::Write;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use zip::write::SimpleFileOptions;
use zip::CompressionMethod;

/// Files in the AppData directory that contain user data.
const USER_DATA_FILES: &[&str] = &[
    "hardware_profile.json",
    "hotkeys.json",
    "consent_audit.log",
    "ai_config.json",
    "schedule.json",
    "consent.json",
    // S27-003: nonces.json excluded — internal anti-replay cache, not user data.
];

/// Export all user data as a ZIP archive (GDPR Art.20 — Right to data portability).
///
/// Collects all user data files from the AppData directory and creates a ZIP
/// archive containing them. Returns the path to the created ZIP file.
#[tauri::command]
pub async fn export_user_data(app: AppHandle) -> Result<String, String> {
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("AppData dir unavailable: {e}"))?;

    // Create the export in a temp directory
    let export_dir = std::env::temp_dir().join("micontrol_export");
    std::fs::create_dir_all(&export_dir)
        .map_err(|e| format!("Cannot create export directory: {e}"))?;

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let zip_path = export_dir.join(format!("micontrol_data_export_{timestamp}.zip"));

    let file =
        std::fs::File::create(&zip_path).map_err(|e| format!("Cannot create ZIP file: {e}"))?;
    let mut zip = zip::ZipWriter::new(file);

    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);

    // Add user data files
    for &filename in USER_DATA_FILES {
        let file_path = app_data.join(filename);
        if file_path.exists() {
            let contents =
                std::fs::read(&file_path).map_err(|e| format!("Cannot read {filename}: {e}"))?;
            zip.start_file(filename, options)
                .map_err(|e| format!("Cannot add {filename} to ZIP: {e}"))?;
            zip.write_all(&contents)
                .map_err(|e| format!("Cannot write {filename} to ZIP: {e}"))?;
        }
    }

    // Add AI performance logs if they exist
    let ai_log_dir = app_data.join("ai_perf_logs");
    if ai_log_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&ai_log_dir) {
            for entry in entries.flatten() {
                if let Ok(metadata) = entry.metadata() {
                    if metadata.is_file() {
                        let path = entry.path();
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            let contents = std::fs::read(&path)
                                .map_err(|e| format!("Cannot read AI log {name}: {e}"))?;
                            let archive_path = format!("ai_perf_logs/{name}");
                            zip.start_file(&archive_path, options)
                                .map_err(|e| format!("Cannot add AI log {name} to ZIP: {e}"))?;
                            zip.write_all(&contents)
                                .map_err(|e| format!("Cannot write AI log {name} to ZIP: {e}"))?;
                        }
                    }
                }
            }
        }
    }

    // Add a manifest file describing the export
    let manifest = format!(
        "MiControl Data Export\n\
         Generated: {timestamp}\n\
         \n\
         Files included:\n\
         - hardware_profile.json: Detected hardware configuration\n\
         - hotkeys.json: Custom keyboard shortcut mappings\n\
         - consent_audit.log: Telemetry consent history\n\
         - ai_config.json: AI analysis configuration\n\
         - schedule.json: Scheduled task configuration\n\
         - consent.json: Current consent state\n\
         - ai_perf_logs/: AI performance log entries\n"
    );
    zip.start_file("MANIFEST.txt", options)
        .map_err(|e| format!("Cannot add MANIFEST.txt to ZIP: {e}"))?;
    zip.write_all(manifest.as_bytes())
        .map_err(|e| format!("Cannot write MANIFEST.txt to ZIP: {e}"))?;

    zip.finish()
        .map_err(|e| format!("Cannot finalize ZIP archive: {e}"))?;

    // S25-004: Restrict ACL on the export ZIP file to prevent other users from reading it.
    if let Err(e) = crate::util::auth::restrict_file_acl(&zip_path) {
        log::warn!("Failed to restrict ACL on export ZIP file: {e}");
    }

    Ok(zip_path.to_string_lossy().into_owned())
}

/// Open a file path in the system file explorer (selects the file).
///
/// The path must resolve to within the app data directory or the export
/// (temp) directory — arbitrary paths are rejected to prevent directory
/// traversal attacks.
#[tauri::command]
pub async fn reveal_in_explorer(app: AppHandle, path: String) -> Result<(), String> {
    let p = PathBuf::from(&path);
    if !p.exists() {
        return Err(format!("File does not exist: {path}"));
    }

    // Resolve the canonical path to prevent traversal tricks (e.g. "..\..").
    let canonical = p
        .canonicalize()
        .map_err(|e| format!("Cannot resolve path: {e}"))?;

    // Allow only paths within the app data directory or the export temp directory.
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("AppData dir unavailable: {e}"))?;
    let export_dir = std::env::temp_dir().join("micontrol_export");

    let is_allowed = canonical.starts_with(&app_data) || canonical.starts_with(&export_dir);
    if !is_allowed {
        return Err("Access denied: path is outside the allowed directories".to_string());
    }

    #[cfg(windows)]
    {
        // S27-002: Pass canonical path to explorer.exe to prevent TOCTOU.
        std::process::Command::new("explorer.exe")
            .args(["/select,", &canonical.to_string_lossy()])
            .spawn()
            .map_err(|e| format!("Cannot open explorer: {e}"))?;
    }

    Ok(())
}
