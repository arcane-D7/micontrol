//! Lifecycle markers for the application log.
//!
//! Finding **P4** of the 2026-11-07 audit: the log went silent for **30 h 44
//! min** with no shutdown marker at all — there was no way to tell a clean exit
//! from a crash, a `TerminateProcess` or a power loss.
//!
//! This module keeps a tiny `session.marker` file next to the log:
//!
//! ```text
//! state=running
//! started_at=2026-11-06T17:39:33.125Z
//! pid=4242
//! version=0.2.20
//! ```
//!
//! - [`begin_session`] runs at startup: it reports how the **previous**
//!   session ended and arms the marker for this one.
//! - [`mark_clean`] runs on the Tauri `RunEvent::Exit` path: it logs
//!   `app shutdown initiated (clean)` and rewrites the marker with
//!   `state=clean`.
//! - [`mark_panic`] runs from the panic hook: it records `state=panic` so the
//!   next start can say "the previous session panicked" instead of guessing.
//!
//! If the process is killed the marker is left at `state=running`, and the next
//! start emits:
//!
//! ```text
//! [WARN ] applog: previous session ended without shutdown marker — possible crash/kill
//! ```
//!
//! Writes are best effort: a failure to write the marker must never affect the
//! application. The marker lives in the same directory as the log and is
//! human-readable on purpose — no registry, no binary format, no new deps.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// File name of the marker inside the log directory.
pub const MARKER_FILE_NAME: &str = "session.marker";

/// Session states written to the marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    /// The process is (or was) running and has not recorded a shutdown.
    Running,
    /// The process logged `app shutdown initiated (clean)` and exited.
    Clean,
    /// The process panicked; the panic hook recorded it.
    Panic,
    /// A state written by a newer build / unknown value.
    Unknown(String),
}

impl SessionState {
    pub fn as_str(&self) -> &str {
        match self {
            SessionState::Running => "running",
            SessionState::Clean => "clean",
            SessionState::Panic => "panic",
            SessionState::Unknown(raw) => raw.as_str(),
        }
    }
}

/// Parsed contents of `session.marker`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerState {
    pub state: SessionState,
    pub started_at: Option<String>,
    pub clean_at: Option<String>,
    pub panic_at: Option<String>,
    pub pid: Option<u32>,
    pub version: Option<String>,
}

impl MarkerState {
    fn new(state: SessionState) -> Self {
        Self {
            state,
            started_at: None,
            clean_at: None,
            panic_at: None,
            pid: None,
            version: None,
        }
    }
}

/// How the previous session ended, as observed at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreviousSession {
    /// No marker at all (first run, install, or the log dir was wiped).
    None,
    /// The previous session recorded a clean shutdown.
    Clean { clean_at: Option<String> },
    /// `state=running` survived — the process never reached its shutdown path.
    Running { started_at: Option<String> },
    /// The panic hook fired and the marker was never upgraded to `clean`.
    Panic { at: Option<String> },
    /// Unknown state from a different build.
    Unknown { state: String },
}

impl PreviousSession {
    /// Human description used in the startup log line.
    pub fn describe(&self) -> String {
        match self {
            PreviousSession::None => "no previous session marker (first run or clean log dir)".into(),
            PreviousSession::Clean { clean_at } => match clean_at {
                Some(at) => format!("previous session ended cleanly at {at}"),
                None => "previous session ended cleanly".into(),
            },
            PreviousSession::Running { started_at } => match started_at {
                Some(at) => format!(
                    "previous session ended without shutdown marker — possible crash/kill (session started {at})"
                ),
                None => "previous session ended without shutdown marker — possible crash/kill".into(),
            },
            PreviousSession::Panic { at } => match at {
                Some(at) => format!("previous session ended in a panic at {at}"),
                None => "previous session ended in a panic".into(),
            },
            PreviousSession::Unknown { state } => {
                format!("previous session marker has unknown state '{state}'")
            }
        }
    }

    /// True when the previous session did not end through the clean path.
    pub fn is_suspicious(&self) -> bool {
        matches!(
            self,
            PreviousSession::Running { .. } | PreviousSession::Panic { .. }
        )
    }
}

/// Path of the marker inside `log_dir`.
pub fn marker_path(log_dir: &Path) -> PathBuf {
    log_dir.join(MARKER_FILE_NAME)
}

/// Current time as RFC 3339 with milliseconds (same format as the log lines).
pub fn now_iso() -> String {
    humantime::format_rfc3339_millis(std::time::SystemTime::now()).to_string()
}

/// Parse the marker text. Unknown keys and malformed lines are ignored so a
/// partially written file never breaks startup.
pub fn parse_marker(text: &str) -> MarkerState {
    let mut state = SessionState::Running;
    let mut parsed = MarkerState::new(SessionState::Running);
    let mut saw_state = false;

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "state" => {
                state = match value {
                    "running" => SessionState::Running,
                    "clean" => SessionState::Clean,
                    "panic" => SessionState::Panic,
                    other => SessionState::Unknown(other.to_string()),
                };
                saw_state = true;
            }
            "started_at" => parsed.started_at = Some(value.to_string()),
            "clean_at" => parsed.clean_at = Some(value.to_string()),
            "panic_at" => parsed.panic_at = Some(value.to_string()),
            "pid" => parsed.pid = value.parse().ok(),
            "version" => parsed.version = Some(value.to_string()),
            _ => {}
        }
    }

    parsed.state = if saw_state {
        state
    } else {
        // A marker without a state line is treated as "running": it exists, so
        // a session did start, and no clean shutdown was recorded.
        SessionState::Running
    };
    parsed
}

/// Read the marker, if it exists and is readable.
pub fn read(log_dir: &Path) -> Option<MarkerState> {
    let text = fs::read_to_string(marker_path(log_dir)).ok()?;
    Some(parse_marker(&text))
}

/// Classify the previous session from a marker state.
pub fn classify(previous: Option<MarkerState>) -> PreviousSession {
    match previous {
        None => PreviousSession::None,
        Some(marker) => match marker.state {
            SessionState::Running => PreviousSession::Running {
                started_at: marker.started_at,
            },
            SessionState::Clean => PreviousSession::Clean {
                clean_at: marker.clean_at,
            },
            SessionState::Panic => PreviousSession::Panic {
                at: marker.panic_at.or(marker.started_at),
            },
            SessionState::Unknown(state) => PreviousSession::Unknown { state },
        },
    }
}

/// Read the previous marker, write the new `running` marker for this session
/// and return how the previous session ended.
///
/// The write is atomic (temp file + rename) so a crash mid-write cannot leave a
/// truncated marker that the next start would misread.
pub fn begin_session(log_dir: &Path) -> PreviousSession {
    let previous = classify(read(log_dir));

    let mut marker = MarkerState::new(SessionState::Running);
    marker.started_at = Some(now_iso());
    marker.pid = Some(std::process::id());
    marker.version = Some(env!("CARGO_PKG_VERSION").to_string());
    let _ = write_marker(log_dir, &marker);

    previous
}

/// Record a clean shutdown: keeps `started_at`/`pid`, sets `clean_at`.
pub fn mark_clean(log_dir: &Path) -> io::Result<MarkerState> {
    let mut marker = read(log_dir).unwrap_or_else(|| MarkerState::new(SessionState::Clean));
    marker.state = SessionState::Clean;
    marker.clean_at = Some(now_iso());
    if marker.pid.is_none() {
        marker.pid = Some(std::process::id());
    }
    write_marker(log_dir, &marker)?;
    Ok(marker)
}

/// Called on the Tauri `RunEvent::Exit` path (every clean shutdown: tray
/// Quit, `app.exit(0)`, dev window close).
///
/// Logs `app shutdown initiated (clean)` exactly once per process and records
/// `state=clean` in the marker. Returns `false` when it was already recorded.
pub fn mark_clean_shutdown() -> bool {
    static RECORDED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if RECORDED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return false;
    }

    log::info!(target: "applog", "app shutdown initiated (clean)");

    match crate::debug_log::resolve_log_dir() {
        Ok(dir) => {
            if let Err(e) = mark_clean(&dir) {
                log::warn!(target: "applog", "could not write session marker: {e}");
            }
        }
        Err(e) => log::warn!(target: "applog", "could not resolve log dir for session marker: {e}"),
    }
    true
}

/// Called from the panic hook: persist `state=panic` so the next start can
/// report "previous session ended in a panic" instead of "possible crash/kill".
pub fn record_panic_from_hook() {
    if let Ok(dir) = crate::debug_log::resolve_log_dir() {
        let _ = mark_panic(&dir);
    }
}

/// Record a panic (called from the panic hook; never fails the caller).
pub fn mark_panic(log_dir: &Path) -> Option<MarkerState> {
    let mut marker = read(log_dir).unwrap_or_else(|| MarkerState::new(SessionState::Panic));
    marker.state = SessionState::Panic;
    marker.panic_at = Some(now_iso());
    if marker.started_at.is_none() {
        marker.started_at = Some(now_iso());
    }
    write_marker(log_dir, &marker).ok()?;
    Some(marker)
}

/// Serialise and atomically replace the marker file.
fn write_marker(log_dir: &Path, marker: &MarkerState) -> io::Result<()> {
    fs::create_dir_all(log_dir)?;
    let final_path = marker_path(log_dir);
    let tmp_path = log_dir.join(format!("{MARKER_FILE_NAME}.tmp"));

    let mut body = String::new();
    body.push_str("# MiControl session marker — written by the app, read at startup.\n");
    body.push_str(&format!("state={}\n", marker.state.as_str()));
    if let Some(v) = &marker.started_at {
        body.push_str(&format!("started_at={v}\n"));
    }
    if let Some(v) = &marker.clean_at {
        body.push_str(&format!("clean_at={v}\n"));
    }
    if let Some(v) = &marker.panic_at {
        body.push_str(&format!("panic_at={v}\n"));
    }
    if let Some(v) = marker.pid {
        body.push_str(&format!("pid={v}\n"));
    }
    if let Some(v) = &marker.version {
        body.push_str(&format!("version={v}\n"));
    }

    {
        let mut file = fs::File::create(&tmp_path)?;
        file.write_all(body.as_bytes())?;
        file.flush()?;
    }
    // `fs::rename` replaces an existing destination on Windows
    // (MOVEFILE_REPLACE_EXISTING), so this is a real atomic swap.
    fs::rename(&tmp_path, &final_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "micontrol-session-marker-{tag}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn first_run_has_no_marker_and_arms_running_state() {
        let dir = temp_dir("first");
        assert_eq!(begin_session(&dir), PreviousSession::None);
        let marker = read(&dir).expect("marker written");
        assert_eq!(marker.state, SessionState::Running);
        assert!(marker.started_at.is_some());
        assert_eq!(marker.pid, Some(std::process::id()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_running_marker_is_reported_as_possible_crash() {
        let dir = temp_dir("running");
        begin_session(&dir);
        // Simulate a kill: no mark_clean, just start again.
        let previous = begin_session(&dir);
        assert!(matches!(previous, PreviousSession::Running { .. }));
        assert!(previous.is_suspicious());
        let text = previous.describe();
        assert!(
            text.contains("ended without shutdown marker"),
            "startup warning must name the missing marker, got: {text}"
        );
        assert!(text.contains("possible crash/kill"), "got: {text}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn clean_shutdown_is_recorded_and_detected() {
        let dir = temp_dir("clean");
        begin_session(&dir);
        let marker = mark_clean(&dir).expect("clean marker");
        assert_eq!(marker.state, SessionState::Clean);
        assert!(marker.clean_at.is_some());
        assert!(marker.started_at.is_some(), "started_at is preserved");

        let previous = begin_session(&dir);
        assert!(matches!(previous, PreviousSession::Clean { .. }));
        assert!(!previous.is_suspicious());
        assert!(previous.describe().contains("ended cleanly"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn panic_marker_is_detected_on_next_start() {
        let dir = temp_dir("panic");
        begin_session(&dir);
        let marker = mark_panic(&dir).expect("panic marker");
        assert_eq!(marker.state, SessionState::Panic);
        assert!(marker.panic_at.is_some());

        let previous = begin_session(&dir);
        assert!(matches!(previous, PreviousSession::Panic { .. }));
        assert!(previous.is_suspicious());
        assert!(previous.describe().contains("panic"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn marker_without_state_counts_as_running() {
        let dir = temp_dir("nostate");
        fs::write(marker_path(&dir), "started_at=2026-01-01T00:00:00.000Z\n").unwrap();
        let previous = begin_session(&dir);
        assert!(matches!(previous, PreviousSession::Running { .. }));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_lines_are_ignored() {
        let parsed = parse_marker(
            "# comment\n\nnot a pair\nstate=clean\npid=abc\nclean_at=2026-11-06T22:19:40.517Z\n",
        );
        assert_eq!(parsed.state, SessionState::Clean);
        assert_eq!(parsed.pid, None, "unparseable pid is ignored");
        assert_eq!(parsed.clean_at.as_deref(), Some("2026-11-06T22:19:40.517Z"));
    }

    #[test]
    fn unknown_state_is_reported_but_not_suspicious() {
        let dir = temp_dir("unknown");
        fs::write(marker_path(&dir), "state=quarantined\n").unwrap();
        let previous = begin_session(&dir);
        assert!(matches!(previous, PreviousSession::Unknown { .. }));
        assert!(!previous.is_suspicious());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn marker_is_rewritten_atomically_without_leaving_temp_files() {
        let dir = temp_dir("atomic");
        begin_session(&dir);
        mark_clean(&dir).unwrap();
        assert!(!dir.join(format!("{MARKER_FILE_NAME}.tmp")).exists());
        let marker = read(&dir).unwrap();
        assert_eq!(marker.state, SessionState::Clean);
        let _ = fs::remove_dir_all(&dir);
    }
}
