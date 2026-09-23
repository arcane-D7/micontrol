//! Size-based log rotation shared by every file logger in the project.
//!
//! The app log (`tauri-app.log` / `tauri-dev-trace.log`), the bridge service
//! log (`bridge.log`) and the watchdog log are all plain append-only files.
//! Left alone they grow without bound — `tauri-app.log` reached 77 MB and
//! `bridge.log` 18 MB in a single release cycle, which makes them expensive to
//! ship with a bug report and slow to open.
//!
//! `docs/stability-report-2026-06-24.md` lists "Sem log rotation —
//! `tauri-dev-trace.log` cresce indefinidamente" as a MEDIUM finding, so this
//! module implements the fix that report asks for.
//!
//! Strategy: rotate **before** appending when the current file is at or above
//! the cap. Rotated files are numbered `.1` (newest) … `.N` (oldest); `.N` is
//! dropped. A single instance owns each log file, so no cross-process locking
//! is needed — only the process-wide mutex that serialises our own rotations.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

/// Default cap for the app and bridge logs.
pub const DEFAULT_MAX_BYTES: u64 = 8 * 1024 * 1024; // 8 MiB
/// Default number of rotated files kept (excluding the live one).
pub const DEFAULT_MAX_FILES: usize = 3;

/// Serialises rotations inside this process so two threads cannot rotate the
/// same file concurrently.
fn rotation_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
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
    let _guard = crate::util::panic::lock_or_recover(rotation_lock());

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

    /// Create a sink with the project defaults (8 MiB, 3 rotated files).
    pub fn with_defaults(path: PathBuf) -> Self {
        Self::new(path, DEFAULT_MAX_BYTES, DEFAULT_MAX_FILES)
    }

    fn ensure_open(&mut self) -> std::io::Result<&mut fs::File> {
        if self.file.is_none() {
            let file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.path)?;
            self.file = Some(file);
        }
        // `self.file` was just set when it was None, so this cannot fail.
        Ok(self.file.as_mut().expect("log file opened above"))
    }
}

impl Write for RotatingFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        // Rotate BEFORE writing so the new record starts the fresh file.
        if rotate_if_needed(&self.path, self.max_bytes, self.max_files) {
            // Drop the handle to the (now renamed) file and reopen the live one.
            self.file = None;
        }
        self.ensure_open()?.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self.file.as_mut() {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }
}

/// Append a line to a log file, rotating first when needed.
///
/// Convenience for the service logs (bridge/watchdog) that write with
/// `eprintln`-style calls rather than a logger facade.
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
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let ts = humantime::format_rfc3339_millis(SystemTime::now());
    writeln!(file, "{ts} {line}")
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
}
