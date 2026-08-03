//! Phone Link integration — detect, launch, and monitor Windows Phone Link.
//!
//! Phone Link (formerly "Your Phone") is built into Windows 10/11 and pairs
//! with the "Link to Windows" Android app. The Xiaomi 14T is officially
//! supported for all Phone Link features.
//!
//! MiControl orchestrates Phone Link via:
//! - URI scheme `ms-phone:` to launch the app
//! - Registry `HKCU\Software\Microsoft\YourPhone` to detect pairing
//! - PowerShell `Get-AppxPackage` to detect installation

use crate::hw::errors::{HardwareError, HardwareResult};
use serde::{Deserialize, Serialize};

// ── Data structures ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PhoneLinkStatus {
    /// Whether Phone Link is installed on this PC
    pub installed: bool,
    /// Whether a phone is paired
    pub paired: bool,
    /// Paired device name (if available)
    pub device_name: Option<String>,
    /// Phone Link package version (if available)
    pub package_version: Option<String>,
    /// Whether Phone Link is currently running
    pub running: bool,
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Detect if Phone Link is installed on this Windows machine.
pub fn detect_phone_link() -> bool {
    #[cfg(windows)]
    {
        // Check via PowerShell Get-AppxPackage
        let output = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-AppxPackage *YourPhone* | Select-Object -ExpandProperty Name",
            ])
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .output();

        match output {
            Ok(out) => {
                let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
                !name.is_empty()
            }
            Err(_) => {
                // Fallback: check if the SystemApps directory exists
                let path = r"C:\Windows\SystemApps\Microsoft.YourPhone";
                std::path::Path::new(path).exists()
            }
        }
    }
    #[cfg(not(windows))]
    false
}

/// Get the full Phone Link status (installed, paired, running).
pub fn get_phone_link_status() -> PhoneLinkStatus {
    let installed = detect_phone_link();
    if !installed {
        return PhoneLinkStatus::default();
    }

    let package_version = get_package_version();
    let paired = check_paired();
    let device_name = read_paired_device_name();
    let running = check_running();

    PhoneLinkStatus {
        installed,
        paired,
        device_name,
        package_version,
        running,
    }
}

/// Launch Phone Link app via URI scheme.
pub fn launch_phone_link() -> HardwareResult<()> {
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .args(["/c", "start", "", "ms-phone:"])
            .creation_flags(0x0800_0000)
            .spawn()
            .map_err(|e| HardwareError::Other(format!("Failed to launch Phone Link: {e}")))?;
        Ok(())
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "Phone Link only available on Windows".into(),
    ))
}

/// Launch Phone Link with a specific feature deep link.
///
/// Known deep links (may vary by Windows version):
/// - `Phone` — calls
/// - `Messages` — SMS
/// - `Photos` — photos
/// - `ScreenMirror` — screen mirroring
/// - `Apps` — app streaming
pub fn launch_phone_link_feature(feature: &str) -> HardwareResult<()> {
    // Validate feature against allow-list to prevent URI injection
    const ALLOWED_FEATURES: &[&str] = &["Phone", "Messages", "Photos", "ScreenMirror", "Apps"];
    if !ALLOWED_FEATURES.contains(&feature) {
        return Err(HardwareError::Other(format!(
            "Unknown Phone Link feature: '{feature}'. Allowed: {ALLOWED_FEATURES:?}"
        )));
    }

    #[cfg(windows)]
    {
        let uri = format!("ms-phone:{}", feature);
        std::process::Command::new("cmd")
            .args(["/c", "start", "", &uri])
            .creation_flags(0x0800_0000)
            .spawn()
            .map_err(|e| {
                HardwareError::Other(format!("Failed to launch Phone Link feature: {e}"))
            })?;
        Ok(())
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "Phone Link only available on Windows".into(),
    ))
}

/// Open Phone Link settings page in Windows Settings.
pub fn open_phone_link_settings() -> HardwareResult<()> {
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .args(["/c", "start", "", "ms-settings:mobile-devices"])
            .creation_flags(0x0800_0000)
            .spawn()
            .map_err(|e| {
                HardwareError::Other(format!("Failed to open Phone Link settings: {e}"))
            })?;
        Ok(())
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "Phone Link settings only available on Windows".into(),
    ))
}

// ── Internal helpers ─────────────────────────────────────────────────────────

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
fn get_package_version() -> Option<String> {
    let output = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-AppxPackage *YourPhone* | Select-Object -ExpandProperty Version",
        ])
        .creation_flags(0x0800_0000)
        .output()
        .ok()?;

    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if version.is_empty() {
        None
    } else {
        Some(version)
    }
}

#[cfg(windows)]
fn check_paired() -> bool {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    // Check multiple possible registry locations
    let keys = [
        r"Software\Microsoft\YourPhone",
        r"Software\Microsoft\Windows\CurrentVersion\PhoneLink",
    ];

    for key_path in &keys {
        if let Ok(key) = hkcu.open_subkey(key_path) {
            // If the key exists and has any values, consider it paired
            let values: Vec<_> = key.enum_values().collect();
            if !values.is_empty() {
                return true;
            }
        }
    }

    false
}

#[cfg(windows)]
fn read_paired_device_name() -> Option<String> {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    // Try different possible value names
    let value_names = ["DeviceName", "PairedDeviceName", "PhoneName"];
    let key_paths = [
        r"Software\Microsoft\YourPhone",
        r"Software\Microsoft\Windows\CurrentVersion\PhoneLink",
    ];

    for key_path in &key_paths {
        if let Ok(key) = hkcu.open_subkey(key_path) {
            for name in &value_names {
                if let Ok(val) = key.get_value::<String, _>(name) {
                    if !val.is_empty() {
                        return Some(val);
                    }
                }
            }
        }
    }

    None
}

#[cfg(windows)]
fn check_running() -> bool {
    let output = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-Process -Name 'YourPhone' -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty Id",
        ])
        .creation_flags(0x0800_0000)
        .output();

    match output {
        Ok(out) => {
            let pid = String::from_utf8_lossy(&out.stdout).trim().to_string();
            !pid.is_empty()
        }
        Err(_) => false,
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_phone_link_does_not_panic() {
        let _ = detect_phone_link();
    }

    #[test]
    fn test_get_status_does_not_panic() {
        let _ = get_phone_link_status();
    }
}
