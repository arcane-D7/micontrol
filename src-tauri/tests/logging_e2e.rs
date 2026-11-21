//! End-to-end check of the persistent log pipeline.
//!
//! Everything here runs against a **temporary `LOCALAPPDATA`**, so the test
//! never touches the developer's or the user's real `%LOCALAPPDATA%\MiControl\
//! logs` directory.
//!
//! What it proves (the acceptance criteria of the 2026-11-07 logging fix):
//!
//! 1. `init_logging()` creates a file-backed log with the documented rotation
//!    banner (8 MiB × 7).
//! 2. Repeated `WARN`s are suppressed: three identical warnings produce one
//!    physical line.
//! 3. Every physical line is one complete record: timestamp first, one level
//!    marker per line, no blank lines, no two timestamps glued together (P5).
//! 4. A clean shutdown writes `app shutdown initiated (clean)` and flips
//!    `session.marker` to `state=clean`, so the next start reports "ended
//!    cleanly" instead of "possible crash/kill" (P4).
//! 5. Duration fields never print an epoch (P6) — covered by unit tests, the
//!    tray log lines use `log_fields::duration_field`.

use std::path::PathBuf;

fn temp_local_appdata(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "micontrol-logging-e2e-{tag}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp root");
    dir
}

#[test]
fn logging_pipeline_produces_clean_readable_files() {
    let root = temp_local_appdata("main");
    std::env::set_var("LOCALAPPDATA", &root);

    micontrol_lib::debug_log::init_logging().expect("init logging with a writable temp dir");

    let log_path = micontrol_lib::debug_log::log_file_path()
        .expect("file-backed logging must be active in the test build")
        .to_path_buf();
    let expected_dir = root.join("MiControl").join("logs");
    assert_eq!(
        log_path.parent(),
        Some(expected_dir.as_path()),
        "log must live under %LOCALAPPDATA%\\MiControl\\logs"
    );
    assert_eq!(
        log_path.file_name().and_then(|n| n.to_str()),
        Some("tauri-dev-trace.log"),
        "debug builds use the dev trace log (and the helper has its own file)"
    );

    log::info!("e2e: startup line");
    for _ in 0..3 {
        log::warn!("e2e: identical warning 0x80041002");
    }
    log::error!("e2e: distinct error");
    log::info!("e2e: after the warning burst");

    // Clean shutdown: log marker + `session.marker` state.
    assert!(
        micontrol_lib::util::session_marker::mark_clean_shutdown(),
        "the first call records the clean shutdown"
    );
    assert!(
        !micontrol_lib::util::session_marker::mark_clean_shutdown(),
        "the clean shutdown marker is idempotent"
    );

    let text = std::fs::read_to_string(&log_path).expect("log file must be readable");
    let lines: Vec<&str> = text.lines().collect();
    assert!(!lines.is_empty(), "log file is empty: {text:?}");

    // 1. Banner documents the shipping rotation.
    assert!(
        text.contains("dev logging enabled at"),
        "startup banner missing: {text}"
    );
    assert!(
        text.contains("8 MiB x 7 files"),
        "banner must document the 8 MiB x 7 retention: {text}"
    );

    // 2. Repeated WARN suppressed to a single line.
    let identical = lines
        .iter()
        .filter(|line| line.contains("e2e: identical warning"))
        .count();
    assert_eq!(
        identical, 1,
        "three identical warnings must yield one line, got {identical}:\n{text}"
    );

    // 3. One record per physical line.
    for line in &lines {
        assert!(!line.trim().is_empty(), "blank line in the log:\n{text}");
        let bytes = line.as_bytes();
        assert!(
            bytes.len() > 24
                && bytes[..2].iter().all(u8::is_ascii_digit)
                && bytes[4] == b'-'
                && bytes[10] == b'T'
                && bytes[23] == b'Z',
            "every record starts with an RFC 3339 timestamp, got: {line}"
        );
        let level_markers = ["[ERROR]", "[WARN ]", "[INFO ]", "[DEBUG]", "[TRACE]"]
            .iter()
            .filter(|marker| line.contains(*marker))
            .count();
        assert_eq!(level_markers, 1, "torn/merged line: {line}");
        // The exact P5 shape: a second record appended right after the first
        // timestamp, without a newline in between.
        let rest = &line[24..];
        assert!(
            !rest.contains("Z20"),
            "two timestamps glued on one line (P5 regression): {line}"
        );
        assert!(
            !rest.starts_with(|c: char| c.is_ascii_digit()),
            "second record starts immediately after the timestamp (P5): {line}"
        );
    }

    // 4. Lifecycle marker present in the log and persisted.
    assert!(
        text.contains("app shutdown initiated (clean)"),
        "clean shutdown marker missing: {text}"
    );
    let marker = micontrol_lib::util::session_marker::read(&expected_dir)
        .expect("session.marker must exist");
    assert_eq!(
        marker.state,
        micontrol_lib::util::session_marker::SessionState::Clean,
        "session.marker must be clean after mark_clean_shutdown"
    );
    assert!(marker.started_at.is_some());
    assert!(marker.clean_at.is_some());

    // 5. A new session on the same directory classifies the previous one as
    //    clean (this is the line the next real start logs).
    let previous = micontrol_lib::util::session_marker::begin_session(&expected_dir);
    assert!(
        !previous.is_suspicious(),
        "a cleanly closed session must not look like a crash: {}",
        previous.describe()
    );
    assert!(previous.describe().contains("ended cleanly"));

    let _ = std::fs::remove_dir_all(&root);
}
