//! Logging initialisation for development and production.
//!
//! Sets up `fern` logging to stdout + a persistent rolling file in
//! `%LOCALAPPDATA%\MiControl\logs` (both dev and the installed app).
//!
//! ## One file per process family (P5)
//!
//! The 2026-11-07 audit found 15 physically corrupted lines in `tauri-app.log`:
//! the desktop app and the 3 728 short-lived elevated helpers were appending to
//! the **same** file from different processes, and their writes interleaved
//! (`…00.115Z2026-11-06T22:21:04.592Z [WARN ]` — two records on one line).
//!
//! The helper now logs to its own file, with its own rotation:
//!
//! | process                     | file                       |
//! |-----------------------------|----------------------------|
//! | `micontrol.exe`             | `tauri-app.log`            |
//! | `micontrol.exe --elevated`  | `tauri-app-helper.log`     |
//! | `cargo tauri dev`           | `tauri-dev-trace.log`      |
//! | dev helper                  | `tauri-dev-trace-helper.log` |
//!
//! Everything else (the bridge and watchdog logs under `%PROGRAMDATA%`) is
//! already process-private. Records are written with a single `write_all` on an
//! append handle — see `util::log_rotation` for the whole-line guarantee.
//!
//! ## Repeat suppression
//!
//! `WARN`/`ERROR` records with the same signature inside 60 s are suppressed
//! and reported as `×N repetições suprimidas` (see `util::log_ratelimit`).
//! Lifecycle markers and `PANIC:` lines are never rate-limited.
//!
//! In every mode the log file is persistent so errors, warnings and
//! lifecycle events survive restarts and can be shipped with a bug report.
//! If the log file cannot be created (e.g. ACL after an elevated run) we
//! fall back to console-only logging rather than failing the app.

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::SystemTime;

/// Release log of the desktop app.
pub const APP_LOG_FILE: &str = "tauri-app.log";
/// Release log of the elevated helper (separate file since the P5 fix).
pub const HELPER_LOG_FILE: &str = "tauri-app-helper.log";
/// Dev log of the desktop app.
pub const DEV_LOG_FILE: &str = "tauri-dev-trace.log";
/// Dev log of the elevated helper.
pub const DEV_HELPER_LOG_FILE: &str = "tauri-dev-trace-helper.log";

/// Path of the file-backed log (set on first successful init), used by
/// `log_file_path()` so the frontend can display / export it.
static DEV_LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Whether the current log is file-backed (used to decide file-level verbosity).
static DEV_FILE_ACTIVE: OnceLock<bool> = OnceLock::new();

/// True when this process was launched as the privileged helper
/// (`micontrol.exe --elevated`, dispatched by the scheduled task).
///
/// The helper is a **different process** that runs for a few hundred
/// milliseconds and owns its own log file; it must never touch the app log or
/// the app session marker.
pub fn is_elevated_helper_process() -> bool {
    std::env::args_os().any(|arg| arg == "--elevated")
}

/// File name for this process in the given mode.
fn log_file_name(dev: bool) -> &'static str {
    log_file_name_for(dev, is_elevated_helper_process())
}

/// Pure mapping used by [`log_file_name`] (separated so the helper/app split
/// is unit-testable without re-spawning the process with `--elevated`).
fn log_file_name_for(dev: bool, helper: bool) -> &'static str {
    match (dev, helper) {
        (true, false) => DEV_LOG_FILE,
        (true, true) => DEV_HELPER_LOG_FILE,
        (false, false) => APP_LOG_FILE,
        (false, true) => HELPER_LOG_FILE,
    }
}

/// Log directory: `%LOCALAPPDATA%\MiControl\logs` (fallback: `<exe dir>\logs`).
pub fn resolve_log_dir() -> Result<PathBuf> {
    if let Some(local_appdata) = std::env::var_os("LOCALAPPDATA") {
        return Ok(PathBuf::from(local_appdata).join("MiControl").join("logs"));
    }

    let exe = std::env::current_exe().context("current_exe for log dir")?;
    let parent = exe
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Cannot derive parent directory for log path"))?;
    Ok(parent.join("logs"))
}

/// Path of the file-backed log after `init_logging()` succeeded
/// (dev: `tauri-dev-trace*.log`, release: `tauri-app*.log`).
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

pub fn init_logging() -> Result<()> {
    let dev = is_tauri_dev();
    let result = if dev {
        init_dev_file_logger()
    } else {
        init_release_file_logger()
    };

    // Lifecycle marker: report how the previous session ended. Only the app
    // owns the marker — a helper process must not overwrite the app's session.
    //
    // `cfg(not(test))`: the crate's unit tests call `init_logging()`, and they
    // must not overwrite the developer's real `session.marker` (that would make
    // the next real start report a phantom "possible crash/kill").
    #[cfg(not(test))]
    if result.is_ok() && !is_elevated_helper_process() {
        begin_app_session();
    }
    result
}

/// Read the previous session marker, log the verdict and arm the new marker.
///
/// Only compiled outside test builds: the unit tests call `init_logging()` and
/// must not overwrite the real `session.marker` of whoever runs the suite.
#[cfg(not(test))]
fn begin_app_session() {
    let Ok(log_dir) = resolve_log_dir() else {
        return;
    };
    let previous = crate::util::session_marker::begin_session(&log_dir);
    if previous.is_suspicious() {
        log::warn!(target: "applog", "{}", previous.describe());
    } else {
        log::info!(target: "applog", "{}", previous.describe());
    }
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

    let log_path = log_dir.join(log_file_name(true));
    // S64: same size-based rotation as the release logger — the trace log was
    // the file called out by `docs/stability-report-2026-06-24.md` as growing
    // without bound (it reached 39 MB in one dev cycle).
    let log_file = match open_rotating_log(&log_path) {
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
    let _ = DEV_FILE_ACTIVE.set(log_file.is_some());

    let trace_enabled = dev_trace_enabled();
    let base_level = if trace_enabled {
        log::LevelFilter::Trace
    } else {
        log::LevelFilter::Info
    };

    let mut dispatch = base_dispatch(base_level, true);
    if let Some(file) = log_file {
        dispatch = dispatch.chain(fern::Output::writer(file, ""));
    }

    dispatch.apply().context("apply fern logger")?;

    log::info!(
        target: "devlog",
        "dev logging enabled at {} (trace={}, role={}, rotation: {} MiB x {} files)",
        log_path.display(),
        trace_enabled,
        process_role(),
        crate::util::log_rotation::DEFAULT_MAX_BYTES / (1024 * 1024),
        crate::util::log_rotation::DEFAULT_MAX_FILES
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

    let log_path = log_dir.join(log_file_name(false));
    // S64: the log used to be a plain unbounded append file (77 MB in one
    // release cycle). `docs/stability-report-2026-06-24.md` lists the missing
    // rotation as a MEDIUM finding. Rotate at 8 MiB keeping 7 files (raised
    // from 3 on 2026-11-07, see `docs/LOGGING.md`), so the live log is always
    // small enough to ship with a bug report and the rotated set covers days.
    let log_file = match open_rotating_log(&log_path) {
        Ok(f) => Some(f),
        Err(e) => {
            // The file cannot be created — the dir may exist with a broken
            // ACL (e.g. created long ago by an elevated instance). Repair the
            // ACL if possible, then retry once.
            let _ = repair_log_dir_acl(&log_dir);
            match open_rotating_log(&log_path) {
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
    let _ = DEV_FILE_ACTIVE.set(log_file.is_some());

    let mut dispatch = base_dispatch(log::LevelFilter::Info, false);
    if let Some(file) = log_file {
        dispatch = dispatch.chain(fern::Output::writer(file, ""));
    }

    dispatch.apply().context("apply fern logger")?;

    log::info!(
        target: "applog",
        "persistent app logging enabled at {} (role={}, rotation: {} MiB x {} files)",
        log_path.display(),
        process_role(),
        crate::util::log_rotation::DEFAULT_MAX_BYTES / (1024 * 1024),
        crate::util::log_rotation::DEFAULT_MAX_FILES
    );
    Ok(())
}

fn process_role() -> &'static str {
    if is_elevated_helper_process() {
        "elevated-helper"
    } else {
        "app"
    }
}

/// Build the shared dispatch: level filters + record formatter + stdout.
///
/// **Outputs are chained with an empty line separator on purpose.** The
/// formatter writes the trailing `\n` itself, which lets a suppressed record
/// `finish("")` without leaving a blank line in the file (fern always writes
/// `formatted + line_sep`).
fn base_dispatch(level: log::LevelFilter, with_thread_name: bool) -> fern::Dispatch {
    let mut dispatch = fern::Dispatch::new()
        .level(level)
        .level_for("hyper", log::LevelFilter::Info)
        .level_for("mio", log::LevelFilter::Info)
        .level_for("want", log::LevelFilter::Info)
        .format(move |out, message, record| emit_record(out, message, record, with_thread_name));

    // Chain stdout BEFORE the caller appends the file sink.
    dispatch = dispatch.chain(fern::Output::stdout(""));
    dispatch
}

/// Format (and possibly suppress) one record.
///
/// * `WARN` / `ERROR` go through the repeat limiter (60 s window, 64
///   signatures). A suppressed record writes nothing; a summary line is
///   emitted instead when the window rolls over.
/// * Everything else is formatted as
///   `2026-11-07T10:00:00.000Z [INFO ] target: message` (plus the thread name
///   in dev), terminated by `\n`.
fn emit_record(
    out: fern::FormatCallback<'_>,
    message: &std::fmt::Arguments<'_>,
    record: &log::Record<'_>,
    with_thread_name: bool,
) {
    emit_record_with(
        out,
        message,
        record,
        with_thread_name,
        crate::util::log_ratelimit::global(),
    )
}

/// Same as [`emit_record`] but against an explicit limiter.
///
/// Production always passes `log_ratelimit::global()`; tests pass a private
/// limiter so their assertions are not disturbed by records logged from other
/// tests running in parallel in the same process.
fn emit_record_with(
    out: fern::FormatCallback<'_>,
    message: &std::fmt::Arguments<'_>,
    record: &log::Record<'_>,
    with_thread_name: bool,
    limiter: &std::sync::Mutex<crate::util::log_ratelimit::RateLimiter>,
) {
    let level = record.level();
    let body = message.to_string();

    // ── Repeat suppression (WARN/ERROR only) ────────────────────────────
    let mut suppressed_prefix = String::new();
    if matches!(level, log::Level::Warn | log::Level::Error) {
        use crate::util::log_ratelimit::Decision;

        let signature = crate::util::log_ratelimit::signature(level as u8, record.target(), &body);
        // `try_lock` — never block or deadlock (the panic hook logs too).
        // If the limiter is busy the record is emitted unfiltered.
        let decision = match limiter.try_lock() {
            Ok(mut guard) => guard.check(signature, now_ms()),
            Err(_) => Decision::Emit { suppressed: 0 },
        };

        match decision {
            Decision::Suppress { summary } => {
                match summary {
                    Some(count) => out.finish(format_args!("{}\n", suppressed_summary(count))),
                    // Empty payload + empty line separator → zero bytes.
                    None => out.finish(format_args!("")),
                }
                return;
            }
            Decision::Emit { suppressed } => {
                if suppressed > 0 {
                    suppressed_prefix = format!("{}\n", suppressed_summary(suppressed));
                }
            }
        }
    }

    let ts = humantime::format_rfc3339_millis(SystemTime::now());
    let target = record.target();
    if with_thread_name {
        let thread = std::thread::current();
        let thread_name = thread.name().unwrap_or("unnamed");
        out.finish(format_args!(
            "{suppressed_prefix}{ts} [{level:<5}] [{thread_name}] {target}: {body}\n"
        ));
    } else {
        out.finish(format_args!(
            "{suppressed_prefix}{ts} [{level:<5}] {target}: {body}\n"
        ));
    }
}

/// `2026-11-07T10:00:00.000Z [INFO ] applog: ×12 repetições suprimidas (janela 60s)`
fn suppressed_summary(count: u32) -> String {
    let ts = humantime::format_rfc3339_millis(SystemTime::now());
    format!(
        "{ts} [INFO ] applog: ×{count} repetições suprimidas (mesma assinatura em {}s)",
        crate::util::log_ratelimit::DEFAULT_WINDOW_MS / 1000
    )
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Open the persistent app log with size-based rotation (S64).
///
/// Returns a boxed writer so the `fern` chain keeps working unchanged while
/// the file is rotated before it can grow past the cap. `fern::Output` only
/// accepts `File`, `Stderr`/`Stdout` or `Box<dyn Write + Send>`, hence the box.
fn open_rotating_log(path: &std::path::Path) -> std::io::Result<Box<dyn std::io::Write + Send>> {
    use crate::util::log_rotation::{RotatingFile, DEFAULT_MAX_BYTES, DEFAULT_MAX_FILES};
    // Touch the file so a permission problem surfaces here (and can trigger the
    // ACL repair path) rather than on the first log record.
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    Ok(Box::new(RotatingFile::new(
        path.to_path_buf(),
        DEFAULT_MAX_BYTES,
        DEFAULT_MAX_FILES,
    )))
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
    use crate::util::process_spawn;
    use std::process::Command;
    use std::time::Duration;
    let user = std::env::var("USERNAME").unwrap_or_else(|_| "Users".into());
    // icacls "dir" /grant "<user>:(OI)(CI)F" /T
    let mut cmd = Command::new("icacls");
    cmd.arg(log_dir)
        .arg("/grant")
        .arg(format!("{user}:(OI)(CI)F"))
        .arg("/T")
        .arg("/Q");
    process_spawn::run_capture(&mut cmd, Duration::from_secs(10))
        .is_ok_and(|out| out.status.success())
}

/// Console-only logger used when the dev log file cannot be opened
/// (e.g. ACL denies access after an elevated run). Ensures the app still
/// starts and logs to stdout in Tauri dev mode.
fn init_console_only_logger() -> Result<()> {
    fern::Dispatch::new()
        .level(log::LevelFilter::Info)
        .format(|out, message, record| emit_record(out, message, record, false))
        .chain(fern::Output::stdout(""))
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

#[cfg(test)]
mod tests {
    use super::*;
    use log::Log;
    use std::sync::{Arc, Mutex};

    /// In-memory sink that lets a test read exactly what the formatter wrote.
    #[derive(Clone)]
    struct SharedSink(Arc<Mutex<Vec<u8>>>);

    impl SharedSink {
        fn new() -> Self {
            Self(Arc::new(Mutex::new(Vec::new())))
        }

        fn text(&self) -> String {
            String::from_utf8(self.0.lock().unwrap().clone()).expect("utf-8 log")
        }
    }

    impl std::io::Write for SharedSink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Private limiter per test (leaked: a handful of bytes). The production
    /// limiter is process-global, and `emit_record` intentionally bypasses it
    /// when it is busy — which is correct for a logger but would make exact
    /// assertions racy while the test binary runs its 500+ tests in parallel.
    fn test_limiter() -> &'static Mutex<crate::util::log_ratelimit::RateLimiter> {
        Box::leak(Box::new(Mutex::new(
            crate::util::log_ratelimit::RateLimiter::with_defaults(),
        )))
    }

    fn logger_to(
        sink: &SharedSink,
        limiter: &'static Mutex<crate::util::log_ratelimit::RateLimiter>,
    ) -> Box<dyn Log> {
        let (_, logger) = fern::Dispatch::new()
            .level(log::LevelFilter::Info)
            .format(move |out, message, record| {
                emit_record_with(out, message, record, false, limiter)
            })
            .chain(fern::Output::writer(Box::new(sink.clone()), ""))
            .into_log();
        logger
    }

    fn log_record(logger: &dyn Log, level: log::Level, target: &str, message: &str) {
        logger.log(
            &log::Record::builder()
                .args(format_args!("{message}"))
                .level(level)
                .target(target)
                .build(),
        );
    }

    /// End-to-end formatter check: the first `WARN` is written, an identical
    /// repeat (and a repeat that differs only in digits) is dropped, and other
    /// messages/levels still go through.
    #[test]
    fn formatter_suppresses_identical_warns_only() {
        let sink = SharedSink::new();
        let logger = logger_to(&sink, test_limiter());

        // Unique text per test: the limiter is process-global.
        log_record(
            logger.as_ref(),
            log::Level::Warn,
            "hw::battery",
            "fmt-test battery probe 0x80041002",
        );
        log_record(
            logger.as_ref(),
            log::Level::Warn,
            "hw::battery",
            "fmt-test battery probe 0x80041002",
        );
        log_record(
            logger.as_ref(),
            log::Level::Warn,
            "hw::battery",
            "fmt-test battery probe 0x80041003",
        );

        let text = sink.text();
        assert_eq!(
            text.lines().count(),
            1,
            "two repeats must be suppressed, got:\n{text}"
        );
        assert!(text.contains("[WARN ] hw::battery: fmt-test battery probe"));
        assert!(text.ends_with('\n'), "records are newline-terminated");

        // A different message and a different level are never conflated.
        log_record(
            logger.as_ref(),
            log::Level::Warn,
            "hw::battery",
            "fmt-test fan probe failed",
        );
        log_record(
            logger.as_ref(),
            log::Level::Error,
            "hw::battery",
            "fmt-test battery probe 0x80041002",
        );
        let text = sink.text();
        assert_eq!(text.lines().count(), 3, "got:\n{text}");
        assert!(text.contains("[ERROR] hw::battery"));
    }

    /// `INFO` is never deduplicated (the audit's probe spam is ERROR/WARN, and
    /// INFO carries the lifecycle narrative).
    #[test]
    fn formatter_never_suppresses_info() {
        let sink = SharedSink::new();
        let logger = logger_to(&sink, test_limiter());
        for _ in 0..3 {
            log_record(
                logger.as_ref(),
                log::Level::Info,
                "applog",
                "fmt-test info repeat",
            );
        }
        let text = sink.text();
        assert_eq!(text.lines().count(), 3, "got:\n{text}");
    }

    /// The suppression mechanism relies on `finish("")` + an empty line
    /// separator producing **zero bytes**. If the separator ever becomes "\n"
    /// again, every suppressed record would leave a blank line in the log and
    /// this test fails with the reason.
    #[test]
    fn empty_finish_with_empty_separator_writes_nothing() {
        let sink = SharedSink::new();
        let (_, logger) = fern::Dispatch::new()
            .format(|out, _message, _record| out.finish(format_args!("")))
            .chain(fern::Output::writer(Box::new(sink.clone()), ""))
            .into_log();

        log_record(logger.as_ref(), log::Level::Warn, "applog", "suppressed");
        assert!(
            sink.text().is_empty(),
            "a suppressed record must not write even a newline, got: {:?}",
            sink.text()
        );
    }

    /// Same as above but through the production dispatch: proves that a real
    /// suppression does not leave a blank physical line.
    #[test]
    fn suppressed_record_leaves_no_blank_line() {
        let sink = SharedSink::new();
        let logger = logger_to(&sink, test_limiter());
        log_record(
            logger.as_ref(),
            log::Level::Warn,
            "applog",
            "fmt-test blank line probe",
        );
        log_record(
            logger.as_ref(),
            log::Level::Warn,
            "applog",
            "fmt-test blank line probe",
        );
        let text = sink.text();
        assert_eq!(text.lines().count(), 1, "got: {text:?}");
        assert!(
            !text.contains("\n\n"),
            "no blank line may be written: {text:?}"
        );
    }

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

    /// The names are part of the fix: the helper must never resolve to the
    /// app log. This test pins them because `docs/LOGGING.md` documents them.
    #[test]
    fn log_file_names_are_distinct_per_role() {
        assert_eq!(log_file_name_for(false, false), "tauri-app.log");
        assert_eq!(log_file_name_for(false, true), "tauri-app-helper.log");
        assert_eq!(log_file_name_for(true, false), "tauri-dev-trace.log");
        assert_eq!(log_file_name_for(true, true), "tauri-dev-trace-helper.log");
        // The whole point of the P5 fix: the helper never resolves to the app
        // log, in either mode.
        assert_ne!(
            log_file_name_for(false, true),
            log_file_name_for(false, false)
        );
        assert_ne!(
            log_file_name_for(true, true),
            log_file_name_for(true, false)
        );
    }

    #[test]
    fn suppressed_summary_carries_the_marker_and_count() {
        let line = suppressed_summary(37);
        assert!(line.contains("×37 repetições suprimidas"), "got: {line}");
        assert!(line.contains("[INFO ] applog:"), "got: {line}");
        assert!(line.starts_with("20"), "timestamp first, got: {line}");
    }
}
