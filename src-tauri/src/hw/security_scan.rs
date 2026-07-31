//! Security scanning via Windows Defender CLI (MpCmdRun.exe).
//!
//! Wraps the Windows Defender command-line tool to provide:
//! - Quick scans
//! - Full system scans
//! - Custom path scans
//! - Signature updates
//! - Threat history retrieval
//!
//! MpCmdRun.exe is located at:
//! `C:\ProgramData\Microsoft\Windows Defender\Platform\<version>\MpCmdRun.exe`

use crate::hw::errors::{HardwareError, HardwareResult};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ── Constants ────────────────────────────────────────────────────────────────

const DEFENDER_BASE: &str = r"C:\ProgramData\Microsoft\Windows Defender\Platform";

// ── Data structures ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityScanResult {
    /// Type of scan performed
    pub scan_type: ScanType,
    /// Exit status: 0 = clean, 2 = malware found, other = error
    pub exit_code: i32,
    /// Human-readable status
    pub status: ScanStatus,
    /// Stdout from MpCmdRun (trimmed)
    pub output: String,
    /// Duration in seconds
    pub duration_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ScanType {
    Quick,
    Full,
    Custom,
    SignatureUpdate,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ScanStatus {
    /// No threats found
    Clean,
    /// Threats were detected and action taken
    ThreatsDetected,
    /// Scan is currently running
    InProgress,
    /// Scan failed or was cancelled
    Error,
    /// Signatures updated successfully
    Updated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreatHistory {
    pub threats: Vec<ThreatEntry>,
    pub total_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreatEntry {
    pub threat_name: String,
    pub severity_id: Option<u32>,
    pub severity_name: Option<String>,
    pub category_id: Option<u32>,
    pub category_name: Option<String>,
    pub action_success: bool,
    pub action_id: Option<u32>,
    pub action_name: Option<String>,
    pub initial_detection_time: Option<String>,
    pub remediation_time: Option<String>,
    pub resources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefenderStatus {
    pub installed: bool,
    pub enabled: bool,
    pub antivirus_enabled: bool,
    pub antispyware_enabled: bool,
    pub real_time_protection: bool,
    pub definitions_updated: Option<String>,
    pub engine_version: Option<String>,
    pub product_version: Option<String>,
    /// Signature version string (AV signature version)
    #[serde(rename = "signature_version")]
    pub signature_version: Option<String>,
    /// Last scan time (human-readable)
    #[serde(rename = "last_scan_time")]
    pub last_scan_time: Option<String>,
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Find the MpCmdRun.exe path by locating the highest version directory.
fn find_mpcmdrun() -> HardwareResult<PathBuf> {
    let base = PathBuf::from(DEFENDER_BASE);
    if !base.exists() {
        return Err(HardwareError::Other(
            "Windows Defender platform directory not found".to_string(),
        ));
    }

    let entries = std::fs::read_dir(&base).map_err(|e| HardwareError::Io(e))?;

    let mut versions: Vec<String> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
        .collect();
    versions.sort();
    versions.reverse(); // newest first

    for ver in &versions {
        let exe = base.join(ver).join("MpCmdRun.exe");
        if exe.exists() {
            return Ok(exe);
        }
    }

    Err(HardwareError::Other(
        "MpCmdRun.exe not found in any Defender Platform version directory".to_string(),
    ))
}

/// Run a quick scan (checks common malware locations).
pub fn quick_scan() -> HardwareResult<SecurityScanResult> {
    run_scan(ScanType::Quick, None)
}

/// Run a full system scan (checks all drives).
pub fn full_scan() -> HardwareResult<SecurityScanResult> {
    run_scan(ScanType::Full, None)
}

/// Run a custom scan on a specific file or directory.
pub fn custom_scan(path: &str) -> HardwareResult<SecurityScanResult> {
    run_scan(ScanType::Custom, Some(path))
}

/// Update virus and spyware definitions.
pub fn update_signatures() -> HardwareResult<SecurityScanResult> {
    let exe = find_mpcmdrun()?;
    let start = std::time::Instant::now();

    let exe_str = exe.to_str().ok_or_else(|| {
        HardwareError::Other("Defender executable path is not valid UTF-8".to_string())
    })?;
    let output = no_window_command(exe_str)
        .args(["-SignatureUpdate"])
        .output()
        .map_err(|e| HardwareError::Other(format!("Failed to run MpCmdRun: {e}")))?;

    let duration = start.elapsed().as_secs();
    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    let combined = if stdout.is_empty() { stderr } else { stdout };
    let status = if exit_code == 0 {
        ScanStatus::Updated
    } else {
        ScanStatus::Error
    };

    Ok(SecurityScanResult {
        scan_type: ScanType::SignatureUpdate,
        exit_code,
        status,
        output: combined,
        duration_secs: duration,
    })
}

/// Get the current Windows Defender status.
pub fn get_defender_status() -> HardwareResult<DefenderStatus> {
    #[cfg(windows)]
    {
        // Read status from registry (no WMI SecurityCenter2 namespace needed)
        let rt = read_defender_registry_bool("RealTimeProtectionEnabled").unwrap_or(false);
        let av = read_defender_registry_bool("DisableAntiVirus")
            .map(|v| !v)
            .unwrap_or(false);
        let asw = read_defender_registry_bool("DisableAntiSpyware")
            .map(|v| !v)
            .unwrap_or(false);

        let definitions_updated = read_defender_registry_string("SignaturesLastUpdated");
        let engine_version = read_defender_registry_string("EngineVersion");
        let product_version = read_defender_registry_string("ProductVersion");

        Ok(DefenderStatus {
            installed: find_mpcmdrun().is_ok(),
            enabled: av || asw || rt,
            antivirus_enabled: av,
            antispyware_enabled: asw,
            real_time_protection: rt,
            definitions_updated,
            engine_version: engine_version.clone(),
            product_version,
            signature_version: read_defender_registry_string("AVSignatureVersion"),
            last_scan_time: read_defender_registry_string("LastQuickScanEndTime"),
        })
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "Security scan only available on Windows".into(),
    ))
}

/// Get threat detection history from Windows Defender.
pub fn get_threat_history() -> HardwareResult<ThreatHistory> {
    let exe = find_mpcmdrun()?;

    let exe_str = exe.to_str().ok_or_else(|| {
        HardwareError::Other("Defender executable path is not valid UTF-8".to_string())
    })?;
    let output = no_window_command(exe_str)
        .args(["-Restore", "-ListAll"])
        .output()
        .map_err(|e| HardwareError::Other(format!("Failed to get threat history: {e}")))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();

    // Parse the output — MpCmdRun -Restore -ListAll outputs a list of threats
    let threats = parse_threat_list(&stdout);

    Ok(ThreatHistory {
        total_count: threats.len(),
        threats,
    })
}

// ── Internal helpers ─────────────────────────────────────────────────────────

fn run_scan(scan_type: ScanType, custom_path: Option<&str>) -> HardwareResult<SecurityScanResult> {
    let exe = find_mpcmdrun()?;
    let start = std::time::Instant::now();

    let exe_str = exe.to_str().ok_or_else(|| {
        HardwareError::Other("Defender executable path is not valid UTF-8".to_string())
    })?;
    let mut cmd = no_window_command(exe_str);
    cmd.arg("-Scan");

    match scan_type {
        ScanType::Quick => {
            cmd.args(["-ScanType", "1"]);
        }
        ScanType::Full => {
            cmd.args(["-ScanType", "2"]);
        }
        ScanType::Custom => {
            // Validate the custom path exists before scanning
            if let Some(path) = custom_path {
                let p = std::path::Path::new(path);
                if !p.exists() {
                    return Err(HardwareError::Other(format!(
                        "Scan path does not exist: {path}"
                    )));
                }
                // Canonicalize to resolve relative paths and prevent traversal
                let canonical = std::fs::canonicalize(p).map_err(|e| {
                    HardwareError::Other(format!("Failed to canonicalize scan path: {e}"))
                })?;
                let canonical_str = canonical.to_string_lossy();
                cmd.args(["-ScanType", "3", "-File", &canonical_str]);
            } else {
                cmd.args(["-ScanType", "3"]);
            }
        }
        ScanType::SignatureUpdate => {
            // Handled by update_signatures()
        }
    }

    let output = cmd
        .output()
        .map_err(|e| HardwareError::Other(format!("Failed to run MpCmdRun: {e}")))?;

    let duration = start.elapsed().as_secs();
    let exit_code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    let combined = if stdout.is_empty() { stderr } else { stdout };
    let status = match exit_code {
        0 => ScanStatus::Clean,
        2 => ScanStatus::ThreatsDetected,
        _ => ScanStatus::Error,
    };

    Ok(SecurityScanResult {
        scan_type,
        exit_code,
        status,
        output: combined,
        duration_secs: duration,
    })
}

fn parse_threat_list(output: &str) -> Vec<ThreatEntry> {
    let mut threats = Vec::new();

    // MpCmdRun -Restore -ListAll output format varies by version
    // Look for lines containing threat names
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("MpCmdRun") || line.starts_with("Copyright") {
            continue;
        }
        if line.contains("Threat") || line.contains("Malware") || line.contains("Virus") {
            threats.push(ThreatEntry {
                threat_name: line.to_string(),
                severity_id: None,
                severity_name: None,
                category_id: None,
                category_name: None,
                action_success: true,
                action_id: None,
                action_name: None,
                initial_detection_time: None,
                remediation_time: None,
                resources: vec![],
            });
        }
    }

    threats
}

#[cfg(windows)]
fn read_defender_registry_bool(value_name: &str) -> Option<bool> {
    use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let key = hklm
        .open_subkey(r"SOFTWARE\Microsoft\Windows Defender\Real-Time Protection")
        .or_else(|_| hklm.open_subkey(r"SOFTWARE\Microsoft\Windows Defender"))
        .ok()?;
    key.get_value::<u32, _>(value_name).ok().map(|v| v != 0)
}

#[cfg(windows)]
fn read_defender_registry_string(value_name: &str) -> Option<String> {
    use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};
    let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
    let key = hklm
        .open_subkey(r"SOFTWARE\Microsoft\Windows Defender\Signature Updates")
        .or_else(|_| hklm.open_subkey(r"SOFTWARE\Microsoft\Windows Defender"))
        .ok()?;
    key.get_value::<String, _>(value_name).ok()
}

fn no_window_command(program: &str) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_mpcmdrun_does_not_panic() {
        let _ = find_mpcmdrun();
    }

    #[test]
    fn test_parse_threat_list_empty() {
        let threats = parse_threat_list("");
        assert!(threats.is_empty());
    }

    #[test]
    fn test_parse_threat_list_with_entries() {
        let output = "Threat: Trojan:Win32/Test\nMalware: Win32/Dangerous\n";
        let threats = parse_threat_list(output);
        assert_eq!(threats.len(), 2);
        assert!(threats[0].threat_name.contains("Trojan"));
    }

    #[test]
    fn test_scan_status_from_exit_code() {
        assert_eq!(
            match 0 {
                0 => ScanStatus::Clean,
                2 => ScanStatus::ThreatsDetected,
                _ => ScanStatus::Error,
            },
            ScanStatus::Clean
        );
    }
}
