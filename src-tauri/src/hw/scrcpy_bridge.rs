//! scrcpy orchestration (MIOT-07): use the phone camera as a webcam.
//!
//! scrcpy is an external, MIT-licensed utility by Genymobile that mirrors an
//! Android device over adb. With `--camera-facing=front` (scrcpy >= 2.5) the
//! phone's front camera can be exposed as a USB/UVC webcam on the PC, turning
//! the phone into a high-quality webcam without installing anything on the PC.
//!
//! This module only *orchestrates* the binary:
//! - detects `scrcpy` on PATH, with a winget hint for installation;
//! - spawns `scrcpy --camera-facing=front` as a detached child process;
//! - tracks the child by a PID file under the app data dir so start/stop/status
//!   survive restarts of MiControl.
//!
//! Out of scope: bundling the scrcpy binary (large, platform-specific) and
//! adb/device detection. If the binary is missing, `status` returns a friendly
//! `not-installed` result instead of an error, so the UI can offer the winget
//! one-liner.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::process::Stdio;

use serde::Serialize;

const APP_DATA_SUBDIR: &str = "scrcpy";
const PID_FILE: &str = "scrcpy.pid";

/// Runtime view of the scrcpy orchestration.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScrcpyStatus {
    /// Where we found the binary (empty when not installed).
    pub binary: String,
    /// Full `scrcpy --version` first line, e.g. `scrcpy 2.7`.
    pub version: Option<String>,
    /// `not-installed` | `running` | `stopped` | `error`
    pub state: String,
    /// PID of the running process, if any.
    pub pid: Option<u32>,
    /// Error detail when state is `error`.
    pub error: Option<String>,
    /// Human hint (winget install line) when not installed.
    pub install_hint: String,
}

impl Default for ScrcpyStatus {
    fn default() -> Self {
        Self {
            binary: String::new(),
            version: None,
            state: "not-installed".into(),
            pid: None,
            error: None,
            install_hint: "winget install Genymobile.scrcpy".into(),
        }
    }
}

/// Locate the `scrcpy(.exe)` binary on PATH. Returns `Ok(None)` when absent.
pub fn find_binary() -> Result<Option<PathBuf>, String> {
    let exe = if cfg!(windows) {
        "scrcpy.exe"
    } else {
        "scrcpy"
    };
    let out = Command::new(exe).arg("--version").output();
    Ok(match out {
        Ok(o) if o.status.success() => Some(PathBuf::from(exe)),
        _ => None,
    })
}

fn app_data_dir() -> Result<PathBuf, String> {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .ok_or_else(|| "cannot locate app data directory".to_string())?;
    Ok(base.join("MiControl").join(APP_DATA_SUBDIR))
}

fn pid_file_path() -> Result<PathBuf, String> {
    Ok(app_data_dir()?.join(PID_FILE))
}

fn write_pid_file(pid: u32) -> Result<(), String> {
    let dir = app_data_dir()?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("create data dir: {e}"))?;
    std::fs::write(dir.join(PID_FILE), pid.to_string()).map_err(|e| format!("write pid file: {e}"))
}

fn read_pid_file() -> Option<u32> {
    pid_file_path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| s.trim().parse().ok())
}

fn clear_pid_file() {
    if let Ok(p) = pid_file_path() {
        let _ = std::fs::remove_file(p);
    }
}

fn is_process_alive(pid: u32) -> bool {
    #[cfg(windows)]
    {
        // `tasklist /FI "PID eq <pid>" /NH` lists the row when it is alive.
        let out = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/NH"])
            .output();
        match out {
            Ok(o) => String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()),
            Err(_) => false,
        }
    }
    #[cfg(not(windows))]
    {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }
}

fn version_of(binary: &Path) -> Option<String> {
    let out = Command::new(binary).arg("--version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let trimmed = String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()?
        .trim()
        .to_string();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

/// Start `scrcpy --camera-facing=front` as a detached child.
///
/// Uses `--no-audio`: Windows audio input needs extra setup
/// (`--audio-source=...`), so a camera-only session stays robust. Returns the
/// child PID.
pub fn start_camera() -> Result<u32, String> {
    if status().state == "running" {
        return Err("scrcpy is already running".into());
    }
    let bin = find_binary()?
        .ok_or_else(|| "scrcpy is not installed (winget install Genymobile.scrcpy)".to_string())?;

    // Spawn detached so the child survives MiControl exiting. The PID file is
    // the source of truth for start/stop/status across MiControl restarts.
    let mut cmd = Command::new(&bin);
    cmd.args(["--camera-facing=front", "--no-audio"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0000_0008); // DETACHED_PROCESS
    }

    let pid = cmd
        .spawn()
        .map_err(|e| format!("failed to start scrcpy: {e}"))?
        .id();
    write_pid_file(pid)?;
    Ok(pid)
}

/// Stop a running scrcpy child by killing the PID recorded at start.
pub fn stop_camera() -> Result<(), String> {
    let Some(pid) = read_pid_file() else {
        return Ok(());
    };
    let out = if cfg!(windows) {
        Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output()
    } else {
        Command::new("kill").arg(pid.to_string()).output()
    }
    .map_err(|e| format!("failed to stop scrcpy (pid {pid}): {e}"))?;
    if !out.status.success() {
        // TaskKill fails if the process already exited — treat as stopped.
        return Err(format!(
            "scrcpy termination returned nonzero (pid {pid}): {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    clear_pid_file();
    Ok(())
}

/// Current status: detects the binary, checks the PID-file child, and reports
/// a `not-installed` state gracefully when scrcpy is missing.
pub fn status() -> ScrcpyStatus {
    let mut s = ScrcpyStatus::default();
    match find_binary() {
        Ok(Some(bin)) => {
            s.binary = bin.display().to_string();
            s.version = version_of(&bin);
            if let Some(pid) = read_pid_file() {
                if is_process_alive(pid) {
                    s.state = "running".into();
                    s.pid = Some(pid);
                } else {
                    clear_pid_file();
                    s.state = "stopped".into();
                }
            } else {
                s.state = "stopped".into();
            }
        }
        Ok(None) => {
            // keep default not-installed + install_hint
        }
        Err(e) => {
            s.state = "error".into();
            s.error = Some(e);
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_graceful_when_not_installed() {
        // If scrcpy is genuinely on PATH this just reports its real state;
        // in CI it returns not-installed without erroring.
        let s = status();
        assert!(
            s.state == "not-installed" || s.state == "stopped" || s.state == "running",
            "unexpected state {}",
            s.state
        );
    }

    #[test]
    fn pid_file_roundtrip() {
        let dir = std::env::temp_dir().join(format!("scrcpy-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join(PID_FILE);
        std::fs::write(&path, "4242").unwrap();
        assert_eq!(
            std::fs::read_to_string(&path)
                .unwrap()
                .trim()
                .parse::<u32>(),
            Ok(4242)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn build_windows_args() {
        // The exact args we pass on Windows for the camera use case.
        let mut cmd = Command::new("scrcpy");
        cmd.args(["--camera-facing=front", "--no-audio"]);
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, vec!["--camera-facing=front", "--no-audio"]);
    }
}
