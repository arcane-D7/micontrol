//! Logging initialisation for development and production.
//!
//! Sets up `fern` logging to stdout (production) or a rolling file
//! in `%LOCALAPPDATA%\MiControl\logs` (Tauri dev mode).

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::SystemTime;

static DEV_LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn init_logging() -> Result<()> {
    if is_tauri_dev() {
        init_dev_file_logger()
    } else {
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
            .context("init fern logger")?;
        Ok(())
    }
}

pub fn dev_log_path() -> Option<&'static std::path::Path> {
    DEV_LOG_PATH.get().map(PathBuf::as_path)
}

fn is_tauri_dev() -> bool {
    cfg!(debug_assertions)
        || std::env::var_os("TAURI_DEV_HOST").is_some()
        || std::env::var_os("VITE_DEV_SERVER_URL").is_some()
}

fn init_dev_file_logger() -> Result<()> {
    let log_dir = resolve_log_dir()?;
    std::fs::create_dir_all(&log_dir).with_context(|| format!("create log dir {log_dir:?}"))?;

    let log_path = log_dir.join("tauri-dev-trace.log");
    let log_file =
        fern::log_file(&log_path).with_context(|| format!("open dev log file {log_path:?}"))?;

    let _ = DEV_LOG_PATH.set(log_path.clone());

    let trace_enabled = dev_trace_enabled();
    let base_level = if trace_enabled {
        log::LevelFilter::Trace
    } else {
        log::LevelFilter::Info
    };

    fern::Dispatch::new()
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
        .chain(std::io::stdout())
        .chain(log_file)
        .apply()
        .context("apply fern logger")?;

    log::info!(
        target: "devlog",
        "dev logging enabled at {} (trace={})",
        log_path.display(),
        trace_enabled
    );
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
