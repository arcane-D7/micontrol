//! Tauri commands for direct hardware I/O (audio, charging, IoT, performance).
//!
//! Wraps IoTService IPC, audio, and performance control for frontend invocation.

use crate::elev_bridge;
use crate::hw::audio::{
    list_audio_devices as hw_list_audio, set_playback_mute as hw_set_mute,
    set_playback_volume as hw_set_volume, AudioDeviceList, AudioVolumeResult,
};
use crate::hw::charging::{get_charging_threshold as hw_get_charge, ChargingResult};
use crate::hw::errors::{ErrorResponse, HardwareError};
use crate::hw::iotservice;
use crate::hw::iotservice::{
    BindStatusInfo, IotDeviceInfo, LaptopStatus, PowerEvent, WiFiItem, WiFiItemInfo, WiFiStatusInfo,
};
use crate::hw::performance::{
    get_perf_debug as hw_perf_debug, get_performance_mode as hw_get_perf, PerfDebugInfo,
    PerformanceResult,
};
use crate::hw::screen_cast::{
    list_cast_devices as hw_list_cast, start_casting as hw_start_cast,
    stop_casting as hw_stop_cast, CastDevice, CastResult,
};
use crate::hw::wifi::{self, WifiNetwork, WifiStatus};
use crate::state::{AppState, PerformanceMode};
use crate::util::blocking::run_blocking;
use crate::util::panic::lock_or_recover;
use tauri::State;

const RAW_ECRAM_WRITE_ENABLE_ENV: &str = "MICONTROL_ENABLE_RAW_ECRAM_WRITE";
const RAW_ECRAM_WRITE_MAX_BYTES: usize = 32;

#[tauri::command]
pub async fn get_performance_mode(
    _state: State<'_, AppState>,
) -> Result<PerformanceMode, ErrorResponse> {
    hw_get_perf().map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn set_performance_mode(
    mode: PerformanceMode,
    state: State<'_, AppState>,
) -> Result<PerformanceResult, ErrorResponse> {
    let raw =
        elev_bridge::run_elevated("set_performance_mode", serde_json::json!({ "mode": mode }))
            .await?;
    let result: PerformanceResult =
        serde_json::from_value(raw).map_err(|e| format!("Unexpected elevated result: {e}"))?;
    *lock_or_recover(&state.performance_mode) = result.mode;
    Ok(result)
}

#[tauri::command]
pub async fn get_charging_threshold(_state: State<'_, AppState>) -> Result<u8, ErrorResponse> {
    hw_get_charge().map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn set_charging_threshold(
    threshold: u8,
    state: State<'_, AppState>,
) -> Result<ChargingResult, ErrorResponse> {
    let raw = elev_bridge::run_elevated(
        "set_charging_threshold",
        serde_json::json!({ "threshold": threshold }),
    )
    .await?;
    let result: ChargingResult =
        serde_json::from_value(raw).map_err(|e| format!("Unexpected elevated result: {e}"))?;
    *lock_or_recover(&state.charging_threshold) = result.threshold;
    Ok(result)
}

/// Returns diagnostic information about the performance mode control channel:
/// - which WMI instance was found
/// - whether a live SetPerformanceMode call succeeds
/// - current registry and overlay mode
/// - VHF device path if discovered
/// This runs in the main (non-elevated) process since it's read-only.
#[tauri::command]
pub async fn get_perf_debug() -> Result<PerfDebugInfo, ErrorResponse> {
    Ok(hw_perf_debug())
}

/// Read all ACPI ERAM fields via direct IoTDriver IOCTL access.
///
/// Returns the decoded `EramMap` with all known register fields.
#[tauri::command]
pub async fn get_ecram_map() -> Result<crate::hw::ecram::EramMap, ErrorResponse> {
    run_blocking(crate::hw::ecram::read_eram_map)
        .await
        .map_err(ErrorResponse::from)
}

/// Read a named IoT region via direct IoTDriver IOCTL and return it as hex.
///
/// Supported values: `ERAM`, `SMA2`, `IOT_STATUS`, `IOT_SENSORS`.
#[tauri::command]
pub async fn get_iot_region_hex(region: String) -> Result<String, ErrorResponse> {
    run_blocking(move || {
        let bytes = crate::hw::ecram::read_named_region(&region)?;
        Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
    })
    .await
    .map_err(ErrorResponse::from)
}

/// Write raw hex bytes into EC RAM via direct IoTDriver IOCTL.
#[tauri::command]
pub async fn write_iot_hex(address: String, hex_data: String) -> Result<(), ErrorResponse> {
    run_blocking(move || {
        let addr = u64::from_str_radix(address.trim_start_matches("0x"), 16)
            .map_err(|e| HardwareError::Other(format!("invalid address: {e}")))?;

        let normalized: String = hex_data
            .chars()
            .filter(|c| !c.is_ascii_whitespace() && *c != ',' && *c != '-')
            .collect();

        if normalized.is_empty() || !normalized.len().is_multiple_of(2) {
            return Err(HardwareError::Other(
                "hex_data must contain an even number of hex digits".to_string(),
            ));
        }

        let bytes = (0..normalized.len())
            .step_by(2)
            .map(|i| {
                u8::from_str_radix(&normalized[i..i + 2], 16)
                    .map_err(|e| HardwareError::Other(format!("invalid hex byte: {e}")))
            })
            .collect::<Result<Vec<u8>, HardwareError>>()?;

        let is_known_safe = is_known_safe_single_byte_write(addr, bytes.as_slice());
        if !is_known_safe {
            if !raw_ecram_write_enabled() {
                return Err(HardwareError::Other(format!(
                    "Raw ECRAM writes are disabled. Set {}=1 to enable advanced writes.",
                    RAW_ECRAM_WRITE_ENABLE_ENV
                )));
            }
            if bytes.len() > RAW_ECRAM_WRITE_MAX_BYTES {
                return Err(HardwareError::Other(format!(
                    "Raw write too large: {} bytes (max {})",
                    bytes.len(),
                    RAW_ECRAM_WRITE_MAX_BYTES
                )));
            }
            if !is_eram_range(addr, bytes.len()) {
                return Err(HardwareError::Other(format!(
                    "Raw write denied: address range must stay inside ERAM (0x{:#X}..0x{:#X})",
                    crate::hw::ecram::ERAM_BASE,
                    crate::hw::ecram::ERAM_BASE + crate::hw::ecram::ERAM_SIZE as u64
                )));
            }
        }

        crate::hw::ecram::write_ecram(addr, &bytes)
    })
    .await
    .map_err(ErrorResponse::from)
}

/// Read `count` bytes (1–256) from ECRAM at `address` via direct IoTDriver IOCTL.
///
/// Returns the bytes as a lowercase hex string.  Requires the process to be
/// running elevated (administrator).
#[tauri::command]
pub async fn read_ecram_raw(address: String, count: u32) -> Result<String, ErrorResponse> {
    run_blocking(move || {
        let addr = u64::from_str_radix(address.trim_start_matches("0x"), 16)
            .map_err(|e| HardwareError::Other(format!("invalid address: {e}")))?;

        if !(1..=256).contains(&count) {
            return Err(HardwareError::Other("count must be 1–256".to_string()));
        }

        let bytes = crate::hw::ecram::read_ecram(addr, count as usize)?;
        Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
    })
    .await
    .map_err(ErrorResponse::from)
}

/// Returns whether the current process is running with an elevated (Administrator) token.
#[tauri::command]
pub fn is_elevated() -> bool {
    crate::hw::ecram::is_process_elevated()
}

// ── IoTService IPC commands ──────────────────────────────────────────────────

/// Check whether the IoTService named pipe is available.
#[tauri::command]
pub async fn iot_pipe_available() -> Result<bool, ErrorResponse> {
    Ok(iotservice::is_available())
}

/// Get all available IoT device info via IoTService IPC.
#[tauri::command]
pub async fn get_iot_device_info() -> Result<IotDeviceInfo, ErrorResponse> {
    run_blocking(move || Ok(iotservice::get_device_info()))
        .await
        .map_err(ErrorResponse::from)
}

/// Get the device model via IoTService IPC.
#[tauri::command]
pub async fn get_iot_model() -> Result<String, ErrorResponse> {
    run_blocking(iotservice::get_model)
        .await
        .map_err(ErrorResponse::from)
}

/// Get the firmware version via IoTService IPC.
#[tauri::command]
pub async fn get_iot_fw_version() -> Result<String, ErrorResponse> {
    run_blocking(iotservice::get_fw_version)
        .await
        .map_err(ErrorResponse::from)
}

/// Get the IoT device bind status via IoTService IPC.
#[tauri::command]
pub async fn get_iot_bind_status() -> Result<BindStatusInfo, ErrorResponse> {
    run_blocking(iotservice::get_bind_status)
        .await
        .map_err(ErrorResponse::from)
}

/// Get the IoT device ID via IoTService IPC.
#[tauri::command]
pub async fn get_iot_device_id() -> Result<i64, ErrorResponse> {
    run_blocking(iotservice::get_device_id)
        .await
        .map_err(ErrorResponse::from)
}

/// Get the current device status via IoTService IPC.
#[tauri::command]
pub async fn get_iot_device_status() -> Result<String, ErrorResponse> {
    run_blocking(iotservice::get_device_status)
        .await
        .map_err(ErrorResponse::from)
}

/// Report laptop status to the IoT device via IPC.
///
/// Valid status values: `win_ready`, `suspending`, `shutting`.
#[tauri::command]
pub async fn send_iot_laptop_status(status: String) -> Result<(), ErrorResponse> {
    let status = match status.as_str() {
        "win_ready" => LaptopStatus::WinReady,
        "suspending" => LaptopStatus::Suspending,
        "shutting" => LaptopStatus::Shutting,
        _ => {
            return Err(ErrorResponse {
                code: "INVALID_STATUS".to_string(),
                message: format!("Invalid laptop status: {status}"),
            })
        }
    };
    run_blocking(move || iotservice::send_laptop_status(status))
        .await
        .map_err(ErrorResponse::from)
}

/// Send WinReady status via IoTService IPC.
#[tauri::command]
pub async fn iot_report_windows_ready() -> Result<(), ErrorResponse> {
    run_blocking(iotservice::report_windows_ready)
        .await
        .map_err(ErrorResponse::from)
}

/// Get WiFi connection status via IoTService IPC.
#[tauri::command]
pub async fn get_iot_wifi_status() -> Result<WiFiStatusInfo, ErrorResponse> {
    run_blocking(iotservice::read_wifi_status)
        .await
        .map_err(ErrorResponse::from)
}

/// Get the number of provisioned WiFi networks via IoTService IPC.
#[tauri::command]
pub async fn get_iot_wifi_count() -> Result<u32, ErrorResponse> {
    run_blocking(iotservice::read_wifi_count)
        .await
        .map_err(ErrorResponse::from)
}

/// Get a WiFi item by index via IoTService IPC.
#[tauri::command]
pub async fn get_iot_wifi_by_index(index: u32) -> Result<WiFiItemInfo, ErrorResponse> {
    run_blocking(move || iotservice::get_wifi_by_index(index))
        .await
        .map_err(ErrorResponse::from)
}

/// Force IoT device to connect to provisioned WiFi via IoTService IPC.
#[tauri::command]
pub async fn iot_connect_wifi() -> Result<(), ErrorResponse> {
    run_blocking(iotservice::connect_wifi)
        .await
        .map_err(ErrorResponse::from)
}

/// Write a WiFi network to the IoT device provisioning list via IPC.
#[tauri::command]
pub async fn iot_write_wifi_item(item: WiFiItem) -> Result<(), ErrorResponse> {
    run_blocking(move || iotservice::write_wifi_item(&item))
        .await
        .map_err(ErrorResponse::from)
}

/// Delete a WiFi network from the IoT device by SSID via IPC.
#[tauri::command]
pub async fn iot_delete_wifi_item(ssid: String) -> Result<(), ErrorResponse> {
    run_blocking(move || iotservice::delete_wifi_item(&ssid))
        .await
        .map_err(ErrorResponse::from)
}

/// Clear all provisioned WiFi networks on the IoT device via IPC.
#[tauri::command]
pub async fn iot_empty_wifi_items() -> Result<(), ErrorResponse> {
    run_blocking(iotservice::empty_wifi_items)
        .await
        .map_err(ErrorResponse::from)
}

/// Set the IoT device status via IPC.
#[tauri::command]
pub async fn iot_set_device_status(status: String) -> Result<(), ErrorResponse> {
    run_blocking(move || iotservice::set_device_status(&status))
        .await
        .map_err(ErrorResponse::from)
}

/// Reset the IoT device via IPC.
#[tauri::command]
pub async fn iot_reset_device() -> Result<(), ErrorResponse> {
    run_blocking(iotservice::reset_device)
        .await
        .map_err(ErrorResponse::from)
}

/// Send a power event notification to IoTService via IPC.
#[tauri::command]
pub async fn iot_notify_power_event(event: PowerEvent) -> Result<(), ErrorResponse> {
    run_blocking(move || iotservice::notify_power_event(&event))
        .await
        .map_err(ErrorResponse::from)
}

/// Send an EC event notification to IoTService via IPC.
#[tauri::command]
pub async fn iot_notify_ec_event(event_func: u32, event_value: u32) -> Result<(), ErrorResponse> {
    run_blocking(move || iotservice::notify_ec_event(event_func, event_value))
        .await
        .map_err(ErrorResponse::from)
}

/// Send Suspending status via IoTService IPC.
#[tauri::command]
pub async fn iot_report_suspending() -> Result<(), ErrorResponse> {
    run_blocking(iotservice::report_suspending)
        .await
        .map_err(ErrorResponse::from)
}

/// Send Shutting status via IoTService IPC.
#[tauri::command]
pub async fn iot_report_shutting_down() -> Result<(), ErrorResponse> {
    run_blocking(iotservice::report_shutting_down)
        .await
        .map_err(ErrorResponse::from)
}

/// Re-launch the application as administrator (UAC prompt) and exit the current instance.
///
/// This triggers the standard Windows UAC prompt.  If the user approves, a new
/// elevated instance of the app starts and this instance exits.
#[tauri::command]
pub async fn relaunch_as_admin(app: tauri::AppHandle) -> Result<(), ErrorResponse> {
    #[cfg(windows)]
    {
        crate::elev_bridge::relaunch_self_as_admin()?;
        app.exit(0);
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        return Err(ErrorResponse::new(
            "NOT_SUPPORTED",
            "re-launch as admin is only supported on Windows",
        ));
    }
    #[allow(unreachable_code)]
    Ok(())
}

fn raw_ecram_write_enabled() -> bool {
    std::env::var(RAW_ECRAM_WRITE_ENABLE_ENV)
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            v == "1" || v == "true" || v == "yes" || v == "on"
        })
        .unwrap_or(false)
}

fn is_eram_range(addr: u64, len: usize) -> bool {
    if len == 0 {
        return false;
    }
    let start = crate::hw::ecram::ERAM_BASE;
    let end = start + crate::hw::ecram::ERAM_SIZE as u64;
    let write_end = addr.saturating_add(len as u64);
    addr >= start && write_end <= end
}

fn is_known_safe_single_byte_write(addr: u64, data: &[u8]) -> bool {
    if data.len() != 1 || !is_eram_range(addr, 1) {
        return false;
    }
    let offset = (addr - crate::hw::ecram::ERAM_BASE) as usize;
    matches!(
        offset,
        0x1B | 0x40 | 0x42 | 0x4A | 0x4B | 0x68 | 0x96 | 0xAE | 0xB2
    )
}

// ── WiFi commands ────────────────────────────────────────────────────────

/// Scan for available WiFi networks.
#[tauri::command]
pub async fn wifi_scan() -> Result<Vec<WifiNetwork>, ErrorResponse> {
    run_blocking(wifi::scan_networks)
        .await
        .map_err(ErrorResponse::from)
}

/// Get current WiFi connection status.
#[tauri::command]
pub async fn wifi_status() -> Result<WifiStatus, ErrorResponse> {
    run_blocking(wifi::get_status)
        .await
        .map_err(ErrorResponse::from)
}

/// Connect to a WiFi network.
#[tauri::command]
pub async fn wifi_connect(ssid: String, password: Option<String>) -> Result<(), ErrorResponse> {
    run_blocking(move || wifi::connect(&ssid, password.as_deref()))
        .await
        .map_err(ErrorResponse::from)
}

/// Disconnect from current WiFi network.
#[tauri::command]
pub async fn wifi_disconnect() -> Result<(), ErrorResponse> {
    run_blocking(wifi::disconnect)
        .await
        .map_err(ErrorResponse::from)
}

// ── Audio device commands ──────────────────────────────────────────────────

/// List all audio devices grouped by playback/capture.
#[tauri::command]
pub async fn get_audio_devices() -> Result<AudioDeviceList, ErrorResponse> {
    run_blocking(hw_list_audio)
        .await
        .map_err(ErrorResponse::from)
}

/// Get the current playback volume and mute state.
#[tauri::command]
pub async fn get_audio_volume() -> Result<AudioVolumeResult, ErrorResponse> {
    run_blocking(crate::hw::audio::get_playback_volume)
        .await
        .map_err(ErrorResponse::from)
}

/// Set the master playback volume (0-100).
#[tauri::command]
pub async fn set_audio_volume(volume: u8) -> Result<AudioVolumeResult, ErrorResponse> {
    run_blocking(move || hw_set_volume(volume))
        .await
        .map_err(ErrorResponse::from)
}

/// Mute/unmute the default playback device.
#[tauri::command]
pub async fn set_audio_mute(muted: bool) -> Result<AudioVolumeResult, ErrorResponse> {
    run_blocking(move || hw_set_mute(muted))
        .await
        .map_err(ErrorResponse::from)
}

// ── Screen Cast commands ──────────────────────────────────────────────────

/// List available Miracast/WiDi receivers.
#[tauri::command]
pub async fn get_cast_devices() -> Result<Vec<CastDevice>, ErrorResponse> {
    run_blocking(hw_list_cast)
        .await
        .map_err(ErrorResponse::from)
}

/// Start casting to a device.
#[tauri::command]
pub async fn start_casting(device_id: String) -> Result<CastResult, ErrorResponse> {
    run_blocking(move || hw_start_cast(&device_id))
        .await
        .map_err(ErrorResponse::from)
}

/// Stop casting.
#[tauri::command]
pub async fn stop_casting() -> Result<CastResult, ErrorResponse> {
    run_blocking(hw_stop_cast)
        .await
        .map_err(ErrorResponse::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_safe_offsets_are_allowed() {
        let addr = crate::hw::ecram::ERAM_BASE + 0x96;
        assert!(is_known_safe_single_byte_write(addr, &[80]));
    }

    #[test]
    fn non_whitelisted_offset_is_not_known_safe() {
        let addr = crate::hw::ecram::ERAM_BASE + 0x10;
        assert!(!is_known_safe_single_byte_write(addr, &[1]));
    }

    #[test]
    fn eram_range_check_rejects_outside() {
        let addr = crate::hw::ecram::ERAM_BASE + crate::hw::ecram::ERAM_SIZE as u64 + 1;
        assert!(!is_eram_range(addr, 1));
    }
}
