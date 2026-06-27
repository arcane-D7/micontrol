//! Bridge from the main (unprivileged) process to the elevated helper task.
//!
//! Every privileged hardware operation is dispatched through here:
//!   1. Write a JSON command to `%LOCALAPPDATA%\MiControl\elev_cmd_<request_id>.json`
//!   2. Trigger the `MiControlElevated` scheduled task via `schtasks /run`
//!      (the task was created at install time with RunLevel = Highest,
//!      so it runs with administrator rights, no UAC prompt)
//!   3. Poll `%LOCALAPPDATA%\MiControl\elev_result_<request_id>.json` until it appears
//!      (timeout: 15 s)
//!   4. Return the `data` field on success, or `Err(error_message)`.
//!
//! **Dev-mode fallback**: when the scheduled task is absent (e.g. running
//! straight from `cargo tauri dev` without an installer), step 2 falls back to
//! `ShellExecuteExW` with verb "runas", which triggers a UAC prompt and runs
//! the current binary as `micontrol.exe --elevated --request-id <id>`.
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
    // S26-006: Wrap in spawn_blocking — cleanup_stale_elev_files() uses std::fs::read_dir.
    let dir_clone = dir.clone();
    tokio::task::spawn_blocking(move || cleanup_stale_elev_files(&dir_clone))
        .await
        .map_err(|e| format!("cleanup_stale_elev_files task panicked: {e}"))?;

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
    // S22-002: Wrap in spawn_blocking — get_or_create_key() does sync file I/O
    // with a 5-second polling loop, which would block the async runtime.
    let key = tokio::task::spawn_blocking(auth::get_or_create_key)
        .await
        .map_err(|e| format!("HMAC key task panicked: {e}"))?
        .map_err(|e| format!("Cannot obtain HMAC key: {e}"))?;
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
    if let Err(e) = auth::restrict_file_acl(&cmd_path) {
        log::warn!("Failed to restrict ACL on command file: {e}");
    }

    // Launch the scheduled task (returns immediately; task runs asynchronously).
    // CREATE_NO_WINDOW prevents the flash of a console window on every call.
    let task_ok = run_schtasks_run().await;

    if !task_ok {
        // Self-healing: try to re-register the scheduled task with the correct
        // path before falling back to UAC. This fixes the case where the task
        // was registered during `cargo tauri dev` and points to the debug exe.
        let healed = tokio::task::spawn_blocking(ensure_task_correct_path)
            .await
            .map_err(|e| format!("task heal task panicked: {e}"))?
            || false;

        if healed {
            // Retry the task after healing.
            let retry_ok = run_schtasks_run().await;
            if retry_ok {
                // Task ran successfully after healing — skip UAC fallback,
                // fall through to the polling loop below.
            } else {
                log::warn!("Scheduled task still failed after self-healing, falling back to UAC");
                launch_uac_fallback(&request_id, &cmd_path).await?;
            }
        } else {
            // Healing failed (not admin or other error) — fall back to UAC.
            launch_uac_fallback(&request_id, &cmd_path).await?;
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
                // S26-005: Wrap in spawn_blocking — launch_elevated_via_uac() blocks
                // for up to 30 s via WaitForSingleObject.
                let req_id_owned = request_id.clone();
                let uac_result =
                    tokio::task::spawn_blocking(move || launch_elevated_via_uac(&req_id_owned))
                        .await
                        .map_err(|e| format!("UAC timeout fallback task panicked: {e}"))?;
                if let Err(e) = uac_result {
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

/// Run `schtasks /run /tn MiControlElevated` with CREATE_NO_WINDOW to avoid
/// flashing a console window on every elevated operation.
async fn run_schtasks_run() -> bool {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        tokio::process::Command::new("schtasks")
            .args(["/run", "/tn", TASK_NAME])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// UAC fallback: launch the current binary as administrator via ShellExecuteExW
/// "runas" so that a single UAC prompt lets us run `micontrol.exe --elevated`.
async fn launch_uac_fallback(request_id: &str, cmd_path: &std::path::Path) -> Result<(), String> {
    #[cfg(windows)]
    {
        let req_id_owned = request_id.to_string();
        let uac_result =
            tokio::task::spawn_blocking(move || launch_elevated_via_uac(&req_id_owned))
                .await
                .map_err(|e| format!("UAC launch task panicked: {e}"))?;

        if let Err(e) = uac_result {
            let _ = tokio::fs::remove_file(cmd_path).await;
            return Err(format!(
                "Scheduled task '{}' not found AND UAC fallback failed: {e}. \
                 Reinstall MiControl to register the scheduled task.",
                TASK_NAME
            ));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = tokio::fs::remove_file(cmd_path).await;
        Err(format!("Scheduled task '{TASK_NAME}' not found."))
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

/// Check if the scheduled task exists and points to the current executable.
/// If the task is missing or points to a different path (e.g. debug exe from
/// `cargo tauri dev`), re-register it with the correct path.
///
/// Tries non-elevated `schtasks` first. If that fails (Access Denied), falls
/// back to `ShellExecuteExW "runas"` to elevate just the schtasks command.
///
/// Returns `true` if the task was (re-)registered successfully.
#[cfg(windows)]
fn ensure_task_correct_path() -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    // Get the current executable path.
    let current_exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            log::warn!("Cannot get current exe path for task healing: {e}");
            return false;
        }
    };
    let current_path = current_exe.to_string_lossy().to_string();

    // Query the existing task's action path.
    let output = std::process::Command::new("schtasks")
        .args(["/query", "/tn", TASK_NAME, "/xml"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    let need_reregister = match output {
        Ok(out) => {
            let xml = String::from_utf8_lossy(&out.stdout);
            // Check if the task points to the current exe.
            let path_matches =
                xml.contains(&current_path) || xml.contains(&current_path.replace('\\', "/"));
            if path_matches {
                false
            } else {
                log::info!(
                    "Scheduled task points to wrong path, re-registering with: {current_path}"
                );
                true
            }
        }
        Err(_) => {
            log::info!("Scheduled task not found, registering with: {current_path}");
            true
        }
    };

    if !need_reregister {
        return false;
    }

    // Build the task XML with the correct path.
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-16"?><Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task"><Triggers><TimeTrigger><StartBoundary>2000-01-01T00:00:00</StartBoundary><Enabled>false</Enabled></TimeTrigger></Triggers><Principals><Principal id="Author"><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals><Settings><MultipleInstancesPolicy>StopExisting</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT30S</ExecutionTimeLimit><Enabled>true</Enabled></Settings><Actions Context="Author"><Exec><Command>"{current_path}"</Command><Arguments>--elevated</Arguments></Exec></Actions></Task>"#
    );

    let temp_dir = std::env::temp_dir();
    let xml_path = temp_dir.join("MCElev_heal.xml");
    if let Err(e) = std::fs::write(&xml_path, &xml) {
        log::warn!("Cannot write task XML for healing: {e}");
        return false;
    }
    let xml_str = xml_path.to_string_lossy().to_string();

    // Try 1: non-elevated schtasks (works if user has rights or is already admin)
    let _ = std::process::Command::new("schtasks")
        .args(["/delete", "/tn", TASK_NAME, "/f"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .status();

    let create_ok = std::process::Command::new("schtasks")
        .args(["/create", "/tn", TASK_NAME, "/xml", &xml_str, "/f"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    let success = match create_ok {
        Ok(out) if out.status.success() => true,
        _ => {
            // Try 2: elevated schtasks via UAC prompt
            log::info!("Non-elevated schtasks failed, trying UAC elevation...");
            let xml_path_owned = xml_str.clone();
            let uac_result = std::thread::spawn(move || run_schtasks_elevated(&xml_path_owned))
                .join()
                .unwrap_or(false);
            uac_result
        }
    };

    let _ = std::fs::remove_file(&xml_path);

    if success {
        log::info!("Scheduled task re-registered successfully with correct path");
    } else {
        log::warn!("Failed to re-register scheduled task (UAC may have been declined)");
    }
    success
}

/// Run `schtasks /delete` + `schtasks /create` elevated via ShellExecuteExW "runas".
/// Shows a single UAC prompt to the user.
#[cfg(windows)]
fn run_schtasks_elevated(xml_path: &str) -> bool {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HWND};
    use windows::Win32::System::Threading::WaitForSingleObject;
    use windows::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };

    // Build a batch script that deletes + creates the task, then signals completion.
    let script = format!(
        r#"@echo off
schtasks /delete /tn "MiControlElevated" /f 2>nul
schtasks /create /tn "MiControlElevated" /xml "{xml_path}" /f
exit /b %ERRORLEVEL%"#
    );

    let temp_dir = std::env::temp_dir();
    let bat_path = temp_dir.join("MCElev_heal.bat");
    if let Err(e) = std::fs::write(&bat_path, &script) {
        log::warn!("Cannot write healing batch script: {e}");
        return false;
    }

    let bat_str = bat_path.to_string_lossy().to_string();
    let verb: Vec<u16> = OsStr::new("runas").encode_wide().chain(Some(0)).collect();
    let file: Vec<u16> = OsStr::new(&bat_str).encode_wide().chain(Some(0)).collect();

    let result = unsafe {
        let mut info = SHELLEXECUTEINFOW {
            cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
            hwnd: HWND(std::ptr::null_mut()),
            lpVerb: PCWSTR(verb.as_ptr()),
            lpFile: PCWSTR(file.as_ptr()),
            lpParameters: PCWSTR::null(),
            nShow: 0, // SW_HIDE
            ..std::mem::zeroed()
        };

        // SAFETY: ShellExecuteExW with "runas" verb launches the batch script
        // elevated. The verb and file are valid null-terminated wide strings.
        if let Err(e) = ShellExecuteExW(&mut info) {
            log::warn!("ShellExecuteExW for task healing failed: {e}");
            return false;
        }

        if !info.hProcess.is_invalid() {
            WaitForSingleObject(info.hProcess, 30_000);
            let _ = CloseHandle(info.hProcess);
        }
        true
    };

    let _ = std::fs::remove_file(&bat_path);
    result
}

#[cfg(not(windows))]
fn ensure_task_correct_path() -> bool {
    false
}
