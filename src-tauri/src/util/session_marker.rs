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
//! Installer-initiated shutdown
//! ────────────────────────────
//! An update or uninstall kills the app with `KillProcess`/`taskkill`, which
//! can never run the `RunEvent::Exit` handler — the marker stays at
//! `state=running` and the next start would report a phantom
//! "possible crash/kill" (audit 2026-11-08: the installer's KillAppProcess was
//! the source of those false alarms).
//!
//! The installer therefore *announces* the intentional kill first, in two
//! places:
//!
//! * `<log_dir>\session.marker` with `state=installer-shutdown` — correct when
//!   the user runs the installer themselves (UAC elevation keeps the profile);
//! * `%ProgramData%\MiControl\installer-shutdown.marker` — the machine-wide
//!   copy, because the bridge launches the installer as SYSTEM, where
//!   `%LOCALAPPDATA%` is the system profile and not the user's log dir.
//!
//! [`begin_session`] reads both. The machine-wide marker is only honoured when
//! it is **newer than the session marker** (i.e. it was written during the
//! session that just ended) and it is deleted afterwards; a leftover marker can
//! therefore never mask a real crash on a later start.
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

/// Machine-wide marker written by the NSIS installer / setup shell right
/// before it force-kills the app (see the module docs).
pub const INSTALLER_MARKER_FILE_NAME: &str = "installer-shutdown.marker";

/// Directory (under `%ProgramData%`) holding the machine-wide marker.
pub const INSTALLER_MARKER_DIR: &str = "MiControl";

/// Session states written to the marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    /// The process is (or was) running and has not recorded a shutdown.
    Running,
    /// The process logged `app shutdown initiated (clean)` and exited.
    Clean,
    /// The process panicked; the panic hook recorded it.
    Panic,
    /// The installer/uninstaller announced the kill before terminating us.
    /// Intentional termination — not a crash.
    InstallerShutdown,
    /// A state written by a newer build / unknown value.
    Unknown(String),
}

impl SessionState {
    pub fn as_str(&self) -> &str {
        match self {
            SessionState::Running => "running",
            SessionState::Clean => "clean",
            SessionState::Panic => "panic",
            SessionState::InstallerShutdown => "installer-shutdown",
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
    /// Timestamp of an installer-announced shutdown.
    pub at: Option<String>,
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
            at: None,
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
    /// The installer/uninstaller announced the kill before terminating the app.
    InstallerShutdown { at: Option<String> },
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
            PreviousSession::InstallerShutdown { at } => match at {
                Some(at) => format!(
                    "previous session was terminated by the installer at {at} (intentional, not a crash)"
                ),
                None => {
                    "previous session was terminated by the installer (intentional, not a crash)"
                        .into()
                }
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

/// Path of the machine-wide installer announcement
/// (`%ProgramData%\MiControl\installer-shutdown.marker`).
///
/// Falls back to `C:\ProgramData` when the environment variable is absent, so
/// the path is always resolvable in unit tests and in stripped environments.
pub fn installer_marker_path() -> PathBuf {
    let base = std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
    base.join(INSTALLER_MARKER_DIR)
        .join(INSTALLER_MARKER_FILE_NAME)
}

/// A shutdown announced by the installer, as read at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallerAnnouncement {
    /// Timestamp written by the installer (`at=`), when present.
    pub at: Option<String>,
    /// True when the announcement was written during the session that just
    /// ended (its mtime is at or after the session marker's mtime).
    pub during_last_session: bool,
}

/// Read the machine-wide installer announcement, if it exists.
///
/// `log_dir` is the directory holding `session.marker`; its mtime is the start
/// of the last session. The announcement is only considered to belong to that
/// session when it was written **after** the session started — a leftover
/// marker from an older install can never mask a real crash.
pub fn read_installer_announcement(
    log_dir: &Path,
    installer_marker: &Path,
) -> Option<InstallerAnnouncement> {
    let text = fs::read_to_string(installer_marker).ok()?;
    let parsed = parse_marker(&text);
    if parsed.state != SessionState::InstallerShutdown {
        return None;
    }
    let marker_mtime = fs::metadata(installer_marker)
        .and_then(|m| m.modified())
        .ok();
    let session_mtime = fs::metadata(marker_path(log_dir))
        .and_then(|m| m.modified())
        .ok();
    let during_last_session = match (marker_mtime, session_mtime) {
        (Some(announced), Some(started)) => announced >= started,
        // Without a session marker there is no session to excuse: a stale
        // machine-wide marker must never turn a missing marker into "clean".
        _ => false,
    };
    Some(InstallerAnnouncement {
        at: parsed.at,
        during_last_session,
    })
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
                    "installer-shutdown" => SessionState::InstallerShutdown,
                    other => SessionState::Unknown(other.to_string()),
                };
                saw_state = true;
            }
            "started_at" => parsed.started_at = Some(value.to_string()),
            "clean_at" => parsed.clean_at = Some(value.to_string()),
            "panic_at" => parsed.panic_at = Some(value.to_string()),
            "at" => parsed.at = Some(value.to_string()),
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
            SessionState::InstallerShutdown => PreviousSession::InstallerShutdown {
                at: marker.at.or(marker.clean_at),
            },
            SessionState::Unknown(state) => PreviousSession::Unknown { state },
        },
    }
}

/// Read the previous marker, write the new `running` marker for this session
/// and return how the previous session ended.
///
/// Consults the machine-wide installer announcement as well: when the previous
/// marker says `running` (i.e. no clean shutdown was recorded) but the
/// installer wrote `installer-shutdown.marker` **during** that session, the
/// session ended intentionally and is reported as
/// [`PreviousSession::InstallerShutdown`] instead of "possible crash/kill".
///
/// The write is atomic (temp file + rename) so a crash mid-write cannot leave a
/// truncated marker that the next start would misread.
pub fn begin_session(log_dir: &Path) -> PreviousSession {
    begin_session_with(log_dir, &installer_marker_path())
}

/// [`begin_session`] with an explicit installer-announcement path (unit tests
/// use a temp file instead of the real `%ProgramData%` one).
fn begin_session_with(log_dir: &Path, installer_marker: &Path) -> PreviousSession {
    let mut previous = classify(read(log_dir));

    // Read (and consume) the machine-wide announcement written by the NSIS
    // installer / setup shell. It is only valid for the session that just
    // ended, which `read_installer_announcement` decides by mtime.
    let announcement = read_installer_announcement(log_dir, installer_marker);
    if announcement.is_some() {
        // Consumed: even if the delete fails (ACL), the mtime rule keeps it
        // from excusing any later session.
        let _ = fs::remove_file(installer_marker);
    }
    if matches!(previous, PreviousSession::Running { .. }) {
        if let Some(a) = announcement.as_ref().filter(|a| a.during_last_session) {
            previous = PreviousSession::InstallerShutdown { at: a.at.clone() };
        }
    }

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
    if let Some(v) = &marker.at {
        body.push_str(&format!("at={v}\n"));
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

    /// Installer-announcement path inside the temp dir: the real
    /// `%ProgramData%` marker must never leak into a test (and a test must
    /// never delete the machine-wide marker of the developer's install).
    fn announcement(dir: &Path) -> PathBuf {
        dir.join(INSTALLER_MARKER_FILE_NAME)
    }

    fn begin(dir: &Path) -> PreviousSession {
        begin_session_with(dir, &announcement(dir))
    }

    #[test]
    fn first_run_has_no_marker_and_arms_running_state() {
        let dir = temp_dir("first");
        assert_eq!(begin(&dir), PreviousSession::None);
        let marker = read(&dir).expect("marker written");
        assert_eq!(marker.state, SessionState::Running);
        assert!(marker.started_at.is_some());
        assert_eq!(marker.pid, Some(std::process::id()));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_running_marker_is_reported_as_possible_crash() {
        let dir = temp_dir("running");
        begin(&dir);
        // Simulate a kill: no mark_clean, just start again.
        let previous = begin(&dir);
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
        begin(&dir);
        let marker = mark_clean(&dir).expect("clean marker");
        assert_eq!(marker.state, SessionState::Clean);
        assert!(marker.clean_at.is_some());
        assert!(marker.started_at.is_some(), "started_at is preserved");

        let previous = begin(&dir);
        assert!(matches!(previous, PreviousSession::Clean { .. }));
        assert!(!previous.is_suspicious());
        assert!(previous.describe().contains("ended cleanly"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn panic_marker_is_detected_on_next_start() {
        let dir = temp_dir("panic");
        begin(&dir);
        let marker = mark_panic(&dir).expect("panic marker");
        assert_eq!(marker.state, SessionState::Panic);
        assert!(marker.panic_at.is_some());

        let previous = begin(&dir);
        assert!(matches!(previous, PreviousSession::Panic { .. }));
        assert!(previous.is_suspicious());
        assert!(previous.describe().contains("panic"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn marker_without_state_counts_as_running() {
        let dir = temp_dir("nostate");
        fs::write(marker_path(&dir), "started_at=2026-01-01T00:00:00.000Z\n").unwrap();
        let previous = begin(&dir);
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
        let previous = begin(&dir);
        assert!(matches!(previous, PreviousSession::Unknown { .. }));
        assert!(!previous.is_suspicious());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn marker_is_rewritten_atomically_without_leaving_temp_files() {
        let dir = temp_dir("atomic");
        begin(&dir);
        mark_clean(&dir).unwrap();
        assert!(!dir.join(format!("{MARKER_FILE_NAME}.tmp")).exists());
        let marker = read(&dir).unwrap();
        assert_eq!(marker.state, SessionState::Clean);
        let _ = fs::remove_dir_all(&dir);
    }

    // ── installer-announced shutdown ────────────────────────────────────────

    #[test]
    fn installer_shutdown_state_is_parsed_and_is_not_a_crash() {
        let parsed = parse_marker("state=installer-shutdown\nat=2026-11-08T15:00:00.000Z\n");
        assert_eq!(parsed.state, SessionState::InstallerShutdown);
        assert_eq!(parsed.at.as_deref(), Some("2026-11-08T15:00:00.000Z"));

        let previous = classify(Some(parsed));
        assert!(matches!(
            previous,
            PreviousSession::InstallerShutdown { .. }
        ));
        assert!(
            !previous.is_suspicious(),
            "an installer kill is intentional, not a crash"
        );
        let text = previous.describe();
        assert!(text.contains("terminated by the installer"), "got: {text}");
        assert!(text.contains("not a crash"), "got: {text}");
        assert!(
            !text.contains("possible crash/kill"),
            "the installer announcement must not produce the crash warning: {text}"
        );
    }

    #[test]
    fn installer_marker_written_into_the_log_dir_is_honoured() {
        // The installer also writes the marker straight into the log dir
        // (interactive install, UAC keeps the user's %LOCALAPPDATA%).
        let dir = temp_dir("instlocal");
        begin(&dir);
        fs::write(marker_path(&dir), "state=installer-shutdown\n").unwrap();
        let previous = begin(&dir);
        assert!(matches!(
            previous,
            PreviousSession::InstallerShutdown { .. }
        ));
        assert!(!previous.is_suspicious());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn machine_wide_marker_rescues_a_running_session_and_is_consumed() {
        // The bridge launches the installer as SYSTEM, so the marker lands in
        // %ProgramData% instead of the user's log dir.
        let dir = temp_dir("instglobal");
        begin(&dir);
        // begin() rewrote session.marker now; write the announcement after it.
        let marker = announcement(&dir);
        fs::write(&marker, "state=installer-shutdown\n").unwrap();

        let previous = begin(&dir);
        assert!(matches!(
            previous,
            PreviousSession::InstallerShutdown { .. }
        ));
        assert!(
            !marker.exists(),
            "the announcement must be consumed on read"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_machine_wide_marker_does_not_mask_a_crash() {
        // A marker left over from an older install (older than the session
        // start) must not turn a genuine crash into "installer shutdown".
        let dir = temp_dir("inststale");
        let marker = announcement(&dir);
        fs::write(&marker, "state=installer-shutdown\n").unwrap();
        // Make the announcement clearly older than the session marker that
        // begin() is about to read: write the session marker last.
        std::thread::sleep(std::time::Duration::from_millis(30));
        begin(&dir); // arms session.marker (mtime now > marker mtime)

        let previous = begin(&dir);
        assert!(
            matches!(previous, PreviousSession::Running { .. }),
            "stale announcement must be ignored, got {previous:?}"
        );
        assert!(previous.is_suspicious());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn announcement_without_a_session_marker_is_not_trusted() {
        let dir = temp_dir("instnosession");
        let marker = announcement(&dir);
        fs::write(&marker, "state=installer-shutdown\n").unwrap();
        // No session.marker at all: there is no session to excuse.
        let a = read_installer_announcement(&dir, &marker).expect("marker readable");
        assert!(!a.during_last_session);
        assert_eq!(begin(&dir), PreviousSession::None);
        let _ = fs::remove_dir_all(&dir);
    }
}
