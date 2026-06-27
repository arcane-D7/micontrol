//! PC WiFi management via Windows netsh wlan.
//!
//! Provides network scanning, connection status, and connect/disconnect
//! functionality by parsing `netsh wlan` command output.

use crate::hw::errors::{HardwareError, HardwareResult};
use crate::util::xml;
use serde::{Deserialize, Serialize};
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::process::Command;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// A WiFi network (SSID) visible to the PC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WifiNetwork {
    pub ssid: String,
    pub signal: u32,      // 0-100 percentage
    pub security: String, // e.g. "WPA2-Personal", "Open"
    pub connected: bool,
}

/// Current WiFi connection status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WifiStatus {
    pub connected: bool,
    pub ssid: Option<String>,
    pub signal: Option<u32>,
    pub interface: Option<String>,
}

/// Scan for available WiFi networks using netsh wlan.
pub fn scan_networks() -> HardwareResult<Vec<WifiNetwork>> {
    let mut cmd = Command::new("netsh");
    cmd.args(["wlan", "show", "networks", "mode=bssid"]);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let output = cmd
        .output()
        .map_err(|e| HardwareError::Wifi(format!("Failed to run netsh: {e}")))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_scan_output(&stdout)
}

/// Get current WiFi connection status.
pub fn get_status() -> HardwareResult<WifiStatus> {
    let mut cmd = Command::new("netsh");
    cmd.args(["wlan", "show", "interfaces"]);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let output = cmd
        .output()
        .map_err(|e| HardwareError::Wifi(format!("Failed to run netsh: {e}")))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_interface_output(&stdout)
}

/// Connect to a WiFi network.
///
/// The SSID and password are validated and XML-escaped before being
/// interpolated into the WLAN profile template to prevent XML injection.
pub fn connect(ssid: &str, password: Option<&str>) -> HardwareResult<()> {
    // Validate the SSID before any XML construction.
    xml::validate_ssid(ssid).map_err(|e| HardwareError::Wifi(format!("Invalid SSID: {e}")))?;

    // Validate the password if provided.
    if let Some(pwd) = password {
        xml::validate_wpa2_passphrase(pwd)
            .map_err(|e| HardwareError::Wifi(format!("Invalid WPA2 passphrase: {e}")))?;
    }

    // Create profile XML and connect
    if let Some(pwd) = password {
        // Escape SSID and password to prevent XML injection.
        let escaped_ssid = xml::escape_xml(ssid);
        let escaped_pwd = xml::escape_xml(pwd);

        let profile_xml = format!(
            r#"<?xml version="1.0"?>
<WLANProfile xmlns="http://www.microsoft.com/networking/WLAN/profile/v1">
    <name>{escaped_ssid}</name>
    <SSIDConfig>
        <SSID>
            <name>{escaped_ssid}</name>
        </SSID>
    </SSIDConfig>
    <connectionType>ESS</connectionType>
    <connectionMode>auto</connectionMode>
    <MSM>
        <security>
            <authEncryption>
                <authentication>WPA2PSK</authentication>
                <encryption>AES</encryption>
                <useOneX>false</useOneX>
            </authEncryption>
            <sharedKey>
                <keyType>passPhrase</keyType>
                <protected>false</protected>
                <keyMaterial>{escaped_pwd}</keyMaterial>
            </sharedKey>
        </security>
    </MSM>
</WLANProfile>"#
        );

        // Write profile to a temp file with a random suffix to prevent
        // path collision with attacker-controlled names.
        let temp_dir = std::env::temp_dir();
        let random_suffix: String = (0..8)
            .map(|_| {
                let n = rand::random::<u8>() % 36;
                if n < 10 {
                    (b'0' + n) as char
                } else {
                    (b'a' + n - 10) as char
                }
            })
            .collect();
        let profile_path = temp_dir.join(format!("micontrol_wifi_{random_suffix}.xml"));

        // Use a cleanup guard to ensure the temp file is deleted even on error.
        let result = (|| -> HardwareResult<()> {
            std::fs::write(&profile_path, &profile_xml).map_err(HardwareError::Io)?;

            // Add profile
            let mut cmd = Command::new("netsh");
            cmd.args(["wlan", "add", "profile", "filename"])
                .arg(&profile_path);
            #[cfg(windows)]
            cmd.creation_flags(CREATE_NO_WINDOW);
            let add = cmd
                .output()
                .map_err(|e| HardwareError::Wifi(format!("Failed to add profile: {e}")))?;

            if !add.status.success() {
                let stderr = String::from_utf8_lossy(&add.stderr);
                return Err(HardwareError::Wifi(format!(
                    "Failed to add WiFi profile: {stderr}"
                )));
            }

            Ok(())
        })();

        // Always clean up the temp file, even on error.
        let _ = std::fs::remove_file(&profile_path);

        result?;
    }

    // Connect
    let mut cmd = Command::new("netsh");
    cmd.args(["wlan", "connect", "name"]).arg(ssid);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let connect = cmd
        .output()
        .map_err(|e| HardwareError::Wifi(format!("Failed to connect: {e}")))?;

    if !connect.status.success() {
        let stderr = String::from_utf8_lossy(&connect.stderr);
        return Err(HardwareError::Wifi(format!("Failed to connect: {stderr}")));
    }

    Ok(())
}

/// Disconnect from current WiFi network.
pub fn disconnect() -> HardwareResult<()> {
    let mut cmd = Command::new("netsh");
    cmd.args(["wlan", "disconnect"]);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let output = cmd
        .output()
        .map_err(|e| HardwareError::Wifi(format!("Failed to disconnect: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(HardwareError::Wifi(format!(
            "Failed to disconnect: {stderr}"
        )));
    }

    Ok(())
}

/// Parse netsh wlan show networks output.
fn parse_scan_output(output: &str) -> HardwareResult<Vec<WifiNetwork>> {
    let mut networks: Vec<WifiNetwork> = Vec::new();
    let mut current_ssid: Option<String> = None;
    let mut current_signal: u32 = 0;
    let mut current_security: String = String::new();

    for line in output.lines() {
        let line = line.trim();
        if line.starts_with("SSID ") && !line.contains("BSSID") {
            // Save previous network
            if let Some(ssid) = current_ssid.take() {
                networks.push(WifiNetwork {
                    ssid,
                    signal: current_signal,
                    security: current_security.clone(),
                    connected: false,
                });
            }
            // Start new network
            let ssid = line.trim_start_matches("SSID").trim();
            let ssid = ssid.trim_matches(':').trim();
            if !ssid.is_empty() {
                current_ssid = Some(ssid.to_string());
                current_signal = 0;
                current_security = String::new();
            }
        } else if line.starts_with("Signal") {
            if let Some(pct) = line.split(':').nth(1) {
                current_signal = pct.trim().trim_matches('%').parse().unwrap_or(0);
            }
        } else if line.starts_with("Authentication") {
            if let Some(auth) = line.split(':').nth(1) {
                current_security = auth.trim().to_string();
            }
        }
    }

    // Don't forget the last network
    if let Some(ssid) = current_ssid {
        networks.push(WifiNetwork {
            ssid,
            signal: current_signal,
            security: current_security,
            connected: false,
        });
    }

    Ok(networks)
}

/// Parse netsh wlan show interfaces output.
fn parse_interface_output(output: &str) -> HardwareResult<WifiStatus> {
    let mut ssid: Option<String> = None;
    let mut signal: Option<u32> = None;
    let mut interface: Option<String> = None;
    let mut state: Option<String> = None;

    for line in output.lines() {
        let line = line.trim();
        if line.starts_with("Name") && line.contains(':') {
            interface = line.split(':').nth(1).map(|s| s.trim().to_string());
        } else if line.starts_with("SSID") && line.contains(':') {
            ssid = line.split(':').nth(1).map(|s| s.trim().to_string());
        } else if line.starts_with("State") && line.contains(':') {
            state = line.split(':').nth(1).map(|s| s.trim().to_string());
        } else if line.starts_with("Signal") && line.contains(':') {
            if let Some(sig_str) = line.split(':').nth(1) {
                signal = sig_str.trim().trim_matches('%').parse().ok();
            }
        }
    }

    let connected = state.as_deref() == Some("connected");

    Ok(WifiStatus {
        connected,
        ssid: if connected { ssid } else { None },
        signal,
        interface,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::xml;

    #[test]
    fn test_get_status_does_not_panic() {
        // This test just verifies the command runs without panicking
        let _ = get_status();
    }

    #[test]
    fn test_scan_networks_does_not_panic() {
        let _ = scan_networks();
    }

    #[test]
    fn test_parse_empty_scan() {
        let result = parse_scan_output("");
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn test_parse_empty_interface() {
        let result = parse_interface_output("");
        assert!(result.is_ok());
    }

    #[test]
    fn test_ssid_injection_prevented() {
        // An SSID that tries to break out of the <name> element
        let malicious = "test</name><name>evil";
        let escaped = xml::escape_xml(malicious);
        // The escaped version should not contain raw XML tags
        assert!(!escaped.contains("</name>"));
        assert!(!escaped.contains("<name>"));
    }

    #[test]
    fn test_password_injection_prevented() {
        // A password that tries to break out of <keyMaterial>
        let malicious = "</keyMaterial><x>";
        let escaped = xml::escape_xml(malicious);
        assert!(!escaped.contains("</keyMaterial>"));
        assert!(escaped.contains("&lt;/keyMaterial&gt;"));
    }

    #[test]
    fn test_oversized_ssid_rejected() {
        let long_ssid = "a".repeat(33);
        assert!(xml::validate_ssid(&long_ssid).is_err());
    }

    #[test]
    fn test_short_password_rejected() {
        assert!(xml::validate_wpa2_passphrase("short").is_err());
    }

    #[test]
    fn test_valid_ssid_accepted() {
        assert!(xml::validate_ssid("MyHomeNetwork").is_ok());
    }

    #[test]
    fn test_valid_password_accepted() {
        assert!(xml::validate_wpa2_passphrase("correct horse battery staple").is_ok());
    }

    #[test]
    fn test_profile_xml_well_formed_after_escape() {
        // Build the profile XML the same way connect() does, with a malicious SSID
        let ssid = "test</name><name>evil";
        let pwd = "password</keyMaterial><x>";
        // validate_ssid should pass (it's 23 bytes, under 32)
        assert!(xml::validate_ssid(ssid).is_ok());
        // validate_wpa2_passphrase should pass (it's > 8 chars)
        assert!(xml::validate_wpa2_passphrase(pwd).is_ok());

        let escaped_ssid = xml::escape_xml(ssid);
        let escaped_pwd = xml::escape_xml(pwd);

        let profile_xml = format!(
            r#"<?xml version="1.0"?>
<WLANProfile xmlns="http://www.microsoft.com/networking/WLAN/profile/v1">
    <name>{escaped_ssid}</name>
    <SSIDConfig>
        <SSID>
            <name>{escaped_ssid}</name>
        </SSID>
    </SSIDConfig>
    <MSM>
        <security>
            <sharedKey>
                <keyMaterial>{escaped_pwd}</keyMaterial>
            </sharedKey>
        </security>
    </MSM>
</WLANProfile>"#
        );

        // The profile XML should not contain any raw injection tags
        assert!(!profile_xml.contains("</name><name>evil"));
        assert!(!profile_xml.contains("</keyMaterial><x>"));
        // It should contain the escaped versions
        assert!(profile_xml.contains("&lt;/name&gt;"));
        assert!(profile_xml.contains("&lt;/keyMaterial&gt;"));
    }
}
