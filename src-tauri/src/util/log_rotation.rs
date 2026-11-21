//! Size-based log rotation shared by every file logger in the project.
//!
//! The app log (`tauri-app.log` / `tauri-dev-trace.log`), the elevated helper
//! log (`tauri-app-helper.log`), the bridge service log (`bridge.log`) and the
//! watchdog log are all append-only files. Left alone they grow without bound —
//! `tauri-app.log` reached 77 MB and `bridge.log` 18 MB in a single release
//! cycle, which makes them expensive to ship with a bug report and slow to open.
//!
//! Strategy: rotate **before** appending when the current file is at or above
//! the cap. Rotated files are numbered `.1` (newest) … `.N` (oldest); `.N` is
//! dropped. Shipping defaults are **8 MiB × 7 rotations** (64 MiB ceiling per
//! file), which keeps roughly a week of normal use and still fits in a bug
//! report upload.
//!
//! ## One writer per file
//!
//! Each process family owns its own file (see `debug_log`): the app writes
//! `tauri-app.log`, the elevated helper writes `tauri-app-helper.log`. That is
//! the fix for the 2026-11-07 **P5** finding — 15 physically corrupted lines
//! caused by the app and 3 728 short-lived helper processes appending to the
//! same file at the same time. Rotation state is per-process (`radix`), so
//! sharing a file between processes also races the rename; never do it.
//!
//! ## Whole-line writes
//!
//! Every record is handed to the OS in a **single** `write_all` call on a
//! handle opened with `append(true)` — on Windows that is `FILE_APPEND_DATA`,
//! which makes the kernel place the whole buffer at the end of the file. Do not
//! build the line with several `write!` calls: each one is a separate syscall
//! and a second writer can land between them (that is exactly how
//! `…00.115Z2026-11-06T22:21:04.592Z [WARN ]` was produced).

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

/// Default cap for the app and bridge logs.
pub const DEFAULT_MAX_BYTES: u64 = 8 * 1024 * 1024; // 8 MiB
/// Default number of rotated files kept (excluding the live one).
///
/// Raised from 3 to 7 in the 2026-11-07 logging fix: at the observed rate one
/// 8 MiB file covers only a few hours of a busy session, so 3 files could not
/// hold the ~31 h window in which the audit needed to look back.
pub const DEFAULT_MAX_FILES: usize = 7;

/// Serialises rotations inside this process so two threads cannot rotate the
/// same file concurrently.
fn rotation_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// Acquire the rotation lock, recovering a poisoned mutex **without logging**.
///
/// `panic::lock_or_recover` logs on poison, and that log record re-enters this
/// module (formatter → rotating file → rotate) while the poisoned guard is
/// still alive inside the `PoisonError` — a self-deadlock. Rotation must stay
/// silent on recovery.
fn rotation_guard() -> std::sync::MutexGuard<'static, ()> {
    match rotation_lock().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

/// Path of a rotated file: `app.log` → `app.log.1`.
fn rotated_path(path: &Path, index: usize) -> PathBuf {
    let mut name = path.as_os_str().to_os_string();
    name.push(format!(".{index}"));
    PathBuf::from(name)
}

/// Rotate `path` when it has reached `max_bytes`.
///
/// Returns `true` when a rotation happened. Safe to call before every write:
/// the size is checked with a single `metadata()` call and the expensive work
/// only runs when the cap is reached.
pub fn rotate_if_needed(path: &Path, max_bytes: u64, max_files: usize) -> bool {
    if max_bytes == 0 || max_files == 0 {
        return false;
    }
    let _guard = rotation_guard();

    let too_big = fs::metadata(path)
        .map(|m| m.len() >= max_bytes)
        .unwrap_or(false);
    if !too_big {
        return false;
    }

    // Drop the oldest, then shift .N-1 → .N … .1 → .2, finally live → .1.
    let _ = fs::remove_file(rotated_path(path, max_files));
    for index in (1..max_files).rev() {
        let from = rotated_path(path, index);
        if from.exists() {
            let _ = fs::rename(&from, rotated_path(path, index + 1));
        }
    }
    let _ = fs::rename(path, rotated_path(path, 1));
    true
}

/// A `Write` adapter that rotates the file before each write.
///
/// Wraps the live `File` so callers keep using it as a normal sink (e.g. as a
/// `fern` chain target) while rotation happens transparently. The file is
/// reopened on demand after a rotation.
pub struct RotatingFile {
    path: PathBuf,
    max_bytes: u64,
    max_files: usize,
    file: Option<fs::File>,
}

impl RotatingFile {
    /// Create a rotating sink. The file is opened lazily on the first write so
    /// a rotation that happens between construction and first use is honoured.
    pub fn new(path: PathBuf, max_bytes: u64, max_files: usize) -> Self {
        Self {
            path,
            max_bytes,
            max_files,
            file: None,
        }
    }

    /// Create a sink with the project defaults (8 MiB, 7 rotated files).
    pub fn with_defaults(path: PathBuf) -> Self {
        Self::new(path, DEFAULT_MAX_BYTES, DEFAULT_MAX_FILES)
    }

    fn ensure_open(&mut self) -> std::io::Result<&mut fs::File> {
        if self.file.is_none() {
            // `append(true)` maps to FILE_APPEND_DATA on Windows: the kernel
            // places each write at EOF, so two handles cannot interleave
            // inside one buffer (that is the P5 guarantee).
            let file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?;
            self.file = Some(file);
        }
        // `self.file` was just set when it was None, so this cannot fail.
        Ok(self.file.as_mut().expect("log file opened above"))
    }

    fn write_record(&mut self, buf: &[u8]) -> std::io::Result<()> {
        // Rotate BEFORE writing so the new record starts the fresh file.
        if rotate_if_needed(&self.path, self.max_bytes, self.max_files) {
            // Drop the handle to the (now renamed) file and reopen the live one.
            self.file = None;
        }
        // One `write_all` per record → one WriteFile syscall for the whole line.
        self.ensure_open()?.write_all(buf)
    }
}

impl Write for RotatingFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.write_record(buf)?;
        Ok(buf.len())
    }

    /// Overridden so a formatted record is written with a single `write_all`
    /// instead of one syscall per formatted fragment (default `write_all`
    /// loops over `write`). This is what keeps a line atomic against other
    /// writers on the same handle.
    fn write_all(&mut self, buf: &[u8]) -> std::io::Result<()> {
        self.write_record(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self.file.as_mut() {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }
}

/// Append one line to a log file, rotating first when needed.
///
/// The timestamped line is built in memory **first** and handed to the file in
/// one `write_all`, so a concurrent writer cannot split it. Convenience for the
/// service logs (bridge/watchdog) that write with `eprintln`-style calls rather
/// than a logger facade.
pub fn append_line(
    path: &Path,
    line: &str,
    max_bytes: u64,
    max_files: usize,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    rotate_if_needed(path, max_bytes, max_files);

    let ts = humantime::format_rfc3339_millis(SystemTime::now()).to_string();
    let mut record = String::with_capacity(ts.len() + line.len() + 2);
    record.push_str(&ts);
    record.push(' ');
    record.push_str(line);
    record.push('\n');

    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(record.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("micontrol-logrot-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create temp dir");
        dir
    }

    #[test]
    fn does_not_rotate_below_cap() {
        let dir = temp_dir("below");
        let path = dir.join("app.log");
        fs::write(&path, b"small").unwrap();
        assert!(!rotate_if_needed(&path, 1024, 3));
        assert!(path.exists());
        assert!(!rotated_path(&path, 1).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotates_at_cap_and_moves_live_to_one() {
        let dir = temp_dir("atcap");
        let path = dir.join("app.log");
        fs::write(&path, vec![b'a'; 2048]).unwrap();
        assert!(rotate_if_needed(&path, 1024, 3));
        // Live file is gone (renamed); .1 holds the old content.
        assert!(!path.exists());
        assert!(rotated_path(&path, 1).exists());
        assert_eq!(fs::metadata(rotated_path(&path, 1)).unwrap().len(), 2048);
        let _ = fs::remove_dir_all(&dir);
    }

    /// The rotation boundary is inclusive: `len == cap - 1` must not rotate,
    /// `len == cap` must. A one-byte slip here is the difference between the
    /// documented 8 MiB ceiling and an unbounded file.
    #[test]
    fn rotation_boundary_is_exactly_at_the_cap() {
        let dir = temp_dir("boundary");
        let path = dir.join("app.log");
        let cap = 1024u64;

        fs::write(&path, vec![b'a'; cap as usize - 1]).unwrap();
        assert!(
            !rotate_if_needed(&path, cap, 7),
            "cap - 1 byte must stay in the live file"
        );
        assert!(!rotated_path(&path, 1).exists());

        fs::write(&path, vec![b'a'; cap as usize]).unwrap();
        assert!(
            rotate_if_needed(&path, cap, 7),
            "exactly at the cap must rotate"
        );
        assert!(rotated_path(&path, 1).exists());
        assert!(!path.exists());

        // A record larger than the cap is rotated ONCE and then written; the
        // next write rotates it away again — never an infinite rotate loop.
        fs::write(&path, vec![b'b'; cap as usize * 4]).unwrap();
        let mut sink = RotatingFile::new(path.clone(), cap, 7);
        sink.write_all(b"huge\n").unwrap();
        assert!(rotated_path(&path, 1).exists());
        assert_eq!(fs::read(&path).unwrap(), b"huge\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn keeps_at_most_max_files_and_drops_the_oldest() {
        let dir = temp_dir("cascade");
        let path = dir.join("app.log");
        for round in 0..5 {
            fs::write(&path, vec![b'a' + round as u8; 2048]).unwrap();
            assert!(rotate_if_needed(&path, 1024, 2));
        }
        assert!(rotated_path(&path, 1).exists());
        assert!(rotated_path(&path, 2).exists());
        // max_files = 2 → the third rotated file must never exist.
        assert!(!rotated_path(&path, 3).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn zero_cap_or_zero_files_disables_rotation() {
        let dir = temp_dir("disabled");
        let path = dir.join("app.log");
        fs::write(&path, vec![b'a'; 4096]).unwrap();
        assert!(!rotate_if_needed(&path, 0, 3));
        assert!(!rotate_if_needed(&path, 1024, 0));
        assert!(path.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotating_file_reopens_after_rotation() {
        let dir = temp_dir("reopen");
        let path = dir.join("app.log");
        let mut sink = RotatingFile::new(path.clone(), 128, 2);
        // First write stays under the cap and creates the file.
        sink.write_all(b"first").unwrap();
        sink.flush().unwrap();
        assert!(path.exists());
        assert!(!rotated_path(&path, 1).exists());
        // Push past the cap. Rotation happens before a write only when the
        // file is ALREADY at/above the cap, so this write lands in the live
        // file (which is now oversized but never unbounded).
        sink.write_all(&vec![b'x'; 256]).unwrap();
        sink.flush().unwrap();
        assert!(
            !rotated_path(&path, 1).exists(),
            "the first crossing write does not rotate; it fills the file"
        );
        // The next write sees a file at the cap, rotates it away, and reopens
        // a fresh live file — the mechanism that bounds growth.
        sink.write_all(b"after").unwrap();
        sink.flush().unwrap();
        assert!(rotated_path(&path, 1).exists(), "oversized file rotated");
        assert!(path.exists(), "live file must be recreated after rotation");
        let live = fs::read_to_string(&path).unwrap();
        assert!(live.contains("after"), "new record landed in the live file");
        // The rotated file holds the OLD live content, so it starts with the
        // first record ever written ("first"), not with the overflow bytes.
        let rotated = fs::read_to_string(rotated_path(&path, 1)).unwrap();
        assert!(rotated.starts_with("first"), "old content moved to .1");
        assert!(rotated.contains('x'), "overflow bytes moved to .1 too");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_line_rotates_and_writes() {
        let dir = temp_dir("append");
        let path = dir.join("bridge.log");
        fs::write(&path, vec![b'a'; 2048]).unwrap();
        append_line(&path, "hello", 1024, 2).unwrap();
        assert!(rotated_path(&path, 1).exists());
        let live = fs::read_to_string(&path).unwrap();
        assert!(live.contains("hello"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_line_creates_missing_parent_directory() {
        let dir = temp_dir("mkdir");
        let path = dir.join("nested").join("deeper").join("x.log");
        append_line(&path, "created", DEFAULT_MAX_BYTES, DEFAULT_MAX_FILES).unwrap();
        assert!(path.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn append_line_writes_one_record_per_call() {
        let dir = temp_dir("oneline");
        let path = dir.join("helper.log");
        for i in 0..50 {
            append_line(
                &path,
                &format!("line {i}"),
                DEFAULT_MAX_BYTES,
                DEFAULT_MAX_FILES,
            )
            .unwrap();
        }
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(text.lines().count(), 50);
        // Every physical line is exactly "<timestamp> line N" — no fragment
        // from a neighbouring write can appear in the middle.
        for (i, line) in text.lines().enumerate() {
            assert!(
                line.contains(&format!(" line {i}")),
                "line {i} is intact, got: {line}"
            );
            assert!(line.starts_with("20"), "timestamp first, got: {line}");
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// P5 regression: N threads, each with its **own** append handle (the same
    /// situation as several processes / helper invocations), write records to
    /// one file. Every physical line must be complete — no line may contain two
    /// records and no record may be lost.
    #[test]
    fn concurrent_writers_never_tear_a_line() {
        let dir = temp_dir("concurrent");
        let path = dir.join("tauri-app-helper.log");
        let writers = 8;
        let per_writer = 200;

        let mut handles = Vec::new();
        for writer in 0..writers {
            let path = path.clone();
            handles.push(std::thread::spawn(move || {
                // Own handle per thread, exactly like a separate process.
                let mut sink = RotatingFile::with_defaults(path);
                for seq in 0..per_writer {
                    let line = format!("writer={writer} seq={seq} payload={}\n", "x".repeat(48));
                    sink.write_all(line.as_bytes()).expect("append record");
                }
                sink.flush().expect("flush");
            }));
        }
        for handle in handles {
            handle.join().expect("writer thread");
        }

        let text = fs::read_to_string(&path).expect("read log");
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            lines.len(),
            writers * per_writer,
            "every record must survive (no lost lines)"
        );

        let mut seen = std::collections::HashSet::new();
        for line in &lines {
            // A torn line would contain two "writer=" markers or a broken tail.
            assert_eq!(
                line.matches("writer=").count(),
                1,
                "line contains more than one record: {line}"
            );
            assert!(
                line.ends_with('x'),
                "line tail was cut by another writer: {line}"
            );
            let (writer, seq) = parse_record(line);
            assert!(seen.insert((writer, seq)), "duplicate record {line}");
        }
        assert_eq!(seen.len(), writers * per_writer);
        let _ = fs::remove_dir_all(&dir);
    }

    fn parse_record(line: &str) -> (u32, u32) {
        let mut writer = 0;
        let mut seq = 0;
        for part in line.split_whitespace() {
            if let Some(v) = part.strip_prefix("writer=") {
                writer = v.parse().unwrap();
            } else if let Some(v) = part.strip_prefix("seq=") {
                seq = v.parse().unwrap();
            }
        }
        (writer, seq)
    }

    /// S64 audit: prove the SHIPPING configuration rotates. The other tests use
    /// tiny caps for speed; this one uses `DEFAULT_MAX_BYTES` (8 MiB) so the
    /// values actually wired into `debug_log.rs` and the bridge loggers are
    /// exercised. The oversize file is created sparsely with `set_len`, so the
    /// test stays fast and does not write 8 MiB to disk.
    #[test]
    fn production_defaults_rotate_a_sparse_oversize_log() {
        let dir = temp_dir("proddefaults");
        let path = dir.join("tauri-app.log");

        // A log just over the production cap.
        {
            let file = fs::File::create(&path).unwrap();
            file.set_len(DEFAULT_MAX_BYTES + 1024).unwrap();
        }
        assert_eq!(fs::metadata(&path).unwrap().len(), DEFAULT_MAX_BYTES + 1024);
        assert!(!rotated_path(&path, 1).exists());

        // The very next append must rotate the oversize file away.
        append_line(
            &path,
            "after-rotation",
            DEFAULT_MAX_BYTES,
            DEFAULT_MAX_FILES,
        )
        .unwrap();

        assert!(
            rotated_path(&path, 1).exists(),
            "production defaults must rotate a log over DEFAULT_MAX_BYTES"
        );
        let live_len = fs::metadata(&path).unwrap().len();
        assert!(
            live_len < DEFAULT_MAX_BYTES,
            "live log must be back under the cap, got {live_len} bytes"
        );
        let live = fs::read_to_string(&path).unwrap();
        assert!(live.contains("after-rotation"));
        let _ = fs::remove_dir_all(&dir);
    }

    /// S64 audit: `DEFAULT_MAX_FILES` rotated files are kept, and older ones are
    /// dropped, so disk usage is bounded at roughly `MAX_BYTES * (1 + MAX_FILES)`.
    #[test]
    fn production_defaults_bound_total_disk_usage() {
        let dir = temp_dir("prodbound");
        let path = dir.join("bridge.log");
        // Two rotations is enough to prove the cascade with the real file count.
        for round in 0..(DEFAULT_MAX_FILES + 1) {
            fs::write(&path, vec![b'z'; 2048]).unwrap();
            assert!(rotate_if_needed(&path, 1024, DEFAULT_MAX_FILES));
            let _ = round;
        }
        for index in 1..=DEFAULT_MAX_FILES {
            assert!(
                rotated_path(&path, index).exists(),
                "rotated file .{index} should exist with DEFAULT_MAX_FILES"
            );
        }
        assert!(
            !rotated_path(&path, DEFAULT_MAX_FILES + 1).exists(),
            "no file beyond DEFAULT_MAX_FILES may be kept"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// The documented retention is 8 MiB × 7 (the value quoted in
    /// `docs/LOGGING.md`). Pin it so a silent change cannot shrink the window
    /// the audit depended on.
    #[test]
    fn documented_retention_constants_are_pinned() {
        assert_eq!(DEFAULT_MAX_BYTES, 8 * 1024 * 1024);
        assert_eq!(DEFAULT_MAX_FILES, 7);
    }
}
