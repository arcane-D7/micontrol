//! Logging initialisation for development and production.
//!
//! Sets up `fern` logging to stdout + a persistent rolling file in
//! `%LOCALAPPDATA%\MiControl\logs` (both dev and the installed app).
//!
//! - Dev mode (`cargo tauri dev`): `tauri-dev-trace.log` at Trace level.
//! - Installed app (release): `tauri-app.log` at Info level.
//!
//! In every mode the log file is persistent so errors, warnings and
//! lifecycle events survive restarts and can be shipped with a bug report.
//! If the log file cannot be created (e.g. ACL after an elevated run) we
//! fall back to console-only logging rather than failing the app.

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::SystemTime;

/// Path of the file-backed log (set on first successful init), used by
/// `log_file_path()` so the frontend can display / export it.
static DEV_LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Whether the current log is file-backed (used to decide file-level verbosity).
static DEV_FILE_ACTIVE: OnceLock<bool> = OnceLock::new();

pub fn init_logging() -> Result<()> {
    if is_tauri_dev() {
        init_dev_file_logger()
    } else {
        init_release_file_logger()
    }
}

/// Path of the file-backed log after `init_logging()` succeeded
/// (dev: `tauri-dev-trace.log`, release: `tauri-app.log`).
pub fn dev_log_path() -> Option<&'static std::path::Path> {
    DEV_LOG_PATH.get().map(PathBuf::as_path)
}

/// Same as `dev_log_path()` — semantic alias used by the frontend to expose
/// the persistent log location in the installed app.
pub fn log_file_path() -> Option<&'static std::path::Path> {
    dev_log_path()
}

/// Whether the log is backed by a file (vs console-only fallback).
pub fn is_file_logged() -> bool {
    DEV_FILE_ACTIVE.get().copied().unwrap_or(false)
}

fn is_tauri_dev() -> bool {
    cfg!(debug_assertions)
        || std::env::var_os("TAURI_DEV_HOST").is_some()
        || std::env::var_os("VITE_DEV_SERVER_URL").is_some()
}

/// (Dev) file logger — used by `cargo tauri dev`.
fn init_dev_file_logger() -> Result<()> {
    let log_dir = resolve_log_dir()?;
    // The log dir can become unreadable if a previous elevated instance left
    // an ACL that denies the current user (e.g. after an admin relaunch or a
    // Windows profile permission change). Do NOT fail the whole app for this —
    // fall back to console-only logging instead (after a best-effort ACL fix).
    if !ensure_log_dir_writable(&log_dir) {
        eprintln!(
            "[debug_log] WARNING: cannot create log dir {:?} — falling back to console-only logging",
            log_dir
        );
        return init_console_only_logger();
    }

    let log_path = log_dir.join("tauri-dev-trace.log");
    let log_file = match fern::log_file(&log_path) {
        Ok(f) => Some(f),
        Err(e) => {
            eprintln!(
                "[debug_log] WARNING: cannot open dev log file {:?} ({e}) — falling back to console-only logging",
                log_path
            );
            None
        }
    };

    let _ = DEV_LOG_PATH.set(log_path.clone());

    let trace_enabled = dev_trace_enabled();
    let base_level = if trace_enabled {
        log::LevelFilter::Trace
    } else {
        log::LevelFilter::Info
    };

    let mut dispatch = fern::Dispatch::new()
        .level(base_level)
        .level_for("hyper", log::LevelFilter::Info)
        .level_for("mio", log::LevelFilter::Info)
        .level_for("want", log::LevelFilter::Info)
        .format(|out, message, record| {
            let ts = humantime::format_rfc3339_millis(SystemTime::now());
            let thread = std::thread::current();
            let thread_name = thread.name().unwrap_or("unnamed");
            out.finish(format_args!(
                "{ts} [{level:<5}] [{thread_name}] {target}: {message}",
                level = record.level(),
                target = record.target(),
            ))
        })
        .chain(std::io::stdout());

    if let Some(file) = log_file {
        dispatch = dispatch.chain(file);
    }

    dispatch.apply().context("apply fern logger")?;

    log::info!(
        target: "devlog",
        "dev logging enabled at {} (trace={})",
        log_path.display(),
        trace_enabled
    );
    Ok(())
}

/// (Release / installed app) file logger — same persistent file logging as
/// dev mode, but without thread names and at Info level. The app MUST keep a
/// persistent, human-readable log in the final installed build.
fn init_release_file_logger() -> Result<()> {
    let log_dir = resolve_log_dir()?;
    if !ensure_log_dir_writable(&log_dir) {
        eprintln!(
            "[debug_log] WARNING: cannot create log dir {:?} — falling back to console-only logging",
            log_dir
        );
        return init_console_only_logger();
    }

    let log_path = log_dir.join("tauri-app.log");
    let log_file = match fern::log_file(&log_path) {
        Ok(f) => Some(f),
        Err(e) => {
            // The file cannot be created — the dir may exist with a broken
            // ACL (e.g. created long ago by an elevated instance). Repair the
            // ACL if possible, then retry once.
            let _ = repair_log_dir_acl(&log_dir);
            match fern::log_file(&log_path) {
                Ok(f) => Some(f),
                Err(_) => {
                    eprintln!(
                        "[debug_log] WARNING: cannot open app log file {:?} ({e}) — falling back to console-only logging",
                        log_path
                    );
                    None
                }
            }
        }
    };

    let _ = DEV_LOG_PATH.set(log_path.clone());
    if log_file.is_some() {
        let _ = DEV_FILE_ACTIVE.set(true);
    }

    let mut dispatch = fern::Dispatch::new()
        .level(log::LevelFilter::Info)
        .level_for("hyper", log::LevelFilter::Info)
        .level_for("mio", log::LevelFilter::Info)
        .level_for("want", log::LevelFilter::Info)
        .format(|out, message, record| {
            let ts = humantime::format_rfc3339_millis(SystemTime::now());
            out.finish(format_args!(
                "{ts} [{level:<5}] {target}: {message}",
                level = record.level(),
                target = record.target(),
            ))
        })
        .chain(std::io::stdout());

    if let Some(file) = log_file {
        dispatch = dispatch.chain(file);
    }

    dispatch.apply().context("apply fern logger")?;

    log::info!(
        target: "applog",
        "persistent app logging enabled at {}",
        log_path.display()
    );
    Ok(())
}

/// Ensure the log directory exists and is writable. Returns false if it
/// cannot be made writable (in which case callers fall back to console-only).
fn ensure_log_dir_writable(log_dir: &std::path::Path) -> bool {
    if std::fs::create_dir_all(log_dir).is_ok() {
        // Verify we can actually create a file inside (the dir may exist with
        // a broken ACL that silently denies writes).
        let probe = log_dir.join(".write_probe");
        match std::fs::File::create(&probe) {
            Ok(_) => {
                let _ = std::fs::remove_file(&probe);
                return true;
            }
            Err(_) => {
                // Dir exists but not writable — try to repair the ACL.
                return repair_log_dir_acl(log_dir) && std::fs::File::create(&probe).is_ok() && {
                    let _ = std::fs::remove_file(&probe);
                    true
                };
            }
        }
    }
    false
}

/// Best-effort repair of the log dir ACL when it is not writable.
/// Uses `icacls` to grant the current user Full Control (the dir may have a
/// broken ACL from a previous elevated instance). This can only succeed if we
/// are elevated or own the folder — otherwise we report failure and the
/// caller falls back to console-only logging.
fn repair_log_dir_acl(log_dir: &std::path::Path) -> bool {
    use std::process::Command;
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "Users".into());
    // icacls "dir" /grant "<user>:(OI)(CI)F" /T
    Command::new("icacls")
        .arg(log_dir)
        .arg("/grant")
        .arg(format!("{user}:(OI)(CI)F"))
        .arg("/T")
        .arg("/Q")
        .output()
        .is_ok_and(|out| out.status.success())
}

/// Console-only logger used when the dev log file cannot be opened
/// (e.g. ACL denies access after an elevated run). Ensures the app still
/// starts and logs to stdout in Tauri dev mode.
fn init_console_only_logger() -> Result<()> {
    fern::Dispatch::new()
        .level(log::LevelFilter::Info)
        .format(|out, message, record| {
            let ts = humantime::format_rfc3339_millis(SystemTime::now());
            out.finish(format_args!(
                "{ts} [{level:<5}] {target}: {message}",
                level = record.level(),
                target = record.target(),
            ))
        })
        .chain(std::io::stdout())
        .apply()
        .context("apply console-only fern logger")?;
    Ok(())
}

fn dev_trace_enabled() -> bool {
    match std::env::var("MICONTROL_DEV_TRACE") {
        Ok(v) => {
            let s = v.trim().to_ascii_lowercase();
            !(s == "0" || s == "false" || s == "off" || s == "no")
        }
        Err(_) => true,
    }
}

fn resolve_log_dir() -> Result<PathBuf> {
    if let Some(local_appdata) = std::env::var_os("LOCALAPPDATA") {
        return Ok(PathBuf::from(local_appdata).join("MiControl").join("logs"));
    }

    let exe = std::env::current_exe().context("current_exe for log dir")?;
    let parent = exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Cannot derive parent directory for log path"))?;
    Ok(parent.join("logs"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_init_logging_does_not_panic() {
        // init_logging may fail if the logger is already initialized
        // (e.g., when running multiple tests). That's OK — we just want
        // to verify it doesn't panic.
        let _ = init_logging();
    }

    #[test]
    fn test_dev_trace_enabled_returns_bool() {
        // Just verify the function runs and returns a bool
        let _ = dev_trace_enabled();
    }

    #[test]
    fn test_resolve_log_dir_with_localappdata() {
        let orig = std::env::var_os("LOCALAPPDATA");
        let tmp = std::env::temp_dir().join("micontrol_test_logdir");
        std::env::set_var("LOCALAPPDATA", &tmp);

        let dir = resolve_log_dir().expect("resolve_log_dir should succeed");
        assert!(
            dir.starts_with(&tmp),
            "Log dir should be under LOCALAPPDATA"
        );

        // Cleanup
        match orig {
            Some(v) => std::env::set_var("LOCALAPPDATA", v),
            None => std::env::remove_var("LOCALAPPDATA"),
        }
    }
}
