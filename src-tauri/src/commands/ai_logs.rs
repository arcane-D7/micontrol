//! AI performance mode log commands.
//!
//! When Smart or Smart Acceleration is active the frontend writes a log entry
//! every ~30 s via `write_ai_perf_log`.  Entries are appended as JSONL lines to
//! `<AppData>\MiControl\ai_perf_logs\YYYY-MM-DD.jsonl`.
//! The log directory is created automatically on first write.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

// ── Entry type ───────────────────────────────────────────────────────────────

/// One timed snapshot written while an AI mode is active.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AiPerfLogEntry {
    /// ISO-8601 timestamp (local time, e.g. "2026-05-17T14:32:01")
    pub ts: String,
    /// Active performance mode ("smart" | "smart_acceleration")
    pub mode: String,
    /// CPU package temperature (°C)
    pub cpu_temp: f32,
    /// GPU temperature (°C)
    pub gpu_temp: f32,
    /// System TDP from RAPL (W), or null if not yet available
    pub tdp_watts: Option<f32>,
    /// CPU utilisation % (matches Task Manager)
    pub cpu_pct: f64,
    /// GPU 3D engine utilisation %
    pub gpu_pct: f64,
    /// Optional note from AI model or rule engine (reserved for future use)
    pub note: Option<String>,
}

// ── Helper: log directory ────────────────────────────────────────────────────

fn log_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("AppData dir unavailable: {e}"))?
        .join("ai_perf_logs");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot create log dir: {e}"))?;
    Ok(dir)
}

fn today_log_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = log_dir(app)?;
    // Use UTC date to keep filenames consistent across timezone changes
    #[cfg(windows)]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let days = secs / 86400;
        // Simple Gregorian date from Unix epoch (days since 1970-01-01)
        let date = epoch_days_to_ymd(days as u32);
        Ok(dir.join(format!("{date}.jsonl")))
    }
    #[cfg(not(windows))]
    {
        Ok(dir.join("log.jsonl"))
    }
}

/// Convert days since 1970-01-01 to "YYYY-MM-DD" string.
fn epoch_days_to_ymd(mut d: u32) -> String {
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    d += 719468;
    let era = d / 146097;
    let doe = d % 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let day = doy - (153 * mp + 2) / 5 + 1;
    let year = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}", year, m, day)
}

// ── Tauri commands ───────────────────────────────────────────────────────────

/// Append one log entry to today's JSONL file.
#[tauri::command]
pub async fn write_ai_perf_log(entry: AiPerfLogEntry, app: AppHandle) -> Result<(), String> {
    let path = today_log_path(&app)?;
    let line = serde_json::to_string(&entry).map_err(|e| e.to_string())? + "\n";
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("Cannot open log file: {e}"))?;
    f.write_all(line.as_bytes())
        .map_err(|e| format!("Write failed: {e}"))?;
    Ok(())
}

/// Read the last `limit` entries across recent log files (newest first).
/// Returns at most `limit` entries (capped at 500).
#[tauri::command]
pub async fn read_ai_perf_logs(
    limit: Option<u32>,
    app: AppHandle,
) -> Result<Vec<AiPerfLogEntry>, String> {
    let cap = limit.unwrap_or(100).min(500) as usize;
    let dir = log_dir(&app)?;

    // Collect all .jsonl files, sorted by name descending (newest first by YYYY-MM-DD)
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| format!("Cannot list log dir: {e}"))?
        .filter_map(|e| e.ok().map(|de| de.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("jsonl"))
        .collect();
    files.sort_unstable_by(|a, b| b.cmp(a));

    let mut entries: Vec<AiPerfLogEntry> = Vec::new();
    'outer: for file in files {
        let text = std::fs::read_to_string(&file).unwrap_or_default();
        // Read lines in reverse (newest entries last in file = show them first)
        let lines: Vec<&str> = text.lines().collect();
        for line in lines.iter().rev() {
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(e) = serde_json::from_str::<AiPerfLogEntry>(line) {
                entries.push(e);
                if entries.len() >= cap {
                    break 'outer;
                }
            }
        }
    }
    Ok(entries)
}

/// Open the AI log directory in Windows Explorer.
#[tauri::command]
pub async fn open_ai_logs_dir(app: AppHandle) -> Result<(), String> {
    let dir = log_dir(&app)?;
    #[cfg(windows)]
    {
        std::process::Command::new("explorer")
            .arg(dir.as_os_str())
            .spawn()
            .map_err(|e| format!("Cannot open explorer: {e}"))?;
    }
    Ok(())
}
