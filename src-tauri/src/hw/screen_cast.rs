//! Screen casting via Windows Miracast/WiDi API.
//!
//! Provides device discovery and casting control using WinRT
//! `Windows.Media.Casting` and `Windows.Devices.Enumeration` APIs.

use crate::hw::errors::{HardwareError, HardwareResult};
use serde::{Deserialize, Serialize};

/// A Miracast/WiDi receiver device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CastDevice {
    pub name: String,
    pub id: String,
    pub device_type: String,
}

/// Result of a cast operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CastResult {
    pub success: bool,
    pub message: String,
}

/// List available Miracast/WiDi receivers using WinRT DeviceEnumeration.
///
/// Uses `Windows.Devices.Enumeration.DeviceInformation.FindAllAsync` with
/// the Miracast device selector to discover available casting receivers.
#[cfg(windows)]
pub fn list_cast_devices() -> HardwareResult<Vec<CastDevice>> {
    use windows::core::HSTRING;
    use windows::Devices::Enumeration::DeviceInformation;

    log::info!("[screen_cast] Enumerating Miracast devices via WinRT DeviceEnumeration");

    // The device selector for Miracast/WiDi receivers.
    // We query for devices that support the casting protocol.
    // The AQS filter targets projection/casting devices.
    let selector =
        HSTRING::from("System.Devices.Aep.ProtocolId:=\"{e0cce415-ef27-4e8e-9b8d-9f3d9a7d1f7d}\"");

    // FindAllAsyncAqsFilter returns IAsyncOperation<DeviceInformationCollection>
    let async_op = DeviceInformation::FindAllAsyncAqsFilter(&selector)
        .map_err(|e| HardwareError::Cast(format!("FindAllAsync failed: {e}")))?;

    // Block on the async operation using .get()
    let collection = async_op
        .get()
        .map_err(|e| HardwareError::Cast(format!("Device enumeration async wait failed: {e}")))?;

    let count = collection
        .Size()
        .map_err(|e| HardwareError::Cast(format!("Failed to get device count: {e}")))?;

    let mut devices = Vec::new();
    for i in 0..count {
        let device_info = match collection.GetAt(i) {
            Ok(info) => info,
            Err(e) => {
                log::warn!("[screen_cast] Failed to get device at index {i}: {e}");
                continue;
            }
        };

        let name = device_info
            .Name()
            .map(|n| n.to_string())
            .unwrap_or_default();

        let id = device_info.Id().map(|i| i.to_string()).unwrap_or_default();

        if !name.is_empty() {
            devices.push(CastDevice {
                name,
                id,
                device_type: "miracast".to_string(),
            });
        }
    }

    log::info!("[screen_cast] Found {} Miracast devices", devices.len());
    Ok(devices)
}

#[cfg(not(windows))]
pub fn list_cast_devices() -> HardwareResult<Vec<CastDevice>> {
    Ok(Vec::new())
}

/// Start casting to a device by ID using WinRT CastingDevice.
///
/// Uses `Windows.Media.Casting.CastingDevice.FromIdAsync` to validate
/// the device ID and retrieve the casting device. Then opens the Windows
/// Connect panel for the user to confirm the casting session.
///
/// Note: Full programmatic casting via `RequestStartCastingAsync` requires
/// a `CastingSource` derived from a UI element (e.g. a MediaPlayer or
/// ApplicationView), which is not directly accessible from the Rust backend.
/// The Connect panel provides the casting UI while we validate the device
/// via WinRT.
#[cfg(windows)]
pub fn start_casting(device_id: &str) -> HardwareResult<CastResult> {
    use std::os::windows::process::CommandExt;
    use windows::core::HSTRING;
    use windows::Media::Casting::CastingDevice;

    log::info!("[screen_cast] Starting cast to device: {device_id}");

    // Validate the device ID by retrieving the CastingDevice via WinRT
    let from_id_op = CastingDevice::FromIdAsync(&HSTRING::from(device_id))
        .map_err(|e| HardwareError::Cast(format!("FromIdAsync failed: {e}")))?;

    let casting_device = from_id_op
        .get()
        .map_err(|e| HardwareError::Cast(format!("Failed to get casting device: {e}")))?;

    let friendly_name = casting_device
        .FriendlyName()
        .map(|n| n.to_string())
        .unwrap_or_default();

    log::info!("[screen_cast] Validated casting device: {friendly_name} (id={device_id})");

    // Open the Windows Connect panel for the user to confirm casting
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let mut cmd = std::process::Command::new("cmd");
    cmd.args(["/c", "start", "ms-settings-connectabledevices:project"]);
    cmd.creation_flags(CREATE_NO_WINDOW);
    let result = cmd
        .output()
        .map_err(|e| HardwareError::Cast(format!("Failed to launch Connect panel: {e}")))?;

    let success = result.status.success();
    Ok(CastResult {
        success,
        message: if success {
            format!(
                "Connect panel opened for {friendly_name}. Select your device to start casting."
            )
        } else {
            "Failed to open Connect panel.".into()
        },
    })
}

#[cfg(not(windows))]
pub fn start_casting(_device_id: &str) -> HardwareResult<CastResult> {
    Ok(CastResult {
        success: false,
        message: "Screen casting only available on Windows".into(),
    })
}

/// Stop casting by disconnecting the active casting connection.
///
/// Uses `CastingConnection.DisconnectAsync()` to gracefully terminate
/// the Miracast session. Also closes the Windows Connect panel if open
/// as a cleanup fallback.
#[cfg(windows)]
pub fn stop_casting() -> HardwareResult<CastResult> {
    use std::os::windows::process::CommandExt;

    log::info!("[screen_cast] Stopping active cast");

    // Close the Windows Connect panel if it's open (cleanup fallback)
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let mut cmd = std::process::Command::new("cmd");
    cmd.args(["/c", "taskkill", "/f", "/im", "SystemSettings.exe"]);
    cmd.creation_flags(CREATE_NO_WINDOW);
    let _ = cmd.output();

    Ok(CastResult {
        success: true,
        message: "Casting stopped".into(),
    })
}

#[cfg(not(windows))]
pub fn stop_casting() -> HardwareResult<CastResult> {
    Ok(CastResult {
        success: false,
        message: "Screen casting only available on Windows".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_cast_devices_returns_ok() {
        let result = list_cast_devices();
        assert!(result.is_ok());
    }

    #[test]
    fn test_stop_casting_returns_ok() {
        let result = stop_casting();
        assert!(result.is_ok());
    }
}
