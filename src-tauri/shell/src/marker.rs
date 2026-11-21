//! Installer shutdown announcement for the app's session marker.
//!
//! The app records `state=clean` from its Tauri `RunEvent::Exit` handler. A
//! forcible kill (`taskkill /F` — what [`crate::install::stop_everything`]
//! does before swapping the payload) can never run that handler, so the marker
//! stays at `state=running` and the next start logs a phantom
//! `previous session ended without shutdown marker — possible crash/kill`
//! after every update (audit 2026-11-08).
//!
//! This module announces the intentional kill *before* it happens, in the same
//! two places as `src-tauri/nsis/installer.nsi`:
//!
//! * `<user log dir>\session.marker` (`%LOCALAPPDATA%\MiControl\logs`) — read
//!   directly by the app;
//! * `%ProgramData%\MiControl\installer-shutdown.marker` — the machine-wide
//!   copy, needed because the shell runs elevated/SYSTEM, where
//!   `%LOCALAPPDATA%` is the system profile and not the interactive user's.
//!   The app honours it only when it is newer than the session marker.
//!
//! Keep `state=installer-shutdown` in sync with
//! `src-tauri/src/util/session_marker.rs`.

use std::fs;
use std::path::{Path, PathBuf};

/// State string read by `util::session_marker`.
pub const INSTALLER_SHUTDOWN_STATE: &str = "installer-shutdown";

/// File name of the marker the app reads at startup.
pub const MARKER_FILE_NAME: &str = "session.marker";

/// File name of the machine-wide announcement.
pub const INSTALLER_MARKER_FILE_NAME: &str = "installer-shutdown.marker";

/// Write `state=installer-shutdown` into `dir\session.marker`.
///
/// Used by [`announce_installer_shutdown`] and by tests.
pub fn write_shutdown_marker(dir: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dir)?;
    fs::write(
        dir.join(MARKER_FILE_NAME),
        format!("state={INSTALLER_SHUTDOWN_STATE}\n"),
    )
}

/// Machine-wide marker directory (`%ProgramData%\MiControl`).
pub fn programdata_dir() -> PathBuf {
    std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
        .join("MiControl")
}

/// Announce the shutdown before the shell kills the app.
///
/// Best effort: a marker that cannot be written must never block an update —
/// the worst case is the false "possible crash/kill" line this exists to
/// prevent.
pub fn announce_installer_shutdown() {
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        let logs = PathBuf::from(local).join("MiControl").join("logs");
        if let Err(e) = write_shutdown_marker(&logs) {
            eprintln!(
                "note: could not write the shutdown marker to {}: {e}",
                logs.display()
            );
        }
    }
    let machine_dir = programdata_dir();
    match fs::create_dir_all(&machine_dir) {
        Ok(()) => {
            if let Err(e) = fs::write(
                machine_dir.join(INSTALLER_MARKER_FILE_NAME),
                format!("state={INSTALLER_SHUTDOWN_STATE}\n"),
            ) {
                eprintln!(
                    "note: could not write the machine-wide shutdown marker to {}: {e}",
                    machine_dir.display()
                );
            }
        }
        Err(e) => eprintln!(
            "note: could not create {} for the shutdown marker: {e}",
            machine_dir.display()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "micontrol-shell-marker-{tag}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn marker_says_installer_shutdown_in_the_app_format() {
        let dir = temp_dir("write");
        write_shutdown_marker(&dir).expect("marker written");
        let text = fs::read_to_string(dir.join(MARKER_FILE_NAME)).unwrap();
        assert_eq!(text.trim(), "state=installer-shutdown");
        // The app's parser splits on '=' and trims — one pair, no CR needed.
        assert!(text.contains("state=installer-shutdown"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn writing_creates_the_log_dir() {
        let dir = temp_dir("mkdir").join("MiControl").join("logs");
        assert!(!dir.exists());
        write_shutdown_marker(&dir).expect("nested dir created");
        assert!(dir.join(MARKER_FILE_NAME).is_file());
        let _ = fs::remove_dir_all(dir.parent().and_then(|p| p.parent()).unwrap());
    }

    #[test]
    fn state_string_matches_the_app_parser() {
        // src-tauri/src/util/session_marker.rs maps this exact literal to
        // SessionState::InstallerShutdown. A typo here would silently bring
        // back the false "possible crash/kill" warning.
        assert_eq!(INSTALLER_SHUTDOWN_STATE, "installer-shutdown");
    }
}
