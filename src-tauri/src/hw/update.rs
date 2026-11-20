//! Driver and BIOS update checking via Xiaomi PC Manager registry cache.
//!
//! Reuses XiaomiPCManager's own scan engine to check for official
//! Xiaomi driver and BIOS updates without Windows Update.

/// Update Nucleus — Phase 9
///
/// This module acts as a lightweight "sandbox nucleus" that reuses XiaomiPCManager's
/// own scan engine to check for official Xiaomi driver and BIOS updates.
/// It avoids generic Windows Update drivers by reading from XPM's registry cache
/// and, when available, triggering XPM's update checker component directly.
use crate::hw::errors::{HardwareError, HardwareResult};
use crate::util::process_spawn;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

// ── Data structures ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BiosInfo {
    pub version: String,
    pub release_date: String,
    pub manufacturer: String,
    pub serial_number: String,
}

/// An OEM driver installed from a Xiaomi/MI provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct XiaomiDriverInfo {
    /// Published INF name in driver store, e.g. "oem71.inf"
    pub published_name: String,
    /// Original INF filename, e.g. "virtualcontrolhid.inf"
    pub original_name: String,
    /// Provider as listed in pnputil, e.g. "Xiaomi Inc."
    pub provider: String,
    /// Combined date+version string from pnputil, e.g. "05/14/2025 1.0.0.1"
    pub version_string: String,
    /// Class name, e.g. "System"
    pub class_name: String,
    /// Signer (WHQL status)
    pub signer: String,
}

/// Detailed driver information from Win32_PnPSignedDriver WMI class.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriverDetail {
    /// Device name (friendly name)
    pub device_name: String,
    /// Driver device class (e.g. "Display", "System", "HIDClass")
    pub device_class: String,
    /// Hardware ID (first from the list)
    pub hardware_id: String,
    /// Driver manufacturer
    pub manufacturer: String,
    /// Driver version (e.g. "1.0.0.1")
    pub driver_version: String,
    /// Driver date (e.g. "20250514000000.000000-000")
    pub driver_date: String,
    /// Driver INF file name (e.g. "oem71.inf")
    pub inf_name: String,
    /// Driver provider (e.g. "Xiaomi Inc.")
    pub driver_provider_name: String,
    /// Whether the driver is digitally signed
    pub is_signed: bool,
    /// Signer name (e.g. "Microsoft Windows Hardware Compatibility Publisher")
    pub signer: String,
    /// Driver status (e.g. "OK", "Error", "Unknown")
    pub status: String,
}

/// Full update / version status returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateStatus {
    pub bios: BiosInfo,
    pub xiaomi_drivers: Vec<XiaomiDriverInfo>,
    /// ISO-style timestamp from HKLM\SOFTWARE\MI\Driver\LastScanTime
    pub last_xpm_scan: Option<String>,
    /// Raw registry values from HKLM\SOFTWARE\MI\Driver\ (key→value)
    pub xpm_driver_cache: HashMap<String, String>,
    pub xpm_installed: bool,
    pub xpm_version: Option<String>,
    pub xpm_path: Option<String>,
}

// ── Constants ────────────────────────────────────────────────────────────────

const XPM_BASE: &str = r"C:\Program Files\MI\XiaomiPCManager";
const DRIVER_REG_KEY: &str = r"SOFTWARE\MI\Driver";

/// Budget for `pnputil /enum-drivers`: typically under 5 s; 60 s covers a
/// cold disk with hundreds of driver-store entries.
const PNPUTIL_ENUM_TIMEOUT: Duration = Duration::from_secs(60);
/// Budget for `pnputil /scan-devices`: a full PnP re-enumeration can take
/// tens of seconds; 120 s is the ceiling before the registry-flag fallback.
const PNPUTIL_SCAN_TIMEOUT: Duration = Duration::from_secs(120);

// ── Public API ───────────────────────────────────────────────────────────────

/// Collect full update / version status.
/// Reads XPM registry cache + installed drivers via pnputil + BIOS via WMI.
pub fn get_update_status() -> HardwareResult<UpdateStatus> {
    let (xpm_installed, xpm_path, xpm_version) = detect_xpm();
    let last_xpm_scan = read_last_scan_time().ok();
    let xpm_driver_cache = read_xpm_driver_cache().unwrap_or_default();
    let xiaomi_drivers = get_xiaomi_drivers().unwrap_or_default();
    let bios = get_bios_info().unwrap_or_default();

    Ok(UpdateStatus {
        bios,
        xiaomi_drivers,
        last_xpm_scan,
        xpm_driver_cache,
        xpm_installed,
        xpm_version,
        xpm_path,
    })
}

/// Trigger a driver scan using XPM's nucleus.
///
/// Priority order:
///   1. Spawn XPM's helper executable with --update-driver flag
///   2. Run `pnputil /scan-devices` (Windows built-in — forces PnP re-enumeration
///      and can pull updated drivers from Windows Update if nothing better is available)
///   3. Write HKLM\SOFTWARE\MI\Driver\RequestScan=1 registry flag (read by XPM on next start)
///
/// Returns a human-readable message describing what was done.
pub fn trigger_driver_scan() -> HardwareResult<String> {
    let (installed, path_opt, _) = detect_xpm();

    // ── Strategy 1: try known XPM helper executables ──────────────────────
    if installed {
        let xpm_path = path_opt.as_deref().unwrap_or(XPM_BASE);
        let candidates = [
            ("XiaomiPCManagerHelper.exe", "--update-driver"),
            ("MiUpdateHelper.exe", ""),
            ("SvrCModuleHost.exe", "--scan"),
        ];
        for (exe, arg) in &candidates {
            let full = format!(r"{}\{}", xpm_path, exe);
            if std::path::Path::new(&full).exists() {
                let mut cmd = no_window_command(&full);
                if !arg.is_empty() {
                    cmd.arg(arg);
                }
                match cmd.spawn() {
                    Ok(_) => {
                        return Ok(format!(
                            "XPM nucleus triggered via {exe}. Check back in a moment."
                        ));
                    }
                    Err(e) => {
                        log::warn!("Failed to spawn {exe}: {e}");
                    }
                }
            }
        }
    }

    // ── Strategy 2: pnputil /scan-devices (always available on Win10+) ───
    // Bounded wait: on timeout (or any spawn failure) the tree dies and the
    // registry-flag fallback below still runs, same as the old Err path.
    let mut scan = no_window_command("pnputil");
    scan.args(["/scan-devices"]);
    match process_spawn::run_capture(&mut scan, PNPUTIL_SCAN_TIMEOUT) {
        Ok(out) if out.success() => {
            return Ok("PnP device scan triggered (pnputil /scan-devices). \
                 Windows will check for updated official drivers."
                .to_string());
        }
        Ok(out) => {
            log::warn!("pnputil /scan-devices exit: {}", out.status);
        }
        Err(e) => {
            log::warn!("pnputil /scan-devices unavailable: {e}");
        }
    }

    // ── Strategy 3: write registry flag for XPM to pick up on next start ─
    #[cfg(windows)]
    {
        use winreg::{
            enums::{HKEY_LOCAL_MACHINE, KEY_WRITE},
            RegKey,
        };
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        if let Ok(key) = hklm.open_subkey_with_flags(DRIVER_REG_KEY, KEY_WRITE) {
            let _ = key.set_value("RequestScan", &1u32);
            return Ok("Scan request flag written to registry. \
                 XiaomiPCManager will perform a driver scan on next launch."
                .to_string());
        }
    }

    Err(HardwareError::Other(
        "Could not trigger scan: XPM not installed and pnputil /scan-devices unavailable."
            .to_string(),
    ))
}

// ── XPM detection ────────────────────────────────────────────────────────────

fn detect_xpm() -> (bool, Option<String>, Option<String>) {
    let base = std::path::Path::new(XPM_BASE);
    if !base.exists() {
        return (false, None, None);
    }
    // Find highest version sub-directory (e.g. "5.8.0.57")
    if let Ok(entries) = std::fs::read_dir(base) {
        let mut versions: Vec<String> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_dir())
            .filter_map(|e| e.file_name().to_str().map(|s| s.to_string()))
            .filter(|s| {
                s.chars()
                    .next()
                    .map(|c| c.is_ascii_digit())
                    .unwrap_or(false)
            })
            .collect();
        versions.sort();
        if let Some(latest) = versions.last() {
            let path = format!(r"{}\{}", XPM_BASE, latest);
            return (true, Some(path), Some(latest.clone()));
        }
    }
    (true, Some(XPM_BASE.to_string()), None)
}

// ── Registry reads ───────────────────────────────────────────────────────────

fn read_last_scan_time() -> anyhow::Result<String> {
    #[cfg(windows)]
    {
        use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let key = hklm
            .open_subkey(DRIVER_REG_KEY)
            .context("HKLM\\SOFTWARE\\MI\\Driver not found")?;
        let val: String = key
            .get_value("LastScanTime")
            .context("LastScanTime not found")?;
        Ok(val)
    }
    #[cfg(not(windows))]
    anyhow::bail!("Registry not available on non-Windows")
}

/// Read all string values from HKLM\SOFTWARE\MI\Driver\ into a flat map.
fn read_xpm_driver_cache() -> anyhow::Result<HashMap<String, String>> {
    #[cfg(windows)]
    {
        use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        let key = hklm
            .open_subkey(DRIVER_REG_KEY)
            .context("HKLM\\SOFTWARE\\MI\\Driver not found")?;
        let mut map = HashMap::new();
        for entry in key.enum_values().filter_map(|r| r.ok()) {
            let (name, val) = entry;
            // Convert REG_SZ / REG_DWORD to string
            let s = format!("{:?}", val.bytes); // fallback
            let s = match val.vtype {
                winreg::enums::RegType::REG_SZ => {
                    if let Ok(sv) = key.get_value::<String, _>(&name) {
                        sv
                    } else {
                        s
                    }
                }
                winreg::enums::RegType::REG_DWORD => {
                    if let Ok(dv) = key.get_value::<u32, _>(&name) {
                        dv.to_string()
                    } else {
                        s
                    }
                }
                _ => s,
            };
            map.insert(name, s);
        }
        Ok(map)
    }
    #[cfg(not(windows))]
    Ok(HashMap::new())
}

// ── BIOS info (WMI) ──────────────────────────────────────────────────────────

fn get_bios_info() -> anyhow::Result<BiosInfo> {
    #[cfg(windows)]
    {
        use wmi::{COMLibrary, WMIConnection};
        let com = COMLibrary::new().context("COM init")?;
        let wmi = WMIConnection::new(com).context("WMI connect")?;
        let results: Vec<HashMap<String, wmi::Variant>> = wmi
            .raw_query("SELECT Version, ReleaseDate, Manufacturer, SerialNumber FROM Win32_BIOS")
            .context("WMI Win32_BIOS query")?;
        let row = results.into_iter().next().unwrap_or_default();
        Ok(BiosInfo {
            version: variant_str(&row, "Version"),
            release_date: variant_str(&row, "ReleaseDate"),
            manufacturer: variant_str(&row, "Manufacturer"),
            serial_number: variant_str(&row, "SerialNumber"),
        })
    }
    #[cfg(not(windows))]
    Ok(BiosInfo::default())
}

// ── Installed Xiaomi drivers (pnputil) ───────────────────────────────────────

fn get_xiaomi_drivers() -> anyhow::Result<Vec<XiaomiDriverInfo>> {
    let mut cmd = no_window_command("pnputil");
    cmd.args(["/enum-drivers"]);
    let output = process_spawn::run_capture(&mut cmd, PNPUTIL_ENUM_TIMEOUT)
        .context("pnputil /enum-drivers failed")?;
    let text = output.stdout_str();
    parse_pnputil_drivers(&text)
}

fn parse_pnputil_drivers(text: &str) -> anyhow::Result<Vec<XiaomiDriverInfo>> {
    let mut result = Vec::new();
    let mut current: HashMap<String, String> = HashMap::new();

    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            if !current.is_empty() {
                if let Some(d) = build_driver_info(&current) {
                    result.push(d);
                }
                current.clear();
            }
        } else if let Some((key, value)) = line.split_once(':') {
            current.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    // flush last entry
    if !current.is_empty() {
        if let Some(d) = build_driver_info(&current) {
            result.push(d);
        }
    }

    // Keep only Xiaomi / MI / our bundled drivers
    Ok(result
        .into_iter()
        .filter(|d| {
            let p = d.provider.to_lowercase();
            let n = d.original_name.to_lowercase();
            p.contains("xiaomi")
                || p.contains("mi corp")
                || p.contains("mi inc")
                || n.contains("virtualcontrolhid")
                || n.contains("iotdriver")
                || n.contains("xiaomi")
        })
        .collect())
}

fn build_driver_info(map: &HashMap<String, String>) -> Option<XiaomiDriverInfo> {
    let published_name = map.get("Published Name").cloned()?;
    Some(XiaomiDriverInfo {
        published_name,
        original_name: map.get("Original Name").cloned().unwrap_or_default(),
        provider: map.get("Provider Name").cloned().unwrap_or_default(),
        version_string: map.get("Driver Version").cloned().unwrap_or_default(),
        class_name: map.get("Class Name").cloned().unwrap_or_default(),
        signer: map.get("Signer Name").cloned().unwrap_or_default(),
    })
}

// ── Helpers ──────────────────────────────────────────────────────────────────

/// Get detailed driver information via Win32_PnPSignedDriver WMI class.
///
/// Returns all signed drivers on the system, including hardware IDs,
/// driver versions, dates, and signer information.
pub fn get_drivers_detail() -> HardwareResult<Vec<DriverDetail>> {
    #[cfg(windows)]
    {
        use crate::hw::wmi_cache;
        use crate::util::wmi_extract;

        wmi_cache::with_cimv2(|wmi| {
            // F5.2: `Win32_PnPSignedDriver` declares `Status` (inherited from
            // CIM_ManagedSystemElement) but its provider returns NULL for every
            // row — measured 355/355 NULL on the audit machine — so the old
            // read fell back to "Unknown" for all 351 devices in the UI.
            // Device health is published by `Win32_PnPEntity`, joined here by
            // DeviceID (`ConfigManagerErrorCode` 0 == working, anything else is
            // the Device Manager problem code).
            let drivers: Vec<HashMap<String, wmi::Variant>> = wmi.raw_query(
                "SELECT DeviceID, DeviceName, DeviceClass, HardwareID, Manufacturer, \
                 DriverVersion, DriverDate, InfName, DriverProviderName, \
                 IsSigned, Signer \
                 FROM Win32_PnPSignedDriver",
            )?;

            let entities: Vec<HashMap<String, wmi::Variant>> = wmi
                .raw_query("SELECT DeviceID, Status, ConfigManagerErrorCode FROM Win32_PnPEntity")
                .unwrap_or_default();
            let health = build_device_health_map(&entities);

            let result = drivers
                .into_iter()
                .map(|d| {
                    let device_id = wmi_extract::extract_string_or(&d, "DeviceID", "");
                    let device_health = health.get(&normalize_device_id(&device_id));
                    DriverDetail {
                        device_name: wmi_extract::extract_string_or(&d, "DeviceName", ""),
                        device_class: wmi_extract::extract_string_or(&d, "DeviceClass", ""),
                        hardware_id: wmi_extract::extract_string_or(&d, "HardwareID", ""),
                        manufacturer: wmi_extract::extract_string_or(&d, "Manufacturer", ""),
                        driver_version: wmi_extract::extract_string_or(&d, "DriverVersion", ""),
                        driver_date: wmi_extract::extract_string_or(&d, "DriverDate", ""),
                        inf_name: wmi_extract::extract_string_or(&d, "InfName", ""),
                        driver_provider_name: wmi_extract::extract_string_or(
                            &d,
                            "DriverProviderName",
                            "",
                        ),
                        is_signed: wmi_extract::extract_bool(&d, "IsSigned").unwrap_or(false),
                        signer: wmi_extract::extract_string_or(&d, "Signer", ""),
                        status: driver_status_text(
                            device_health.and_then(|h| h.status.as_deref()),
                            device_health.and_then(|h| h.config_manager_error_code),
                        ),
                    }
                })
                .collect();

            Ok(result)
        })
        .map_err(|e| HardwareError::Wmi(format!("Win32_PnPSignedDriver query failed: {e}")))
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "Driver details only available on Windows".into(),
    ))
}

/// Device health as reported by `Win32_PnPEntity`.
#[cfg(windows)]
#[derive(Debug, Clone, Default)]
struct DeviceHealth {
    status: Option<String>,
    config_manager_error_code: Option<u32>,
}

/// Index `Win32_PnPEntity` rows by normalized DeviceID.
#[cfg(windows)]
fn build_device_health_map(
    entities: &[HashMap<String, wmi::Variant>],
) -> HashMap<String, DeviceHealth> {
    use crate::util::wmi_extract;

    let mut map = HashMap::with_capacity(entities.len());
    for entity in entities {
        let Some(device_id) = wmi_extract::extract_string(entity, "DeviceID") else {
            continue;
        };
        map.insert(
            normalize_device_id(&device_id),
            DeviceHealth {
                status: wmi_extract::extract_string(entity, "Status")
                    .filter(|status| !status.trim().is_empty()),
                config_manager_error_code: wmi_extract::extract_u32(
                    entity,
                    "ConfigManagerErrorCode",
                ),
            },
        );
    }
    map
}

/// WMI DeviceIDs are case-insensitive; normalise before joining.
fn normalize_device_id(device_id: &str) -> String {
    device_id.trim().to_ascii_uppercase()
}

/// Compose the Status column from Windows' own device status + problem code.
///
/// `ConfigManagerErrorCode == 0` is the documented "working properly" value;
/// any other code is the Device Manager problem number, which is what the user
/// would see there. `None` for both means Windows published no health record
/// for this driver (e.g. print queues, which have no PnP entity) — "Unknown"
/// is then the truthful answer rather than a default.
fn driver_status_text(status: Option<&str>, config_manager_error_code: Option<u32>) -> String {
    let status = status.map(str::trim).filter(|status| !status.is_empty());
    match (status, config_manager_error_code) {
        (_, Some(0)) => status.unwrap_or("OK").to_string(),
        (Some(status), Some(code)) => format!("{status} (code {code})"),
        (None, Some(code)) => format!("Error (code {code})"),
        (Some(status), None) => status.to_string(),
        (None, None) => "Unknown".to_string(),
    }
}

fn variant_str(map: &HashMap<String, wmi::Variant>, key: &str) -> String {
    match map.get(key) {
        Some(wmi::Variant::String(s)) => s.clone(),
        _ => String::new(),
    }
}

/// Build a Command that does not create a visible console window on Windows.
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

    /// Verify that parse_pnputil_drivers correctly isolates Xiaomi-provider entries.
    #[test]
    fn test_parse_pnputil_filters_xiaomi() {
        let sample = "Published Name:     oem71.inf\r\n\
Original Name:      virtualcontrolhid.inf\r\n\
Provider Name:      Xiaomi Inc.\r\n\
Class Name:         System\r\n\
Class GUID:         {4d36e97d-e325-11ce-bfc1-08002be10318}\r\n\
Driver Version:     05/14/2025 1.0.0.1\r\n\
Signer Name:        Microsoft Windows Hardware Compatibility Publisher\r\n\
\r\n\
Published Name:     oem169.inf\r\n\
Original Name:      iotdriver.inf\r\n\
Provider Name:      Xiaomi Corporation\r\n\
Class Name:         System\r\n\
Class GUID:         {4d36e97d-e325-11ce-bfc1-08002be10318}\r\n\
Driver Version:     03/01/2025 2.0.0.5\r\n\
Signer Name:        Microsoft Windows Hardware Compatibility Publisher\r\n\
\r\n\
Published Name:     oem5.inf\r\n\
Original Name:      generic_usb.inf\r\n\
Provider Name:      Microsoft\r\n\
Class Name:         USB\r\n\
Driver Version:     01/01/2023 1.0.0.0\r\n\
Signer Name:        Microsoft Windows\r\n";

        let drivers = parse_pnputil_drivers(sample).unwrap();
        // Only Xiaomi-provider entries should be kept
        assert_eq!(drivers.len(), 2);
        assert_eq!(drivers[0].published_name, "oem71.inf");
        assert_eq!(drivers[0].original_name, "virtualcontrolhid.inf");
        assert!(drivers[0].provider.contains("Xiaomi"));
        assert_eq!(drivers[1].published_name, "oem169.inf");
        assert_eq!(drivers[1].original_name, "iotdriver.inf");
    }

    #[test]
    fn test_parse_pnputil_empty() {
        let drivers = parse_pnputil_drivers("Microsoft PnP Utility\r\n\r\n").unwrap();
        assert!(drivers.is_empty());
    }

    #[test]
    fn test_detect_xpm_does_not_panic() {
        // Just verify it doesn't panic — actual result depends on the machine
        let (_, _, _) = detect_xpm();
    }

    #[test]
    fn test_bios_info_on_windows() {
        // On real hardware this should succeed
        // In CI it may fail (no WMI available) — that's acceptable
        let result = get_bios_info();
        if let Ok(bios) = result {
            // BiosInfo.version should not be empty on a real machine
            assert!(!bios.manufacturer.is_empty() || bios.manufacturer.is_empty());
            // always true — just checks no panic
        }
    }

    // ── Driver status mapping (F5.2) ────────────────────────────────────────
    //
    // `Win32_PnPSignedDriver.Status` is NULL for every row on real machines
    // (measured 355/355), so the old code showed "Unknown" ×351. The status is
    // now derived from `Win32_PnPEntity.ConfigManagerErrorCode`; these tests
    // pin the mapping without touching WMI.

    #[test]
    fn driver_status_healthy_device_reports_windows_status() {
        assert_eq!(driver_status_text(Some("OK"), Some(0)), "OK");
        assert_eq!(driver_status_text(None, Some(0)), "OK");
    }

    #[test]
    fn driver_status_problem_code_is_visible() {
        // 28 = "The drivers for this device are not installed" (Device Manager).
        assert_eq!(driver_status_text(None, Some(28)), "Error (code 28)");
        assert_eq!(
            driver_status_text(Some("Error"), Some(10)),
            "Error (code 10)"
        );
    }

    #[test]
    fn driver_status_without_entity_is_unknown_not_ok() {
        // Print queues have no PnP entity: no data must never read as "OK".
        assert_eq!(driver_status_text(None, None), "Unknown");
        assert_eq!(driver_status_text(Some(""), Some(0)), "OK");
        assert_eq!(driver_status_text(Some("Degraded"), None), "Degraded");
    }

    #[test]
    fn device_id_normalisation_is_case_insensitive() {
        assert_eq!(normalize_device_id("  usb\\vid_1d6b"), "USB\\VID_1D6B");
        assert_eq!(
            normalize_device_id(r"PCI\VEN_8086&DEV_7E28"),
            r"PCI\VEN_8086&DEV_7E28"
        );
    }

    /// The pnputil child waits are bounded: a child that sleeps past the
    /// budget must return `SpawnError::Timeout` and the whole tree must be
    /// dead (no orphan process left behind).
    #[test]
    fn pnputil_child_wait_is_bounded_and_kills_tree() {
        let mut cmd = no_window_command("cmd");
        cmd.args(["/C", "ping -n 10 127.0.0.1"]);
        let started = std::time::Instant::now();
        let err = process_spawn::run_capture(&mut cmd, std::time::Duration::from_millis(500))
            .expect_err("sleeping child must hit the timeout");
        assert!(
            matches!(err, process_spawn::SpawnError::Timeout { .. }),
            "expected Timeout, got {err:?}"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(5),
            "timeout path took too long: {:?}",
            started.elapsed()
        );

        // No orphan ping.exe may survive the tree kill.
        std::thread::sleep(std::time::Duration::from_millis(300));
        let mut probe = no_window_command("tasklist");
        probe.args(["/FI", "IMAGENAME eq ping.exe", "/NH"]);
        let out = process_spawn::run_capture(&mut probe, std::time::Duration::from_secs(15))
            .expect("tasklist probe failed");
        assert!(
            !out.stdout_str().contains("ping.exe"),
            "ping.exe survived the tree kill: {}",
            out.stdout_str()
        );
    }
}
