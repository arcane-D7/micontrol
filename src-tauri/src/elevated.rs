//! Elevated-process entry point.
//!
//! Invoked as `micontrol.exe --elevated` by the Windows Scheduled Task
//! registered at install time with RunLevel = Highest.
//!
//! Protocol:
//!   1. Main process writes `%LOCALAPPDATA%\MiControl\elev_cmd_<request_id>.json`
//!   2. Main process calls `schtasks /run /tn "MiControlElevated"`
//!   3. This process starts elevated (no UAC prompt), reads the command,
//!      executes the privileged operation, writes
//!      `%LOCALAPPDATA%\MiControl\elev_result_<request_id>.json`, then exits.
//!
//! The main process polls the request-specific result file with a 15-second timeout.

use crate::util::auth;
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::time::SystemTime;

// ── Entry point ──────────────────────────────────────────────────────────────

/// Called from `main()` when `--elevated` is present in argv.
/// Always terminates via `std::process::exit`.
pub fn run() -> ! {
    let dir = elev_dir();
    let wanted_request = request_id_from_argv();
    let pending = match select_pending_command(&dir, wanted_request.as_deref()) {
        Ok(p) => p,
        Err(e) => {
            let fallback_result_path = wanted_request
                .as_deref()
                .map(result_path_for_request)
                .unwrap_or_else(|| dir.join("elev_result.json"));
            let result = make_err(e);
            let json = serde_json::to_string(&result)
                .unwrap_or_else(|_| r#"{"ok":false,"error":"serialize_error"}"#.to_string());
            let _ = std::fs::write(&fallback_result_path, json);
            std::process::exit(0);
        }
    };

    // Remove stale result from a previous run for this same request id.
    let _ = std::fs::remove_file(&pending.result_path);

    let result = match std::fs::read_to_string(&pending.cmd_path) {
        Ok(content) => {
            // Consume the command file immediately to close the read window.
            let _ = std::fs::remove_file(&pending.cmd_path);

            // Parse the raw JSON to verify the HMAC before dispatching.
            match serde_json::from_str::<serde_json::Value>(&content) {
                Ok(mut payload) => {
                    if let Ok(key) = auth::read_key() {
                        // Verify the command HMAC and timestamp freshness.
                        if let Err(e) = auth::verify_payload(&mut payload, &key) {
                            log::warn!("Elevated command rejected (auth failure): {e}");
                            make_err(format!("Command authentication failed: {e}"))
                        } else {
                            // Re-deserialize into ElevCmd after verification.
                            match serde_json::from_value::<ElevCmd>(payload) {
                                Ok(cmd) => dispatch(cmd),
                                Err(e) => make_err(format!("Invalid command: {e}")),
                            }
                        }
                    } else {
                        log::error!("Elevated helper cannot read HMAC key");
                        make_err("Authentication key unavailable".to_string())
                    }
                }
                Err(e) => make_err(format!("Invalid command JSON: {e}")),
            }
        }
        Err(e) => make_err(format!("Cannot read command file: {e}")),
    };

    let mut wrapped = json!({
        "request_id": pending.request_id,
        "ok": result["ok"].as_bool().unwrap_or(false),
        "data": result["data"].clone(),
        "error": result["error"].clone(),
        "created_at_ms": auth::now_ms(),
    });

    // Sign the response with HMAC so the caller can verify integrity.
    if let Ok(key) = auth::read_key() {
        auth::sign_payload(&mut wrapped, &key);
    }

    let json = serde_json::to_string(&wrapped)
        .unwrap_or_else(|_| r#"{"ok":false,"error":"serialize_error"}"#.to_string());
    let _ = std::fs::write(&pending.result_path, json);
    auth::restrict_file_acl(&pending.result_path);
    std::process::exit(0);
}

// ── Command/Result types ─────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ElevCmd {
    #[allow(dead_code)]
    #[serde(default)]
    protocol_version: Option<u32>,
    #[allow(dead_code)]
    #[serde(default)]
    request_id: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    created_at_ms: Option<u64>,
    #[allow(dead_code)]
    #[serde(default)]
    nonce: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    hmac: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    caller_pid: Option<u32>,
    cmd: String,
    #[serde(default)]
    args: Value,
}

// ── Dispatcher ───────────────────────────────────────────────────────────────

fn dispatch(cmd: ElevCmd) -> Value {
    match cmd.cmd.as_str() {
        "set_performance_mode" => {
            let mode: crate::state::PerformanceMode =
                match serde_json::from_value(cmd.args["mode"].clone()) {
                    Ok(m) => m,
                    Err(e) => return make_err(format!("Bad mode arg: {e}")),
                };
            match crate::hw::performance::set_performance_mode(mode) {
                Ok(r) => make_ok(serde_json::to_value(r).unwrap_or(Value::Null)),
                Err(e) => make_err(e.to_string()),
            }
        }

        "set_charging_threshold" => {
            let threshold: u8 = match serde_json::from_value(cmd.args["threshold"].clone()) {
                Ok(v) => v,
                Err(e) => return make_err(format!("Bad threshold arg: {e}")),
            };
            match crate::hw::charging::set_charging_threshold(threshold) {
                Ok(r) => make_ok(serde_json::to_value(r).unwrap_or(Value::Null)),
                Err(e) => make_err(e.to_string()),
            }
        }

        "set_brightness" => {
            let level: u8 = match serde_json::from_value(cmd.args["level"].clone()) {
                Ok(v) => v,
                Err(e) => return make_err(format!("Bad level arg: {e}")),
            };
            match crate::hw::display::set_brightness(level) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "set_hdr" => {
            let enabled: bool = match serde_json::from_value(cmd.args["enabled"].clone()) {
                Ok(v) => v,
                Err(e) => return make_err(format!("Bad enabled arg: {e}")),
            };
            match crate::hw::display::set_hdr(enabled) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "set_ai_brightness" => {
            let enabled: bool = match serde_json::from_value(cmd.args["enabled"].clone()) {
                Ok(v) => v,
                Err(e) => return make_err(format!("Bad enabled arg: {e}")),
            };
            match crate::hw::display::set_ai_brightness(enabled) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "set_ai_brightness_config" => {
            let config: crate::hw::display::AiBrightnessConfig =
                match serde_json::from_value(cmd.args["config"].clone()) {
                    Ok(v) => v,
                    Err(e) => return make_err(format!("Bad config arg: {e}")),
                };
            match crate::hw::display::set_ai_brightness_config(config) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "set_fan_mode" => {
            let mode: crate::hw::fan::FanMode =
                match serde_json::from_value(cmd.args["mode"].clone()) {
                    Ok(v) => v,
                    Err(e) => return make_err(format!("Bad mode arg: {e}")),
                };
            let speed_percent: u8 = match serde_json::from_value(cmd.args["speed_percent"].clone())
            {
                Ok(v) => v,
                Err(e) => return make_err(format!("Bad speed_percent arg: {e}")),
            };
            match crate::hw::fan::set_fan_mode(mode, speed_percent) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "set_touchpad_sensitivity" => {
            let sensitivity: crate::hw::touchpad::TouchpadSensitivity =
                match serde_json::from_value(cmd.args["sensitivity"].clone()) {
                    Ok(v) => v,
                    Err(e) => return make_err(format!("Bad sensitivity arg: {e}")),
                };
            match crate::hw::touchpad::set_touchpad_sensitivity(sensitivity) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "set_touchpad_haptics" => {
            let enabled: bool = match serde_json::from_value(cmd.args["enabled"].clone()) {
                Ok(v) => v,
                Err(e) => return make_err(format!("Bad enabled arg: {e}")),
            };
            match crate::hw::touchpad::set_touchpad_haptics(enabled) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "set_touchpad_haptics_intensity" => {
            let intensity: crate::hw::touchpad::HapticsIntensity =
                match serde_json::from_value(cmd.args["intensity"].clone()) {
                    Ok(v) => v,
                    Err(e) => return make_err(format!("Bad intensity arg: {e}")),
                };
            match crate::hw::touchpad::set_touchpad_haptics_intensity(intensity) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "set_touchpad_gesture_screenshot" => {
            let enabled: bool = match serde_json::from_value(cmd.args["enabled"].clone()) {
                Ok(v) => v,
                Err(e) => return make_err(format!("Bad enabled arg: {e}")),
            };
            match crate::hw::touchpad::set_touchpad_gesture_screenshot(enabled) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "set_touchpad_repress" => {
            let enabled: bool = match serde_json::from_value(cmd.args["enabled"].clone()) {
                Ok(v) => v,
                Err(e) => return make_err(format!("Bad enabled arg: {e}")),
            };
            match crate::hw::touchpad::set_touchpad_repress(enabled) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "set_touchpad_edge_slide" => {
            let enabled: bool = match serde_json::from_value(cmd.args["enabled"].clone()) {
                Ok(v) => v,
                Err(e) => return make_err(format!("Bad enabled arg: {e}")),
            };
            match crate::hw::touchpad::set_touchpad_edge_slide(enabled) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "set_refresh_rate" => {
            let hz: u32 = match serde_json::from_value(cmd.args["hz"].clone()) {
                Ok(v) => v,
                Err(e) => return make_err(format!("Bad hz arg: {e}")),
            };
            match crate::hw::display::set_refresh_rate(hz) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "set_adaptive_refresh_rate" => {
            let enabled: bool = match serde_json::from_value(cmd.args["enabled"].clone()) {
                Ok(v) => v,
                Err(e) => return make_err(format!("Bad enabled arg: {e}")),
            };
            match crate::hw::display::set_intel_drrs(enabled) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "run_hardware_discovery" => {
            let data_dir = std::env::var("APPDATA")
                .ok()
                .map(|a| PathBuf::from(a).join("MiControl"));
            let profile = crate::hw::discovery::rediscover(data_dir);
            make_ok(serde_json::to_value(profile).unwrap_or(Value::Null))
        }

        "install_driver" => {
            let driver_name: String = match serde_json::from_value(cmd.args["driver_name"].clone())
            {
                Ok(v) => v,
                Err(e) => return make_err(format!("Bad driver_name arg: {e}")),
            };
            let inf_path = match crate::hw::discovery::resolve_bundled_inf_by_name(&driver_name) {
                Ok(path) => path,
                Err(e) => return make_err(e.to_string()),
            };
            match crate::hw::discovery::install_driver(&inf_path) {
                Ok(msg) => make_ok(Value::String(msg)),
                Err(e) => make_err(e.to_string()),
            }
        }

        unknown => make_err(format!("Unknown elevated command: {unknown}")),
    }
}

/// Public in-process dispatch: called by `elev_bridge::run_elevated` when the
/// main process is already running as an administrator.  Avoids the scheduled-
/// task round-trip entirely.
pub fn dispatch_cmd(cmd: &str, args: Value) -> Value {
    dispatch(ElevCmd {
        protocol_version: None,
        request_id: None,
        created_at_ms: None,
        nonce: None,
        hmac: None,
        caller_pid: None,
        cmd: cmd.to_string(),
        args,
    })
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Returns `%LOCALAPPDATA%\MiControl`, creating it if needed.
pub fn elev_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA")
        .unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned());
    let dir = PathBuf::from(base).join("MiControl");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn make_ok(data: Value) -> Value {
    json!({ "ok": true, "data": data })
}

fn make_err(msg: String) -> Value {
    json!({ "ok": false, "error": msg })
}

// ── Pending command selection ────────────────────────────────────────────────

struct PendingCommand {
    request_id: String,
    cmd_path: PathBuf,
    result_path: PathBuf,
}

fn request_id_from_argv() -> Option<String> {
    let mut args = std::env::args();
    while let Some(arg) = args.next() {
        if arg == "--request-id" {
            return args.next();
        }
    }
    None
}

fn cmd_path_for_request(request_id: &str) -> PathBuf {
    elev_dir().join(format!("elev_cmd_{request_id}.json"))
}

fn result_path_for_request(request_id: &str) -> PathBuf {
    elev_dir().join(format!("elev_result_{request_id}.json"))
}

fn select_pending_command(
    dir: &std::path::Path,
    wanted: Option<&str>,
) -> Result<PendingCommand, String> {
    if let Some(request_id) = wanted {
        let cmd_path = cmd_path_for_request(request_id);
        if !cmd_path.exists() {
            return Err(format!(
                "request-specific command file not found for request_id={request_id}"
            ));
        }
        return Ok(PendingCommand {
            request_id: request_id.to_string(),
            result_path: result_path_for_request(request_id),
            cmd_path,
        });
    }

    // Scheduled-task path: no request-id in argv. Pick the newest request file.
    let newest = std::fs::read_dir(dir)
        .map_err(|e| format!("Cannot read elevated command directory: {e}"))?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            if !(name.starts_with("elev_cmd_") && name.ends_with(".json")) {
                return None;
            }
            let request_id = name
                .strip_prefix("elev_cmd_")?
                .strip_suffix(".json")?
                .to_string();
            let modified = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .unwrap_or(SystemTime::UNIX_EPOCH);
            Some((modified, request_id, path))
        })
        .max_by_key(|(ts, _, _)| *ts);

    if let Some((_, request_id, cmd_path)) = newest {
        return Ok(PendingCommand {
            result_path: result_path_for_request(&request_id),
            request_id,
            cmd_path,
        });
    }

    // Legacy fallback for older protocol (single fixed file).
    let legacy_cmd = dir.join("elev_cmd.json");
    if legacy_cmd.exists() {
        return Ok(PendingCommand {
            request_id: "legacy".to_string(),
            cmd_path: legacy_cmd,
            result_path: dir.join("elev_result.json"),
        });
    }

    Err("No pending elevated command file found".to_string())
}

#[cfg(test)]
mod tests {
    use crate::util::auth;

    /// Regression test: an unauthenticated command (no HMAC) is rejected.
    #[test]
    fn test_unauthenticated_command_rejected() {
        let key = b"test-key-32-bytes-long-1234567890";
        let mut payload = serde_json::json!({
            "cmd": "set_brightness",
            "args": {"level": 80},
            "created_at_ms": auth::now_ms(),
            "nonce": auth::generate_nonce(),
        });
        // Do NOT sign the payload — simulate an attacker who wrote a command
        // file without knowing the key.
        let result = auth::verify_payload(&mut payload, key);
        assert!(
            result.is_err(),
            "Unauthenticated command should be rejected"
        );
        let err_msg = result.unwrap_err();
        assert!(
            err_msg.to_lowercase().contains("hmac"),
            "Error should mention HMAC"
        );
    }

    /// Regression test: a command file swapped after write (HMAC mismatch) is rejected.
    #[test]
    fn test_swapped_command_rejected() {
        let key = b"test-key-32-bytes-long-1234567890";
        let mut payload = serde_json::json!({
            "cmd": "set_brightness",
            "args": {"level": 80},
            "created_at_ms": auth::now_ms(),
            "nonce": auth::generate_nonce(),
        });
        auth::sign_payload(&mut payload, key);

        // Simulate an attacker swapping the command body after the file was
        // written but before the helper reads it.
        payload["cmd"] = serde_json::json!("set_charging_threshold");
        payload["args"] = serde_json::json!({"threshold": 100});

        let result = auth::verify_payload(&mut payload, key);
        assert!(result.is_err(), "Swapped command should be rejected");
    }

    /// A validly-signed command with a fresh timestamp is accepted.
    #[test]
    fn test_valid_command_accepted() {
        let key = b"test-key-32-bytes-long-1234567890";
        let mut payload = serde_json::json!({
            "cmd": "set_brightness",
            "args": {"level": 80},
            "created_at_ms": auth::now_ms(),
            "nonce": auth::generate_nonce(),
        });
        auth::sign_payload(&mut payload, key);
        let result = auth::verify_payload(&mut payload, key);
        assert!(result.is_ok(), "Valid command should be accepted");
    }

    /// A command signed with a different key is rejected.
    #[test]
    fn test_wrong_key_rejected() {
        let key1 = b"test-key-32-bytes-long-1234567890";
        let key2 = b"attacker-key-32-bytes-long-1234567";
        let mut payload = serde_json::json!({
            "cmd": "set_brightness",
            "args": {"level": 80},
            "created_at_ms": auth::now_ms(),
            "nonce": auth::generate_nonce(),
        });
        auth::sign_payload(&mut payload, key1);
        let result = auth::verify_payload(&mut payload, key2);
        assert!(result.is_err(), "Wrong-key command should be rejected");
    }
}
