use crate::hw::battery::{get_battery_info as hw_get_battery, BatteryInfo};
use crate::hw::discovery::{HardwareProfile, global_profile, resources_dir};
use crate::hw::display::{
    get_display_info as hw_get_display,
    get_ai_brightness_config as hw_get_ai_cfg,
    DisplayInfo, AiBrightnessConfig,
};
use crate::hw::fan::{get_fan_info as hw_get_fan, FanInfo, FanMode};
use crate::hw::touchpad::{
    get_touchpad_info as hw_get_touchpad, TouchpadInfo, TouchpadSensitivity,
};
use crate::hw::system_info::{get_system_info as hw_get_sysinfo, SystemInfo};
use crate::hw::startup::{get_autostart as hw_get_autostart, set_autostart as hw_set_autostart};
use crate::hw::update::{get_update_status as hw_get_update_status, trigger_driver_scan as hw_trigger_scan, UpdateStatus};
use crate::elev_bridge;

#[tauri::command]
pub async fn get_battery_info() -> Result<BatteryInfo, String> {
    hw_get_battery().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_display_info() -> Result<DisplayInfo, String> {
    hw_get_display().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_brightness(level: u8) -> Result<(), String> {
    elev_bridge::run_elevated("set_brightness", serde_json::json!({ "level": level }))
        .await
        .map(|_| ())
}

#[tauri::command]
pub async fn set_hdr(enabled: bool) -> Result<(), String> {
    elev_bridge::run_elevated("set_hdr", serde_json::json!({ "enabled": enabled }))
        .await
        .map(|_| ())
}

#[tauri::command]
pub async fn set_ai_brightness(enabled: bool) -> Result<(), String> {
    elev_bridge::run_elevated("set_ai_brightness", serde_json::json!({ "enabled": enabled }))
        .await
        .map(|_| ())
}

#[tauri::command]
pub async fn get_ai_brightness_config() -> Result<AiBrightnessConfig, String> {
    Ok(hw_get_ai_cfg())
}

#[tauri::command]
pub async fn set_ai_brightness_config(config: AiBrightnessConfig) -> Result<(), String> {
    elev_bridge::run_elevated("set_ai_brightness_config", serde_json::json!({ "config": config }))
        .await
        .map(|_| ())
}

#[tauri::command]
pub async fn get_fan_info() -> Result<FanInfo, String> {
    hw_get_fan().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_fan_mode(mode: FanMode, speed_percent: u8) -> Result<(), String> {
    elev_bridge::run_elevated(
        "set_fan_mode",
        serde_json::json!({ "mode": mode, "speed_percent": speed_percent }),
    )
    .await
    .map(|_| ())
}

#[tauri::command]
pub async fn get_touchpad_info() -> Result<TouchpadInfo, String> {
    hw_get_touchpad().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_touchpad_sensitivity(sensitivity: TouchpadSensitivity) -> Result<(), String> {
    elev_bridge::run_elevated(
        "set_touchpad_sensitivity",
        serde_json::json!({ "sensitivity": sensitivity }),
    )
    .await
    .map(|_| ())
}

#[tauri::command]
pub async fn set_touchpad_haptics(enabled: bool) -> Result<(), String> {
    elev_bridge::run_elevated("set_touchpad_haptics", serde_json::json!({ "enabled": enabled }))
        .await
        .map(|_| ())
}

#[tauri::command]
pub async fn get_system_info() -> Result<SystemInfo, String> {
    hw_get_sysinfo().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_autostart() -> Result<bool, String> {
    hw_get_autostart().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_autostart(enabled: bool) -> Result<(), String> {
    hw_set_autostart(enabled).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_update_status() -> Result<UpdateStatus, String> {
    hw_get_update_status().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn trigger_driver_scan() -> Result<String, String> {
    hw_trigger_scan().map_err(|e| e.to_string())
}

// ── Hardware Discovery (Phase 10) ────────────────────────────────────────────

#[tauri::command]
pub async fn get_hardware_profile() -> Option<HardwareProfile> {
    global_profile().cloned()
}

#[tauri::command]
pub async fn run_hardware_discovery() -> Result<HardwareProfile, String> {
    let raw = elev_bridge::run_elevated("run_hardware_discovery", serde_json::Value::Null).await?;
    serde_json::from_value(raw).map_err(|e| format!("Unexpected profile result: {e}"))
}

/// Install a specific driver by name.  The bundled .inf must exist in resources.
/// Runs through the elevated scheduled task (no UAC prompt during install).
#[tauri::command]
pub async fn install_driver(driver_name: String) -> Result<String, String> {
    let inf_path = resolve_driver_inf(&driver_name)?;
    let raw = elev_bridge::run_elevated(
        "install_driver",
        serde_json::json!({ "inf_path": inf_path }),
    )
    .await?;
    Ok(raw.as_str().unwrap_or("installed").to_string())
}

fn resolve_driver_inf(driver_name: &str) -> Result<String, String> {
    let candidates = [
        format!("drivers/{}/{}.inf", driver_name, driver_name.to_lowercase()),
        format!("drivers/{}/driver.inf", driver_name),
    ];
    for rel in &candidates {
        let inf = resources_dir().join(rel);
        if inf.exists() {
            return Ok(inf.to_string_lossy().to_string());
        }
    }
    if let Some(profile) = global_profile() {
        for missing in &profile.missing_drivers {
            if missing.name.eq_ignore_ascii_case(driver_name) {
                if let Some(inf_path) = &missing.bundled_inf {
                    return Ok(inf_path.clone());
                }
            }
        }
    }
    Err(format!("Bundled .inf for driver '{}' not found in resources.", driver_name))
}
