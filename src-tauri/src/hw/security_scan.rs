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

    let entries = std::fs::read_dir(&base).map_err(HardwareError::Io)?;

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
        // F4.6: `MSFT_MPComputerStatus` exposes AMEngineVersion/AMProductVersion
        // — the old `EngineVersion`/`ProductVersion` names do not exist on the
        // class, so both fields were always null in the UI.
        engine_version: crate::util::wmi_extract::extract_string(&status, "AMEngineVersion"),
        product_version: crate::util::wmi_extract::extract_string(&status, "AMProductVersion"),
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

/// PowerShell that returns the Defender detection history as JSON.
///
/// Audit F4.2: `MpCmdRun -Restore -ListAll` requires administrator and the app
/// is not elevated, so the real answer was always an `Access is denied` the old
/// parser silently discarded. `Get-MpThreatDetection`/`Get-MpThreat` work
/// unprivileged and return structured objects.
///
/// The script reports its own outcome (`ok`): a failed query can never be
/// mistaken for "no detections". Objects are serialised one by one and joined
/// so a single detection is still an array (ConvertTo-Json unwraps
/// single-element arrays at the top level).
const THREAT_HISTORY_PS: &str = r#"$ErrorActionPreference = 'Stop';
try { [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false) } catch {}
try {
  $meta = @{};
  foreach ($t in @(Get-MpThreat)) { $meta[[string]$t.ThreatID] = $t }
  $rows = @();
  foreach ($d in @(Get-MpThreatDetection)) {
    $m = $meta[[string]$d.ThreatID];
    $name = $null; $sev = $null; $cat = $null;
    if ($m) { $name = [string]$m.ThreatName; $sev = [int]$m.SeverityID; $cat = [int]$m.CategoryID }
    $init = $null; $rem = $null;
    if ($d.InitialDetectionTime) { $init = $d.InitialDetectionTime.ToString('yyyy-MM-dd HH:mm:ss') }
    if ($d.RemediationTime) { $rem = $d.RemediationTime.ToString('yyyy-MM-dd HH:mm:ss') }
    $res = @();
    foreach ($r in @($d.Resources)) { if ($r) { $res += [string]$r } }
    $rows += [pscustomobject]@{
      threat_name = $name;
      severity_id = $sev;
      category_id = $cat;
      action_id = [int]$d.CleaningActionID;
      action_success = [bool]$d.ActionSuccess;
      initial_detection_time = $init;
      remediation_time = $rem;
      resources = $res
    }
  }
  $json = ($rows | ForEach-Object { $_ | ConvertTo-Json -Depth 4 -Compress }) -join ','
  Write-Output ('{"ok":true,"items":[' + $json + ']}')
} catch {
  $msg = [string]$_.Exception.Message
  Write-Output ('{"ok":false,"error":' + (ConvertTo-Json $msg -Compress) + '}')
}"#;

/// Get threat detection history from Windows Defender.
///
/// Returns `Err` when the history cannot be read — never an empty list. The
/// UI renders an empty list as "✓ no threats", so silence would be a lie.
pub fn get_threat_history() -> HardwareResult<ThreatHistory> {
    let output = no_window_command("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            THREAT_HISTORY_PS,
        ])
        .output()
        .map_err(|e| {
            HardwareError::Other(format!("Failed to run PowerShell for threat history: {e}"))
        })?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    let threats = parse_threat_payload(&stdout).map_err(|e| {
        if stderr.is_empty() {
            e
        } else {
            HardwareError::Other(format!("{e} (PowerShell stderr: {stderr})"))
        }
    })?;

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

/// Payload emitted by [`THREAT_HISTORY_PS`].
#[derive(Debug, serde::Deserialize)]
struct ThreatPayload {
    ok: bool,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    items: Vec<RawThreat>,
}

/// One detection as serialised by the PowerShell script.
#[derive(Debug, serde::Deserialize)]
struct RawThreat {
    #[serde(default)]
    threat_name: Option<String>,
    #[serde(default)]
    severity_id: Option<u32>,
    #[serde(default)]
    category_id: Option<u32>,
    #[serde(default)]
    action_id: Option<u32>,
    #[serde(default)]
    action_success: bool,
    #[serde(default)]
    initial_detection_time: Option<String>,
    #[serde(default)]
    remediation_time: Option<String>,
    #[serde(default)]
    resources: Vec<String>,
}

/// Parse the JSON contract produced by [`THREAT_HISTORY_PS`].
///
/// * `{"ok":true,"items":[]}`  → no detections (the only legitimate empty result)
/// * `{"ok":false,"error":…}`  → `Err` (history unavailable)
/// * anything unparsable       → `Err` (contract/PowerShell failure)
fn parse_threat_payload(stdout: &str) -> HardwareResult<Vec<ThreatEntry>> {
    let trimmed = stdout.trim();
    if trimmed.is_empty() {
        return Err(HardwareError::Other(
            "Threat history query returned no output (Defender cmdlets unavailable?)".to_string(),
        ));
    }

    let payload: ThreatPayload = serde_json::from_str(trimmed).map_err(|e| {
        // Keep the head of the payload in the error: this is what a blocked
        // script policy or a localised PowerShell error looks like.
        let head: String = trimmed.chars().take(300).collect();
        HardwareError::Other(format!(
            "Threat history payload is not valid JSON: {e} — got: {head}"
        ))
    })?;

    if !payload.ok {
        return Err(HardwareError::Other(format!(
            "Threat history unavailable: {}",
            payload
                .error
                .unwrap_or_else(|| "unknown Defender query error".to_string())
        )));
    }

    Ok(payload
        .items
        .into_iter()
        .map(|raw| ThreatEntry {
            threat_name: raw
                .threat_name
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| "Unnamed detection".to_string()),
            severity_id: raw.severity_id,
            severity_name: raw.severity_id.map(severity_name),
            category_id: raw.category_id,
            category_name: raw.category_id.and_then(category_name),
            action_success: raw.action_success,
            action_id: raw.action_id,
            action_name: raw.action_id.and_then(action_name),
            initial_detection_time: raw.initial_detection_time,
            remediation_time: raw.remediation_time,
            resources: raw.resources,
        })
        .collect())
}

/// `MSFT_MpThreat.SeverityID` → label shown in the severity badge.
///
/// Always returns a label: an unmapped ID must not be rendered as "Low" by the
/// UI's fallback.
fn severity_name(id: u32) -> String {
    match id {
        1 => "Low",
        2 => "Moderate",
        4 => "High",
        5 => "Severe",
        _ => "Unknown",
    }
    .to_string()
}

/// `MSFT_MpThreat.CategoryID` → label. `None` for IDs we do not map, so the UI
/// shows the numeric ID instead of an invented name.
fn category_name(id: u32) -> Option<String> {
    let name = match id {
        1 => "Adware",
        2 => "Spyware",
        3 => "Password stealer",
        4 => "Trojan downloader",
        5 => "Worm",
        6 => "Backdoor",
        7 => "Trojan",
        8 => "Virus",
        10 => "Rootkit",
        11 => "Unwanted software",
        _ => return None,
    };
    Some(name.to_string())
}

/// `MSFT_MpThreatDetection.CleaningActionID` → label.
///
/// Only the unambiguous actions are mapped; for the rest the UI falls back to
/// the action-success tick, which is truthful on its own.
fn action_name(id: u32) -> Option<String> {
    let name = match id {
        1 => "Clean",
        2 => "Quarantine",
        3 => "Remove",
        4 => "Allow",
        _ => return None,
    };
    Some(name.to_string())
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
        // An explicit, successful "no detections" is the ONLY empty result.
        let threats = parse_threat_payload("{\"ok\":true,\"items\":[]}").expect("valid payload");
        assert!(threats.is_empty());
    }

    #[test]
    fn test_parse_threat_payload_with_entries() {
        let payload = r#"{"ok":true,"items":[
            {"threat_name":"Trojan:Win32/Test","severity_id":4,"category_id":7,"action_id":2,
             "action_success":true,"initial_detection_time":"2026-11-05 12:00:00",
             "remediation_time":"2026-11-05 12:01:00","resources":["C:\\tmp\\eicar.com"]}
        ]}"#;
        let threats = parse_threat_payload(payload).expect("valid payload");
        assert_eq!(threats.len(), 1);
        assert_eq!(threats[0].threat_name, "Trojan:Win32/Test");
        assert_eq!(threats[0].severity_id, Some(4));
        assert_eq!(threats[0].severity_name.as_deref(), Some("High"));
        assert_eq!(threats[0].category_name.as_deref(), Some("Trojan"));
        assert_eq!(threats[0].action_name.as_deref(), Some("Quarantine"));
        assert!(threats[0].action_success);
        assert_eq!(
            threats[0].initial_detection_time.as_deref(),
            Some("2026-11-05 12:00:00")
        );
        assert_eq!(threats[0].resources, vec!["C:\\tmp\\eicar.com"]);
    }

    #[test]
    fn test_parse_threat_payload_missing_optional_fields() {
        // Defender omits fields the detection does not have; the parser must
        // not drop the row (the old text parser did exactly that).
        let payload = r#"{"ok":true,"items":[{"action_success":false}]}"#;
        let threats = parse_threat_payload(payload).expect("valid payload");
        assert_eq!(threats.len(), 1);
        assert_eq!(threats[0].threat_name, "Unnamed detection");
        assert!(threats[0].severity_name.is_none());
        assert!(!threats[0].action_success);
    }

    #[test]
    fn test_parse_threat_payload_failure_is_an_error() {
        // This is the audit's F4.2 regression test: an access-denied answer
        // must NEVER be turned into an empty list.
        let payload = r#"{"ok":false,"error":"Access is denied."}"#;
        let err = parse_threat_payload(payload).expect_err("must be an error");
        assert!(err.to_string().contains("Access is denied"), "{err}");
    }

    #[test]
    fn test_parse_threat_payload_empty_output_is_an_error() {
        let err = parse_threat_payload("   \r\n").expect_err("must be an error");
        assert!(err.to_string().contains("no output"), "{err}");
    }

    #[test]
    fn test_parse_threat_payload_garbage_is_an_error() {
        let err = parse_threat_payload("You need administrator privilege").expect_err("must fail");
        assert!(err.to_string().contains("not valid JSON"), "{err}");
    }

    #[test]
    fn test_severity_and_action_mappings() {
        assert_eq!(severity_name(1), "Low");
        assert_eq!(severity_name(5), "Severe");
        assert_eq!(severity_name(99), "Unknown");
        assert_eq!(action_name(1).as_deref(), Some("Clean"));
        assert_eq!(action_name(99), None, "unmapped actions stay unmapped");
        assert_eq!(category_name(99), None);
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
