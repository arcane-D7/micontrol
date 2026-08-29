//! Tauri commands for cross-device / IoT features (MIOT-04+).
//!
//! Exposes BLE phone presence status/config to the frontend. Future MIOT
//! features (LocalSend, scrcpy, transcription, KDE Connect, Syncthing) attach
//! their commands here so the cross-device tab has one command surface.

use crate::hw::ble_presence::{PresenceConfig, PresenceStatus};

/// Current BLE presence state (phone near/far/unknown + telemetry).
#[tauri::command]
pub async fn get_presence_status() -> Result<PresenceStatus, String> {
    Ok(crate::hw::ble_presence::get_presence_status())
}

/// Persist presence config (enable toggle, phone MAC/name, RSSI threshold,
/// auto-lock). If enabled and the monitor is not running yet, starts it.
#[tauri::command]
pub async fn set_presence_config(config: PresenceConfig) -> Result<(), String> {
    config.save();
    if config.enabled {
        crate::hw::ble_presence::start_presence_monitor();
    }
    Ok(())
}

/// Trigger a one-off presence scan; returns the updated status.
#[tauri::command]
pub async fn scan_presence_now() -> Result<PresenceStatus, String> {
    crate::hw::ble_presence::scan_now();
    // Give the one-off scanner a moment to land a result (best-effort).
    tokio::time::sleep(std::time::Duration::from_millis(6000)).await;
    Ok(crate::hw::ble_presence::get_presence_status())
}
