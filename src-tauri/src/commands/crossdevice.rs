//! Tauri commands for cross-device / IoT features (MIOT-04+).
//!
//! Exposes BLE phone presence status/config to the frontend. Future MIOT
//! features (LocalSend, scrcpy, transcription, KDE Connect, Syncthing) attach
//! their commands here so the cross-device tab has one command surface.

use crate::hw::ble_presence::{PresenceConfig, PresenceStatus};
use crate::hw::kde_connect::{KdeDevice, KdeMessage};
use crate::hw::localsend::{LocalSendPeer, ReceiverStatus, SendReport};
use crate::hw::scrcpy_bridge::ScrcpyStatus;
use crate::hw::transcription::TranscriptionStatus;
use std::time::Duration;

/// Current BLE presence state (phone near/far/unknown + telemetry).
#[tauri::command]
pub async fn get_presence_status() -> Result<PresenceStatus, String> {
    Ok(crate::hw::ble_presence::get_presence_status())
}

/// Load the persisted presence configuration for the UI to pre-fill.
#[tauri::command]
pub async fn get_presence_config() -> Result<PresenceConfig, String> {
    Ok(PresenceConfig::load())
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

/// Discover LocalSend peers on the LAN for `seconds` (default 3).
#[tauri::command]
pub async fn localsend_discover(seconds: Option<u64>) -> Result<Vec<LocalSendPeer>, String> {
    let secs = seconds.unwrap_or(3).min(15);
    Ok(crate::hw::localsend::discover_peers(Duration::from_secs(secs)).await)
}

/// Send files to a discovered LocalSend peer.
#[tauri::command]
pub async fn localsend_send_files(
    peer: LocalSendPeer,
    paths: Vec<String>,
) -> Result<SendReport, String> {
    crate::hw::localsend::send_files(&peer, &paths).await
}

/// Start the embedded LocalSend receiver (auto-accept → Downloads).
#[tauri::command]
pub async fn localsend_receiver_start() -> Result<ReceiverStatus, String> {
    let port = crate::hw::localsend::start_receiver()?;
    let mut status = crate::hw::localsend::receiver_status();
    status.port = port;
    Ok(status)
}

/// Stop the embedded LocalSend receiver.
#[tauri::command]
pub async fn localsend_receiver_stop() -> Result<ReceiverStatus, String> {
    crate::hw::localsend::stop_receiver();
    Ok(crate::hw::localsend::receiver_status())
}

/// Current LocalSend receiver status.
#[tauri::command]
pub async fn localsend_receiver_status() -> Result<ReceiverStatus, String> {
    Ok(crate::hw::localsend::receiver_status())
}

/// scrcpy orchestration (MIOT-07): turn the phone camera into a webcam.
/// Returns a friendly `not-installed` state when the binary is missing.
#[tauri::command]
pub async fn scrcpy_status() -> Result<ScrcpyStatus, String> {
    Ok(crate::hw::scrcpy_bridge::status())
}

/// Start `scrcpy --camera-facing=front` (detached child, PID-tracked).
#[tauri::command]
pub async fn scrcpy_start() -> Result<u32, String> {
    crate::hw::scrcpy_bridge::start_camera()
}

/// Stop the running scrcpy child process.
#[tauri::command]
pub async fn scrcpy_stop() -> Result<(), String> {
    crate::hw::scrcpy_bridge::stop_camera()
}

/// Local transcription status: binary installed? model downloaded?
#[tauri::command]
pub async fn transcription_status() -> Result<TranscriptionStatus, String> {
    Ok(crate::hw::transcription::status())
}

/// Download the int8 paraformer model into app-data (best-effort, streams
/// from GitHub releases). Returns the model dir when ready.
#[tauri::command]
pub async fn transcription_download_model() -> Result<String, String> {
    let dir = crate::hw::transcription::download_model()?;
    Ok(dir.display().to_string())
}

/// Transcribe a local `.wav` file with sherpa-onnx (on-device, no cloud).
#[tauri::command]
pub async fn transcribe_audio(wav_path: String) -> Result<String, String> {
    crate::hw::transcription::transcribe(&wav_path)
}

/// Discover KDE Connect devices on the LAN (UDP 1716 broadcast identity).
/// Returns the devices found during the sweep.
#[tauri::command]
pub async fn kde_discover(seconds: Option<u32>) -> Result<Vec<KdeDevice>, String> {
    let secs = seconds.unwrap_or(3).clamp(1, 15);
    let devs = crate::hw::kde_connect::discover_devices(Duration::from_secs(secs as u64)).await;
    Ok(devs)
}

/// Send a `kdeconnect.ping` to a discovered device (non-TLS MVP).
#[tauri::command]
pub async fn kde_ping(device: KdeDevice) -> Result<bool, String> {
    crate::hw::kde_connect::send_ping(&device, Duration::from_secs(5)).await
}

/// Refresh the shared KDE Connect cache and return the current device list.
#[tauri::command]
pub async fn kde_status() -> Result<Vec<KdeDevice>, String> {
    let devs = crate::hw::kde_connect::refresh_cache(Duration::from_millis(800)).await;
    Ok(devs)
}

/// Build the local KDE Connect identity message (used by tests/debug UI).
#[tauri::command]
pub fn kde_identity() -> Result<KdeMessage, String> {
    Ok(crate::hw::kde_connect::build_identity())
}
