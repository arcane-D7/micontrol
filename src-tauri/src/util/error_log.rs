//! Error logging system with 7-day retention.
//!
//! Writes error-level logs to `%LOCALAPPDATA%\MiControl\logs\errors.log`
//! with automatic rotation when the file exceeds 1 MB.
//! Logs older than 7 days are purged on startup.
//!
//! This is always on by default. Users can disable it via the
//! `error_logging_enabled` setting.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

const MAX_LOG_SIZE: u64 = 1_048_576; // 1 MB
const RETENTION_DAYS: u64 = 7;
const MAX_ROTATIONS: u32 = 5;

static ENABLED: Mutex<bool> = Mutex::new(true);

/// Settings for error logging.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorLogConfig {
    pub enabled: bool,
    pub retention_days: u64,
    pub log_path: String,
}

/// Initialize error logging — call once at startup.
pub fn init() {
    // Purge old logs on startup
    purge_old_logs();

    // Check if error logging is disabled via registry
    if let Ok(false) = is_enabled_via_registry() {
        if let Ok(mut e) = ENABLED.lock() {
            *e = false;
        }
        log::info!("[error_log] Error logging disabled by user setting");
    } else {
        log::info!(
            "[error_log] Error logging initialized (retention={}d)",
            RETENTION_DAYS
        );
    }
}

/// Log an error message to the error log file.
pub fn log_error(target: &str, error: &str) {
    let enabled = ENABLED.lock().map(|e| *e).unwrap_or(true);
    if !enabled {
        return;
    }

    let path = match log_path() {
        Some(p) => p,
        None => return,
    };

    // Rotate if file is too large
    if let Ok(meta) = fs::metadata(&path) {
        if meta.len() > MAX_LOG_SIZE {
            rotate_log(&path);
        }
    }

    let timestamp = humantime::format_rfc3339_millis(SystemTime::now());
    let line = format!("{timestamp} [{target}] ERROR: {error}\n");

    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = file.write_all(line.as_bytes());
    }
}

/// Get the current error log configuration.
pub fn get_config() -> ErrorLogConfig {
    let enabled = ENABLED.lock().map(|e| *e).unwrap_or(true);
    let log_path = log_path()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    ErrorLogConfig {
        enabled,
        retention_days: RETENTION_DAYS,
        log_path,
    }
}

/// Enable or disable error logging.
pub fn set_enabled(enabled: bool) {
    if let Ok(mut e) = ENABLED.lock() {
        *e = enabled;
    }
    // Persist to registry
    persist_enabled_to_registry(enabled);
    log::info!(
        "[error_log] Error logging {}",
        if enabled { "enabled" } else { "disabled" }
    );
}

/// Read the error log file contents (last N lines).
pub fn read_log(max_lines: usize) -> String {
    let path = match log_path() {
        Some(p) => p,
        None => return String::new(),
    };

    let content = fs::read_to_string(&path).unwrap_or_default();
    if content.is_empty() {
        return String::new();
    }

    let lines: Vec<&str> = content.lines().collect();
    let start = if lines.len() > max_lines {
        lines.len() - max_lines
    } else {
        0
    };
    lines[start..].join("\n")
}

/// Clear the error log file.
pub fn clear_log() {
    let path = match log_path() {
        Some(p) => p,
        None => return,
    };
    let _ = fs::write(&path, "");
    log::info!("[error_log] Error log cleared");
}

/// Get the error log file path.
fn log_path() -> Option<PathBuf> {
    let dir = log_dir()?;
    Some(dir.join("errors.log"))
}

/// Get the log directory.
fn log_dir() -> Option<PathBuf> {
    if let Some(local_appdata) = std::env::var_os("LOCALAPPDATA") {
        return Some(PathBuf::from(local_appdata).join("MiControl").join("logs"));
    }
    let exe = std::env::current_exe().ok()?;
    let parent = exe.parent()?;
    Some(parent.join("logs"))
}

/// Rotate the log file: errors.log → errors.log.1, etc.
fn rotate_log(path: &PathBuf) {
    // Delete the oldest rotation
    let oldest = path.with_extension("log.5");
    let _ = fs::remove_file(&oldest);

    // Shift rotations: .4 → .5, .3 → .4, ... .1 → .2, main → .1
    for n in (1..MAX_ROTATIONS).rev() {
        let from = path.with_extension(format!("log.{}", n));
        let to = path.with_extension(format!("log.{}", n + 1));
        let _ = fs::rename(&from, &to);
    }

    // Move current log to .1
    let rotated = path.with_extension("log.1");
    let _ = fs::rename(path, &rotated);
}

/// Purge log files older than RETENTION_DAYS.
fn purge_old_logs() {
    let dir = match log_dir() {
        Some(d) => d,
        None => return,
    };

    let now = SystemTime::now();
    let max_age = std::time::Duration::from_secs(RETENTION_DAYS * 24 * 60 * 60);

    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            // Only purge error log files
            if !name_str.starts_with("errors.log") {
                continue;
            }

            if let Ok(meta) = entry.metadata() {
                if let Ok(modified) = meta.modified() {
                    if let Ok(age) = now.duration_since(modified) {
                        if age > max_age {
                            let _ = fs::remove_file(&path);
                            log::debug!("[error_log] Purged old log: {}", path.display());
                        }
                    }
                }
            }
        }
    }
}

/// Read the enabled state from the Windows registry.
#[cfg(windows)]
fn is_enabled_via_registry() -> Result<bool, ()> {
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_CURRENT_USER, KEY_READ,
    };

    let subkey: Vec<u16> = "SOFTWARE\\MiControl\0".encode_utf16().collect();
    let mut hkey = windows::Win32::System::Registry::HKEY::default();
    let result = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            windows::core::PCWSTR(subkey.as_ptr()),
            0,
            KEY_READ,
            &mut hkey,
        )
    };
    if result.is_err() {
        return Ok(true); // Default: enabled
    }

    let value_name: Vec<u16> = "ErrorLoggingEnabled\0".encode_utf16().collect();
    let mut buf_val: u32 = 1;
    let mut buf_len: u32 = std::mem::size_of::<u32>() as u32;
    let value_type = windows::Win32::System::Registry::REG_NONE;
    let result = unsafe {
        RegQueryValueExW(
            hkey,
            windows::core::PCWSTR(value_name.as_ptr()),
            None,
            Some(&mut value_type.clone() as *mut _),
            Some(&mut buf_val as *mut u32 as *mut u8),
            Some(&mut buf_len),
        )
    };

    unsafe {
        let _ = RegCloseKey(hkey);
    }

    if result.is_err() {
        return Ok(true);
    }
    Ok(buf_val != 0)
}

#[cfg(not(windows))]
fn is_enabled_via_registry() -> Result<bool, ()> {
    Ok(true)
}

/// Persist the enabled state to the Windows registry.
#[cfg(windows)]
fn persist_enabled_to_registry(enabled: bool) {
    use windows::Win32::System::Registry::{
        RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY_CURRENT_USER, REG_DWORD,
    };

    let subkey: Vec<u16> = "SOFTWARE\\MiControl\0".encode_utf16().collect();
    let mut hkey = windows::Win32::System::Registry::HKEY::default();
    let result = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            windows::core::PCWSTR(subkey.as_ptr()),
            0,
            None,
            0,
            windows::Win32::System::Registry::KEY_WRITE,
            None,
            &mut hkey,
            None,
        )
    };
    if result.is_err() {
        return;
    }

    let value_name: Vec<u16> = "ErrorLoggingEnabled\0".encode_utf16().collect();
    let val: u32 = if enabled { 1 } else { 0 };
    unsafe {
        let _ = RegSetValueExW(
            hkey,
            windows::core::PCWSTR(value_name.as_ptr()),
            None,
            REG_DWORD,
            Some(std::slice::from_raw_parts(
                &val as *const u32 as *const u8,
                std::mem::size_of::<u32>(),
            )),
        );
        let _ = RegCloseKey(hkey);
    }
}

#[cfg(not(windows))]
fn persist_enabled_to_registry(_enabled: bool) {}
