// hw/wifi.rs
//
// PC WiFi management via Windows netsh wlan commands.
// Provides network scanning, connection status, connect/disconnect.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::process::Command;

/// A WiFi network (SSID) visible to the PC.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WifiNetwork {
    pub ssid: String,
    pub signal: u32,       // 0-100 percentage
    pub security: String,   // e.g. "WPA2-Personal", "Open"
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
pub fn scan_networks() -> Result<Vec<WifiNetwork>> {
    let output = Command::new("netsh")
        .args(["wlan", "show", "networks", "mode=bssid"])
        .output()
        .context("Failed to run netsh wlan show networks")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_scan_output(&stdout)
}

/// Get current WiFi connection status.
pub fn get_status() -> Result<WifiStatus> {
    let output = Command::new("netsh")
        .args(["wlan", "show", "interfaces"])
        .output()
        .context("Failed to run netsh wlan show interfaces")?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_interface_output(&stdout)
}

/// Connect to a WiFi network.
pub fn connect(ssid: &str, password: Option<&str>) -> Result<()> {
    // Create profile XML and connect
    if let Some(pwd) = password {
        // Use netsh to connect with password
        let profile_xml = format!(
            r#"<?xml version="1.0"?>
<WLANProfile xmlns="http://www.microsoft.com/networking/WLAN/profile/v1">
    <name>{ssid}</name>
    <SSIDConfig>
        <SSID>
            <name>{ssid}</name>
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
                <keyMaterial>{pwd}</keyMaterial>
            </sharedKey>
        </security>
    </MSM>
</WLANProfile>"#
        );

        // Write profile to temp file and import
        let temp_dir = std::env::temp_dir();
        let profile_path = temp_dir.join(format!("micontrol_wifi_{ssid}.xml"));
        std::fs::write(&profile_path, &profile_xml)
            .context("Failed to write WiFi profile")?;

        // Add profile
        let add = Command::new("netsh")
            .args(["wlan", "add", "profile", "filename"])
            .arg(&profile_path)
            .output()
            .context("Failed to add WiFi profile")?;

        if !add.status.success() {
            let stderr = String::from_utf8_lossy(&add.stderr);
            anyhow::bail!("Failed to add WiFi profile: {stderr}");
        }

        // Clean up temp file
        let _ = std::fs::remove_file(&profile_path);
    }

    // Connect
    let connect = Command::new("netsh")
        .args(["wlan", "connect", "name"])
        .arg(ssid)
        .output()
        .context("Failed to connect to WiFi")?;

    if !connect.status.success() {
        let stderr = String::from_utf8_lossy(&connect.stderr);
        anyhow::bail!("Failed to connect: {stderr}");
    }

    Ok(())
}

/// Disconnect from current WiFi network.
pub fn disconnect() -> Result<()> {
    let output = Command::new("netsh")
        .args(["wlan", "disconnect"])
        .output()
        .context("Failed to disconnect WiFi")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("Failed to disconnect: {stderr}");
    }

    Ok(())
}

/// Parse netsh wlan show networks output.
fn parse_scan_output(output: &str) -> Result<Vec<WifiNetwork>> {
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
fn parse_interface_output(output: &str) -> Result<WifiStatus> {
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
}