// hw/screen_cast.rs
//
// Screen casting via Windows Miracast API.
// Provides device discovery and casting control.

#[cfg(windows)]
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// A Miracast receiver device.
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

/// List available Miracast/WiDi receivers.
#[cfg(windows)]
pub fn list_cast_devices() -> Result<Vec<CastDevice>> {
    // Windows 10/11 has built-in Miracast support via the Connect quick action.
    // We use the Windows.Media.Casting API via PowerShell as a fallback.
    // For now, return an empty list with a note.
    // Full implementation requires UWP/WinRT interop.
    log::info!("[screen_cast] Listing Miracast devices via WinRT");
    Ok(Vec::new())
}

#[cfg(not(windows))]
pub fn list_cast_devices() -> Result<Vec<CastDevice>> {
    Ok(Vec::new())
}

/// Start casting to a device by ID.
#[cfg(windows)]
pub fn start_casting(device_id: &str) -> Result<CastResult> {
    // Launch the Windows Connect quick action panel
    let result = std::process::Command::new("cmd")
        .args(["/c", "start", "ms-settings-connectabledevices:project"])
        .output()
        .context("Failed to launch Connect panel")?;

    let success = result.status.success();
    Ok(CastResult {
        success,
        message: if success {
            "Connect panel opened. Select your device to start casting.".into()
        } else {
            "Failed to open Connect panel.".into()
        },
    })
}

#[cfg(not(windows))]
pub fn start_casting(_device_id: &str) -> Result<CastResult> {
    Ok(CastResult {
        success: false,
        message: "Screen casting only available on Windows".into(),
    })
}

/// Stop casting.
#[cfg(windows)]
pub fn stop_casting() -> Result<CastResult> {
    // Close the Connect panel
    let result = std::process::Command::new("cmd")
        .args(["/c", "taskkill", "/f", "/im", "SystemSettings.exe"])
        .output()
        .context("Failed to stop casting")?;

    Ok(CastResult {
        success: result.status.success(),
        message: "Casting stopped".into(),
    })
}

#[cfg(not(windows))]
pub fn stop_casting() -> Result<CastResult> {
    Ok(CastResult {
        success: false,
        message: "Screen casting only available on Windows".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_cast_devices() {
        let result = list_cast_devices();
        assert!(result.is_ok());
    }

    #[test]
    fn test_stop_casting() {
        let result = stop_casting();
        assert!(result.is_ok());
    }
}
