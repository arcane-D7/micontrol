//! Bridge from the main (unprivileged) process to the elevated helper task.
//!
//! Every privileged hardware operation is dispatched through here:
//!   1. Write a JSON command to `%LOCALAPPDATA%\MiControl\elev_cmd_<request_id>.json`
//!   2. Trigger the `MiControlElevated` scheduled task via `schtasks /run`
//!      (the task was created at install time with RunLevel = Highest,
//!       so it runs with administrator rights, no UAC prompt)
//!   3. Poll `%LOCALAPPDATA%\MiControl\elev_result_<request_id>.json` until it appears
//!      (timeout: 15 s)
//!   4. Return the `data` field on success, or `Err(error_message)`.
//!
//! **Dev-mode fallback**: when the scheduled task is absent (e.g. running
//! straight from `cargo tauri dev` without an installer), step 2 falls back to
//! `ShellExecuteExW` with verb "runas", which triggers a UAC prompt and runs
//! the current binary as `micontrol.exe --elevated --request-id <id>`.  This is intentionally
//! only a dev ergonomics aid; production always uses the scheduled task.

use crate::util::auth;
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Name of the scheduled task registered by the NSIS installer.
const TASK_NAME: &str = "MiControlElevated";
const POLL_INTERVAL_MS: u64 = 150;
const ELEV_TIMEOUT_SECS: u64 = 15;
const STALE_FILE_MAX_AGE_SECS: u64 = 120;
static ELEV_REQUEST_LOCK: Mutex<()> = Mutex::const_new(());
static NEXT_REQ: AtomicU64 = AtomicU64::new(1);

/// Dispatch a privileged command through the scheduled elevated task.
///
/// `cmd` must match one of the branches in `elevated::dispatch()`.
/// `args` is the JSON arguments object (use `serde_json::json!({...})`).
pub async fn run_elevated(cmd: &'static str, args: Value) -> Result<Value, String> {
    // Serialise elevated calls. The scheduled-task path has no request-id argv,
    // so the elevated helper discovers the newest pending file. Running one at a
    // time prevents cross-request mixups.
    let _guard = ELEV_REQUEST_LOCK.lock().await;

    // ── Fast path: already elevated ──────────────────────────────────────────
    // When this process is running as an administrator (dev mode or installed
    // with admin manifest), dispatch the privileged operation directly in a
    // blocking thread.  This eliminates the ~15 s scheduled-task round-trip.
    #[cfg(windows)]
    if is_admin() {
        let args2 = args.clone();
        return tokio::task::spawn_blocking(move || {
            let result = crate::elevated::dispatch_cmd(cmd, args2);
            if result["ok"].as_bool().unwrap_or(false) {
                Ok(result["data"].clone())
            } else {
                Err(result["error"]
                    .as_str()
                    .unwrap_or("elevated dispatch failed")
                    .to_string())
            }
        })
        .await
        .map_err(|e| format!("blocking task panicked: {e}"))?;
    }

    let dir = crate::elevated::elev_dir();
    cleanup_stale_elev_files(&dir);

    let request_id = make_request_id();
    let cmd_path = dir.join(cmd_file_name(&request_id));
    let result_path = dir.join(result_file_name(&request_id));
    let nonce = auth::generate_nonce();
    let mut payload = serde_json::json!({
        "protocol_version": 2,
        "request_id": request_id,
        "created_at_ms": auth::now_ms(),
        "nonce": nonce,
        "caller_pid": std::process::id(),
        "cmd": cmd,
        "args": args,
    });

    // Sign the payload with HMAC-SHA256 using the shared key.
    let key = auth::get_or_create_key().map_err(|e| format!("Cannot obtain HMAC key: {e}"))?;
    auth::sign_payload(&mut payload, &key);

    // Remove any stale result from a previous run for this request id.
    let _ = tokio::fs::remove_file(&result_path).await;

    // Write the command payload atomically: write to a temp file, then rename.
    // This eliminates the TOCTOU race — the elevated helper never sees a
    // partially-written file.
    let tmp_path = dir.join(format!("elev_cmd_{request_id}.tmp"));
    tokio::fs::write(&tmp_path, payload.to_string())
        .await
        .map_err(|e| format!("Cannot write elevated command: {e}"))?;
    tokio::fs::rename(&tmp_path, &cmd_path)
        .await
        .map_err(|e| format!("Cannot rename elevated command file: {e}"))?;
    auth::restrict_file_acl(&cmd_path);

    // Launch the scheduled task (returns immediately; task runs asynchronously).
    // Stdout/stderr are explicitly silenced — schtasks prints
    // "ERROR: The system cannot find the file specified." when the task is
    // absent (dev mode), which would otherwise pollute the console.
    let task_ok = tokio::process::Command::new("schtasks")
        .args(["/run", "/tn", TASK_NAME])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false);

    if !task_ok {
        // Fallback: in dev mode (or when the task is unregistered), launch the
        // current binary as administrator via ShellExecuteExW "runas" so that a
        // single UAC prompt lets us run `micontrol.exe --elevated`.
        #[cfg(windows)]
        {
            if let Err(e) = launch_elevated_via_uac(&request_id) {
                let _ = tokio::fs::remove_file(&cmd_path).await;
                return Err(format!(
                    "Scheduled task '{}' not found AND UAC fallback failed: {e}. \
                     Reinstall MiControl to register the scheduled task.",
                    TASK_NAME
                ));
            }
            // The UAC-elevated process is synchronous (we wait for it inside
            // launch_elevated_via_uac), so by the time we reach the poll loop
            // below the result file should already be there.
        }
        #[cfg(not(windows))]
        {
            let _ = tokio::fs::remove_file(&cmd_path).await;
            return Err(format!("Scheduled task '{TASK_NAME}' not found."));
        }
    }

    // Poll for the result file (check every 150 ms, timeout after 15 s)
    let timeout = Duration::from_secs(ELEV_TIMEOUT_SECS);
    let start = Instant::now();
    loop {
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;

        if result_path.exists() {
            let content = tokio::fs::read_to_string(&result_path)
                .await
                .map_err(|e| format!("Cannot read elevated result: {e}"))?;
            let _ = tokio::fs::remove_file(&result_path).await;
            let _ = tokio::fs::remove_file(&cmd_path).await;

            let mut v: Value =
                serde_json::from_str(&content).map_err(|e| format!("Invalid result JSON: {e}"))?;

            // Verify the response HMAC to detect tampering or spoofing.
            if let Err(e) = auth::verify_payload(&mut v, &key) {
                log::warn!("Elevated response HMAC verification failed: {e}");
                return Err(format!("Elevated response authentication failed: {e}"));
            }

            let result_req = v["request_id"].as_str().unwrap_or_default();
            if result_req != request_id {
                return Err(format!(
                    "Elevated result request_id mismatch (expected {}, got {})",
                    request_id, result_req
                ));
            }

            return if v["ok"].as_bool().unwrap_or(false) {
                Ok(v["data"].clone())
            } else {
                Err(v["error"]
                    .as_str()
                    .unwrap_or("elevated process failed")
                    .to_string())
            };
        }

        if start.elapsed() > timeout {
            // The scheduled task ran but produced no result.  This usually means
            // the task is registered without the `--elevated` argument (so the
            // full GUI launched instead of the helper).  Try the UAC fallback
            // as a self-healing one-shot before giving up.
            #[cfg(windows)]
            {
                // Re-write the command file in case the bad task process
                // consumed or deleted it.
                let _ = tokio::fs::write(&cmd_path, payload.to_string()).await;
                if let Err(e) = launch_elevated_via_uac(&request_id) {
                    let _ = tokio::fs::remove_file(&cmd_path).await;
                    return Err(format!(
                        "Elevated process timed out after 15 s and UAC fallback \
                         failed: {e}. Reinstall MiControl to fix the scheduled task."
                    ));
                }
                // UAC helper ran synchronously; result should be present now.
                if result_path.exists() {
                    let content = tokio::fs::read_to_string(&result_path)
                        .await
                        .map_err(|e| format!("Cannot read elevated result: {e}"))?;
                    let _ = tokio::fs::remove_file(&result_path).await;
                    let _ = tokio::fs::remove_file(&cmd_path).await;
                    let mut v: Value = serde_json::from_str(&content)
                        .map_err(|e| format!("Invalid result JSON: {e}"))?;

                    // Verify the response HMAC.
                    if let Err(e) = auth::verify_payload(&mut v, &key) {
                        log::warn!("Elevated response HMAC verification failed: {e}");
                        return Err(format!("Elevated response authentication failed: {e}"));
                    }

                    let result_req = v["request_id"].as_str().unwrap_or_default();
                    if result_req != request_id {
                        return Err(format!(
                            "Elevated result request_id mismatch (expected {}, got {})",
                            request_id, result_req
                        ));
                    }
                    return if v["ok"].as_bool().unwrap_or(false) {
                        Ok(v["data"].clone())
                    } else {
                        Err(v["error"]
                            .as_str()
                            .unwrap_or("elevated process failed")
                            .to_string())
                    };
                }
                return Err("Elevated process timed out after 15 s. UAC fallback ran \
                     but produced no result."
                    .to_string());
            }
            #[cfg(not(windows))]
            {
                let _ = tokio::fs::remove_file(&cmd_path).await;
                return Err(format!(
                    "Elevated process timed out after 15 s. \
                     Ensure the '{}' scheduled task is registered.",
                    TASK_NAME
                ));
            }
        }
    }
}

/// Launch the current executable as administrator using `ShellExecuteExW`
/// with verb `"runas"` and argument `"--elevated --request-id <id>"`.
///
/// Blocks until the spawned process exits (max 30 s).
/// Returns `Ok(())` if the process was launched successfully; the caller
/// must still poll for `elev_result.json`.
#[cfg(windows)]
fn launch_elevated_via_uac(request_id: &str) -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HWND};
    use windows::Win32::System::Threading::WaitForSingleObject;
    use windows::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };

    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let exe_str = exe.to_string_lossy().into_owned();

    let verb: Vec<u16> = OsStr::new("runas").encode_wide().chain(Some(0)).collect();
    let file: Vec<u16> = OsStr::new(&exe_str).encode_wide().chain(Some(0)).collect();
    let params_text = format!("--elevated --request-id {}", request_id);
    let params: Vec<u16> = OsStr::new(&params_text)
        .encode_wide()
        .chain(Some(0))
        .collect();

    unsafe {
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
            hwnd: HWND(std::ptr::null_mut()),
            lpVerb: PCWSTR(verb.as_ptr()),
            lpFile: PCWSTR(file.as_ptr()),
            lpParameters: PCWSTR(params.as_ptr()),
            nShow: 0, // SW_HIDE — no visible window
            ..std::mem::zeroed()
        };

        // SAFETY: ShellExecuteExW with SEE_MASK_NOCLOSEPROCESS launches the executable and returns a process handle. The verb ("runas"), file, and parameters are all valid null-terminated wide strings. hProcess is checked for validity before WaitForSingleObject/CloseHandle. zeroed() is safe for the remaining fields as cbSize is explicitly set and Windows ignores unspecified fields.
        ShellExecuteExW(&mut info).map_err(|e| format!("ShellExecuteExW: {e}"))?;

        if !info.hProcess.is_invalid() {
            // Wait up to 30 s for the elevated helper to finish writing its result
            WaitForSingleObject(info.hProcess, 30_000);
            let _ = CloseHandle(info.hProcess);
        }
    }
    Ok(())
}

/// Re-launch the current executable as administrator using `ShellExecuteExW` "runas".
///
/// Unlike [`launch_elevated_via_uac`] this function:
/// - does NOT pass `--elevated` to the new instance (normal startup)
/// - shows the new window (`SW_SHOWNORMAL`)
/// - does NOT wait for the new process to finish
///
/// After this returns the caller should call `app.exit(0)` to shut down the
/// current (non-elevated) instance and let the elevated instance take over.
#[cfg(windows)]
pub fn relaunch_self_as_admin() -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOASYNC, SHELLEXECUTEINFOW};

    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let exe_str = exe.to_string_lossy().into_owned();

    let verb: Vec<u16> = OsStr::new("runas").encode_wide().chain(Some(0)).collect();
    let file: Vec<u16> = OsStr::new(&exe_str).encode_wide().chain(Some(0)).collect();

    unsafe {
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOASYNC,
            hwnd: HWND(std::ptr::null_mut()),
            lpVerb: PCWSTR(verb.as_ptr()),
            lpFile: PCWSTR(file.as_ptr()),
            lpParameters: PCWSTR::null(),
            nShow: 1, // SW_SHOWNORMAL
            ..std::mem::zeroed()
        };

        // SAFETY: ShellExecuteExW with "runas" verb launches the process with elevation request. The verb, file, and parameters are valid null-terminated wide strings. zeroed() is safe for remaining fields since cbSize is explicitly set.
        ShellExecuteExW(&mut info).map_err(|e| format!("ShellExecuteExW: {e}"))?;
    }

    Ok(())
}

/// Returns true if the current process token has the Administrators group enabled
/// (i.e. the process is running elevated / as administrator).
#[cfg(windows)]
fn is_admin() -> bool {
    use windows::Win32::UI::Shell::IsUserAnAdmin;
    // SAFETY: IsUserAnAdmin() is a simple Win32 check with no safety invariants — it always succeeds and returns a BOOL.
    unsafe { IsUserAnAdmin().as_bool() }
}

fn make_request_id() -> String {
    let seq = NEXT_REQ.fetch_add(1, Ordering::Relaxed);
    format!("{:08x}-{:016x}-{:08x}", std::process::id(), now_ms(), seq)
}

fn cmd_file_name(request_id: &str) -> String {
    format!("elev_cmd_{request_id}.json")
}

fn result_file_name(request_id: &str) -> String {
    format!("elev_result_{request_id}.json")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn cleanup_stale_elev_files(dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let looks_like_elev_file = (name.starts_with("elev_cmd_")
            || name.starts_with("elev_result_"))
            && name.ends_with(".json");
        if !looks_like_elev_file {
            continue;
        }
        let is_stale = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|ts| now.duration_since(ts).ok())
            .map(|age| age.as_secs() >= STALE_FILE_MAX_AGE_SECS)
            .unwrap_or(false);
        if is_stale {
            let _ = std::fs::remove_file(path);
        }
    }
}
