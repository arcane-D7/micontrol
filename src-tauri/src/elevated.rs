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
use crate::util::panic::lock_or_recover;
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

// ── Entry point ──────────────────────────────────────────────────────────────

/// Called from `main()` when `--elevated` is present in argv.
/// Processes all pending elevated commands and returns (clean shutdown).
pub fn run() {
    // Initialize logging so elevated helper errors are visible in the dev trace log.
    if let Err(e) = crate::debug_log::init_logging() {
        eprintln!("Elevated helper: failed to initialize logging: {e}");
    }
    log::info!("Elevated helper started");

    let dir = elev_dir();
    let wanted_request = request_id_from_argv();

    // S37-002: Process ALL pending commands, not just the most recent one.
    // The app polls hardware every ~2 s (get_fan_info → thermal) and every
    // 15 s (perf/battery), each writing its own elev_cmd. The scheduled task
    // dispatches one helper process per /run, and that process must drain the
    // whole queue — including set_performance_mode — or those requests starve
    // behind the newer thermal poll until their 15 s timeout expires.
    let pending_list = match select_all_pending_commands(&dir, wanted_request.as_deref()) {
        Ok(list) if list.is_empty() => {
            log::info!("Elevated helper: no pending commands");
            let fallback_result_path = wanted_request
                .as_deref()
                .map(result_path_for_request)
                .unwrap_or_else(|| dir.join("elev_result.json"));
            let result = make_err("No pending elevated command file found".to_string());
            let json = serde_json::to_string(&result)
                .unwrap_or_else(|_| r#"{"ok":false,"error":"serialize_error"}"#.to_string());
            let _ = std::fs::write(&fallback_result_path, json);
            // S24-001: Flush nonces before exit to prevent nonce loss.
            flush_nonces();
            std::process::exit(0);
        }
        Ok(list) => list,
        Err(e) => {
            let fallback_result_path = wanted_request
                .as_deref()
                .map(result_path_for_request)
                .unwrap_or_else(|| dir.join("elev_result.json"));
            let result = make_err(e);
            let json = serde_json::to_string(&result)
                .unwrap_or_else(|_| r#"{"ok":false,"error":"serialize_error"}"#.to_string());
            let _ = std::fs::write(&fallback_result_path, json);
            // S24-001: Flush nonces before exit to prevent nonce loss.
            flush_nonces();
            std::process::exit(0);
        }
    };

    for pending in &pending_list {
        process_pending(pending);
    }

    // S37-003: Flush nonces BEFORE the process ends, then return normally.
    // Using std::process::exit(0) here while COM/WMI state is alive triggered
    // 0xc0000005 in combase.dll at shutdown — returning from main lets the
    // runtime unwind properly.
    flush_nonces();
}

fn process_pending(pending: &PendingCommand) {
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
                                Ok(cmd) => {
                                    // Check nonce anti-replay to prevent replay attacks.
                                    if let Some(ref nonce) = cmd.nonce {
                                        let mut seen = lock_or_recover(&SEEN_NONCES);
                                        if seen.is_none() {
                                            *seen = Some(load_nonces());
                                        }
                                        let now = std::time::SystemTime::now()
                                            .duration_since(std::time::UNIX_EPOCH)
                                            .unwrap_or_default()
                                            .as_secs();
                                        let map = seen.as_mut().unwrap();
                                        if map.contains_key(nonce) {
                                            log::warn!(
                                                "Replay attack detected: duplicate nonce {nonce}"
                                            );
                                            make_err(format!("Duplicate nonce: {nonce}"))
                                        } else {
                                            map.insert(nonce.clone(), now);
                                            // Persist every 3 nonces as a batch (S18-08)
                                            if map.len().is_multiple_of(3) {
                                                save_nonces(map);
                                            }
                                            log::info!(
                                                "Elevated dispatching command: {} (request_id={})",
                                                cmd.cmd,
                                                pending.request_id
                                            );
                                            let result = dispatch(cmd);
                                            log::info!(
                                                "Elevated command result: ok={} error={:?}",
                                                result["ok"].as_bool().unwrap_or(false),
                                                result["error"].as_str()
                                            );
                                            result
                                        }
                                    } else {
                                        log::warn!(
                                            "Elevated command rejected: missing required nonce field"
                                        );
                                        make_err("Missing required nonce field".to_string())
                                    }
                                }
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
    log::info!(
        "Elevated writing result to: {}",
        pending.result_path.display()
    );
    // S36-001: Write result file atomically (temp + rename) so a killed
    // helper never leaves a partial JSON file for the main process to read.
    let tmp_path = pending.result_path.with_extension("json.tmp");
    match std::fs::write(&tmp_path, &json) {
        Ok(()) => {
            if let Err(e) = std::fs::rename(&tmp_path, &pending.result_path) {
                let _ = std::fs::remove_file(&tmp_path);
                log::warn!("Failed to atomically rename result file: {e}");
            } else if let Err(e) = auth::restrict_file_acl(&pending.result_path) {
                log::warn!("Failed to restrict ACL on result file: {e}");
            }
        }
        Err(e) => {
            log::warn!("Failed to write result file: {e}");
        }
    }
    // Nonce persistence happens at the end of run() (S37-003) — this function
    // returns so run() can process the next pending command.
}

/// Tracks seen nonces to detect replay attacks, with timestamps for TTL.
static SEEN_NONCES: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);

/// Path to the nonce store file.
fn nonce_store_path() -> std::path::PathBuf {
    elev_dir().join("nonces.json")
}

/// Persist nonces to disk atomically (temp file + rename).
///
/// Writes to a temporary file in the same directory, then renames it to the
/// final path. This prevents the elevated helper from reading a partially
/// written nonce store if the process is interrupted mid-write.
fn save_nonces(nonces: &HashMap<String, u64>) {
    let path = nonce_store_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string(nonces) {
        // S25-001: Write to a temp file in the same directory, then rename
        // for atomicity. Same pattern as elev_bridge.rs command file writes.
        let tmp_path = path.with_extension("json.tmp");
        if std::fs::write(&tmp_path, &json).is_ok() {
            if std::fs::rename(&tmp_path, &path).is_ok() {
                if let Err(e) = auth::restrict_file_acl(&path) {
                    log::warn!("Failed to restrict ACL on nonce store: {e}");
                }
            } else {
                // Rename failed — clean up the temp file to avoid littering.
                let _ = std::fs::remove_file(&tmp_path);
                log::warn!("Failed to atomically rename nonce store");
            }
        }
    }
}

/// Immediately persist all seen nonces to disk (S18-08).
/// Called on shutdown to ensure no nonces are lost between batch writes.
pub fn flush_nonces() {
    let seen = lock_or_recover(&SEEN_NONCES);
    if let Some(map) = seen.as_ref() {
        save_nonces(map);
    }
}

/// Load nonces from disk, purging expired ones (older than 5 minutes).
fn load_nonces() -> HashMap<String, u64> {
    let path = nonce_store_path();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if let Ok(json) = std::fs::read_to_string(&path) {
        if let Ok(mut nonces) = serde_json::from_str::<HashMap<String, u64>>(&json) {
            // Purge expired nonces (older than 5 minutes)
            nonces.retain(|_, ts| now.saturating_sub(*ts) < 300);
            return nonces;
        }
    }
    HashMap::new()
}

// ── Command/Result types ─────────────────────────────────────────────────────

/// Command structure deserialized from the IPC JSON payload.
///
/// Fields marked `#[serde(default)]` are parsed for protocol completeness and
/// HMAC verification but are not directly read by the dispatcher. They must
/// remain present so the JSON deserialization matches the wire format.
#[derive(Deserialize)]
struct ElevCmd {
    #[serde(default)]
    _protocol_version: Option<u32>,
    #[serde(default)]
    _request_id: Option<String>,
    #[serde(default)]
    _created_at_ms: Option<u64>,
    #[serde(default)]
    nonce: Option<String>,
    #[serde(default)]
    _hmac: Option<String>,
    #[serde(default)]
    _caller_pid: Option<u32>,
    cmd: String,
    #[serde(default)]
    args: Value,
}

// ── Dispatcher ───────────────────────────────────────────────────────────────

/// S32-002: Allowlist of WMAA read combinations used by the app.
/// (fun2, fun3) → purpose. Anything else is rejected at the dispatcher so a
/// compromised webview cannot probe arbitrary EC registers.
fn is_allowed_wmi_read(fun2: u16, fun3: u16) -> bool {
    // FUN2_EC_FUNC=0x0800, FUN2_MI_INFO=0x0A00, FUN2_MISC=0x0C00, FUN2_SENSOR=0x1000
    matches!(
        (fun2, fun3),
        (0x0800, 0x00) // current performance mode
            | (0x1000, 0x01) // battery health
            | (0x1000, 0x06) // adapter power
            | (0x0A00, 0x05) // mi usage type
            | (0x0A00, 0x07) // wmid type
            | (0x0C00, 0x02) // lid open type
            | (0x0C00, 0x03) // removable type
    )
}

/// S32-002: Allowlist of WMAA write combinations used by the app.
/// (fun2, fun3) → purpose. Rejects arbitrary EC writes.
fn is_allowed_wmi_write(fun2: u16, fun3: u16) -> bool {
    matches!(
        (fun2, fun3),
        (0x0800, 5) // set performance mode (Performance)
            | (0x0800, 6) // set performance mode (Balanced)
            | (0x0800, 7) // set performance mode (Quiet)
            | (0x0800, 8) // set performance mode (SuperQuiet)
            | (0x0800, 9) // set performance mode (UltraPerformance)
            | (0x0800, 10) // set performance mode (Extreme)
            | (0x1000, 0x02) // set brightness data
            | (0x0C00, 0x06) // set sagv mode
            | (0x0C00, 0x04) // set pl1 flag
            | (0x0C00, 0x05) // set epof flag
            | (0x0A00, 0x05) // set mi usage type
            | (0x0A00, 0x07) // set wmid type
            | (0x0C00, 0x02) // set lid open type
            | (0x0C00, 0x03) // set removable type
            | (0x0A00, 0x08) // set auto illumination
            | (0x0A00, 0x09) // set label mode
    )
}

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

        "set_battery_care" => {
            let enabled: bool = match serde_json::from_value(cmd.args["enabled"].clone()) {
                Ok(v) => v,
                Err(e) => return make_err(format!("Bad enabled arg: {e}")),
            };
            match crate::hw::charging::set_battery_care(enabled) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "set_eye_protection" => {
            let enabled: bool = match serde_json::from_value(cmd.args["enabled"].clone()) {
                Ok(v) => v,
                Err(e) => return make_err(format!("Bad enabled arg: {e}")),
            };
            let intensity: Option<u8> =
                serde_json::from_value(cmd.args["intensity"].clone()).unwrap_or(None);
            match crate::hw::eye_protection::set_eye_protection(enabled, intensity) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "set_os_turbo" => {
            let enabled: bool = match serde_json::from_value(cmd.args["enabled"].clone()) {
                Ok(v) => v,
                Err(e) => return make_err(format!("Bad enabled arg: {e}")),
            };
            match crate::hw::os_turbo::set_os_turbo(enabled) {
                Ok(r) => make_ok(serde_json::to_value(r).unwrap_or(Value::Null)),
                Err(e) => make_err(e.to_string()),
            }
        }

        "set_function_key" => {
            let mode: crate::hw::fn_key::FnKeyMode =
                match serde_json::from_value(cmd.args["mode"].clone()) {
                    Ok(v) => v,
                    Err(e) => return make_err(format!("Bad mode arg: {e}")),
                };
            match crate::hw::fn_key::set_function_key(mode) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "set_mic_noise_canceling" => {
            let enabled: bool = match serde_json::from_value(cmd.args["enabled"].clone()) {
                Ok(v) => v,
                Err(e) => return make_err(format!("Bad enabled arg: {e}")),
            };
            match crate::hw::audio_effects::set_mic_noise_canceling(enabled) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "set_speaker_noise_canceling" => {
            let enabled: bool = match serde_json::from_value(cmd.args["enabled"].clone()) {
                Ok(v) => v,
                Err(e) => return make_err(format!("Bad enabled arg: {e}")),
            };
            match crate::hw::audio_effects::set_speaker_noise_canceling(enabled) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "set_voice_focus" => {
            let enabled: bool = match serde_json::from_value(cmd.args["enabled"].clone()) {
                Ok(v) => v,
                Err(e) => return make_err(format!("Bad enabled arg: {e}")),
            };
            match crate::hw::audio_effects::set_voice_focus(enabled) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "clean_junk_files" => {
            let categories: Vec<crate::hw::cleanup::CleanupCategory> =
                match serde_json::from_value(cmd.args["categories"].clone()) {
                    Ok(v) => v,
                    Err(e) => return make_err(format!("Bad categories arg: {e}")),
                };
            match crate::hw::cleanup::clean_junk_files(categories) {
                Ok(results) => make_ok(serde_json::to_value(results).unwrap_or(Value::Null)),
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

        // pnputil /scan-devices REQUIRES administrator privileges. When the
        // app runs as a normal user the direct call fails (non-zero exit /
        // access denied), so this must run through the elevated bridge.
        "trigger_driver_scan" => match crate::hw::update::trigger_driver_scan() {
            Ok(msg) => make_ok(Value::String(msg)),
            Err(e) => make_err(e.to_string()),
        },

        // ── Diagnostic commands ───────────────────────────────────────────
        // These are read-only probes used by the test binary to verify which
        // hardware access paths work when elevated.
        "diag_ecram_read" => {
            // Read ERAM (256 bytes) + IoTStatus (8 bytes) + Sensor block (0x78 bytes)
            let eram = crate::hw::ecram::read_ecram(crate::hw::ecram::get_eram_base(), 0x100);
            let iot_status = crate::hw::ecram::read_ecram(crate::hw::ecram::IOT_STATUS_BASE, 8);
            let sensor = crate::hw::ecram::read_ecram(
                crate::hw::ecram::ECRAM_SENSOR_BLOCK,
                crate::hw::ecram::ECRAM_SENSOR_SIZE,
            );

            let mut result = serde_json::json!({});
            match &eram {
                Ok(data) => {
                    let hex: String = data.iter().map(|b| format!("{:02x}", b)).collect();
                    result["eram"] = serde_json::json!({
                        "ok": true,
                        "size": data.len(),
                        "hex": hex,
                        "acin": (data[0x80] & 0x01) != 0,
                        "adpw_watts": data[0x81],
                        "btct_ma": u16::from_le_bytes([data[0x8C], data[0x8D]]),
                        "btpr_mah": u16::from_le_bytes([data[0x8E], data[0x8F]]),
                        "btvt_mv": u16::from_le_bytes([data[0x90], data[0x91]]),
                        "qfan": format!("0x{:02x}", data[0x68]),
                        "touchpad_0x40": format!("0x{:02x}", data[0x40]),
                        "touchpad_0x42": format!("0x{:02x}", data[0x42]),
                        "smart_mode_0x4a": format!("0x{:02x}", data[0x4A]),
                        "smart_mode_0x4b": format!("0x{:02x}", data[0x4B]),
                    });
                }
                Err(e) => {
                    result["eram"] = serde_json::json!({ "ok": false, "error": e.to_string() });
                }
            }
            match &iot_status {
                Ok(data) => {
                    let hex: String = data.iter().map(|b| format!("{:02x}", b)).collect();
                    result["iot_status"] = serde_json::json!({
                        "ok": true,
                        "hex": hex,
                        "status_byte": format!("0x{:02x}", data[0]),
                    });
                }
                Err(e) => {
                    result["iot_status"] =
                        serde_json::json!({ "ok": false, "error": e.to_string() });
                }
            }
            match &sensor {
                Ok(data) => {
                    let hex: String = data.iter().map(|b| format!("{:02x}", b)).collect();
                    result["sensor"] = serde_json::json!({
                        "ok": true,
                        "size": data.len(),
                        "hex": hex,
                    });
                }
                Err(e) => {
                    result["sensor"] = serde_json::json!({ "ok": false, "error": e.to_string() });
                }
            }
            make_ok(result)
        }

        "diag_wmi_query" => {
            // Test WMI access: query HQWmiCommonInterface and MICommonInterface
            let mut result = serde_json::json!({});

            // Test HQWmiCommonInterface (used by performance mode)
            #[cfg(windows)]
            {
                use std::collections::HashMap;
                let hq_result = crate::hw::wmi_cache::with_wmi(|wmi| {
                    let rows: Vec<HashMap<String, wmi::Variant>> = wmi
                        .raw_query(
                            "SELECT InstanceName FROM HQWmiCommonInterface WHERE Active = TRUE",
                        )
                        .unwrap_or_default();
                    Ok(rows)
                });
                match hq_result {
                    Ok(rows) if !rows.is_empty() => {
                        let instances: Vec<String> = rows
                            .iter()
                            .filter_map(|r| {
                                crate::util::wmi_extract::extract_string(r, "InstanceName")
                            })
                            .collect();
                        result["hq_wmi"] = serde_json::json!({
                            "ok": true,
                            "instances": instances,
                            "count": rows.len(),
                        });
                    }
                    Ok(_) => {
                        result["hq_wmi"] = serde_json::json!({
                            "ok": true,
                            "instances": [],
                            "count": 0,
                            "note": "No active HQWmiCommonInterface instances"
                        });
                    }
                    Err(e) => {
                        result["hq_wmi"] = serde_json::json!({
                            "ok": false,
                            "error": e.to_string(),
                        });
                    }
                }

                // Test MICommonInterface (IoTService WMI)
                let mi_result = crate::hw::wmi_cache::with_wmi(|wmi| {
                    let rows: Vec<HashMap<String, wmi::Variant>> = wmi
                        .raw_query("SELECT * FROM MICommonInterface")
                        .unwrap_or_default();
                    Ok(rows)
                });
                match mi_result {
                    Ok(rows) if !rows.is_empty() => {
                        let instances: Vec<String> = rows
                            .iter()
                            .filter_map(|r| {
                                crate::util::wmi_extract::extract_string(r, "InstanceName")
                            })
                            .collect();
                        result["mi_wmi"] = serde_json::json!({
                            "ok": true,
                            "instances": instances,
                            "count": rows.len(),
                        });
                    }
                    Ok(_) => {
                        result["mi_wmi"] = serde_json::json!({
                            "ok": true,
                            "instances": [],
                            "count": 0,
                            "note": "No MICommonInterface instances found"
                        });
                    }
                    Err(e) => {
                        result["mi_wmi"] = serde_json::json!({
                            "ok": false,
                            "error": e.to_string(),
                        });
                    }
                }

                // Test EsifDeviceInformation (thermal readings)
                let esif_result = crate::hw::wmi_cache::with_wmi(|wmi| {
                    let rows: Vec<HashMap<String, wmi::Variant>> = wmi
                        .raw_query(
                            "SELECT InstanceName, Temperature, Power FROM EsifDeviceInformation",
                        )
                        .unwrap_or_default();
                    Ok(rows)
                });
                match esif_result {
                    Ok(rows) => {
                        let temps: Vec<serde_json::Value> = rows.iter().map(|r| {
                            serde_json::json!({
                                "instance": crate::util::wmi_extract::extract_string(r, "InstanceName").unwrap_or_default(),
                                "temp_c": crate::util::wmi_extract::extract_i32(r, "Temperature").unwrap_or(0),
                                "power_dw": crate::util::wmi_extract::extract_i32(r, "Power").unwrap_or(0),
                            })
                        }).collect();
                        result["esif"] = serde_json::json!({
                            "ok": true,
                            "participants": temps,
                            "count": rows.len(),
                        });
                    }
                    Err(e) => {
                        result["esif"] = serde_json::json!({
                            "ok": false,
                            "error": e.to_string(),
                        });
                    }
                }

                // Test Win32_Battery
                let bat_result = crate::hw::wmi_cache::with_cimv2(|wmi| {
                    let rows: Vec<HashMap<String, wmi::Variant>> = wmi
                        .raw_query("SELECT * FROM Win32_Battery")
                        .unwrap_or_default();
                    Ok(rows)
                });
                match bat_result {
                    Ok(rows) if !rows.is_empty() => {
                        let bat = &rows[0];
                        result["battery"] = serde_json::json!({
                            "ok": true,
                            "estimated_charge": crate::util::wmi_extract::extract_u32(bat, "EstimatedChargeRemaining").unwrap_or(0),
                            "battery_status": crate::util::wmi_extract::extract_u32(bat, "BatteryStatus").unwrap_or(0),
                        });
                    }
                    Ok(_) => {
                        result["battery"] =
                            serde_json::json!({ "ok": true, "note": "No battery found" });
                    }
                    Err(e) => {
                        result["battery"] =
                            serde_json::json!({ "ok": false, "error": e.to_string() });
                    }
                }
            }

            #[cfg(not(windows))]
            {
                result["note"] = serde_json::json!("WMI only available on Windows");
            }

            make_ok(result)
        }

        "diag_perf_mode" => {
            // Test setting performance mode via WMI (the path that works)
            let mode: crate::state::PerformanceMode =
                match serde_json::from_value(cmd.args["mode"].clone()) {
                    Ok(m) => m,
                    Err(e) => return make_err(format!("Bad mode arg: {e}")),
                };
            match crate::hw::performance::set_performance_mode(mode) {
                Ok(r) => make_ok(serde_json::json!({
                    "result": serde_json::to_value(r).unwrap_or(Value::Null),
                    "mode_set": format!("{:?}", mode),
                })),
                Err(e) => make_err(e.to_string()),
            }
        }

        #[cfg(feature = "diag")]
        "diag_ps" => {
            // Run elevated PowerShell command (diag builds only).
            // S32-002: Restricted to a hardcoded allowlist of diagnostic
            // scripts. Arbitrary script text is rejected so a compromised
            // webview cannot execute arbitrary PowerShell as SYSTEM.
            let script = cmd.args["script"].as_str().unwrap_or("");
            const ALLOWED_DIAG_SCRIPTS: [&str; 4] = [
                "Get-Service | Select-Object Name,Status",
                "Get-Process | Sort-Object CPU -Descending | Select-Object -First 20 Name,CPU,WorkingSet64",
                "Get-CimInstance Win32_Fan | Select-Object Name,DesiredSpeed,ActiveCooling",
                "Get-CimInstance Win32_TemperatureProbe | Select-Object Name,CurrentReading",
            ];
            if !ALLOWED_DIAG_SCRIPTS.contains(&script) {
                return make_err("diag_ps rejected: script is not in the allowlist".to_string());
            }
            let output = {
                #[cfg(windows)]
                {
                    std::process::Command::new("powershell")
                        .args(["-NoProfile", "-NonInteractive", "-Command", script])
                        .creation_flags(CREATE_NO_WINDOW)
                        .output()
                }
                #[cfg(not(windows))]
                {
                    std::process::Command::new("powershell")
                        .args(["-NoProfile", "-NonInteractive", "-Command", script])
                        .output()
                }
            };
            match output {
                Ok(out) => {
                    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                    make_ok(serde_json::json!({
                        "stdout": stdout,
                        "stderr": stderr,
                        "exit_code": out.status.code().unwrap_or(-1),
                    }))
                }
                Err(e) => make_err(format!("Failed to run PowerShell: {e}")),
            }
        }

        #[cfg(not(feature = "diag"))]
        "diag_ps" => make_err("diag_ps is disabled in production builds".to_string()),

        // ── Copilot key interception fixes ───────────────────────────────
        // Windows 11 24H2+ intercepts the Copilot key (VK 0xC3) at the Shell
        // level before any user-mode hook can see it. These commands provide
        // two kernel/driver-level approaches to make the key visible:
        //
        // 1. set_scancode_map — writes a Scancode Map registry binary that
        //    remaps the Copilot key's scan code to a different key at the
        //    keyboard class driver level. Requires reboot but is permanent.
        //
        // 2. disable_copilot_key — sets registry policies that prevent
        //    Windows Shell from intercepting the Copilot key, allowing it to
        //    pass through to WH_KEYBOARD_LL and Raw Input.
        "set_scancode_map" => {
            #[cfg(windows)]
            {
                use windows::core::PCWSTR;
                use windows::Win32::System::Registry::{
                    RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY_LOCAL_MACHINE,
                    KEY_SET_VALUE, REG_BINARY, REG_OPTION_NON_VOLATILE,
                };

                // The Scancode Map binary format:
                //   Bytes 0-3:  Signature (0x00000000)
                //   Bytes 4-7:  Version (0x00000000 — but some docs say 0x00000100)
                //   Bytes 8-11: Number of mappings (little-endian u32)
                //   Then N × 4-byte entries: [source_scan_lo, source_scan_hi, target_scan_lo, target_scan_hi]
                //   Terminated by a 4-byte null entry.
                //
                // The Copilot key on Win11 24H2+ keyboards emits scan code 0x6E
                // (E0 6E — extended). We remap it to Right Ctrl (scan 0x1D, E0 1D).
                //
                // Args:
                //   mappings: [[src_lo, src_hi, dst_lo, dst_hi], ...]
                //   clear: bool — if true, delete the Scancode Map value instead

                let clear: bool = cmd
                    .args
                    .get("clear")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                let subkey: Vec<u16> = "SYSTEM\\CurrentControlSet\\Control\\Keyboard Layout\0"
                    .encode_utf16()
                    .collect();

                if clear {
                    // Delete the "Scancode Map" value
                    let script = r#"
                        $key = 'HKLM:\SYSTEM\CurrentControlSet\Control\Keyboard Layout'
                        if (Test-Path $key) {
                            $val = Get-ItemProperty -Path $key -Name 'Scancode Map' -ErrorAction SilentlyContinue
                            if ($null -ne $val) {
                                Remove-ItemProperty -Path $key -Name 'Scancode Map' -Force
                                Write-Output 'Scancode Map removed'
                            } else {
                                Write-Output 'Scancode Map not present'
                            }
                        }
                    "#;
                    let output = {
                        #[cfg(windows)]
                        {
                            std::process::Command::new("powershell")
                                .args(["-NoProfile", "-NonInteractive", "-Command", script])
                                .creation_flags(CREATE_NO_WINDOW)
                                .output()
                        }
                        #[cfg(not(windows))]
                        {
                            std::process::Command::new("powershell")
                                .args(["-NoProfile", "-NonInteractive", "-Command", script])
                                .output()
                        }
                    };
                    return match output {
                        Ok(out) => {
                            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                            make_ok(serde_json::json!({
                                "stdout": stdout,
                                "stderr": stderr,
                                "exit_code": out.status.code().unwrap_or(-1),
                            }))
                        }
                        Err(e) => make_err(format!("Failed to remove Scancode Map: {e}")),
                    };
                }

                // Build the Scancode Map binary
                let mappings: Vec<[u8; 4]> = match cmd.args.get("mappings") {
                    Some(arr) => {
                        let mut result = Vec::new();
                        if let Some(items) = arr.as_array() {
                            for item in items {
                                if let Some(entry) = item.as_array() {
                                    if entry.len() == 4 {
                                        let mut e = [0u8; 4];
                                        for (i, v) in entry.iter().enumerate() {
                                            e[i] = v.as_u64().unwrap_or(0) as u8;
                                        }
                                        // S32-002: Only accept mappings whose source
                                        // scan code is the Copilot key and whose
                                        // target is a known remap key. Reject
                                        // arbitrary bytes to prevent a compromised
                                        // webview from writing junk into HKLM.
                                        //
                                        // Accepted Copilot sources:
                                        //   - scan 0x6E extended (VK 0xC3, standard Win11 Copilot key)
                                        //   - scan 0x86 non-extended (VK_F23 = raw Copilot scan on
                                        //     Xiaomi Book Pro 14 2024 and some OEM boards that
                                        //     emit VK 0x86 instead of synthesising VK 0xC3)
                                        let src_lo = e[2];
                                        let src_hi = e[3];
                                        let tgt_lo = e[0];
                                        let tgt_hi = e[1];
                                        let src_is_copilot = (src_lo == 0x6E
                                            && (src_hi == 0x00 || src_hi == 0xE0))
                                            || (src_lo == 0x86 && src_hi == 0x00);
                                        let tgt_known = matches!(
                                            (tgt_lo, tgt_hi),
                                            (0x1D, 0x00) | (0x1D, 0xE0) // Ctrl
                                                | (0x38, 0x00) | (0x38, 0xE0) // Alt
                                                | (0x36, 0xE0) // Right Shift
                                                | (0x2A, 0x00) // Left Shift
                                        );
                                        if src_is_copilot && tgt_known && result.len() < 8 {
                                            result.push(e);
                                        } else {
                                            log::warn!(
                                                "[elevated] Rejected invalid scancode mapping: {e:02X?}"
                                            );
                                        }
                                    }
                                }
                            }
                        }
                        result
                    }
                    None => {
                        // Default: Copilot key (scan 0x6E, extended) → Right Ctrl (scan 0x1D, extended)
                        // Entry format: [target_lo, target_hi, source_lo, source_hi]
                        // For extended keys, the high byte is 0xE0.
                        vec![[0x1D, 0xE0, 0x6E, 0xE0]]
                    }
                };

                let num_mappings = mappings.len() as u32;
                let mut binary: Vec<u8> = Vec::new();
                // Signature (4 bytes) + Version (4 bytes) + Count (4 bytes)
                binary.extend_from_slice(&0u32.to_le_bytes()); // Signature
                binary.extend_from_slice(&0u32.to_le_bytes()); // Version
                binary.extend_from_slice(&num_mappings.to_le_bytes()); // Count
                                                                       // Mappings
                for entry in &mappings {
                    binary.extend_from_slice(entry);
                }
                // Null terminator entry
                binary.extend_from_slice(&[0u8; 4]);

                let value_name: Vec<u16> = "Scancode Map\0".encode_utf16().collect();

                // Use RegCreateKeyExW + RegSetValueExW for direct registry write
                let mut hkey = windows::Win32::System::Registry::HKEY::default();
                let result = unsafe {
                    RegCreateKeyExW(
                        HKEY_LOCAL_MACHINE,
                        PCWSTR(subkey.as_ptr()),
                        0,
                        None,
                        REG_OPTION_NON_VOLATILE,
                        KEY_SET_VALUE,
                        None,
                        &mut hkey,
                        None,
                    )
                };

                if result.is_err() {
                    return make_err(format!("RegCreateKeyExW failed: {:?}", result));
                }

                let set_result = unsafe {
                    RegSetValueExW(
                        hkey,
                        PCWSTR(value_name.as_ptr()),
                        0,
                        REG_BINARY,
                        Some(&binary),
                    )
                };

                unsafe {
                    let _ = RegCloseKey(hkey);
                }

                if set_result.is_err() {
                    return make_err(format!("RegSetValueExW failed: {:?}", set_result));
                }

                let hex: String = binary
                    .iter()
                    .map(|b| format!("{:02X}", b))
                    .collect::<Vec<_>>()
                    .join(" ");
                log::info!(
                    "[elevated] Scancode Map written: {} mappings, {} bytes: {}",
                    num_mappings,
                    binary.len(),
                    hex
                );
                make_ok(serde_json::json!({
                    "mappings": num_mappings,
                    "bytes": binary.len(),
                    "hex": hex,
                    "note": "Reboot required for Scancode Map to take effect"
                }))
            }
            #[cfg(not(windows))]
            make_err("Scancode Map only available on Windows".to_string())
        }

        "disable_copilot_key" => {
            // Set registry policies to prevent Windows Shell from intercepting
            // the Copilot key (VK 0xC3). This allows the key to pass through
            // to WH_KEYBOARD_LL and Raw Input hooks.
            //
            // Registry keys set:
            // 1. HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced\TaskbarMn = 0
            //    (disables Copilot taskbar button)
            // 2. HKCU\Software\Policies\Microsoft\Windows\WindowsCopilot\TurnOffWindowsCopilot = 1
            //    (Group Policy to disable Copilot)
            // 3. HKLM\SOFTWARE\Policies\Microsoft\Windows\WindowsCopilot\TurnOffWindowsCopilot = 1
            //    (machine-wide Group Policy)

            #[cfg(windows)]
            {
                let enabled: bool = cmd
                    .args
                    .get("enabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);

                // TurnOffWindowsCopilot = 1 to disable Copilot (enabled=true → value=1)
                // TaskbarMn = 0 to hide Copilot taskbar button (enabled=true → inverted=0)
                let copilot_off: u32 = if enabled { 1 } else { 0 };
                let taskbar_mn: u32 = if enabled { 0 } else { 1 };

                let script = format!(
                    r#"
                    $results = @()

                    # 1. Disable Copilot taskbar button (HKCU)
                    $key1 = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced'
                    if (Test-Path $key1) {{
                        Set-ItemProperty -Path $key1 -Name 'TaskbarMn' -Value {taskbar_mn} -Type DWord -Force
                        $results += "TaskbarMn set to {taskbar_mn}"
                    }}

                    # 2. Disable Copilot via Group Policy (HKCU)
                    $key2 = 'HKCU:\Software\Policies\Microsoft\Windows\WindowsCopilot'
                    if (!(Test-Path $key2)) {{ New-Item -Path $key2 -Force | Out-Null }}
                    Set-ItemProperty -Path $key2 -Name 'TurnOffWindowsCopilot' -Value {copilot_off} -Type DWord -Force
                    $results += "HKCU TurnOffWindowsCopilot set to {copilot_off}"

                    # 3. Disable Copilot via Group Policy (HKLM)
                    $key3 = 'HKLM:\SOFTWARE\Policies\Microsoft\Windows\WindowsCopilot'
                    if (!(Test-Path $key3)) {{ New-Item -Path $key3 -Force | Out-Null }}
                    Set-ItemProperty -Path $key3 -Name 'TurnOffWindowsCopilot' -Value {copilot_off} -Type DWord -Force
                    $results += "HKLM TurnOffWindowsCopilot set to {copilot_off}"

                    # 4. Set Copilot key behaviour to "Nothing" via Explorer Advanced
                    # On Win11 24H2+, CopilotKey=0 means "do nothing" (let the key pass through)
                    $key4 = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Explorer\Advanced'
                    if (Test-Path $key4) {{
                        try {{
                            Set-ItemProperty -Path $key4 -Name 'CopilotKey' -Value 0 -Type DWord -Force -ErrorAction SilentlyContinue
                            $results += "CopilotKey set to 0 (do nothing)"
                        }} catch {{
                            $results += "CopilotKey not available"
                        }}
                    }}

                    # NOTE: We intentionally do NOT restart Explorer here.
                    # Restarting Explorer on every MiControl startup breaks the
                    # system tray (including the "show more" overflow button).
                    # The registry changes will take effect on next reboot/login,
                    # which is fine because the Scancode Map also requires a reboot.
                    $results += "Explorer restart skipped (requires reboot)"

                    $results -join "`n"
                "#
                );

                let output = {
                    #[cfg(windows)]
                    {
                        std::process::Command::new("powershell")
                            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
                            .creation_flags(CREATE_NO_WINDOW)
                            .output()
                    }
                    #[cfg(not(windows))]
                    {
                        std::process::Command::new("powershell")
                            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
                            .output()
                    }
                };

                match output {
                    Ok(out) => {
                        let stdout = String::from_utf8_lossy(&out.stdout).to_string();
                        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                        let exit_code = out.status.code().unwrap_or(-1);
                        log::info!(
                            "[elevated] disable_copilot_key: exit={}, stdout={}",
                            exit_code,
                            stdout
                        );
                        if !stderr.is_empty() {
                            log::warn!("[elevated] disable_copilot_key stderr: {}", stderr);
                        }
                        make_ok(serde_json::json!({
                            "stdout": stdout,
                            "stderr": stderr,
                            "exit_code": exit_code,
                            "enabled": enabled,
                        }))
                    }
                    Err(e) => make_err(format!("Failed to set Copilot policies: {e}")),
                }
            }
            #[cfg(not(windows))]
            make_err("Copilot key policies only available on Windows".to_string())
        }

        "diag_mi_wmi" => {
            // Test MICommonInterface.MiInterface WMI method
            // This is the WMI class that IoTService uses for EC commands
            #[cfg(windows)]
            {
                use windows::core::{BSTR, VARIANT};
                use windows::Win32::System::Wmi::{
                    WBEM_FLAG_RETURN_WBEM_COMPLETE, WBEM_GENERIC_FLAG_TYPE,
                };
                use wmi::{COMLibrary, WMIConnection};

                let com = match COMLibrary::without_security() {
                    Ok(c) => c,
                    Err(e) => return make_err(format!("COM init failed: {e}")),
                };
                let wmi = match WMIConnection::with_namespace_path("ROOT\\WMI", com) {
                    Ok(w) => w,
                    Err(e) => return make_err(format!("WMI connect failed: {e}")),
                };

                // Find the MICommonInterface instance
                let instance_name: String = {
                    use std::collections::HashMap;
                    let rows: Vec<HashMap<String, wmi::Variant>> = wmi
                        .raw_query("SELECT InstanceName FROM MICommonInterface")
                        .unwrap_or_default();
                    match rows
                        .into_iter()
                        .next()
                        .and_then(|r| crate::util::wmi_extract::extract_string(&r, "InstanceName"))
                    {
                        Some(name) => name,
                        None => return make_err("No MICommonInterface instance found".to_string()),
                    }
                };

                let escaped = instance_name.replace('\\', "\\\\");
                let instance_path =
                    BSTR::from(format!("MICommonInterface.InstanceName=\"{escaped}\""));
                let method_name = BSTR::from("MiInterface");

                let mut result = serde_json::json!({
                    "instance_name": instance_name,
                    "instance_path": instance_path.to_string(),
                    "method": "MiInterface",
                });

                // Try calling MiInterface with GetFwVersion command (cmd_id=0x0A)
                // From Ghidra decompilation: InData = [0x55, cmd_id, 0x01, 0x01, 0x55, cmd_id, 0x01, 0x02]
                // For GetFwVersion: cmd_id = 0x0A
                let cmd_id: u8 = cmd
                    .args
                    .get("cmd_id")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u8)
                    .unwrap_or(0x0A); // Default: GetFwVersion

                let in_data: Vec<u8> = vec![0x55, cmd_id, 0x01, 0x01, 0x55, cmd_id, 0x01, 0x02];
                result["cmd_id"] = serde_json::json!(format!("0x{:02x}", cmd_id));
                result["in_data"] = serde_json::json!(in_data
                    .iter()
                    .map(|b| format!("{:02x}", b))
                    .collect::<Vec<_>>()
                    .join(" "));

                unsafe {
                    // First, get the actual instance object (not the class)
                    let mut instance_obj = None;
                    if let Err(e) = wmi.svc.GetObject(
                        &instance_path,
                        WBEM_FLAG_RETURN_WBEM_COMPLETE,
                        None,
                        Some(&mut instance_obj),
                        None,
                    ) {
                        result["error"] =
                            serde_json::json!(format!("GetObject(instance) failed: {e}"));
                        return make_ok(result);
                    }
                    let instance_obj = match instance_obj {
                        Some(c) => c,
                        None => {
                            result["error"] = serde_json::json!("instance object is None");
                            return make_ok(result);
                        }
                    };
                    let _ = instance_obj; // validated GetObject succeeded
                    result["got_instance"] = serde_json::json!(true);

                    // Get the class definition for method parameters
                    let mut class_obj = None;
                    if let Err(e) = wmi.svc.GetObject(
                        &BSTR::from("MICommonInterface"),
                        WBEM_FLAG_RETURN_WBEM_COMPLETE,
                        None,
                        Some(&mut class_obj),
                        None,
                    ) {
                        result["error"] =
                            serde_json::json!(format!("GetObject(class) failed: {e}"));
                        return make_ok(result);
                    }
                    let class_obj = match class_obj {
                        Some(c) => c,
                        None => {
                            result["error"] = serde_json::json!("class object is None");
                            return make_ok(result);
                        }
                    };

                    // Get the in-params class
                    let mut in_sig: Option<windows::Win32::System::Wmi::IWbemClassObject> = None;
                    let mut out_sig: Option<windows::Win32::System::Wmi::IWbemClassObject> = None;
                    if let Err(e) = class_obj.GetMethod(
                        &method_name,
                        0,
                        &mut in_sig as *mut _,
                        &mut out_sig as *mut _,
                    ) {
                        result["error"] = serde_json::json!(format!("GetMethod failed: {e}"));
                        // List available methods
                        match class_obj.GetNames(
                            None,
                            windows::Win32::System::Wmi::WBEM_FLAG_NONSYSTEM_ONLY,
                            std::ptr::null(),
                        ) {
                            Ok(psa) => {
                                if !psa.is_null() {
                                    let sa = &*psa;
                                    let accessor =
                                        wmi::safearray::SafeArrayAccessor::<BSTR>::new(sa);
                                    if let Ok(acc) = accessor {
                                        let names: Vec<String> =
                                            acc.as_slice().iter().map(|b| b.to_string()).collect();
                                        result["available_members"] = serde_json::json!(names);
                                    }
                                }
                            }
                            Err(e) => {
                                result["getnames_error"] = serde_json::json!(format!("{e}"));
                            }
                        }
                    }
                    let in_sig = match in_sig {
                        Some(s) => s,
                        None => {
                            result["error"] = serde_json::json!("in-params class is None");
                            return make_ok(result);
                        }
                    };

                    // Spawn an instance
                    let in_params = match in_sig.SpawnInstance(0) {
                        Ok(p) => p,
                        Err(e) => {
                            result["error"] =
                                serde_json::json!(format!("SpawnInstance failed: {e}"));
                            return make_ok(result);
                        }
                    };

                    // Set InData parameter (uint8[])
                    // Create a VARIANT containing a SAFEARRAY of UI1 (unsigned bytes)
                    let in_data_variant = {
                        use windows::Win32::System::Com::SAFEARRAYBOUND;
                        use windows::Win32::System::Ole::{SafeArrayCreate, SafeArrayPutElement};
                        use windows::Win32::System::Variant::*;

                        let bounds = [SAFEARRAYBOUND {
                            cElements: in_data.len() as u32,
                            lLbound: 0,
                        }];
                        let psa = SafeArrayCreate(VT_UI1, 1, bounds.as_ptr());
                        if psa.is_null() {
                            result["error"] = serde_json::json!("SafeArrayCreate returned null");
                            return make_ok(result);
                        }
                        for (i, &byte) in in_data.iter().enumerate() {
                            let idx = [i as i32];
                            let _ = SafeArrayPutElement(
                                psa,
                                idx.as_ptr(),
                                &byte as *const u8 as *const _,
                            );
                        }
                        // Build VARIANT manually by writing to raw memory
                        // VARIANT is #[repr(transparent)] over imp::VARIANT
                        // imp::VARIANT layout: vt(u16) at offset 0, then wReserved1-3 (3x u16),
                        // then union at offset 8. parray is a pointer in the union.
                        let mut vt = VARIANT::new();
                        {
                            let raw_ptr = &mut vt as *mut VARIANT as *mut u8;
                            // vt field at offset 0
                            *(raw_ptr as *mut u16) = (VT_ARRAY | VT_UI1).0;
                            // parray pointer at offset 8 (after vt + 3 reserved u16s)
                            let union_ptr = (raw_ptr as *const u8).add(8)
                                as *mut *mut windows::Win32::System::Com::SAFEARRAY;
                            *union_ptr = psa;
                        }
                        vt
                    };
                    if let Err(e) = in_params.Put(&BSTR::from("InData"), 0, &in_data_variant, 0) {
                        result["error"] = serde_json::json!(format!("Put InData failed: {e}"));
                        return make_ok(result);
                    }
                    result["in_data_set"] = serde_json::json!(true);

                    // Execute the method on the instance object path
                    let mut out_params = None;
                    match wmi.svc.ExecMethod(
                        &instance_path,
                        &method_name,
                        WBEM_GENERIC_FLAG_TYPE(0),
                        None,
                        Some(&in_params),
                        Some(&mut out_params),
                        None,
                    ) {
                        Ok(_) => {
                            result["method_called"] = serde_json::json!(true);
                            if let Some(out) = out_params {
                                // Read ReturnCode
                                let mut rc = VARIANT::default();
                                let _ = out.Get(&BSTR::from("ReturnCode"), 0, &mut rc, None, None);
                                result["return_code"] = serde_json::json!(format!("{:?}", rc));

                                // Read OutData
                                let mut od = VARIANT::default();
                                let _ = out.Get(&BSTR::from("OutData"), 0, &mut od, None, None);
                                result["out_data"] = serde_json::json!(format!("{:?}", od));
                            }
                        }
                        Err(e) => {
                            result["error"] = serde_json::json!(format!("ExecMethod failed: {e}"));
                        }
                    }
                }

                make_ok(result)
            }

            #[cfg(not(windows))]
            {
                make_err("WMI only available on Windows".to_string())
            }
        }

        // ── WMAA / WMI MiInterface commands (admin required) ──────────────
        "wmi_ec_read" => {
            let fun2 = cmd.args["fun2"].as_u64().unwrap_or(0) as u16;
            let fun3 = cmd.args["fun3"].as_u64().unwrap_or(0) as u16;
            // S32-002: Allowlist check — only permit the exact (fun2, fun3)
            // combinations the app uses. Arbitrary WMAA reads would expose
            // unvalidated hardware registers to a compromised webview.
            if !is_allowed_wmi_read(fun2, fun3) {
                return make_err(format!(
                    "wmi_ec_read rejected: (fun2=0x{fun2:04X}, fun3=0x{fun3:04X}) is not in the allowlist"
                ));
            }
            match crate::hw::wmi_ec::wmi_read(fun2, fun3) {
                Ok(resp) => make_ok(serde_json::to_value(resp).unwrap_or(Value::Null)),
                Err(e) => make_err(e.to_string()),
            }
        }

        "wmi_ec_write" => {
            let fun2 = cmd.args["fun2"].as_u64().unwrap_or(0) as u16;
            let fun3 = cmd.args["fun3"].as_u64().unwrap_or(0) as u16;
            let fun4 = cmd.args["fun4"].as_u64().unwrap_or(0) as u32;
            // S32-002: Allowlist check — arbitrary (fun2, fun3, fun4) writes
            // could corrupt EC state (power limits, thermal, charging, fan).
            // Only the combinations used by the app's own set_* functions pass.
            if !is_allowed_wmi_write(fun2, fun3) {
                return make_err(format!(
                    "wmi_ec_write rejected: (fun2=0x{fun2:04X}, fun3=0x{fun3:04X}) is not in the allowlist"
                ));
            }
            match crate::hw::wmi_ec::wmi_write(fun2, fun3, fun4) {
                Ok(resp) => make_ok(serde_json::to_value(resp).unwrap_or(Value::Null)),
                Err(e) => make_err(e.to_string()),
            }
        }

        "wmi_ec_get_performance_mode" => match crate::hw::wmi_ec::get_performance_mode() {
            Ok(mode) => make_ok(serde_json::json!(format!("{mode:?}"))),
            Err(e) => make_err(e.to_string()),
        },

        "wmi_ec_set_performance_mode" => {
            let mode_val = cmd.args["mode"].as_u64().unwrap_or(6) as u16;
            let mode = crate::hw::wmi_ec::EcPerformanceMode::from_raw(mode_val)
                .unwrap_or(crate::hw::wmi_ec::EcPerformanceMode::Balanced);
            match crate::hw::wmi_ec::set_performance_mode(mode) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "wmi_ec_read_battery_health" => match crate::hw::wmi_ec::read_battery_health() {
            Ok(val) => make_ok(serde_json::json!(val)),
            Err(e) => make_err(e.to_string()),
        },

        "wmi_ec_read_adapter_power" => match crate::hw::wmi_ec::read_adapter_power() {
            Ok(val) => make_ok(serde_json::json!(val)),
            Err(e) => make_err(e.to_string()),
        },

        "wmi_ec_read_sensor_data" => match crate::hw::wmi_ec::read_sensor_data() {
            Ok(data) => make_ok(serde_json::to_value(data).unwrap_or(Value::Null)),
            Err(e) => make_err(e.to_string()),
        },

        "wmi_ec_set_brightness_data" => {
            let level = cmd.args["level"].as_u64().unwrap_or(0) as u32;
            match crate::hw::wmi_ec::set_brightness_data(level) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "wmi_ec_set_sagv_mode" => {
            let mode = cmd.args["mode"].as_u64().unwrap_or(0) as u32;
            match crate::hw::wmi_ec::set_sagv_mode(mode) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "wmi_ec_set_pl1_flag" => {
            let enabled = cmd.args["enabled"].as_bool().unwrap_or(false);
            match crate::hw::wmi_ec::set_pl1_flag(enabled) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "wmi_ec_set_epof_flag" => {
            let enabled = cmd.args["enabled"].as_bool().unwrap_or(false);
            match crate::hw::wmi_ec::set_epof_flag(enabled) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "wmi_ec_set_mi_usage_type" => {
            let enabled = cmd.args["enabled"].as_bool().unwrap_or(false);
            match crate::hw::wmi_ec::set_mi_usage_type(enabled) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "wmi_ec_set_wmid_type" => {
            let val = cmd.args["val"].as_u64().unwrap_or(0) as u32;
            match crate::hw::wmi_ec::set_wmid_type(val) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "wmi_ec_set_lid_open_type" => {
            let val = cmd.args["val"].as_u64().unwrap_or(0) as u32;
            match crate::hw::wmi_ec::set_lid_open_type(val) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "wmi_ec_set_removable_type" => {
            let val = cmd.args["val"].as_u64().unwrap_or(0) as u32;
            match crate::hw::wmi_ec::set_removable_type(val) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "wmi_ec_set_auto_illumination" => {
            let enabled = cmd.args["enabled"].as_bool().unwrap_or(false);
            match crate::hw::wmi_ec::set_auto_illumination(enabled) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "wmi_ec_set_label_mode" => {
            let enabled = cmd.args["enabled"].as_bool().unwrap_or(false);
            match crate::hw::wmi_ec::set_label_mode(enabled) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(e.to_string()),
            }
        }

        "ensure_ecram_service" => match crate::hw::ecram_service_mgmt::ensure_service_running() {
            Ok(status) => make_ok(serde_json::to_value(status).unwrap_or(Value::Null)),
            Err(e) => make_err(e.to_string()),
        },

        // Read thermal readings (ESIF + ACPI) as the elevated SYSTEM process.
        // The unprivileged MiControl process is DENIED access to the
        // `EsifDeviceInformation` and `MSAcpi_ThermalZoneTemperature` WMI
        // classes (error 0x80041003 "Access to a CIM resource was not
        // available to the client"). Running inside the bridge service (which
        // executes as NT AUTHORITY\SYSTEM) grants access, so CPU/GPU
        // temperature and TDP can be read via the pipe instead of failing.
        "read_thermal_readings" => {
            use std::collections::HashMap;
            let mut result = serde_json::json!({});

            // ESIF participants (CPU hotspot + GPU/secondary SoC _10).
            let esif = crate::hw::wmi_cache::with_wmi(|wmi| {
                let rows: Vec<HashMap<String, wmi::Variant>> = wmi
                    .raw_query("SELECT InstanceName, Temperature, Power FROM EsifDeviceInformation")
                    .unwrap_or_default();
                Ok(rows)
            });
            match esif {
                Ok(rows) if !rows.is_empty() => {
                    let participants: Vec<serde_json::Value> = rows
                        .iter()
                        .map(|r| {
                            serde_json::json!({
                                "instance": crate::util::wmi_extract::extract_string(r, "InstanceName").unwrap_or_default(),
                                "temp_c": crate::util::wmi_extract::extract_u32(r, "Temperature").unwrap_or(0),
                                "power_dw": crate::util::wmi_extract::extract_u32(r, "Power").unwrap_or(0),
                            })
                        })
                        .collect();
                    result["esif"] = serde_json::json!({
                        "ok": true,
                        "participants": participants,
                        "count": rows.len(),
                    });
                }
                Ok(_) => {
                    result["esif"] = serde_json::json!({ "ok": false, "error": "no participants" });
                }
                Err(e) => {
                    result["esif"] = serde_json::json!({ "ok": false, "error": e.to_string() });
                }
            }

            // ACPI thermal zones.
            let acpi = crate::hw::wmi_cache::with_wmi(|wmi| {
                let rows: Vec<HashMap<String, wmi::Variant>> = wmi
                    .raw_query(
                        "SELECT InstanceName, Active, CurrentTemperature, CriticalTripPoint \
                         FROM MSAcpi_ThermalZoneTemperature",
                    )
                    .unwrap_or_default();
                Ok(rows)
            });
            match acpi {
                Ok(rows) if !rows.is_empty() => {
                    let zones: Vec<serde_json::Value> = rows
                        .iter()
                        .map(|r| {
                            serde_json::json!({
                                "instance": crate::util::wmi_extract::extract_string(r, "InstanceName").unwrap_or_default(),
                                "active": crate::util::wmi_extract::extract_bool(r, "Active").unwrap_or(false),
                                // tenths of Kelvin → Celsius
                                "temp_c": crate::util::wmi_extract::extract_i32(r, "CurrentTemperature").map(|t| (t as f64 / 10.0) - 273.15).unwrap_or(0.0),
                            })
                        })
                        .collect();
                    result["acpi"] = serde_json::json!({
                        "ok": true,
                        "zones": zones,
                        "count": rows.len(),
                    });
                }
                Ok(_) => {
                    result["acpi"] = serde_json::json!({ "ok": false, "error": "no zones" });
                }
                Err(e) => {
                    result["acpi"] = serde_json::json!({ "ok": false, "error": e.to_string() });
                }
            }

            make_ok(result)
        }

        // S32-002: Install + start the autonomous MiControlBridge service.
        // Called once (usually at app startup) when the service pipe is not
        // available. Uses the bundled micontrol_bridge.exe `install` command.
        // Runs via the scheduled task (already elevated) so it never prompts UAC.
        "ensure_bridge_service" => match install_bridge_service() {
            Ok(status) => make_ok(serde_json::to_value(status).unwrap_or(Value::Null)),
            Err(e) => make_err(e.to_string()),
        },

        // S42-020: Install + start the MiControlFace auth service (LocalSystem).
        // Called from the Face Unlock tab when `service_installed` is false.
        // Uses the bundled micontrol_face_svc.exe `install` command. Runs via
        // the elevated helper (scheduled task / service pipe / UAC), so the
        // SCM `OpenSCManager` write access never fails with ERROR_ACCESS_DENIED.
        "face_service_install" => match install_face_service() {
            Ok(status) => make_ok(serde_json::to_value(status).unwrap_or(Value::Null)),
            Err(e) => make_err(e.to_string()),
        },

        // Post-reboot self-heal: MiControlFace crashes ~60 min after boot
        // (0xc0000005 in FrameServerClient.dll_unloaded — MSMF camera in a
        // Session-0 SYSTEM service) and the SCM never restarts it because the
        // service has NO failure actions configured (unlike MiControlBridge /
        // IoTSvc which have RESTART 5/10/30s and therefore survive). This
        // command queries the SCM; if the service exists but is not RUNNING it
        // (a) configures failure actions (RESTART 5/10/30s, reset 1 day) so the
        // SCM auto-restarts it after future crashes, and (b) starts it if it is
        // STOPPED. Runs via the autonomous MiControlBridge service or the
        // MiControlElevated task — never a UAC prompt.
        "ensure_face_service" => match ensure_face_service() {
            Ok(status) => make_ok(serde_json::to_value(status).unwrap_or(Value::Null)),
            Err(e) => make_err(e.to_string()),
        },

        // S43-001: Store the Windows sign-in password in an LSA Secret. Requires
        // `POLICY_CREATE_SECRET` (SYSTEM/elevated) — routed through the elevated
        // helper so the unprivileged app UI can save it once at enrollment time.
        // The password is wiped from memory by the caller-side SecureZeroMemory.
        "face_set_password" => {
            let user = cmd.args["user"].as_str().unwrap_or_default().to_string();
            let password = cmd.args["password"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            if user.is_empty() || password.is_empty() {
                return make_err("face_set_password: missing user or password".to_string());
            }
            match crate::hw::face::credvault::store_password(&user, &password) {
                Ok(()) => make_ok(Value::Null),
                Err(e) => make_err(format!("face_set_password: {e}")),
            }
        }

        // S43-002: Check whether an LSA Secret exists for `user`. Read requires
        // `POLICY_GET_PRIVATE_INFORMATION` (SYSTEM/elevated), so this must run
        // elevated too — but it should never prompt UAC (read-only probe).
        "face_password_configured" => {
            let user = cmd.args["user"].as_str().unwrap_or_default().to_string();
            if user.is_empty() {
                return make_err("face_password_configured: missing user".to_string());
            }
            let configured = crate::hw::face::credvault::read_password(&user).is_ok();
            make_ok(serde_json::json!({ "configured": configured, "unknown": false }))
        }

        unknown => make_err(format!("Unknown elevated command: {unknown}")),
    }
}

/// Public in-process dispatch: called by `elev_bridge::run_elevated` when the
/// main process is already running as an administrator.  Avoids the scheduled-
/// task round-trip entirely.
pub fn dispatch_cmd(cmd: &str, args: Value) -> Value {
    dispatch(ElevCmd {
        _protocol_version: None,
        _request_id: None,
        _created_at_ms: None,
        nonce: None,
        _hmac: None,
        _caller_pid: None,
        cmd: cmd.to_string(),
        args,
    })
}

/// S32-002: Install and start the autonomous `MiControlBridge` service.
///
/// Locates the bundled `micontrol_bridge.exe` (next to the current exe, in
/// Program Files, or in the dev target dir) and runs its `install` command.
/// Returns a status object describing the outcome.
#[cfg(windows)]
fn install_bridge_service() -> Result<Value, String> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    // Find the bridge executable.
    let bridge_exe = find_bridge_exe().ok_or_else(|| {
        "micontrol_bridge.exe not found (not bundled with this installation)".to_string()
    })?;

    // S36-006 (pipe-DACL self-heal): ensure the deployed bridge binary is the
    // one we just found. If the found binary lives in the dev target dir while
    // the installed copy under Program Files is stale (e.g. pre-SDDL-DACL),
    // copy the fresh binary over the installed path so the service is
    // (re)installed from the current code. This runs inside the elevated
    // helper, so writing to `C:\Program Files\miControl\` is permitted.
    let installed_bridge = std::env::var("ProgramFiles")
        .map(|pf| {
            PathBuf::from(&pf)
                .join("miControl")
                .join("micontrol_bridge.exe")
        })
        .unwrap_or_else(|_| bridge_exe.clone());
    if installed_bridge != bridge_exe {
        let _ = std::fs::create_dir_all(
            installed_bridge
                .parent()
                .unwrap_or(PathBuf::new().as_path()),
        );
        if let Err(e) = std::fs::copy(&bridge_exe, &installed_bridge) {
            log::warn!(
                "install_bridge_service: could not refresh installed bridge copy \
                 ({installed_bridge:?}) from {bridge_exe:?}: {e}"
            );
        } else {
            log::info!(
                "install_bridge_service: refreshed installed bridge binary from {bridge_exe:?}"
            );
        }
    }

    // Re-locate AFTER the copy: the service must be installed from the
    // deployed (Program Files) copy when available, so the SCM record points
    // at a stable path.
    let install_exe = find_deployed_bridge_exe().unwrap_or(bridge_exe);

    let output = Command::new(&install_exe)
        .arg("install")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Failed to run micontrol_bridge install: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let combined = if stdout.is_empty() { stderr } else { stdout };
    let exit_ok = output.status.success();

    Ok(json!({
        "exit_code": output.status.code().unwrap_or(-1),
        "output": combined,
        "service_installed": exit_ok,
    }))
}

#[cfg(not(windows))]
fn install_bridge_service() -> Result<Value, String> {
    Err("Bridge service only supported on Windows".to_string())
}

/// S42-020: Install and start the `MiControlFace` auth service (LocalSystem).
///
/// Locates the bundled `micontrol_face_svc.exe` (next to the current exe, in
/// Program Files, or in the dev target dir) and runs its `install` command.
/// Mirrors `install_bridge_service`: deploys a fresh copy under Program Files
/// before registering the SCM service so the record points at a stable path.
#[cfg(windows)]
fn install_face_service() -> Result<Value, String> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    let svc_exe = find_face_svc_exe().ok_or_else(|| {
        "micontrol_face_svc.exe not found (did the build enable the `face` feature?)".to_string()
    })?;

    // Deploy/refresh the Program Files copy so the SCM record is stable.
    let installed_svc = std::env::var("ProgramFiles")
        .map(|pf| {
            PathBuf::from(&pf)
                .join("miControl")
                .join("micontrol_face_svc.exe")
        })
        .unwrap_or_else(|_| svc_exe.clone());
    if installed_svc != svc_exe {
        if let Some(parent) = installed_svc.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = std::fs::copy(&svc_exe, &installed_svc) {
            log::warn!(
                "install_face_service: could not refresh installed copy \
                 ({installed_svc:?}) from {svc_exe:?}: {e}"
            );
        } else {
            log::info!("install_face_service: refreshed installed copy from {svc_exe:?}");
        }
    }

    let install_exe = find_deployed_face_svc_exe().unwrap_or(svc_exe);

    let output = Command::new(&install_exe)
        .arg("install")
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Failed to run micontrol_face_svc install: {e}"))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let combined = if stdout.is_empty() { stderr } else { stdout };
    let exit_ok = output.status.success();

    Ok(json!({
        "exit_code": output.status.code().unwrap_or(-1),
        "output": combined,
        "service_installed": exit_ok,
    }))
}

#[cfg(not(windows))]
fn install_face_service() -> Result<Value, String> {
    Err("Face auth service only supported on Windows".to_string())
}

/// Post-reboot self-heal for the `MiControlFace` auth service.
///
/// The service crashes periodically with `0xc0000005` in
/// `FrameServerClient.dll_unloaded` (MSMF webcam capture inside a Session-0
/// SYSTEM service); because no `sc failure` actions were ever configured
/// (unlike MiControlBridge / IoTSvc), the SCM leaves it STOPPED-1067 forever
/// after the first crash — breaking Face Unlock after every reboot.
///
/// This performs three idempotent steps (all elevated):
///   1. Configure `sc failure` = RESTART 5000/10000/30000 ms, reset 86400 s,
///      so future crashes are auto-restarted by the SCM.
///   2. If the service exists but is not RUNNING, start it (`sc start`, which
///      tolerates ERROR_SERVICE_ALREADY_RUNNING and the SCM start races).
///   3. Return a status object (service_installed / running / action taken).
#[cfg(windows)]
fn ensure_face_service() -> Result<Value, String> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    // ── 1. Query SCM state via `sc query` (robust parsing, no win32 structs).
    let query = Command::new("sc.exe")
        .args(["query", "MiControlFace"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("sc query MiControlFace failed: {e}"))?;
    let query_text = String::from_utf8_lossy(&query.stdout).to_string();
    let state = query_text
        .lines()
        .find(|l| l.contains("STATE"))
        .map(|l| l.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    if query_text.contains("does not exist") || query_text.contains("service has not been started")
    {
        // Service not registered — nothing to heal. The UI installer path
        // (`face_service_install`) handles a fresh install.
        return Ok(json!({
            "service_installed": false,
            "service_running": false,
            "state": state,
            "action": "not_installed",
        }));
    }

    let running = state.contains("RUNNING");
    let stopped = state.contains("STOPPED");
    let start_pending = state.contains("START_PENDING");

    // ── 2. Configure failure actions (idempotent; `sc failure` overwrites).
    let failure = Command::new("sc.exe")
        .args([
            "failure",
            "MiControlFace",
            "reset=",
            "86400",
            "actions=",
            "restart/5000/restart/10000/restart/30000",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("sc failure MiControlFace failed: {e}"))?;
    let failure_ok = failure.status.success();
    if !failure_ok {
        log::warn!(
            "[ensure_face_service] sc failure action failed: {}",
            String::from_utf8_lossy(&failure.stderr).trim()
        );
    }

    // ── 3. Start if stopped (or pending — let it settle, report as started).
    let mut action = "already_running".to_string();
    if stopped || start_pending {
        let start = Command::new("sc.exe")
            .args(["start", "MiControlFace"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("sc start MiControlFace failed: {e}"))?;
        let start_text = String::from_utf8_lossy(&start.stdout).to_string();
        // 1056 = ERROR_SERVICE_ALREADY_RUNNING (benign), 1058 = DISABLED.
        let started = start.status.success()
            || start_text.contains("1056")
            || start_text.contains("ALREADY_RUNNING");
        action = if started {
            "started".to_string()
        } else {
            "start_failed".to_string()
        };
    }

    Ok(json!({
        "service_installed": true,
        "service_running": running,
        "state": state,
        "action": action,
        "failure_actions_configured": failure_ok,
    }))
}

#[cfg(not(windows))]
fn ensure_face_service() -> Result<Value, String> {
    Err("Face auth service only supported on Windows".to_string())
}

/// Locate the bundled `micontrol_face_svc.exe` (dev target dir or next to the
/// current exe / Program Files).
#[cfg(windows)]
fn find_face_svc_exe() -> Option<PathBuf> {
    // 1. Same directory as current exe (installed mode).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("micontrol_face_svc.exe");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    // 2. Program Files\miControl.
    let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".to_string());
    let candidate = PathBuf::from(&pf)
        .join("miControl")
        .join("micontrol_face_svc.exe");
    if candidate.exists() {
        return Some(candidate);
    }
    // 3. Dev target dir.
    let manifest = std::env::var("CARGO_MANIFEST_DIR").ok()?;
    let candidate = PathBuf::from(manifest)
        .join("target")
        .join("debug")
        .join("micontrol_face_svc.exe");
    if candidate.exists() {
        return Some(candidate);
    }
    None
}

/// Locate the *deployed* face service binary (Program Files over dev dir).
#[cfg(windows)]
fn find_deployed_face_svc_exe() -> Option<PathBuf> {
    let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".to_string());
    let candidate = PathBuf::from(&pf)
        .join("miControl")
        .join("micontrol_face_svc.exe");
    if candidate.exists() {
        return Some(candidate);
    }
    find_face_svc_exe()
}

#[cfg(not(windows))]
fn find_deployed_face_svc_exe() -> Option<PathBuf> {
    None
}

/// Locate the bundled `micontrol_bridge.exe`.
#[cfg(windows)]
fn find_bridge_exe() -> Option<PathBuf> {
    // 0. %LOCALAPPDATA%\MiControl\micontrol_bridge.exe — an explicitly updated
    // copy the app can write to without admin (used to deploy a fixed bridge).
    if let Ok(base) = std::env::var("LOCALAPPDATA") {
        let candidate = PathBuf::from(base)
            .join("MiControl")
            .join("micontrol_bridge.exe");
        if candidate.exists() {
            return Some(candidate);
        }
    }
    // 1. Same directory as current exe (installed mode).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join("micontrol_bridge.exe");
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }
    // 2. Program Files\miControl.
    let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".to_string());
    let candidate = PathBuf::from(&pf)
        .join("miControl")
        .join("micontrol_bridge.exe");
    if candidate.exists() {
        return Some(candidate);
    }
    // 3. Dev target dir.
    let manifest = std::env::var("CARGO_MANIFEST_DIR").ok()?;
    let candidate = PathBuf::from(manifest)
        .join("target")
        .join("debug")
        .join("micontrol_bridge.exe");
    if candidate.exists() {
        return Some(candidate);
    }
    None
}

/// Locate the *deployed* bridge binary (Program Files over dev/localappdata)
/// so the SCM service record points at a stable path after self-healing.
#[cfg(windows)]
fn find_deployed_bridge_exe() -> Option<PathBuf> {
    let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".to_string());
    let candidate = PathBuf::from(&pf)
        .join("miControl")
        .join("micontrol_bridge.exe");
    if candidate.exists() {
        return Some(candidate);
    }
    find_bridge_exe()
}

#[cfg(not(windows))]
fn find_deployed_bridge_exe() -> Option<PathBuf> {
    None
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Returns `%LOCALAPPDATA%\MiControl`, creating it if needed.
pub fn elev_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA")
        .unwrap_or_else(|_| std::env::temp_dir().to_string_lossy().into_owned());
    let dir = PathBuf::from(base).join("MiControl");
    let _ = std::fs::create_dir_all(&dir);
    // S36-002: Lock down the directory itself so other users can't list files
    // or pre-create symlink attacks against elev_key.bin.
    let _ = crate::util::auth::restrict_file_acl(&dir);
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

/// Section a list of ALL pending commands (S37-002).
///
/// When the scheduled task dispatches the helper without a `--request-id`,
/// the helper drains every `elev_cmd_*.json` that has no matching result yet,
/// ordered oldest-first. This prevents newer high-frequency polls (thermal via
/// `get_fan_info`) from starving lower-frequency writes such as
/// `set_performance_mode` / `set_charging_threshold`.
fn select_all_pending_commands(
    dir: &std::path::Path,
    wanted: Option<&str>,
) -> Result<Vec<PendingCommand>, String> {
    // Fast path: explicit --request-id (UAC fallback) — exactly one command.
    if let Some(request_id) = wanted {
        let cmd_path = cmd_path_for_request(request_id);
        if !cmd_path.exists() {
            return Err(format!(
                "request-specific command file not found for request_id={request_id}"
            ));
        }
        return Ok(vec![PendingCommand {
            request_id: request_id.to_string(),
            result_path: result_path_for_request(request_id),
            cmd_path,
        }]);
    }

    let entries = std::fs::read_dir(dir).map_err(|e| format!("Cannot read elev dir: {e}"))?;

    let mut cmds: Vec<(std::time::SystemTime, String)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with("elev_cmd_") || !name.ends_with(".json") {
            continue;
        }
        // Extract request_id from filename: elev_cmd_<id>.json
        let request_id = &name["elev_cmd_".len()..name.len() - ".json".len()];

        // Skip if a result already exists for this request (already processed)
        let result_path = result_path_for_request(request_id);
        if result_path.exists() {
            continue;
        }

        let mtime = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(std::time::UNIX_EPOCH);

        // Verify the file is fresh enough to dispatch (avoid stale files from
        // previous app sessions with long-pending requests).
        cmds.push((mtime, request_id.to_string()));
    }

    // Oldest-first so earlier writes (e.g. set_performance_mode) get processed
    // before the newer thermal poll in the same batch.
    cmds.sort_by_key(|(mtime, _)| *mtime);

    Ok(cmds
        .into_iter()
        .map(|(_, request_id)| PendingCommand {
            request_id: request_id.clone(),
            cmd_path: cmd_path_for_request(&request_id),
            result_path: result_path_for_request(&request_id),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use crate::util::auth;
    use std::sync::Mutex;

    /// Serialize tests that modify LOCALAPPDATA or SEEN_NONCES.
    static NONCE_TEST_LOCK: Mutex<()> = Mutex::new(());

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

    // ── S19-08: HMAC and nonce tests ─────────────────────────────────────────

    #[test]
    fn test_hmac_sign_verify_roundtrip() {
        let key = b"test-key-32-bytes-long-1234567890";
        let data = b"elevated bridge test data";
        let tag = auth::compute_hmac(key, data).expect("HMAC should succeed");
        assert!(auth::verify_hmac(key, data, &tag));
        assert!(!auth::verify_hmac(key, b"tampered", &tag));
    }

    #[test]
    fn test_nonce_replay_detection() {
        use std::collections::HashMap;
        let _lock = NONCE_TEST_LOCK.lock().unwrap();

        // Simulate adding a nonce to the seen set
        let nonce = "replay-test-nonce-001";
        let mut map = HashMap::new();
        map.insert(nonce.to_string(), 0u64);

        // The nonce should be detected as a duplicate
        assert!(map.contains_key(nonce));

        // A different nonce should not be a duplicate
        assert!(!map.contains_key("different-nonce"));
    }

    #[test]
    fn test_nonce_persistence_save_load() {
        use std::collections::HashMap;
        let _lock = NONCE_TEST_LOCK.lock().unwrap();

        let orig = std::env::var("LOCALAPPDATA").ok();
        let tmp = std::env::temp_dir().join("micontrol_test_nonce_persist");
        std::env::set_var("LOCALAPPDATA", &tmp);

        // Use current epoch seconds so load_nonces() doesn't purge them
        // (load_nonces purges nonces older than 5 minutes).
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut map = HashMap::new();
        map.insert("nonce_a".to_string(), now);
        map.insert("nonce_b".to_string(), now + 100);

        super::save_nonces(&map);

        let loaded = super::load_nonces();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded.get("nonce_a"), Some(&now));
        assert_eq!(loaded.get("nonce_b"), Some(&(now + 100)));

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
        if let Some(orig_val) = orig {
            std::env::set_var("LOCALAPPDATA", orig_val);
        }
    }

    #[test]
    fn test_load_nonces_purges_expired() {
        use std::collections::HashMap;
        let _lock = NONCE_TEST_LOCK.lock().unwrap();

        let orig = std::env::var("LOCALAPPDATA").ok();
        let tmp = std::env::temp_dir().join("micontrol_test_nonce_expire");
        std::env::set_var("LOCALAPPDATA", &tmp);

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        let mut map = HashMap::new();
        map.insert("fresh_nonce".to_string(), now);
        map.insert("expired_nonce".to_string(), now - 400); // > 5 minutes old
        super::save_nonces(&map);

        let loaded = super::load_nonces();
        assert_eq!(loaded.len(), 1, "Expired nonce should be purged");
        assert!(loaded.contains_key("fresh_nonce"));
        assert!(!loaded.contains_key("expired_nonce"));

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
        if let Some(orig_val) = orig {
            std::env::set_var("LOCALAPPDATA", orig_val);
        }
    }

    #[test]
    fn test_flush_nonces_persists_to_disk() {
        use crate::util::panic::lock_or_recover;
        use std::collections::HashMap;
        let _lock = NONCE_TEST_LOCK.lock().unwrap();

        let orig = std::env::var("LOCALAPPDATA").ok();
        let tmp = std::env::temp_dir().join("micontrol_test_flush");
        std::env::set_var("LOCALAPPDATA", &tmp);

        let mut map = HashMap::new();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        map.insert("flush_nonce".to_string(), now);

        {
            let mut seen = lock_or_recover(&super::SEEN_NONCES);
            *seen = Some(map);
        }

        super::flush_nonces();

        let nonce_path = super::nonce_store_path();
        assert!(nonce_path.exists(), "Nonce file should exist after flush");

        let content = std::fs::read_to_string(&nonce_path).unwrap();
        assert!(content.contains("flush_nonce"));

        // Cleanup
        *lock_or_recover(&super::SEEN_NONCES) = None;
        let _ = std::fs::remove_dir_all(&tmp);
        if let Some(orig_val) = orig {
            std::env::set_var("LOCALAPPDATA", orig_val);
        }
    }
}
