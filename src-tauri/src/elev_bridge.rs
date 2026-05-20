//! Bridge from the main (unprivileged) process to the elevated helper task.
//!
//! Every privileged hardware operation is dispatched through here:
//!   1. Write a JSON command to `%LOCALAPPDATA%\MiControl\elev_cmd.json`
//!   2. Trigger the `MiControlElevated` scheduled task via `schtasks /run`
//!      (the task was created at install time with RunLevel = Highest,
//!       so it runs with administrator rights, no UAC prompt)
//!   3. Poll `%LOCALAPPDATA%\MiControl\elev_result.json` until it appears
//!      (timeout: 15 s)
//!   4. Return the `data` field on success, or `Err(error_message)`.
//!
//! **Dev-mode fallback**: when the scheduled task is absent (e.g. running
//! straight from `cargo tauri dev` without an installer), step 2 falls back to
//! `ShellExecuteExW` with verb "runas", which triggers a UAC prompt and runs
//! the current binary as `micontrol.exe --elevated`.  This is intentionally
//! only a dev ergonomics aid; production always uses the scheduled task.

use serde_json::Value;
use std::time::{Duration, Instant};

/// Name of the scheduled task registered by the NSIS installer.
const TASK_NAME: &str = "MiControlElevated";

/// Dispatch a privileged command through the scheduled elevated task.
///
/// `cmd` must match one of the branches in `elevated::dispatch()`.
/// `args` is the JSON arguments object (use `serde_json::json!({...})`).
pub async fn run_elevated(cmd: &'static str, args: Value) -> Result<Value, String> {
    let dir = crate::elevated::elev_dir();
    let cmd_path = dir.join("elev_cmd.json");
    let result_path = dir.join("elev_result.json");

    // Remove any stale result from a previous call
    let _ = tokio::fs::remove_file(&result_path).await;

    // Write the command payload
    let payload = serde_json::json!({ "cmd": cmd, "args": args });
    tokio::fs::write(&cmd_path, payload.to_string())
        .await
        .map_err(|e| format!("Cannot write elevated command: {e}"))?;

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
            if let Err(e) = launch_elevated_via_uac() {
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
            return Err(format!(
                "Scheduled task '{TASK_NAME}' not found."
            ));
        }
    }

    // Poll for the result file (check every 150 ms, timeout after 15 s)
    let timeout = Duration::from_secs(15);
    let start = Instant::now();
    loop {
        tokio::time::sleep(Duration::from_millis(150)).await;

        if result_path.exists() {
            let content = tokio::fs::read_to_string(&result_path)
                .await
                .map_err(|e| format!("Cannot read elevated result: {e}"))?;
            let _ = tokio::fs::remove_file(&result_path).await;

            let v: Value = serde_json::from_str(&content)
                .map_err(|e| format!("Invalid result JSON: {e}"))?;

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
                if let Err(e) = launch_elevated_via_uac() {
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
                    let v: Value = serde_json::from_str(&content)
                        .map_err(|e| format!("Invalid result JSON: {e}"))?;
                    return if v["ok"].as_bool().unwrap_or(false) {
                        Ok(v["data"].clone())
                    } else {
                        Err(v["error"]
                            .as_str()
                            .unwrap_or("elevated process failed")
                            .to_string())
                    };
                }
                return Err(
                    "Elevated process timed out after 15 s. UAC fallback ran \
                     but produced no result."
                        .to_string(),
                );
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
/// with verb `"runas"` and argument `"--elevated"`.
///
/// Blocks until the spawned process exits (max 30 s).
/// Returns `Ok(())` if the process was launched successfully; the caller
/// must still poll for `elev_result.json`.
#[cfg(windows)]
fn launch_elevated_via_uac() -> Result<(), String> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HWND};
    use windows::Win32::System::Threading::WaitForSingleObject;
    use windows::Win32::UI::Shell::{
        ShellExecuteExW, SHELLEXECUTEINFOW, SEE_MASK_NOCLOSEPROCESS, SEE_MASK_NOASYNC,
    };

    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let exe_str = exe.to_string_lossy().into_owned();

    let verb: Vec<u16> = OsStr::new("runas").encode_wide().chain(Some(0)).collect();
    let file: Vec<u16> = OsStr::new(&exe_str).encode_wide().chain(Some(0)).collect();
    let params: Vec<u16> = OsStr::new("--elevated").encode_wide().chain(Some(0)).collect();

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

        ShellExecuteExW(&mut info).map_err(|e| format!("ShellExecuteExW: {e}"))?;

        if !info.hProcess.is_invalid() {
            // Wait up to 30 s for the elevated helper to finish writing its result
            WaitForSingleObject(info.hProcess, 30_000);
            let _ = CloseHandle(info.hProcess);
        }
    }
    Ok(())
}
