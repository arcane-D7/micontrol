//! Application event-log archival (elevated).
//!
//! The Windows `Application` channel has been at 100 % of its maximum size
//! since 2026-10-22, so new events are being dropped. Archiving it needs an
//! elevated token: `wevtutil epl` (export) and `wevtutil cl` (clear) both
//! require administrator rights on a channel owned by the EventLog service.
//!
//! ## Execution context
//!
//! This module is only reachable through `elevated::dispatch()`, i.e. from the
//! two sanctioned elevation paths:
//!
//! * `MiControlBridge` — the autonomous service, running as
//!   `NT AUTHORITY\SYSTEM` (pipe path);
//! * `MiControlElevated` — the scheduled task, `RunLevel=HighestAvailable` on
//!   the interactive user's token (command-file path).
//!
//! Both carry an administrator token, which is what `wevtutil` needs. The task
//! path is *not* SYSTEM — it is the user's own elevated token — so nothing
//! here may assume SYSTEM-only privileges (e.g. it must not resolve paths
//! through `%USERPROFILE%`, which would be the service profile on the pipe
//! path and the user's profile on the task path).
//!
//! Verified on this host: `wevtutil gl Application` reports channel access
//! `(A;;0x7;;;BA)(A;;0x3;;;IU)` — interactive users may read/append (0x1|0x2)
//! but only Builtin Administrators (0x7, which includes 0x4 = clear),
//! SYSTEM (0xf0007) and Server Operators may clear the channel.
//!
//! ## Protections
//!
//! 1. **The channel is a constant.** [`CHANNEL`] is `Application` and the
//!    dispatcher arm takes no arguments, so a forged payload cannot retarget
//!    the export or the clear at `Security` or `System`. [`check_channel`]
//!    re-asserts the allowlist at the point of use.
//! 2. **One archival per hour.** A latch file
//!    (`%PROGRAMDATA%\MiControl\logs\.archive_latch`, unix seconds) refuses a
//!    second run inside [`MIN_INTERVAL_SECS`] with an explicit `too soon`
//!    error instead of silently re-exporting.
//! 3. **Bounded retention.** Only the newest [`KEEP_ARCHIVES`] `.evtx` files
//!    are kept; older ones are deleted after a successful export.
//! 4. **The latch is written only after a real export.** A failed export
//!    leaves no latch, so the operator can retry immediately; a successful
//!    export latches even when the clear fails (the archive exists, and
//!    re-exporting it would only duplicate it).

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The only event-log channel this module may touch.
pub const CHANNEL: &str = "Application";

/// Minimum spacing between two archivals of the same machine.
pub const MIN_INTERVAL_SECS: u64 = 3600;

/// Number of `.evtx` archives kept in the archive directory.
pub const KEEP_ARCHIVES: usize = 5;

const ARCHIVE_PREFIX: &str = "Application-";
const ARCHIVE_EXT: &str = ".evtx";

/// Result of a successful archival, serialized straight into the elevated
/// response `data` object.
#[derive(Debug, Clone, Serialize)]
pub struct ArchiveOutcome {
    /// Absolute path of the exported `.evtx` file.
    pub archived_to: String,
    /// `true` when `wevtutil cl Application` succeeded.
    pub cleared: bool,
    /// Size of the exported file in bytes.
    pub size_bytes: u64,
    /// Why the clear failed, when `cleared` is `false`. Omitted when `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub clear_error: Option<String>,
    /// Archive files deleted by the retention pass (oldest first).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub pruned: Vec<String>,
}

/// Archive the `Application` channel and clear it.
///
/// Returns `Err` with a `too soon` message when the latch is younger than
/// [`MIN_INTERVAL_SECS`], and with the `wevtutil` diagnostic when the export
/// itself fails (in which case nothing was cleared and no latch was written).
pub fn archive_event_log() -> Result<ArchiveOutcome, String> {
    let logs = program_data_dir().join("logs");
    archive_with(
        &SystemRunner,
        &logs.join("archive"),
        &logs.join(".archive_latch"),
        now_unix(),
        KEEP_ARCHIVES,
    )
}

/// `%PROGRAMDATA%\MiControl` — the machine-wide state directory shared with
/// the bridge/watchdog logs.
fn program_data_dir() -> PathBuf {
    std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
        .join("MiControl")
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Testable core: every side effect is either a path argument or a
/// [`CmdRunner`] call, so unit tests never touch the real event log.
fn archive_with<R: CmdRunner>(
    runner: &R,
    archive_dir: &Path,
    latch_path: &Path,
    now: u64,
    keep: usize,
) -> Result<ArchiveOutcome, String> {
    check_channel(CHANNEL)?;

    if let Some(last) = read_latch(latch_path) {
        if let Some(remaining) = too_soon(last, now) {
            return Err(format!(
                "too soon: the {CHANNEL} log was archived {} s ago \
                 (minimum {MIN_INTERVAL_SECS} s between runs; retry in {remaining} s)",
                now.saturating_sub(last)
            ));
        }
    }

    std::fs::create_dir_all(archive_dir).map_err(|e| {
        format!(
            "cannot create archive directory {}: {e}",
            archive_dir.display()
        )
    })?;

    let target = archive_dir.join(archive_file_name(now));
    if target.exists() {
        return Err(format!(
            "archive target already exists: {}",
            target.display()
        ));
    }

    let target_arg = target.to_string_lossy().into_owned();
    run_wevtutil(runner, &["epl", CHANNEL, &target_arg])?;

    // `wevtutil epl` can exit 0 without producing a usable file (e.g. an
    // unwritable target). Trust the file, not the exit code.
    let size_bytes = std::fs::metadata(&target).map(|m| m.len()).map_err(|e| {
        format!(
            "wevtutil epl reported success but {} is unreadable: {e}",
            target.display()
        )
    })?;
    if size_bytes == 0 {
        return Err(format!(
            "wevtutil epl produced an empty archive: {}",
            target.display()
        ));
    }

    // Latch before the clear: the archive is durable at this point, and a
    // failed clear must not cause a second export of the same events.
    write_latch(latch_path, now)?;

    let (cleared, clear_error) = match run_wevtutil(runner, &["cl", CHANNEL]) {
        Ok(()) => (true, None),
        Err(e) => {
            log::warn!("archive_event_log: export ok but clear failed: {e}");
            (false, Some(e))
        }
    };

    let pruned = prune_archives(archive_dir, keep);

    Ok(ArchiveOutcome {
        archived_to: target.display().to_string(),
        cleared,
        size_bytes,
        clear_error,
        pruned,
    })
}

/// Allowlist guard: only [`CHANNEL`] may be exported or cleared.
fn check_channel(channel: &str) -> Result<(), String> {
    if channel.eq_ignore_ascii_case(CHANNEL) {
        Ok(())
    } else {
        Err(format!(
            "refusing to touch event-log channel '{channel}': \
             only '{CHANNEL}' may be archived or cleared"
        ))
    }
}

/// `Application-YYYYMMDD-HHMMSS.evtx` in UTC.
///
/// Zero-padded and fixed-width, so lexicographic order equals chronological
/// order — the retention pass relies on that instead of file mtimes.
fn archive_file_name(now: u64) -> String {
    let (y, mo, d, h, mi, s) = utc_parts(now);
    format!("{ARCHIVE_PREFIX}{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}{ARCHIVE_EXT}")
}

/// Seconds since the unix epoch → `(year, month, day, hour, minute, second)`
/// in UTC (Howard Hinnant's civil-from-days algorithm; no chrono dependency).
fn utc_parts(secs: u64) -> (i64, u32, u32, u32, u32, u32) {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, mo, d) = civil_from_days(days);
    (
        y,
        mo,
        d,
        (rem / 3600) as u32,
        ((rem % 3600) / 60) as u32,
        (rem % 60) as u32,
    )
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// `Some(seconds_remaining)` when the last archival is inside the window.
///
/// A latch timestamp in the future (clock moved backwards) counts as "just
/// now" — conservative, never a free extra run.
fn too_soon(last: u64, now: u64) -> Option<u64> {
    let elapsed = now.saturating_sub(last);
    if elapsed < MIN_INTERVAL_SECS {
        Some(MIN_INTERVAL_SECS - elapsed)
    } else {
        None
    }
}

fn read_latch(path: &Path) -> Option<u64> {
    let raw = std::fs::read_to_string(path).ok()?;
    match raw.trim().parse::<u64>() {
        Ok(ts) => Some(ts),
        Err(e) => {
            // A corrupt latch must not wedge the command forever: treat it as
            // absent (the export itself is the safety-critical part).
            log::warn!(
                "archive_event_log: ignoring unparsable latch {} ({e})",
                path.display()
            );
            None
        }
    }
}

/// Atomic latch write (temp file + rename), same pattern as the nonce store.
fn write_latch(path: &Path, now: u64) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create latch directory {}: {e}", parent.display()))?;
    }
    let tmp = path.with_extension("latch.tmp");
    std::fs::write(&tmp, now.to_string())
        .map_err(|e| format!("cannot write latch {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("cannot commit latch {}: {e}", path.display())
    })
}

/// Keep the newest `keep` archives, delete the rest. Returns the deleted names.
fn prune_archives(dir: &Path, keep: usize) -> Vec<String> {
    let keep = keep.max(1);
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| e.file_name().to_str().map(str::to_owned))
        .filter(|n| n.starts_with(ARCHIVE_PREFIX) && n.ends_with(ARCHIVE_EXT))
        .collect();
    names.sort();
    if names.len() <= keep {
        return Vec::new();
    }
    let mut removed = Vec::new();
    for name in &names[..names.len() - keep] {
        match std::fs::remove_file(dir.join(name)) {
            Ok(()) => removed.push(name.clone()),
            Err(e) => log::warn!("archive retention: cannot delete {name}: {e}"),
        }
    }
    removed
}

fn run_wevtutil<R: CmdRunner>(runner: &R, args: &[&str]) -> Result<(), String> {
    let out = runner.run("wevtutil", args)?;
    if out.success {
        return Ok(());
    }
    let detail = if out.stderr.trim().is_empty() {
        out.stdout.trim().to_string()
    } else {
        out.stderr.trim().to_string()
    };
    Err(format!(
        "wevtutil {} failed (exit {}): {detail}",
        args.join(" "),
        out.code
            .map(|c| c.to_string())
            .unwrap_or_else(|| "?".to_string())
    ))
}

// ── Command execution (injectable for tests) ─────────────────────────────────

/// Captured result of one external command.
#[derive(Debug, Clone, Default)]
pub struct CmdOutput {
    pub success: bool,
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Minimal seam so unit tests can exercise the whole flow without running
/// `wevtutil` (which would clear the real log).
pub trait CmdRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<CmdOutput, String>;
}

/// Runs the command for real, without a console window.
pub struct SystemRunner;

impl CmdRunner for SystemRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<CmdOutput, String> {
        let mut cmd = std::process::Command::new(program);
        cmd.args(args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let out = cmd
            .output()
            .map_err(|e| format!("cannot run {program}: {e}"))?;
        Ok(CmdOutput {
            success: out.status.success(),
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// Unique, empty scratch directory under %TEMP% (removed on drop).
    struct ScratchDir(PathBuf);

    impl ScratchDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "micontrol-eventlog-test-{tag}-{}",
                std::process::id()
            ));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create scratch dir");
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for ScratchDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Records every command and fakes the export by creating the target file.
    struct FakeRunner {
        calls: RefCell<Vec<(String, Vec<String>)>>,
        /// Exit status for `epl` / `cl` respectively.
        epl_ok: bool,
        cl_ok: bool,
        /// Bytes written to the fake archive.
        payload: usize,
    }

    impl FakeRunner {
        fn new() -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                epl_ok: true,
                cl_ok: true,
                payload: 4096,
            }
        }

        fn calls(&self) -> Vec<(String, Vec<String>)> {
            self.calls.borrow().clone()
        }
    }

    impl CmdRunner for FakeRunner {
        fn run(&self, program: &str, args: &[&str]) -> Result<CmdOutput, String> {
            self.calls.borrow_mut().push((
                program.to_string(),
                args.iter().map(|a| a.to_string()).collect(),
            ));
            let sub = args.first().copied().unwrap_or_default();
            let ok = match sub {
                "epl" => {
                    if self.epl_ok {
                        let target = Path::new(args[2]);
                        std::fs::write(target, vec![0u8; self.payload])
                            .map_err(|e| format!("fake epl write failed: {e}"))?;
                    }
                    self.epl_ok
                }
                "cl" => self.cl_ok,
                _ => true,
            };
            Ok(CmdOutput {
                success: ok,
                code: Some(if ok { 0 } else { 1 }),
                stdout: String::new(),
                stderr: if ok {
                    String::new()
                } else {
                    format!("fake {sub} failure")
                },
            })
        }
    }

    // ── File name / timestamp ───────────────────────────────────────────────

    #[test]
    fn archive_file_name_is_utc_and_sortable() {
        // 2023-11-14T22:13:20Z
        assert_eq!(
            archive_file_name(1_700_000_000),
            "Application-20231114-221320.evtx"
        );
        // Epoch.
        assert_eq!(archive_file_name(0), "Application-19700101-000000.evtx");
        // Leap day, 2000-02-29T00:00:00Z.
        assert_eq!(
            archive_file_name(951_782_400),
            "Application-20000229-000000.evtx"
        );
        // 2026-11-08T00:00:00Z (the day this command was written).
        assert_eq!(
            archive_file_name(1_794_096_000),
            "Application-20261108-000000.evtx"
        );
    }

    #[test]
    fn archive_file_names_sort_chronologically() {
        let mut names = vec![
            archive_file_name(1_794_096_000),      // 2026-11-08 00:00:00
            archive_file_name(1_700_000_000),      // 2023-11-14 22:13:20
            archive_file_name(1_794_096_000 + 59), // 2026-11-08 00:00:59
        ];
        names.sort();
        assert_eq!(
            names,
            vec![
                "Application-20231114-221320.evtx".to_string(),
                "Application-20261108-000000.evtx".to_string(),
                "Application-20261108-000059.evtx".to_string(),
            ]
        );
    }

    // ── Latch ───────────────────────────────────────────────────────────────

    #[test]
    fn latch_window_is_one_hour() {
        let now = 1_794_096_000;
        assert_eq!(too_soon(now, now), Some(MIN_INTERVAL_SECS));
        assert_eq!(too_soon(now - 1, now), Some(MIN_INTERVAL_SECS - 1));
        assert_eq!(too_soon(now - 3_599, now), Some(1));
        assert_eq!(too_soon(now - 3_600, now), None);
        assert_eq!(too_soon(now - 86_400, now), None);
        // Clock moved backwards: still "too soon", never a free run.
        assert_eq!(too_soon(now + 600, now), Some(MIN_INTERVAL_SECS));
    }

    #[test]
    fn latch_round_trips_and_survives_garbage() {
        let scratch = ScratchDir::new("latch");
        let latch = scratch.path().join(".archive_latch");

        assert_eq!(read_latch(&latch), None, "absent latch reads as None");
        write_latch(&latch, 1_794_096_000).expect("write latch");
        assert_eq!(read_latch(&latch), Some(1_794_096_000));

        std::fs::write(&latch, "not-a-timestamp").expect("corrupt latch");
        assert_eq!(read_latch(&latch), None, "corrupt latch is ignored");
    }

    // ── Channel guard ───────────────────────────────────────────────────────

    #[test]
    fn only_the_application_channel_is_allowed() {
        assert!(check_channel("Application").is_ok());
        assert!(check_channel("application").is_ok());
        for forbidden in ["Security", "System", "Setup", "Application/Operational"] {
            let err = check_channel(forbidden).expect_err("must be refused");
            assert!(
                err.contains("refusing to touch event-log channel"),
                "unexpected error for {forbidden}: {err}"
            );
        }
    }

    // ── Full flow (fake wevtutil) ───────────────────────────────────────────

    #[test]
    fn archives_then_clears_and_reports_size() {
        let scratch = ScratchDir::new("flow");
        let archive_dir = scratch.path().join("logs").join("archive");
        let latch = scratch.path().join("logs").join(".archive_latch");
        let runner = FakeRunner::new();
        let now = 1_794_096_000;

        let outcome = archive_with(&runner, &archive_dir, &latch, now, KEEP_ARCHIVES)
            .expect("archival must succeed");

        assert_eq!(
            outcome.archived_to,
            archive_dir
                .join("Application-20261108-000000.evtx")
                .display()
                .to_string()
        );
        assert!(outcome.cleared);
        assert_eq!(outcome.size_bytes, 4096);
        assert!(outcome.clear_error.is_none());
        assert!(outcome.pruned.is_empty());

        let calls = runner.calls();
        assert_eq!(calls.len(), 2, "exactly one export and one clear");
        assert_eq!(calls[0].0, "wevtutil");
        assert_eq!(calls[0].1[0], "epl");
        assert_eq!(calls[0].1[1], "Application");
        assert_eq!(calls[0].1[2], outcome.archived_to);
        assert_eq!(
            calls[1].1,
            vec!["cl".to_string(), "Application".to_string()]
        );

        assert_eq!(read_latch(&latch), Some(now), "latch records the run");
    }

    #[test]
    fn second_run_within_the_hour_is_refused_as_too_soon() {
        let scratch = ScratchDir::new("too-soon");
        let archive_dir = scratch.path().join("archive");
        let latch = scratch.path().join(".archive_latch");
        let runner = FakeRunner::new();
        let now = 1_794_096_000;

        archive_with(&runner, &archive_dir, &latch, now, KEEP_ARCHIVES).expect("first run");
        let err = archive_with(&runner, &archive_dir, &latch, now + 60, KEEP_ARCHIVES)
            .expect_err("second run must be refused");

        assert!(err.starts_with("too soon:"), "unexpected error: {err}");
        assert!(err.contains("retry in 3540 s"), "unexpected error: {err}");
        assert_eq!(
            runner.calls().len(),
            2,
            "the refused run must not touch wevtutil"
        );
        // Exactly one archive on disk.
        let count = std::fs::read_dir(&archive_dir).unwrap().count();
        assert_eq!(count, 1);
    }

    #[test]
    fn run_is_allowed_again_after_the_window() {
        let scratch = ScratchDir::new("after-window");
        let archive_dir = scratch.path().join("archive");
        let latch = scratch.path().join(".archive_latch");
        let runner = FakeRunner::new();
        let now = 1_794_096_000;

        archive_with(&runner, &archive_dir, &latch, now, KEEP_ARCHIVES).expect("first run");
        let outcome = archive_with(
            &runner,
            &archive_dir,
            &latch,
            now + MIN_INTERVAL_SECS,
            KEEP_ARCHIVES,
        )
        .expect("second run after the window");
        assert!(outcome.cleared);
        assert_eq!(read_latch(&latch), Some(now + MIN_INTERVAL_SECS));
    }

    #[test]
    fn failed_export_leaves_no_latch_and_clears_nothing() {
        let scratch = ScratchDir::new("epl-fail");
        let archive_dir = scratch.path().join("archive");
        let latch = scratch.path().join(".archive_latch");
        let mut runner = FakeRunner::new();
        runner.epl_ok = false;

        let err = archive_with(&runner, &archive_dir, &latch, 1_794_096_000, KEEP_ARCHIVES)
            .expect_err("export failure must surface");
        assert!(err.contains("wevtutil epl"), "unexpected error: {err}");
        assert_eq!(read_latch(&latch), None, "no latch after a failed export");
        let calls = runner.calls();
        assert_eq!(
            calls.len(),
            1,
            "the clear must not run after a failed export"
        );
        assert_eq!(calls[0].1[0], "epl");
    }

    #[test]
    fn failed_clear_is_reported_honestly_and_still_latches() {
        let scratch = ScratchDir::new("cl-fail");
        let archive_dir = scratch.path().join("archive");
        let latch = scratch.path().join(".archive_latch");
        let mut runner = FakeRunner::new();
        runner.cl_ok = false;
        let now = 1_794_096_000;

        let outcome = archive_with(&runner, &archive_dir, &latch, now, KEEP_ARCHIVES)
            .expect("export succeeded, so the command succeeds");
        assert!(!outcome.cleared);
        assert!(outcome
            .clear_error
            .as_deref()
            .unwrap_or_default()
            .contains("wevtutil cl"));
        assert_eq!(outcome.size_bytes, 4096);
        assert_eq!(
            read_latch(&latch),
            Some(now),
            "the archive exists, so a re-export must be refused"
        );
    }

    #[test]
    fn retention_keeps_the_newest_five() {
        let scratch = ScratchDir::new("retention");
        let archive_dir = scratch.path().join("archive");
        std::fs::create_dir_all(&archive_dir).unwrap();
        // Six older archives, one hour apart, plus a decoy that must survive.
        for i in 0..6u64 {
            let name = archive_file_name(1_794_000_000 + i * 3_600);
            std::fs::write(archive_dir.join(&name), b"old").unwrap();
        }
        std::fs::write(archive_dir.join("notes.txt"), b"keep me").unwrap();

        let latch = scratch.path().join(".archive_latch");
        let runner = FakeRunner::new();
        let now = 1_794_096_000;
        let outcome = archive_with(&runner, &archive_dir, &latch, now, KEEP_ARCHIVES)
            .expect("archival must succeed");

        assert_eq!(outcome.pruned.len(), 2, "7 archives → 5 kept, 2 pruned");
        let mut remaining: Vec<String> = std::fs::read_dir(&archive_dir)
            .unwrap()
            .flatten()
            .filter_map(|e| e.file_name().to_str().map(str::to_owned))
            .filter(|n| n.ends_with(ARCHIVE_EXT))
            .collect();
        remaining.sort();
        assert_eq!(remaining.len(), KEEP_ARCHIVES);
        assert!(
            remaining.contains(&"Application-20261108-000000.evtx".to_string()),
            "the fresh archive must survive: {remaining:?}"
        );
        assert!(
            archive_dir.join("notes.txt").exists(),
            "non-archive files are never touched"
        );
    }

    // ── Dispatcher wiring ───────────────────────────────────────────────────

    /// The dispatcher arm is the allowlist: if it disappears, the command is
    /// unreachable from both elevation paths. Source-marker test because
    /// calling it would run the real `wevtutil` against the live log.
    #[test]
    fn dispatcher_exposes_archive_event_log() {
        let src = include_str!("../elevated.rs");
        assert!(
            src.contains("\"archive_event_log\" =>"),
            "elevated::dispatch must expose the archive_event_log arm"
        );
        assert!(
            src.contains("crate::hw::event_log::archive_event_log()"),
            "the arm must call hw::event_log::archive_event_log()"
        );
    }

    // ── Live (ignored) ──────────────────────────────────────────────────────

    /// LIVE and DESTRUCTIVE: archives and then CLEARS the real `Application`
    /// channel on this machine. Ignored by default; run it from an elevated
    /// shell only when you actually intend to clear the log:
    ///
    /// ```text
    /// cargo test --lib hw::event_log::tests::live_archive_application_log -- --ignored --nocapture
    /// ```
    ///
    /// It prints the `wevtutil gl` preflight and the outcome, so the operator
    /// can compare a local run with the same command dispatched through the
    /// auth bridge.
    #[test]
    #[ignore = "clears the real Application event log; run manually from an elevated shell"]
    fn live_archive_application_log() {
        let preflight = SystemRunner
            .run("wevtutil", &["gl", CHANNEL])
            .expect("wevtutil gl must be runnable");
        println!(
            "wevtutil gl {CHANNEL} (exit {:?}):\n{}{}",
            preflight.code, preflight.stdout, preflight.stderr
        );
        assert!(
            preflight.success,
            "wevtutil gl failed — is this shell elevated? {}",
            preflight.stderr
        );

        let outcome = archive_event_log().expect("archive_event_log must succeed");
        println!("{outcome:#?}");
        assert!(
            Path::new(&outcome.archived_to).exists(),
            "archive file must exist: {}",
            outcome.archived_to
        );
        assert!(outcome.size_bytes > 0, "archive must not be empty");
    }
}
