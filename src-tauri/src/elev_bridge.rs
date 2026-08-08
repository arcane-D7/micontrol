//! Bridge from the main (unprivileged) process to the elevated helper task.
//!
//! Every privileged hardware operation is dispatched through here:
//!   0. Try the autonomous `MiControlBridge` Windows service via named pipe
//!      `\\.\pipe\micontrol_bridge` (installed at install time, runs as
//!      `NT AUTHORITY\SYSTEM` — NO UAC prompt ever, even after reboot).
//!   1. If the service is not installed/running, fall back to the
//!      `MiControlElevated` scheduled task: write a JSON command to
//!      `%LOCALAPPDATA%\MiControl\elev_cmd_<request_id>.json`, trigger the task
//!      via `schtasks /run`, poll the result file.
//!   2. As a last resort (dev mode / no installer), fall back to a UAC prompt.
//!
//! The service pipe is the preferred, fully-autonomous path: after
//! installation the app NEVER prompts for elevation. The scheduled task and
//! UAC paths remain only for development and for self-healing a broken install.

use crate::util::auth;
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::Mutex;

/// Name of the scheduled task registered by the NSIS installer.
const TASK_NAME: &str = "MiControlElevated";
/// Named pipe of the autonomous elevated service installed at install time.
const BRIDGE_PIPE_NAME: &str = r"\\.\pipe\micontrol_bridge";
const POLL_INTERVAL_MS: u64 = 150;
const ELEV_TIMEOUT_SECS: u64 = 15;
const STALE_FILE_MAX_AGE_SECS: u64 = 120;
static ELEV_REQUEST_LOCK: Mutex<()> = Mutex::const_new(());

/// Returns `true` when the autonomous `MiControlBridge` service pipe is
/// currently available (i.e. the service is installed and running).
///
/// Cheap check — a single `CreateFileW` on the pipe. Used at startup to
/// decide whether to install the service.
pub fn is_bridge_service_available() -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::{
            CloseHandle, GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE,
        };
        use windows::Win32::Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        };

        let path_w: Vec<u16> = std::ffi::OsStr::new(BRIDGE_PIPE_NAME)
            .encode_wide()
            .chain(Some(0))
            .collect();
        // SAFETY: path_w is a valid null-terminated wide string.
        let handle = unsafe {
            CreateFileW(
                PCWSTR(path_w.as_ptr()),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                windows::Win32::Foundation::HANDLE::default(),
            )
        };
        match handle {
            Ok(h) if h != INVALID_HANDLE_VALUE => {
                unsafe {
                    CloseHandle(h).ok();
                }
                true
            }
            _ => false,
        }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Ensure the autonomous `MiControlBridge` service is installed and running.
///
/// If the service pipe is already available, returns immediately. Otherwise
/// dispatches `ensure_bridge_service` through the elevated path (scheduled
/// task, which is already elevated — no UAC). Callers typically invoke this
/// once at app startup and log the result.
///
/// IMPORTANT: This never shows a UAC prompt. If the scheduled task is also
/// unavailable, it returns an error and the app simply falls back to the
/// scheduled-task path for individual commands — a UAC popup at boot (the
/// classic "do you want to install this driver?" experience) is exactly what
/// the autonomous-bridge design is meant to prevent.
pub async fn ensure_bridge_service() -> Result<Value, String> {
    if is_bridge_service_available() {
        return Ok(serde_json::json!({ "status": "already_running" }));
    }
    log::info!("MiControlBridge service not available — installing via elevated path");
    run_elevated_no_prompt("ensure_bridge_service", serde_json::json!({})).await
}
static NEXT_REQ: AtomicU64 = AtomicU64::new(1);

/// Outcome of a task-path self-healing attempt.
enum TaskHealResult {
    /// Task already exists and points to the current exe — no action needed.
    AlreadyCorrect,
    /// Task was missing or mis-pointed and has been re-registered successfully.
    Healed,
    /// Healing was attempted but failed (not admin, UAC declined, etc.).
    Failed,
}

/// Timeout for slow commands that do WMI/IOCTL probes or driver installs.
const ELEV_TIMEOUT_SLOW_SECS: u64 = 90;
/// Timeout for medium commands (WMI queries on cold start).
const ELEV_TIMEOUT_MEDIUM_SECS: u64 = 45;

/// Returns the timeout for a given elevated command.
///
/// Slow commands (hardware discovery, driver install) do WMI + IOCTL probes
/// or run `pnputil`, which can take 30–60 s on a cold system. The task
/// scheduler's `ExecutionTimeLimit` is PT120S, so the bridge timeout must be
/// shorter than that to avoid waiting for a killed helper.
fn timeout_for_cmd(cmd: &str) -> Duration {
    match cmd {
        "run_hardware_discovery" | "install_driver" => Duration::from_secs(ELEV_TIMEOUT_SLOW_SECS),
        "wmi_ec_read_sensor_data"
        | "wmi_ec_read_battery_health"
        | "wmi_ec_read_adapter_power"
        | "wmi_ec_get_performance_mode"
        | "diag_wmi_query"
        | "set_battery_care"
        | "set_eye_protection"
        | "set_os_turbo"
        | "set_function_key"
        | "set_mic_noise_canceling"
        | "set_speaker_noise_canceling"
        | "set_voice_focus" => Duration::from_secs(ELEV_TIMEOUT_MEDIUM_SECS),

        "clean_junk_files" => Duration::from_secs(ELEV_TIMEOUT_SLOW_SECS),
        _ => Duration::from_secs(ELEV_TIMEOUT_SECS),
    }
}

/// Dispatch a privileged command through the autonomous elevated service,
/// the scheduled elevated task, or (last resort) a UAC prompt.
///
/// `cmd` must match one of the branches in `elevated::dispatch()`.
/// `args` is the JSON arguments object (use `serde_json::json!({...})`).
pub async fn run_elevated(cmd: &'static str, args: Value) -> Result<Value, String> {
    run_elevated_impl(cmd, args, true).await
}

/// Dispatch a privileged command WITHOUT ever showing a UAC prompt.
///
/// Uses the same bridge-first / scheduled-task fallback chain as
/// [`run_elevated`], but if no path succeeds it returns an error instead of
/// popping the elevation consent dialog. Use this for non-critical reads
/// (e.g. thermal/temperature polling at startup) where a UAC prompt at boot
/// would be a terrible user experience — the worst case is a missing value.
pub async fn run_elevated_no_prompt(cmd: &'static str, args: Value) -> Result<Value, String> {
    run_elevated_impl(cmd, args, false).await
}

/// Shared implementation; `allow_uac` controls whether the function may
/// escalate via a `ShellExecuteExW("runas")` UAC prompt as a last resort.
async fn run_elevated_impl(
    cmd: &'static str,
    args: Value,
    allow_uac: bool,
) -> Result<Value, String> {
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

    // ── Preferred path: autonomous MiControlBridge service pipe ─────────────
    // The service runs as SYSTEM since installation — no UAC prompt ever.
    if let Ok(response) = run_via_service_pipe(cmd, args.clone()).await {
        return Ok(response);
    }
    // Service pipe unavailable — fall through to the scheduled task.

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
        // path. This fixes the case where the task was registered during
        // `cargo tauri dev` and points to the debug exe.
        let healed = tokio::task::spawn_blocking(ensure_task_correct_path)
            .await
            .map_err(|e| format!("task heal task panicked: {e}"))?;

        let no_uac = || async {
            log::warn!(
                "Scheduled task unavailable and UAC prompts are disabled for this \
                 command — returning error instead"
            );
            let _ = tokio::fs::remove_file(&cmd_path).await;
            Err(format!(
                "Elevated command not executed: scheduled task '{}' is unavailable \
                 and UAC fallback is disabled for this operation.",
                TASK_NAME
            ))
        };

        match healed {
            TaskHealResult::AlreadyCorrect => {
                // Task was fine — the original /run failure was transient.
                // Retry once before falling back.
                let retry_ok = run_schtasks_run().await;
                if !retry_ok {
                    if allow_uac {
                        log::warn!(
                            "Scheduled task still failing after AlreadyCorrect, falling back to UAC"
                        );
                        launch_uac_fallback(&request_id, &cmd_path).await?;
                    } else {
                        no_uac().await?;
                    }
                }
            }
            TaskHealResult::Healed => {
                let retry_ok = run_schtasks_run().await;
                if !retry_ok {
                    if allow_uac {
                        log::warn!(
                            "Scheduled task still failed after self-healing, falling back to UAC"
                        );
                        launch_uac_fallback(&request_id, &cmd_path).await?;
                    } else {
                        no_uac().await?;
                    }
                }
            }
            TaskHealResult::Failed => {
                if allow_uac {
                    launch_uac_fallback(&request_id, &cmd_path).await?;
                } else {
                    no_uac().await?;
                }
            }
        }
    }

    // Poll for the result file (check every 150 ms, timeout per-command)
    let timeout = timeout_for_cmd(cmd);
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
            // full GUI launched instead of the helper).
            #[cfg(windows)]
            {
                if !allow_uac {
                    let _ = tokio::fs::remove_file(&cmd_path).await;
                    return Err(format!(
                        "Elevated command '{}' timed out after {} s and UAC fallback is \
                         disabled for this operation.",
                        cmd,
                        timeout.as_secs()
                    ));
                }
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
                        "Elevated process timed out after {} s and UAC fallback \
                         failed: {e}. Reinstall MiControl to fix the scheduled task.",
                        timeout.as_secs()
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
                return Err(format!(
                    "Elevated process timed out after {} s. UAC fallback ran \
                     but produced no result.",
                    timeout.as_secs()
                ));
            }
            #[cfg(not(windows))]
            {
                let _ = tokio::fs::remove_file(&cmd_path).await;
                return Err(format!(
                    "Elevated process timed out after {} s. \
                     Ensure the '{}' scheduled task is registered.",
                    timeout.as_secs(),
                    TASK_NAME
                ));
            }
        }
    }
}

/// Send a privileged command to the autonomous `MiControlBridge` service over
/// the named pipe `\\.\pipe\micontrol_bridge`.
///
/// The bridge service runs as `NT AUTHORITY\SYSTEM` since installation, so
/// this path NEVER triggers a UAC prompt. Authentication uses the same
/// HMAC-SHA256 shared key as the scheduled-task path, plus a freshness window.
///
/// Returns `Err` when the service is not installed/running or the command
/// fails; the caller falls back to the scheduled task in that case.
#[cfg(windows)]
async fn run_via_service_pipe(cmd: &str, args: Value) -> Result<Value, String> {
    let request_id = make_request_id();
    let nonce = auth::generate_nonce();
    let mut payload = serde_json::json!({
        "protocol_version": 3,
        "request_id": request_id,
        "created_at_ms": auth::now_ms(),
        "nonce": nonce,
        "caller_pid": std::process::id(),
        "cmd": cmd,
        "args": args,
    });

    // Sign the payload (same shared key as the scheduled-task bridge).
    let key = tokio::task::spawn_blocking(auth::get_or_create_key)
        .await
        .map_err(|e| format!("HMAC key task panicked: {e}"))?
        .map_err(|e| format!("Cannot obtain HMAC key: {e}"))?;
    auth::sign_payload(&mut payload, &key);

    // Open the pipe (blocking — do it on a blocking thread). Bound the whole
    // round-trip with a timeout so a wedged bridge service can never hold the
    // serialized ELEV_REQUEST_LOCK forever (a hung pipe previously stalled
    // every subsequent elevated command and froze the UI).
    let body = payload.to_string();
    let result = tokio::time::timeout(
        Duration::from_secs(ELEV_TIMEOUT_SECS),
        tokio::task::spawn_blocking(move || pipe_request(&body)),
    )
    .await
    .map_err(|_| {
        format!(
            "Bridge service pipe round-trip timed out after {ELEV_TIMEOUT_SECS}s \
             (service did not respond)"
        )
    })?
    .map_err(|e| format!("pipe request task panicked: {e}"))?;

    let content = result?;

    // Parse + verify response.
    let mut v: Value =
        serde_json::from_str(&content).map_err(|e| format!("Invalid bridge response JSON: {e}"))?;
    if let Err(e) = auth::verify_payload(&mut v, &key) {
        log::warn!("Bridge response HMAC verification failed: {e}");
        return Err(format!("Bridge response authentication failed: {e}"));
    }

    let resp_req = v["request_id"].as_str().unwrap_or_default();
    if resp_req != request_id {
        return Err(format!(
            "Bridge response request_id mismatch (expected {}, got {})",
            request_id, resp_req
        ));
    }

    if v["ok"].as_bool().unwrap_or(false) {
        Ok(v["data"].clone())
    } else {
        Err(v["error"]
            .as_str()
            .unwrap_or("bridge service failed")
            .to_string())
    }
}

#[cfg(not(windows))]
async fn run_via_service_pipe(_cmd: &str, _args: Value) -> Result<Value, String> {
    Err("Bridge service pipe only available on Windows".to_string())
}

/// Perform a synchronous request/response round-trip on the bridge named pipe.
///
/// Uses OVERLAPPED I/O with a bounded wait so a wedged bridge service can
/// never block the calling thread indefinitely (a previous synchronous
/// `ReadFile` could hang forever and stall the whole elevated-command chain).
#[cfg(windows)]
fn pipe_request(body: &str) -> Result<String, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{
        CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
        WAIT_TIMEOUT,
    };
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_FLAG_OVERLAPPED,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
    use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult};

    const PIPE_OP_TIMEOUT_MS: u32 = 8_000; // per read/write wait
    const MAX_RESPONSE_BYTES: usize = 16_384;

    let path_w: Vec<u16> = std::ffi::OsStr::new(BRIDGE_PIPE_NAME)
        .encode_wide()
        .chain(Some(0))
        .collect();

    let handle = unsafe {
        CreateFileW(
            PCWSTR(path_w.as_ptr()),
            (GENERIC_READ | GENERIC_WRITE).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OVERLAPPED,
            HANDLE::default(),
        )
        .map_err(|e| format!("Open bridge pipe: {e}"))?
    };

    if handle == INVALID_HANDLE_VALUE {
        return Err("INVALID_HANDLE_VALUE opening bridge pipe".to_string());
    }

    // Event used for both the write and read overlapped waits.
    let event = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }
        .map_err(|e| format!("CreateEventW bridge pipe: {e}"))?;

    let mut write_result: Result<(), windows::core::Error> = Ok(());
    let mut written = 0u32;

    // Write the request via overlapped I/O with a bounded wait.
    let req_bytes = body.as_bytes();
    let mut write_ov: windows::Win32::System::IO::OVERLAPPED = unsafe { std::mem::zeroed() };
    write_ov.hEvent = event;
    let mut write_pending = false;
    unsafe {
        // SAFETY: handle/event valid, req_bytes valid for the operation duration.
        let op = WriteFile(
            handle,
            Some(req_bytes),
            Some(&mut written),
            Some(&mut write_ov),
        );
        if op.is_err() {
            let last_err = std::io::Error::last_os_error();
            let code = last_err.raw_os_error().unwrap_or(0);
            if code == 997
            /* ERROR_IO_PENDING */
            {
                write_pending = true;
            } else {
                write_result = Err(windows::core::Error::from_win32());
            }
        }
        if write_pending {
            // Wait for completion with timeout, then collect the result.
            let wait = WaitForSingleObject(event, PIPE_OP_TIMEOUT_MS);
            if wait == WAIT_TIMEOUT {
                let _ = CancelIoEx(handle, Some(&write_ov));
                write_result = Err(windows::core::Error::from_win32());
            } else if wait != WAIT_OBJECT_0 {
                write_result = Err(windows::core::Error::from_win32());
            } else {
                let mut transferred = 0u32;
                if GetOverlappedResult(handle, &write_ov, &mut transferred, false).is_err() {
                    write_result = Err(windows::core::Error::from_win32());
                }
                // transferred bytes are not needed — a successful write of the
                // full request body is implied by GetOverlappedResult OK.
            }
        }
    }
    write_result.map_err(|e| format!("WriteFile bridge pipe: {e}"))?;

    // Read the response via overlapped I/O with per-read bounded waits until
    // the full JSON object is received (ends with '}').
    let mut response_buf = [0u8; MAX_RESPONSE_BYTES];
    let mut total_read = 0usize;
    loop {
        if total_read >= response_buf.len() {
            break; // Cap the response size; bridge responses are bounded.
        }
        let mut bytes_read = 0u32;
        let mut read_ov: windows::Win32::System::IO::OVERLAPPED = unsafe { std::mem::zeroed() };
        read_ov.hEvent = event;
        unsafe {
            // SAFETY: buffers valid, handle valid.
            let op = ReadFile(
                handle,
                Some(&mut response_buf[total_read..]),
                Some(&mut bytes_read),
                Some(&mut read_ov),
            );
            if op.is_err() {
                let last_err = std::io::Error::last_os_error();
                let code = last_err.raw_os_error().unwrap_or(0);
                if code != 997
                /* ERROR_IO_PENDING */
                {
                    break; // EOF or real error — stop reading.
                }
                // Pending: wait with timeout, then collect the result.
                let wait = WaitForSingleObject(event, PIPE_OP_TIMEOUT_MS);
                if wait == WAIT_TIMEOUT {
                    let _ = CancelIoEx(handle, Some(&read_ov));
                    break;
                }
                if wait != WAIT_OBJECT_0 {
                    break;
                }
                let mut transferred = 0u32;
                if GetOverlappedResult(handle, &read_ov, &mut transferred, false).is_err() {
                    break;
                }
                bytes_read = transferred;
            }
        }
        if bytes_read == 0 {
            break; // EOF.
        }
        total_read += bytes_read as usize;
        if total_read > 0 && response_buf[total_read - 1] == b'}' {
            break;
        }
    }

    unsafe {
        CloseHandle(event).ok();
        CloseHandle(handle).ok();
    }

    if total_read == 0 {
        return Err("No response from bridge service (timed out or empty)".to_string());
    }
    Ok(String::from_utf8_lossy(&response_buf[..total_read]).to_string())
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
/// Returns the outcome of the self-healing attempt.
#[cfg(windows)]
fn ensure_task_correct_path() -> TaskHealResult {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    // Get the current executable path.
    let current_exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            log::warn!("Cannot get current exe path for task healing: {e}");
            return TaskHealResult::Failed;
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
        return TaskHealResult::AlreadyCorrect;
    }

    // Build the task XML with the correct path.
    let xml = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task"><Triggers><TimeTrigger><StartBoundary>2000-01-01T00:00:00</StartBoundary><Enabled>false</Enabled></TimeTrigger></Triggers><Principals><Principal id="Author"><LogonType>InteractiveToken</LogonType><RunLevel>HighestAvailable</RunLevel></Principal></Principals><Settings><MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy><DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries><StopIfGoingOnBatteries>false</StopIfGoingOnBatteries><ExecutionTimeLimit>PT120S</ExecutionTimeLimit><Enabled>true</Enabled></Settings><Actions Context="Author"><Exec><Command>"{current_path}"</Command><Arguments>--elevated</Arguments></Exec></Actions></Task>"#
    );

    let temp_dir = std::env::temp_dir();
    let xml_path = temp_dir.join("MCElev_heal.xml");
    if let Err(e) = std::fs::write(&xml_path, &xml) {
        log::warn!("Cannot write task XML for healing: {e}");
        return TaskHealResult::Failed;
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
            std::thread::spawn(move || run_schtasks_elevated(&xml_path_owned))
                .join()
                .unwrap_or(false)
        }
    };

    let _ = std::fs::remove_file(&xml_path);

    if success {
        log::info!("Scheduled task re-registered successfully with correct path");
        TaskHealResult::Healed
    } else {
        log::warn!("Failed to re-register scheduled task (UAC may have been declined)");
        TaskHealResult::Failed
    }
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
