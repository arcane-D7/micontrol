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
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
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

/// UAC prompts from background elevated dispatch are one-shot by default:
/// once the user cancels a consent dialog (or the elevated helper fails to
/// launch), every subsequent elevated call in this process skips the prompt
/// and returns an error instead. This prevents the MIOT-38 UAC storm — the
/// `MCElev_heal.bat` consent popup firing every few seconds — when the
/// scheduled task is broken AND the bridge service is absent.
static UAC_DECLINED: AtomicBool = AtomicBool::new(false);

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

/// Post-reboot self-heal for the `MiControlFace` auth service.
///
/// The face service crashes ~60 min after boot (0xc0000005 in
/// FrameServerClient.dll_unloaded) and — having no SCM failure actions — is
/// left STOPPED-1067 forever, breaking Face Unlock after every reboot.
///
/// This walks the same launch-ordering ladder as `ensure_bridge_service`:
///   0. If the MiControlBridge service pipe is already available, dispatch
///      `ensure_face_service` through it (SYSTEM, no UAC, works even when the
///      face service is dead).
///   1. Otherwise fall back to the MiControlElevated scheduled task.
///
/// Never prompts UAC. Callers invoke this once at app startup and on the
/// Face Unlock tab when `service_running` is false.
pub async fn ensure_face_service() -> Result<Value, String> {
    run_elevated_no_prompt("ensure_face_service", serde_json::json!({})).await
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
        "run_hardware_discovery" | "install_driver" | "trigger_driver_scan" => {
            Duration::from_secs(ELEV_TIMEOUT_SLOW_SECS)
        }
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

        "clean_junk_files" | "set_sys_opt_tweak" => {
            // S55 FIX 7: set_sys_opt_tweak can run heavy operations — OneDrive
            // uninstall (PowerShell + OneDriveSetup wait), services sweep
            // (multiple sc stop), Appx removals. The default 15 s timeout
            // killed the pipe round-trip mid-operation: the UI showed an
            // error while the bridge kept working, leaving the toggle state
            // inconsistent ("clico num e o outro some"). Give it the slow
            // budget (90 s).
            Duration::from_secs(ELEV_TIMEOUT_SLOW_SECS)
        }
        // SCM query + start is quick (~1-2 s), but `sc failure` may hit a
        // busy SCM on cold boot — give it the medium timeout.
        "ensure_face_service" => Duration::from_secs(ELEV_TIMEOUT_MEDIUM_SECS),
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

// ── S51: session circuit breaker for a degraded elevated infrastructure ─────
//
// When neither the bridge service NOR the scheduled task is available (e.g.
// the service was deleted by a previous update and the task is missing), every
// elevated call runs the full failure chain: pipe attempt → schtasks spawn →
// heal attempt → schtasks retry → 3-4 WARN logs. At boot ~12 features fire
// elevated commands simultaneously, producing ~90 WARNs and a dozen pointless
// process spawns in a single second.
//
// The breaker tracks consecutive "no elevated path" failures. After
// BREAKER_TRIP_THRESHOLD consecutive failures it short-circuits with the
// cached error (no schtasks spawn, single log line) until
// `note_elevated_recovery()` is called — either by a successful call or by the
// periodic `ensure_bridge_service` health check succeeding.
const BREAKER_TRIP_THRESHOLD: u32 = 3;

static ELEVATED_BROKEN_STREAK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
static ELEVATED_BROKEN_SINCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// True when the elevated infrastructure is known-degraded this session and
/// further attempts would just repeat the same failure chain.
fn elevated_broken() -> bool {
    ELEVATED_BROKEN_STREAK.load(std::sync::atomic::Ordering::Relaxed) >= BREAKER_TRIP_THRESHOLD
}

/// Record an elevated-path failure (pipe + task both unavailable).
fn note_elevated_failure() {
    let streak = ELEVATED_BROKEN_STREAK.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    if streak == BREAKER_TRIP_THRESHOLD {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or_default();
        ELEVATED_BROKEN_SINCE.store(now, std::sync::atomic::Ordering::Relaxed);
        log::warn!(
            target: "elev_bridge",
            "Elevated infrastructure unavailable (bridge service missing, scheduled task \
             missing) — circuit open, elevated commands will fail fast this session. \
             Reinstall MiControl or run 'micontrol_bridge.exe install' (as admin) to repair."
        );
    }
}

/// Record an elevated-path success — closes the circuit.
fn note_elevated_success() {
    let was_broken = ELEVATED_BROKEN_STREAK.swap(0, std::sync::atomic::Ordering::Relaxed);
    if was_broken >= BREAKER_TRIP_THRESHOLD {
        log::info!(target: "elev_bridge", "Elevated infrastructure recovered — circuit closed");
    }
    ELEVATED_BROKEN_SINCE.store(0, std::sync::atomic::Ordering::Relaxed);
}

/// Manually close the circuit breaker (e.g. after a successful bridge-service
/// health check that did not go through `run_elevated`).
pub fn note_elevated_recovery() {
    note_elevated_success();
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
        note_elevated_success();
        return Ok(response);
    }
    // Service pipe unavailable — fall through to the scheduled task.

    // ── S51 circuit breaker: short-circuit when the infra is known-degraded ─
    // Only for no-prompt calls: critical user-initiated writes (UAC allowed)
    // always get the full attempt chain, since the user explicitly asked.
    if !allow_uac && elevated_broken() {
        return Err(format!(
            "Elevated command '{cmd}' skipped: elevated infrastructure is degraded \
             (bridge service and scheduled task both unavailable) — circuit open"
        ));
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

    // S37-001: Release the serialization lock BEFORE the result-poll loop.
    // Holding ELEV_REQUEST_LOCK across the 15 s poll serializes every
    // elevated call behind a single slot: while the thermal poll (which runs
    // every ~15 s) holds the lock, a set_performance_mode invoke would starve
    // indefinitely. The lock only needs to guard the write+dispatch critical
    // section above — the request_id is unique, so concurrent poll loops are
    // safe.
    drop(_guard);

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
        // S51: pipe + task both failed without UAC — the infrastructure is
        // degraded this session. Count it so later no-prompt calls fail fast
        // instead of repeating the whole spawn/heal chain.
        note_elevated_failure();
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
                note_elevated_success();
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
        // MIOT-38: this silent failure previously hid WHY the app fell back
        // to the scheduled-task/UAC path — log it so the root cause is
        // visible in tauri-app.log.
        log::warn!(
            "Bridge service pipe round-trip timed out after {ELEV_TIMEOUT_SECS}s \
             (service did not respond) — falling back to scheduled task path"
        );
        format!(
            "Bridge service pipe round-trip timed out after {ELEV_TIMEOUT_SECS}s \
             (service did not respond)"
        )
    })?
    .map_err(|e| {
        log::warn!("pipe request task panicked: {e}");
        format!("pipe request task panicked: {e}")
    })?;

    let content = match result {
        Ok(c) => c,
        Err(e) => {
            log::warn!(
                "Bridge service pipe request failed: {e} — falling back to \
                 scheduled task path"
            );
            return Err(e);
        }
    };

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
    // S32-005c: ERROR_PIPE_BUSY (0xE7 / 231) means the service's listener had
    // no free instance at this instant. Previously this immediately fell
    // through to the scheduled-task path (which spawns 3-4 failing schtasks
    // processes and 4 WARN log lines). A short bounded retry is correct:
    // the service accepts the next client within milliseconds.
    const PIPE_BUSY_RETRIES: u32 = 20;
    const PIPE_BUSY_BACKOFF_MS: u32 = 50;

    let path_w: Vec<u16> = std::ffi::OsStr::new(BRIDGE_PIPE_NAME)
        .encode_wide()
        .chain(Some(0))
        .collect();

    let mut handle = INVALID_HANDLE_VALUE;
    for attempt in 0..=PIPE_BUSY_RETRIES {
        let attempt_result = unsafe {
            CreateFileW(
                PCWSTR(path_w.as_ptr()),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OVERLAPPED,
                HANDLE::default(),
            )
        };
        match attempt_result {
            Ok(h) => {
                handle = h;
                break;
            }
            Err(e) => {
                let busy = e.code() == windows::core::HRESULT(0x8007_00E7_u32 as i32);
                if busy && attempt < PIPE_BUSY_RETRIES {
                    std::thread::sleep(std::time::Duration::from_millis(
                        PIPE_BUSY_BACKOFF_MS as u64,
                    ));
                    continue;
                }
                return Err(format!("Open bridge pipe: {e}"));
            }
        }
    }

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
        let status = tokio::process::Command::new("schtasks")
            .args(["/run", "/tn", TASK_NAME])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .await;
        match status {
            Ok(s) if s.success() => true,
            Ok(s) => {
                log::warn!("schtasks /run MiControlElevated failed with exit {s}");
                false
            }
            Err(e) => {
                log::warn!("schtasks /run MiControlElevated failed to spawn: {e}");
                false
            }
        }
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
///
/// MIOT-38: this is the ONLY remaining UAC prompt in the elevated chain. It
/// is gated by [`UAC_DECLINED`] — once the user dismisses the dialog, every
/// later call in this process skips the prompt (no more prompt storms).
#[cfg(windows)]
fn launch_elevated_via_uac(request_id: &str) -> Result<(), String> {
    // Session memory: the user already declined a UAC prompt — do NOT ask
    // again (this is what turned into the MCElev_heal.bat prompt storm).
    if UAC_DECLINED.load(Ordering::SeqCst) {
        return Err(
            "UAC fallback was declined earlier in this session — skipping prompt".to_string(),
        );
    }

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
        if let Err(e) = ShellExecuteExW(&mut info) {
            // ERROR_CANCELLED (0x800704C7) — the user closed/dismissed the
            // UAC consent dialog. Remember it and NEVER re-prompt in this
            // session (MIOT-38 prompt storm). NOTE: HRESULT is i32, so the
            // high bit makes this negative (−2147023673).
            const ERROR_CANCELLED_HRESULT: i32 = 0x8007_04C7u32 as i32;
            if e.code().0 == ERROR_CANCELLED_HRESULT {
                UAC_DECLINED.store(true, Ordering::SeqCst);
                log::warn!(
                    "UAC prompt declined by the user — suppressing further UAC \
                     prompts for this session"
                );
            }
            return Err(format!("ShellExecuteExW: {e}"));
        }

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
/// Tries non-elevated `schtasks` only. If that fails (Access Denied on a
/// standard user session) it does NOT elevate — the scheduled task is merely
/// a fallback behind the MiControlBridge SYSTEM service (the primary
/// elevated path, installed post-install, never prompts). MIOT-38: elevating
/// here (ShellExecuteExW "runas" → MCElev_heal.bat) is what produced the
/// UAC prompt storm every few seconds when the task was missing.
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
            // Check if the task points to the current exe. The task XML
            // stores the <Command> WITH quotes around the path
            // ("C:\...\micontrol.exe") and may use forward slashes — match
            // on all forms. MIOT-38: comparing against the UNQUOTED path made
            // a correct task always look "wrong", so the self-heal
            // re-registered an identical task on every elevated call and
            // looped UAC prompts.
            let path_quoted = format!("\"{current_path}\"");
            let path_quoted_fwd = format!("\"{}\"", current_path.replace('\\', "/"));
            let path_matches = xml.contains(&current_path)
                || xml.contains(&current_path.replace('\\', "/"))
                || xml.contains(&path_quoted)
                || xml.contains(&path_quoted_fwd);
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
            // MIOT-38: NO UAC prompt here. The non-elevated create failed
            // (access denied on a standard user session) and the previous
            // code elevated via ShellExecuteExW "runas" → wrote
            // MCElev_heal.bat → popped a consent dialog from a BACKGROUND
            // dispatch chain (thermal/battery polls fire this loop every few
            // seconds) — the prompt storm the user reported. The scheduled
            // task is only a fallback: the MiControlBridge service (installed
            // post-install as SYSTEM, no UAC) is the primary elevated path.
            // Task missing → return a clean failure; reinstall repairs it
            // (or the service path is used instead).
            log::warn!(
                "Non-elevated schtasks re-register of '{TASK_NAME}' failed — NOT \
                 prompting UAC (scheduled task is a fallback; the MiControlBridge \
                 service is the primary elevated path). Reinstall MiControl or run \
                 'micontrol_bridge.exe install' to repair."
            );
            false
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

#[cfg(not(windows))]
fn ensure_task_correct_path() -> TaskHealResult {
    TaskHealResult::Failed
}
