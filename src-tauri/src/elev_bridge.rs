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

    // Launch the scheduled task (returns immediately; task runs asynchronously)
    let status = tokio::process::Command::new("schtasks")
        .args(["/run", "/tn", TASK_NAME])
        .status()
        .await
        .map_err(|e| format!("schtasks /run failed: {e}"))?;

    if !status.success() {
        // Clean up the command file so it's not picked up by a future run
        let _ = tokio::fs::remove_file(&cmd_path).await;
        return Err(format!(
            "Scheduled task '{}' not found (exit {}). \
             Reinstall MiControl to register the task.",
            TASK_NAME,
            status.code().unwrap_or(-1)
        ));
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
            let _ = tokio::fs::remove_file(&cmd_path).await;
            return Err(format!(
                "Elevated process timed out after 15 s. \
                 Ensure the '{}' scheduled task is registered.",
                TASK_NAME
            ));
        }
    }
}
