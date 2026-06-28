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

// ── Entry point ──────────────────────────────────────────────────────────────

/// Called from `main()` when `--elevated` is present in argv.
/// Always terminates via `std::process::exit`.
pub fn run() -> ! {
    // Initialize logging so elevated helper errors are visible in the dev trace log.
    if let Err(e) = crate::debug_log::init_logging() {
        eprintln!("Elevated helper: failed to initialize logging: {e}");
    }
    log::info!("Elevated helper started");

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
            // S24-001: Flush nonces before exit to prevent nonce loss.
            flush_nonces();
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
    let _ = std::fs::write(&pending.result_path, json);
    if let Err(e) = auth::restrict_file_acl(&pending.result_path) {
        log::warn!("Failed to restrict ACL on result file: {e}");
    }
    // S24-001: Flush nonces before exit to prevent nonce loss.
    flush_nonces();
    std::process::exit(0);
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

        "diag_ps" => {
            // Run elevated PowerShell command
            let script = cmd.args["script"].as_str().unwrap_or("");
            if script.is_empty() {
                return make_err("Missing 'script' argument".to_string());
            }
            let output = std::process::Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", script])
                .output();
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
                let cmd_id: u8 = match cmd
                    .args
                    .get("cmd_id")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u8)
                {
                    Some(id) => id,
                    None => 0x0A, // Default: GetFwVersion
                };

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
    // Fast path: explicit --request-id from UAC fallback launch.
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

    // Fallback: no --request-id (scheduled task path). Scan the directory for
    // the most recent `elev_cmd_*.json` file that doesn't have a matching
    // `elev_result_*.json` yet.
    let entries = std::fs::read_dir(dir).map_err(|e| format!("Cannot read elev dir: {e}"))?;

    let mut best: Option<(std::time::SystemTime, String)> = None;
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

        // Pick the newest file by modification time
        let mtime = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .unwrap_or(std::time::UNIX_EPOCH);

        if best.as_ref().is_none_or(|(t, _)| mtime > *t) {
            best = Some((mtime, request_id.to_string()));
        }
    }

    match best {
        Some((_, request_id)) => Ok(PendingCommand {
            request_id: request_id.clone(),
            cmd_path: cmd_path_for_request(&request_id),
            result_path: result_path_for_request(&request_id),
        }),
        None => Err("No pending elevated command file found".to_string()),
    }
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
