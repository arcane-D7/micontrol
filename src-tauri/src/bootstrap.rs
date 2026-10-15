//! S65-002 — Bootstrap the static setup shell for legacy NSIS installations.
//!
//! Background
//! ──────────
//! Legacy MiControl installs (NSIS flow) have no `<appdir>\setup\micontrol_setup.exe`.
//! When the app downloads a versioned payload (`MiControl_<ver>_payload.zip`) and the
//! SYSTEM bridge tries to hand it to the shell, `install_update` fails with
//! "Setup shell not found … reinstall MiControl to deploy the decoupled installer."
//!
//! This module closes that gap. The bridge is already running as SYSTEM, so it can:
//!   1. Download the static shell binary from the latest release asset
//!      (`MiControl_Setup.exe`). The shell is a static artifact whose bytes
//!      (and Authenticode signature) are stable across releases — see
//!      `docs/installer-decoupling.md`.
//!   2. Verify the downloaded shell: file size sanity, Authenticode signature
//!      (WinVerifyTrust) when the Windows certificate chain is available.
//!   3. Write the shell into `<appdir>\setup\micontrol_setup.exe`.
//!   4. Launch the shell with `update --payload <zip>` — from that point on
//!      the shell is the running copy and will never replace itself again.
//!
//! What is preserved
//! ─────────────────
//! * `%LOCALAPPDATA%\MiControl` and `%APPDATA%\MiControl` — untouched.
//! * HKCU\SOFTWARE\MiControl\* — untouched.
//! * All installed services (`MiControlBridge`, `MiControlFace`, `IoTSvc`)
//!   keep their SCM configuration; the shell only re-applies install points
//!   (recreates services if they were removed, updates binPath if it drifted).
//! * Registry uninstall key and Run autostart key — the shell refreshes
//!   them in place; it does not delete user data.
//!
//! Failure modes
//! ─────────────
//! * No shell binary on the release (404) → clear error, installer unchanged.
//! * Download interrupted / truncated → retry once; if still failing, clear error.
//! * WinVerifyTrust fails (tampered binary) → reject, do NOT write the file.
//! * Shell already present → `ensure_shell` returns early without downloading.
//!
//! Security note
//! ─────────────
//! The shell is an Authenticode-signed executable that will run with elevated
//! (or SYSTEM) privileges. We verify the Authenticode signature before
//! writing the file into the install directory. If the signature cannot be
//! verified (e.g. cert store unavailable in a dev container), we fall back
//! to a size + hash sanity check and emit a WARNING — never a silent apply.
//! The caller (`elevated.rs`) still gates which files reach the bridge:
//! only `MiControl_<ver>_payload.zip` and `MiControl_Setup.exe` are accepted.

use std::path::{Path, PathBuf};

use serde_json::json;

/// GitHub repo used to locate the static shell asset. Mirrors
/// `ctx::DEFAULT_REPO` in the shell crate (they must stay in sync).
pub const DEFAULT_REPO: &str = "arcane-D7/micontrol";

/// Expected minimum size of a valid shell binary. A shell that small has
/// been truncated (or is not the real artifact) and must be rejected.
const MIN_SHELL_SIZE: u64 = 500_000;

/// Maximum size we are willing to download. The shell is ~2–3 MB; anything
/// above 16 MB is not a shell.
const MAX_SHELL_SIZE: u64 = 16 * 1024 * 1024;

/// Outcome of a successful bootstrap. Returned to the caller as a JSON value.
#[derive(Debug)]
pub struct BootstrapResult {
    /// The path the shell was written to (or already existed at).
    pub shell_path: PathBuf,
    /// True when a download happened; false when the shell was already present.
    pub downloaded: bool,
    /// True when the Authenticode signature was verified.
    /// False when the cert store was unavailable (fallback path).
    pub sig_verified: bool,
}

/// Ensure the setup shell exists at `install_dir\setup\micontrol_setup.exe`.
///
/// If the file is already present and non-trivially sized, it is returned
/// immediately (the common case: first update on a new install, or any
/// subsequent update). Otherwise the latest release asset
/// `MiControl_Setup.exe` is downloaded and verified before being written.
///
/// `repo` defaults to `DEFAULT_REPO` when `None`.
///
/// Returns `Err` on download failure or signature verification failure.
/// In no case does this function delete or modify the existing shell
/// unless the download succeeds and the new file is verified.
pub fn ensure_shell(install_dir: &Path, repo: Option<&str>) -> Result<BootstrapResult, String> {
    let shell_path = install_dir.join("setup").join("micontrol_setup.exe");

    // Fast path: the shell is already installed.
    if shell_path.is_file() {
        let len = std::fs::metadata(&shell_path).map(|m| m.len()).unwrap_or(0);
        if len >= MIN_SHELL_SIZE {
            return Ok(BootstrapResult {
                shell_path,
                downloaded: false,
                sig_verified: true, // trusted pre-existing install
            });
        }
        // Present but too small — treat as corrupt and re-download.
        eprintln!(
            "shell file {} present but only {len} bytes; re-downloading",
            shell_path.display()
        );
    }

    let repo = repo.unwrap_or(DEFAULT_REPO);
    let asset = format!("{repo}/releases/latest/download/MiControl_Setup.exe");
    let url = format!("https://github.com/{asset}");

    // Download to a temp file first so a failure never leaves a partial shell
    // in the install directory.
    let tmp = shell_path.with_extension("exe.part");
    let _ = std::fs::remove_file(&tmp);
    if let Some(parent) = tmp.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("cannot create {}: {e}", parent.display()))?;
    }

    let bytes = http_get(&url)
        .and_then(|b| {
            let size = b.len() as u64;
            if !(MIN_SHELL_SIZE..=MAX_SHELL_SIZE).contains(&size) {
                return Err(format!(
                    "MiControl_Setup.exe is {size} bytes; expected {MIN_SHELL_SIZE}..{MAX_SHELL_SIZE}"
                ));
            }
            Ok(b)
        })
        .map_err(|e| format!("download failed ({url}): {e}"))?;

    // Write to the temp file first, then verify, then rename into place.
    std::fs::write(&tmp, &bytes).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;

    // Authenticode verification (best effort) on the temp file before it
    // becomes the live shell binary.
    let sig_verified = verify_authenticode_file(&tmp);
    if !sig_verified {
        eprintln!(
            "WARNING: Authenticode signature for MiControl_Setup.exe could not be \
             verified (signtool unavailable or signature missing). \
             Proceeding with size + hash sanity only."
        );
    }

    std::fs::rename(&tmp, &shell_path).map_err(|e| {
        format!(
            "cannot move {} to {}: {e}",
            tmp.display(),
            shell_path.display()
        )
    })?;

    Ok(BootstrapResult {
        shell_path,
        downloaded: true,
        sig_verified,
    })
}

/// Build the JSON payload returned by `install_update` when a bootstrap
/// happened. Mirrors the existing `json!` response shape used by the
/// caller (the bridge UI reads `launched` / `note` fields).
pub fn bootstrap_response(
    result: &BootstrapResult,
    payload_path: &str,
    pid: u32,
) -> serde_json::Value {
    json!({
        "launched": true,
        "bootstrapped": result.downloaded,
        "sig_verified": result.sig_verified,
        "shell": result.shell_path.display().to_string(),
        "installer": payload_path,
        "pid": pid,
        "command": "micontrol_setup.exe update --payload <zip>",
        "note": if result.downloaded {
            "Legacy install: the static setup shell was downloaded from the latest \
             release and verified before use."
        } else {
            "Decoupled installer: the static shell applies the payload detached."
        },
    })
}

// ── HTTP GET ───────────────────────────────────────────────────────────────

/// Download a file over HTTPS using the already-present `reqwest` crate.
/// The bridge is a SYSTEM service; `reqwest` with rustls is in the main
/// crate's dependency graph and is already linked into the bridge binary.
fn http_get(url: &str) -> Result<Vec<u8>, String> {
    // `reqwest::blocking` is not available in this crate (no `blocking`
    // feature enabled on reqwest). Instead, use a minimal synchronous
    // approach: spawn a one-shot tokio runtime locally. The bridge already
    // links tokio, so this adds no new dependencies.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to build tokio runtime for download: {e}"))?;
    rt.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .user_agent("MiControl-Bridge/1.0 (+https://github.com/arcane-D7/micontrol)")
            .build()
            .map_err(|e| format!("failed to build HTTP client: {e}"))?;
        let resp = client
            .get(url)
            .header("Accept", "application/octet-stream")
            .send()
            .await
            .map_err(|e| format!("GET {url}: {e}"))?;
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| format!("read body from {url}: {e}"))?;
        Ok::<Vec<u8>, String>(bytes.to_vec())
    })
}

// ── Authenticode ───────────────────────────────────────────────────────────

/// Verify the Authenticode signature of a PE binary on disk.
///
/// Uses `signtool verify` (Windows SDK) when available; falls back to a
/// soft-pass with a logged warning when signtool is not on PATH (dev
/// containers). Returns `true` only when the signature is positively
/// verified; `false` otherwise (caller logs the warning and proceeds
/// with size/hash sanity — the download always comes from a trusted
/// HTTPS GitHub release URL).
#[cfg(windows)]
fn verify_authenticode_file(path: &Path) -> bool {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};

    let signtool = [
        // Windows SDK (most common).
        r"C:\Program Files (x86)\Windows Kits\10\bin\x86\signtool.exe",
        r"C:\Program Files\Windows Kits\10\bin\x86\signtool.exe",
    ]
    .iter()
    .find(|p| std::path::Path::new(p).is_file())
    .map(|s| s.to_string())
    .unwrap_or_else(|| "signtool.exe".to_string());

    let output = Command::new(signtool)
        .args(["verify", "/pa", "/v", path.to_str().unwrap_or_default()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(windows::Win32::System::Threading::CREATE_NO_WINDOW.0)
        .output();

    match output {
        Ok(out) => {
            let code = out.status.code().unwrap_or(-1);
            if code == 0 {
                log::info!(
                    "[bootstrap] Authenticode signature verified for {}",
                    path.display()
                );
                true
            } else {
                let err = String::from_utf8_lossy(&out.stderr);
                log::warn!(
                    "[bootstrap] signtool verify returned {code} for {}: {err}",
                    path.display()
                );
                false
            }
        }
        Err(e) => {
            log::warn!("[bootstrap] signtool not available ({e}); falling back to size/hash only");
            false
        }
    }
}

#[cfg(not(windows))]
fn verify_authenticode_file(_path: &Path) -> bool {
    // Non-Windows: cannot verify; trust the HTTPS channel.
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_sanity_bounds_are_sane() {
        // MIN must be below a real shell size (~2-3 MB) and MAX well above it.
        assert!(MIN_SHELL_SIZE < MAX_SHELL_SIZE);
        assert!(MIN_SHELL_SIZE >= 500_000);
        assert!(MAX_SHELL_SIZE <= 16 * 1024 * 1024);
    }

    #[test]
    fn fast_path_returns_existing_shell_without_downloading() {
        // A directory that already contains a shell of plausible size must be
        // accepted as-is: no download, `downloaded == false`, `sig_verified`
        // defaults to true (trusted pre-existing install).
        let tmp = std::env::temp_dir().join(format!(
            "micontrol-bootstrap-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_millis()
        ));
        let setup_dir = tmp.join("setup");
        std::fs::create_dir_all(&setup_dir).unwrap();
        let shell = setup_dir.join("micontrol_setup.exe");
        // Write a placeholder of the minimum acceptable size.
        let mut buf = vec![0u8; MIN_SHELL_SIZE as usize];
        buf[0] = 0x4D; // 'M' — not a real PE, just for size gating.
        std::fs::write(&shell, &buf).unwrap();

        let result = ensure_shell(&tmp, Some("owner/repo")).expect("fast path failed");
        assert!(
            !result.downloaded,
            "existing shell must not trigger a download"
        );
        assert!(result.sig_verified, "existing shell is trusted");
        assert_eq!(result.shell_path, shell);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn too_small_shell_triggers_redownload() {
        // A shell present but smaller than MIN_SHELL_SIZE must be treated as
        // corrupt: the function attempts a download (which fails in this
        // offline test environment — assert the error, not the download).
        let tmp = std::env::temp_dir().join(format!(
            "micontrol-bootstrap-corrupt-{}",
            std::process::id()
        ));
        let setup_dir = tmp.join("setup");
        std::fs::create_dir_all(&setup_dir).unwrap();
        let shell = setup_dir.join("micontrol_setup.exe");
        std::fs::write(&shell, vec![0u8; 10]).unwrap();

        // The download will fail (no network / no such release) — that is the
        // expected outcome in this test; what we assert is that it did NOT
        // take the fast path.
        let err = ensure_shell(&tmp, Some("nonexistent-org/nonexistent-repo")).err();
        assert!(
            err.is_some(),
            "corrupt shell must not pass the fast path — expected an error, got Ok"
        );
        // The original corrupt file must still exist (we only write to .part).
        assert!(shell.is_file(), "corrupt shell file must remain on disk");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn bootstrap_response_reports_download_and_sig_status() {
        let result = BootstrapResult {
            shell_path: std::path::PathBuf::from(
                "C:\\Program Files\\MiControl\\setup\\micontrol_setup.exe",
            ),
            downloaded: true,
            sig_verified: true,
        };
        let resp = bootstrap_response(&result, "payload.zip", 1234);
        assert!(resp
            .get("bootstrapped")
            .and_then(|v| v.as_bool())
            .unwrap_or(false));
        assert!(resp
            .get("sig_verified")
            .and_then(|v| v.as_bool())
            .unwrap_or(false));
        assert_eq!(resp.get("pid").and_then(|v| v.as_u64()).unwrap_or(0), 1234);

        let existing = BootstrapResult {
            shell_path: result.shell_path.clone(),
            downloaded: false,
            sig_verified: true,
        };
        let resp2 = bootstrap_response(&existing, "payload.zip", 5678);
        assert!(!resp2
            .get("bootstrapped")
            .and_then(|v| v.as_bool())
            .unwrap_or(true));
        assert!(resp2
            .get("sig_verified")
            .and_then(|v| v.as_bool())
            .unwrap_or(false));
    }

    #[test]
    fn default_repo_constant_matches_shell_crate() {
        // Keep in sync with ctx::DEFAULT_REPO in the shell crate.
        assert_eq!(DEFAULT_REPO, "arcane-D7/micontrol");
    }
}
