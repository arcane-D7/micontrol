//! Tauri commands for system-level operations (display, fan, battery, etc.).
//!
//! Delegates to the `hw` module for each hardware domain and wraps
//! results in Tauri-compatible response types.

use crate::elev_bridge;
use crate::hw::audio::{get_playback_volume as hw_get_audio, AudioVolumeResult};
use crate::hw::battery::{get_battery_info as hw_get_battery, BatteryInfo};
use crate::hw::charging::get_charging_threshold as hw_get_charge;
use crate::hw::discovery::{global_profile, HardwareProfile};
use crate::hw::display::{
    get_ai_brightness_config as hw_get_ai_cfg, get_available_refresh_rates as hw_get_refresh_rates,
    get_display_info as hw_get_display, set_hdr as hw_set_hdr, AiBrightnessConfig, DisplayInfo,
};
use crate::hw::errors::ErrorResponse;
use crate::hw::fan::{get_fan_info as hw_get_fan, FanInfo, FanMode};
use crate::hw::performance::get_performance_mode as hw_get_perf;
use crate::hw::processes::{get_process_list as hw_get_processes, ProcessInfo};
use crate::hw::startup::{get_autostart as hw_get_autostart, set_autostart as hw_set_autostart};
use crate::hw::system_info::{get_system_info as hw_get_sysinfo, SystemInfo};
use crate::hw::touchpad::{
    get_touchpad_info as hw_get_touchpad, set_touchpad_edge_slide as hw_set_touchpad_edge_slide,
    set_touchpad_gesture_screenshot as hw_set_touchpad_gesture_screenshot,
    set_touchpad_haptics as hw_set_touchpad_haptics,
    set_touchpad_haptics_intensity as hw_set_touchpad_haptics_intensity,
    set_touchpad_repress as hw_set_touchpad_repress,
    set_touchpad_sensitivity as hw_set_touchpad_sensitivity, TouchpadInfo, TouchpadSensitivity,
};
use crate::hw::update::{
    get_update_status as hw_get_update_status, trigger_driver_scan as hw_trigger_scan, UpdateStatus,
};
use crate::state::PerformanceMode;

#[tauri::command]
pub async fn get_battery_info() -> Result<BatteryInfo, ErrorResponse> {
    let started = std::time::Instant::now();
    log::debug!(target: "cmd::system", "get_battery_info: start");
    let result = tokio::task::spawn_blocking(hw_get_battery)
        .await
        .map_err(|e| format!("blocking task panicked: {e}"))?
        .map_err(ErrorResponse::from);
    match &result {
        Ok(info) => log::debug!(
            target: "cmd::system",
            "get_battery_info: ok plugged={} charging={} voltage_mv={} charge_rate_mw={} ac_input_power_mw={:?} elapsed_ms={}",
            info.is_plugged,
            info.is_charging,
            info.voltage_mv,
            info.charge_rate_mw,
            info.ac_input_power_mw,
            started.elapsed().as_millis()
        ),
        Err(error) => log::warn!(
            target: "cmd::system",
            "get_battery_info: failed after {} ms: {}",
            started.elapsed().as_millis(),
            error.message
        ),
    }
    result
}

#[tauri::command]
pub async fn get_display_info() -> Result<DisplayInfo, ErrorResponse> {
    tokio::task::spawn_blocking(hw_get_display)
        .await
        .map_err(|e| format!("blocking task panicked: {e}"))?
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn set_brightness(level: u8) -> Result<(), ErrorResponse> {
    // If auto-brightness is active, record the delta so the adaptive loop
    // uses the user's chosen value as the new shifted baseline rather than
    // reverting to the pure lux-based calculation.
    let cfg = hw_get_ai_cfg();
    if cfg.enabled {
        crate::hw::display::record_user_brightness_override(level);
    }
    elev_bridge::run_elevated("set_brightness", serde_json::json!({ "level": level }))
        .await
        .map(|_| ())
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn set_hdr(enabled: bool) -> Result<(), ErrorResponse> {
    // DisplayConfigSetDeviceInfo operates on the current user's interactive
    // session and does NOT require administrator privileges — call directly.
    tokio::task::spawn_blocking(move || hw_set_hdr(enabled))
        .await
        .map_err(|e| format!("blocking task panicked: {e}"))?
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn set_ai_brightness(enabled: bool) -> Result<(), ErrorResponse> {
    // Always reset the user override when toggling auto-brightness so the
    // loop starts fresh with no inherited delta.
    crate::hw::display::clear_user_brightness_override();
    elev_bridge::run_elevated(
        "set_ai_brightness",
        serde_json::json!({ "enabled": enabled }),
    )
    .await
    .map(|_| ())
    .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn get_ai_brightness_config() -> Result<AiBrightnessConfig, ErrorResponse> {
    Ok(hw_get_ai_cfg())
}

#[tauri::command]
pub async fn set_ai_brightness_config(config: AiBrightnessConfig) -> Result<(), ErrorResponse> {
    // Config change invalidates the old offset (different curve parameters).
    crate::hw::display::clear_user_brightness_override();
    elev_bridge::run_elevated(
        "set_ai_brightness_config",
        serde_json::json!({ "config": config }),
    )
    .await
    .map(|_| ())
    .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn get_fan_info() -> Result<FanInfo, ErrorResponse> {
    tokio::task::spawn_blocking(hw_get_fan)
        .await
        .map_err(|e| format!("blocking task panicked: {e}"))?
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn set_fan_mode(mode: FanMode, speed_percent: u8) -> Result<(), ErrorResponse> {
    elev_bridge::run_elevated(
        "set_fan_mode",
        serde_json::json!({ "mode": mode, "speed_percent": speed_percent }),
    )
    .await
    .map(|_| ())
    .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn get_touchpad_info() -> Result<TouchpadInfo, ErrorResponse> {
    let started = std::time::Instant::now();
    log::debug!(target: "cmd::system", "get_touchpad_info: start");
    let result = tokio::task::spawn_blocking(hw_get_touchpad)
        .await
        .map_err(|e| format!("blocking task panicked: {e}"))?
        .map_err(ErrorResponse::from);
    match &result {
        Ok(info) => log::debug!(
            target: "cmd::system",
            "get_touchpad_info: ok sensitivity={:?} haptics={} gesture_screenshot={} repress={} edge_slide={} elapsed_ms={}",
            info.sensitivity,
            info.haptics_enabled,
            info.gesture_screenshot,
            info.trackpad_repress,
            info.edge_slide,
            started.elapsed().as_millis()
        ),
        Err(error) => log::warn!(
            target: "cmd::system",
            "get_touchpad_info: failed after {} ms: {}",
            started.elapsed().as_millis(),
            error.message
        ),
    }
    result
}

#[tauri::command]
pub async fn set_touchpad_sensitivity(
    sensitivity: TouchpadSensitivity,
) -> Result<(), ErrorResponse> {
    tokio::task::spawn_blocking(move || {
        hw_set_touchpad_sensitivity(sensitivity).map_err(ErrorResponse::from)
    })
    .await
    .map_err(|e| format!("blocking task panicked: {e}"))?
}

#[tauri::command]
pub async fn set_touchpad_haptics(enabled: bool) -> Result<(), ErrorResponse> {
    tokio::task::spawn_blocking(move || {
        hw_set_touchpad_haptics(enabled).map_err(ErrorResponse::from)
    })
    .await
    .map_err(|e| format!("blocking task panicked: {e}"))?
}

#[tauri::command]
pub async fn set_touchpad_haptics_intensity(
    intensity: crate::hw::touchpad::HapticsIntensity,
) -> Result<(), ErrorResponse> {
    tokio::task::spawn_blocking(move || {
        hw_set_touchpad_haptics_intensity(intensity).map_err(ErrorResponse::from)
    })
    .await
    .map_err(|e| format!("blocking task panicked: {e}"))?
}

#[tauri::command]
pub async fn set_touchpad_gesture_screenshot(enabled: bool) -> Result<(), ErrorResponse> {
    tokio::task::spawn_blocking(move || {
        hw_set_touchpad_gesture_screenshot(enabled).map_err(ErrorResponse::from)
    })
    .await
    .map_err(|e| format!("blocking task panicked: {e}"))?
}

#[tauri::command]
pub async fn set_touchpad_repress(enabled: bool) -> Result<(), ErrorResponse> {
    tokio::task::spawn_blocking(move || {
        hw_set_touchpad_repress(enabled).map_err(ErrorResponse::from)
    })
    .await
    .map_err(|e| format!("blocking task panicked: {e}"))?
}

#[tauri::command]
pub async fn set_touchpad_edge_slide(enabled: bool) -> Result<(), ErrorResponse> {
    tokio::task::spawn_blocking(move || {
        hw_set_touchpad_edge_slide(enabled).map_err(ErrorResponse::from)
    })
    .await
    .map_err(|e| format!("blocking task panicked: {e}"))?
}

#[tauri::command]
pub async fn get_system_info() -> Result<SystemInfo, ErrorResponse> {
    tokio::task::spawn_blocking(hw_get_sysinfo)
        .await
        .map_err(|e| format!("blocking task panicked: {e}"))?
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn get_process_list() -> Result<Vec<ProcessInfo>, ErrorResponse> {
    tokio::task::spawn_blocking(hw_get_processes)
        .await
        .map_err(|e| ErrorResponse::from(format!("blocking task panicked: {e}")))
}

#[tauri::command]
pub async fn get_available_refresh_rates() -> Vec<u32> {
    tokio::task::spawn_blocking(hw_get_refresh_rates)
        .await
        .unwrap_or_default()
}

#[tauri::command]
pub async fn set_refresh_rate(hz: u32) -> Result<(), ErrorResponse> {
    elev_bridge::run_elevated("set_refresh_rate", serde_json::json!({ "hz": hz }))
        .await
        .map(|_| ())
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn set_adaptive_refresh_rate(enabled: bool) -> Result<(), ErrorResponse> {
    // Writes HKLM registry key — requires elevation.
    // The UI should inform the user that a driver restart / reboot is needed.
    elev_bridge::run_elevated(
        "set_adaptive_refresh_rate",
        serde_json::json!({ "enabled": enabled }),
    )
    .await
    .map(|_| ())
    .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn get_autostart() -> Result<bool, ErrorResponse> {
    tokio::task::spawn_blocking(hw_get_autostart)
        .await
        .map_err(|e| format!("blocking task panicked: {e}"))?
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn set_autostart(enabled: bool) -> Result<(), ErrorResponse> {
    tokio::task::spawn_blocking(move || hw_set_autostart(enabled))
        .await
        .map_err(|e| format!("blocking task panicked: {e}"))?
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn get_update_status() -> Result<UpdateStatus, ErrorResponse> {
    tokio::task::spawn_blocking(hw_get_update_status)
        .await
        .map_err(|e| format!("blocking task panicked: {e}"))?
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn trigger_driver_scan() -> Result<String, ErrorResponse> {
    tokio::task::spawn_blocking(hw_trigger_scan)
        .await
        .map_err(|e| format!("blocking task panicked: {e}"))?
        .map_err(ErrorResponse::from)
}

// ── Hardware Discovery (Phase 10) ────────────────────────────────────────────

#[tauri::command]
pub async fn get_hardware_profile() -> Option<HardwareProfile> {
    global_profile()
}

#[tauri::command]
pub async fn run_hardware_discovery() -> Result<HardwareProfile, ErrorResponse> {
    let raw = elev_bridge::run_elevated("run_hardware_discovery", serde_json::Value::Null).await?;
    serde_json::from_value(raw)
        .map_err(|e| ErrorResponse::from(anyhow::anyhow!("Unexpected profile result: {e}")))
}

/// Install a specific driver by name.  The bundled .inf must exist in resources.
/// Runs through the elevated scheduled task (no UAC prompt during install).
#[tauri::command]
pub async fn install_driver(driver_name: String) -> Result<String, ErrorResponse> {
    let raw = elev_bridge::run_elevated(
        "install_driver",
        serde_json::json!({ "driver_name": driver_name }),
    )
    .await?;
    Ok(raw.as_str().unwrap_or("installed").to_string())
}

/// Read raw ECRAM bytes for debugging.
/// Returns a hex dump string (one line per 16 bytes) of the EC's known data ranges.
/// Use this to identify which byte offset corresponds to charger wattage:
/// plug/unplug the charger and call this command to see which bytes change.
#[tauri::command]
pub async fn debug_ecram_dump() -> Result<String, ErrorResponse> {
    crate::hw::ecram::debug_ecram_hex().map_err(ErrorResponse::from)
}

// ── Batched hardware state (S4-002) ──────────────────────────────────────────

/// Consolidated snapshot of all polled hardware properties returned in a single
/// IPC call. Each field is `Option<T>` so partial failures don't block the whole batch.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HardwareState {
    pub battery: Option<BatteryInfo>,
    pub display: Option<DisplayInfo>,
    pub fan: Option<FanInfo>,
    pub touchpad: Option<TouchpadInfo>,
    pub system_info: Option<SystemInfo>,
    pub performance_mode: Option<PerformanceMode>,
    pub charging_threshold: Option<u8>,
    pub audio: Option<AudioVolumeResult>,
}

/// Poll all hardware state at once with parallel queries via rayon.
///
/// `rayon::join` runs closures in parallel using the global rayon thread pool.
/// Since WMI connections are thread-local (see `wmi_cache`), each closure
/// lazily creates its own WMI connection on the first query, making shared
/// rayon threads safe for concurrent WMI access.
///
/// Each subsystem query is wrapped in `ok()` so a transient WMI/pipe failure
/// on one sensor doesn't prevent the rest from returning.
#[tauri::command]
pub async fn get_hardware_state_batch() -> Result<HardwareState, ErrorResponse> {
    tokio::task::spawn_blocking(|| {
        // Wave 1: battery, display, fan, touchpad in parallel
        let ((battery, display), (fan, touchpad)) = rayon::join(
            || rayon::join(|| hw_get_battery().ok(), || hw_get_display().ok()),
            || rayon::join(|| hw_get_fan().ok(), || hw_get_touchpad().ok()),
        );

        // Wave 2: system_info, performance_mode, charging_threshold, audio in parallel
        let ((system_info, performance_mode), (charging_threshold, audio)) = rayon::join(
            || rayon::join(|| hw_get_sysinfo().ok(), || hw_get_perf().ok()),
            || rayon::join(|| hw_get_charge().ok(), || hw_get_audio().ok()),
        );

        Ok(HardwareState {
            battery,
            display,
            fan,
            touchpad,
            system_info,
            performance_mode,
            charging_threshold,
            audio,
        })
    })
    .await
    .map_err(|e| format!("blocking task panicked: {e}"))?
}
