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

/// Launch the Phone Link pairing flow via the `ms-phone-link:` deep link.
///
/// The NFC handshake builds URIs like
/// `ms-phone-link:pairing?pc=MiControl` which open the Phone Link app's
/// pairing wizard directly (the reliable path the phone's Link-to-Windows
/// app expects). Only `ms-phone-link:` and `ms-phone:` schemes are allowed;
/// anything else is rejected to avoid opening arbitrary protocols.
pub fn launch_phone_link_pairing(uri: &str) -> HardwareResult<()> {
    if uri.is_empty() {
        return Err(HardwareError::Other(
            "Empty Phone Link pairing URI".to_string(),
        ));
    }
    let lower = uri.to_ascii_lowercase();
    if !lower.starts_with("ms-phone-link:") && !lower.starts_with("ms-phone:") {
        return Err(HardwareError::Other(format!(
            "Refusing to open non-Phone-Link URI: {uri}"
        )));
    }

    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .args(["/c", "start", "", uri])
            .creation_flags(0x0800_0000)
            .spawn()
            .map_err(|e| {
                HardwareError::Other(format!("Failed to open Phone Link pairing link: {e}"))
            })?;
        Ok(())
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "Phone Link only available on Windows".into(),
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
            "Get-AppxPackage -Name 'Microsoft.YourPhone','Microsoft.PhoneExperience' | Select-Object -First 1 -ExpandProperty Version",
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

/// Resolve the Phone Link package local-app-data directory.
///
/// Phone Link stores pairing metadata in
/// `%LOCALAPPDATA%\Packages\Microsoft.YourPhone_*\LocalCache\DeviceMetadataStorage.json`
/// (the same file the app uses). On new Windows 11 builds the package may be
/// `Microsoft.PhoneExperience_*`.
#[cfg(windows)]
fn phone_link_package_dir() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from)?;
    let packages = base.join("Packages");
    let names = ["Microsoft.YourPhone", "Microsoft.PhoneExperience"];
    for name in names {
        if let Ok(entries) = std::fs::read_dir(&packages) {
            for entry in entries.flatten() {
                let dir = entry.path();
                if dir
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(|s| s.starts_with(name))
                    .unwrap_or(false)
                {
                    return Some(dir);
                }
            }
        }
    }
    None
}

/// Minimal serde shapes for `DeviceMetadataStorage.json` (only the fields we
/// need; unknown fields are ignored by serde).
#[cfg(windows)]
#[derive(serde::Deserialize)]
struct PhoneMetadataEntry {
    #[serde(rename = "IsLinked")]
    is_linked: bool,
    #[serde(rename = "Metadata")]
    metadata: PhoneDeviceMetadata,
}

#[cfg(windows)]
#[derive(serde::Deserialize)]
struct PhoneDeviceMetadata {
    /// The JSON nests the real device info under `Metadata` again:
    /// `Metadata.Metadata.{ClientType,DisplayName,...}`.
    #[serde(rename = "Metadata")]
    #[serde(default)]
    inner: Option<PhoneInnerMetadata>,
}

#[cfg(windows)]
#[derive(serde::Deserialize)]
struct PhoneInnerMetadata {
    #[serde(rename = "ClientType")]
    client_type: String,
    #[serde(rename = "DisplayName")]
    display_name: String,
}

/// Read the Phone Link pairing metadata JSON and return
/// `(is_paired, device_display_name)`.
///
/// Observed shape (Windows 11, Phone Link 1.26x):
/// ```json
/// { "ConfigVersion":1,
///   "DeviceMetadatas": {
///     "<id>": [ {
///       "Certificates": {...},
///       "IsLinked": true,
///       "Metadata": {
///         "Id":"...","LastSeenTime":"...",
///         "Metadata": { "ClientType":"LTW","DisplayName":"Xiaomi 14T",... }
///       }
///     } ]
///   }
/// }
/// ```
/// The phone is the entry with `ClientType == "LTW"` (Link to Windows); the
/// PC itself is `WEA`. We only report paired when the LTW entry is linked.
/// Pure parser for the `DeviceMetadatas` JSON payload. Testable without fs.
#[cfg(windows)]
fn parse_device_metadatas(json: &str) -> (bool, Option<String>) {
    #[derive(serde::Deserialize)]
    struct Storage {
        #[serde(rename = "DeviceMetadatas")]
        device_metadatas: std::collections::HashMap<String, Vec<PhoneMetadataEntry>>,
    }

    let storage = match serde_json::from_str::<Storage>(json) {
        Ok(storage) => storage,
        Err(_) => return (false, None),
    };

    let mut paired = false;
    let mut name: Option<String> = None;
    for (_id, entries) in storage.device_metadatas {
        for entry in entries {
            let Some(inner) = entry.metadata.inner.as_ref() else {
                continue;
            };
            if inner.client_type == "LTW" {
                // This is the phone entry.
                if entry.is_linked {
                    paired = true;
                    name = Some(inner.display_name.clone());
                }
                // The LTW entry is authoritative for the phone — stop here.
                return (paired, name);
            }
            if entry.is_linked && inner.client_type != "WEA" {
                // Some other linked device (not the PC): remember paired.
                paired = true;
            }
        }
    }
    (paired, name)
}

#[cfg(windows)]
fn read_paired_metadata() -> (bool, Option<String>) {
    let Some(dir) = phone_link_package_dir() else {
        return (false, None);
    };
    let path = dir.join("LocalCache").join("DeviceMetadataStorage.json");
    let raw = match std::fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(_) => return (false, None),
    };
    parse_device_metadatas(&raw)
}

#[cfg(windows)]
fn check_paired() -> bool {
    // Primary: the pairing metadata JSON.
    let (linked, _) = read_paired_metadata();
    if linked {
        return true;
    }

    // Fallback: legacy registry keys (works on some older builds).
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let keys = [
        r"Software\Microsoft\YourPhone",
        r"Software\Microsoft\Windows\CurrentVersion\PhoneLink",
    ];
    for key_path in &keys {
        if let Ok(key) = hkcu.open_subkey(key_path) {
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
    // Primary: from the metadata JSON.
    let (_, name) = read_paired_metadata();
    if let Some(n) = name {
        if !n.is_empty() {
            return Some(n);
        }
    }

    // Fallback: legacy registry keys.
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
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
    // The modern Phone Link UI runs as PhoneExperienceHost; older builds
    // use YourPhoneAppProxy. We check both, non-interactively.
    let output = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-Process -Name 'PhoneExperienceHost','YourPhoneAppProxy' -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty Id",
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

    /// Real observed shape (Phone Link 1.26x, Xiaomi 14T): double-nested
    /// `Metadata.Metadata`, `IsLinked` outside the inner object, plus a
    /// `WEA` PC entry. Only windows (uses winreg import); guard with `cfg`.
    #[cfg(windows)]
    #[test]
    fn parses_real_device_metadata_shape() {
        let json = r#"{
          "ConfigVersion": 1,
          "DeviceMetadatas": {
            "0006BFFDB6667958": [{
              "Certificates": {"SelfSigned":["skip"]},
              "IsLinked": true,
              "Metadata": {
                "Id": "71caaad7-c77e-4cd7-b363-43b5db234210",
                "LastSeenTime": "2026-08-26T16:05:30+00:00",
                "Metadata": {
                  "ClientType": "WEA",
                  "ClientVersion": "1.26071.84.0",
                  "DisplayName": "MF-PC",
                  "OsName": "Windows",
                  "OsVersion": "10.0.26200",
                  "Manufacture": "Unknown",
                  "ModelName": "Unknown"
                }
              }
            }],
            "1234567890ABCDEF": [{
              "Certificates": {"SelfSigned":["skip"]},
              "IsLinked": true,
              "Metadata": {
                "Id": "82cc6c6f-2e4a-4eeb-826d-ab1335b6cf28",
                "LastSeenTime": "2026-08-26T08:37:00+00:00",
                "Metadata": {
                  "ClientType": "LTW",
                  "ClientVersion": "1.26071.104.0",
                  "DisplayName": "Xiaomi 14T",
                  "OsName": "Android",
                  "OsVersion": "16",
                  "Manufacture": "Xiaomi",
                  "ModelName": "2406APNFAG"
                }
              }
            }]
          }
        }"#;
        let (paired, name) = parse_device_metadatas(json);
        assert!(paired);
        assert_eq!(name.as_deref(), Some("Xiaomi 14T"));
    }

    #[cfg(windows)]
    #[test]
    fn unlinked_phone_returns_not_paired() {
        let json = r#"{
          "DeviceMetadatas": {
            "X": [{ "IsLinked": false, "Metadata": { "Metadata": {
              "ClientType": "LTW", "DisplayName": "Xiaomi 14T"
            }}}]
          }
        }"#;
        let (paired, name) = parse_device_metadatas(json);
        assert!(!paired);
        assert_eq!(name, None);
    }

    #[cfg(windows)]
    #[test]
    fn malformed_json_returns_not_paired() {
        let (paired, name) = parse_device_metadatas("not json");
        assert!(!paired);
        assert_eq!(name, None);
    }
}
