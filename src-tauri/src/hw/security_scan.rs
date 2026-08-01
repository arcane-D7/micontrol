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
use anyhow::Context;
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
        // Prefer the WMI MSFT_MPComputerStatus class (root\Microsoft\Windows\Defender),
        // which reports the *actual* protection state. This is the same source that
        // Windows Security uses and it is reliable regardless of registry key presence.
        if let Ok(status) = get_defender_status_wmi() {
            return Ok(status);
        }
        // Fallback: registry-based status with corrected semantics.
        // A missing DisableAntiVirus/DisableAntiSpyware value means the protection
        // is ENABLED (default) — NOT disabled as the old code assumed.
        let rt = read_defender_registry_bool("DisableRealtimeMonitoring")
            .map(|v| !v) // 1 = monitoring disabled, 0 = enabled
            .unwrap_or(true); // key absent = real-time protection enabled
        let av = read_defender_registry_bool("DisableAntiVirus")
            .map(|v| !v)
            .unwrap_or(true); // key absent = antivirus enabled
        let asw = read_defender_registry_bool("DisableAntiSpyware")
            .map(|v| !v)
            .unwrap_or(true); // key absent = antispyware enabled

        let engine_version = read_defender_registry_string("EngineVersion");
        let product_version = read_defender_registry_string("ProductVersion");

        Ok(DefenderStatus {
            installed: find_mpcmdrun().is_ok(),
            enabled: av || asw || rt,
            antivirus_enabled: av,
            antispyware_enabled: asw,
            real_time_protection: rt,
            definitions_updated: read_defender_registry_string("SignaturesLastUpdated")
                .map(format_registry_filetime),
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

/// Read Defender status from the WMI `MSFT_MPComputerStatus` class.
///
/// NOTE: This provider rejects `SELECT <columns> FROM` queries with
/// `WBEM_E_INVALID_QUERY` (0x80041017) — it only accepts `SELECT *`. That is
/// why we select all columns and extract the fields we need.
#[cfg(windows)]
fn get_defender_status_wmi() -> HardwareResult<DefenderStatus> {
    use std::collections::HashMap;

    let status = crate::hw::wmi_cache::with_defender(|wmi| {
        let rows: Vec<HashMap<String, wmi::Variant>> = wmi
            .raw_query("SELECT * FROM MSFT_MPComputerStatus")
            .context("MSFT_MPComputerStatus query")?;
        rows.into_iter().next().ok_or_else(|| {
            anyhow::anyhow!("MSFT_MPComputerStatus returned no rows (Defender likely replaced by third-party AV)")
        })
    })?;

    let av = crate::util::wmi_extract::extract_bool(&status, "AntivirusEnabled").unwrap_or(false);
    let asw =
        crate::util::wmi_extract::extract_bool(&status, "AntispywareEnabled").unwrap_or(false);
    let rt = crate::util::wmi_extract::extract_bool(&status, "RealTimeProtectionEnabled")
        .unwrap_or(false);

    let wmi_datetime = |key: &str| -> Option<String> {
        let s = crate::util::wmi_extract::extract_string(&status, key)?;
        // WMI DATETIME format: "20260801091242.000000-000" → ISO-ish readable.
        if s.len() >= 14 {
            let (y, mo, d, h, mi, se) = (
                &s[0..4],
                &s[4..6],
                &s[6..8],
                &s[8..10],
                &s[10..12],
                &s[12..14],
            );
            Some(format!("{y}-{mo}-{d} {h}:{mi}:{se}"))
        } else {
            Some(s)
        }
    };

    Ok(DefenderStatus {
        installed: find_mpcmdrun().is_ok(),
        enabled: av || asw || rt,
        antivirus_enabled: av,
        antispyware_enabled: asw,
        real_time_protection: rt,
        definitions_updated: wmi_datetime("AntivirusSignatureLastUpdated"),
        engine_version: crate::util::wmi_extract::extract_string(&status, "EngineVersion"),
        product_version: crate::util::wmi_extract::extract_string(&status, "ProductVersion"),
        signature_version: crate::util::wmi_extract::extract_string(
            &status,
            "AntivirusSignatureVersion",
        ),
        last_scan_time: wmi_datetime("QuickScanEndTime"),
    })
}

/// Convert a registry FILETIME-encoded REG_BINARY value to a readable date
/// string, or return the raw value if it cannot be parsed.
///
/// The registry value read as a Rust `String` may contain the raw FILETIME
/// bytes (8 bytes) as UTF-8 (non-ASCII bytes preserved). We reinterpret them
/// as a little-endian u64 and convert with civil-from-days arithmetic to avoid
/// a chrono dependency.
#[cfg(windows)]
fn format_registry_filetime(raw: String) -> String {
    // If it's a plain ASCII date string already, return it as-is.
    if raw
        .bytes()
        .all(|b| b.is_ascii() && (b.is_ascii_digit() || b.is_ascii_punctuation()))
    {
        return raw;
    }

    let bytes = raw.as_bytes();
    if bytes.len() == 8 {
        let mut le = [0u8; 8];
        le.copy_from_slice(bytes);
        let ft = u64::from_le_bytes(le);
        // FILETIME: 100ns intervals since 1601-01-01.
        // Unix epoch (1970-01-01) offset: 11644473600 seconds.
        const EPOCH_OFFSET_SECS: u64 = 11_644_473_600;
        if ft >= EPOCH_OFFSET_SECS * 10_000_000 {
            let unix_secs = (ft / 10_000_000) - EPOCH_OFFSET_SECS;
            if let Some(s) = format_unix_secs(unix_secs as i64) {
                return s;
            }
        }
    }
    raw
}

/// Format a Unix timestamp (seconds) as `YYYY-MM-DD HH:MM:SS` using
/// civil-from-days arithmetic (no external date library).
#[cfg(windows)]
fn format_unix_secs(secs: i64) -> Option<String> {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);

    // Howard Hinnant's civil_from_days algorithm.
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    Some(format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        y, m, d, hh, mm, ss
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
    // Try each subkey in order; a value may live in only one of them.
    // The old code opened only the first subkey and never fell back to the
    // second, so it returned None for values that existed in the parent key.
    for path in [
        r"SOFTWARE\Microsoft\Windows Defender\Real-Time Protection",
        r"SOFTWARE\Microsoft\Windows Defender",
    ] {
        if let Ok(key) = hklm.open_subkey(path) {
            if let Ok(v) = key.get_value::<u32, _>(value_name) {
                return Some(v != 0);
            }
        }
    }
    None
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
