/// Hardware Discovery — Phase 10
///
/// On first launch (or when the profile is stale / missing), this module
/// probes the system to find every hardware path that would otherwise be
/// hardcoded.  Results are written to %APPDATA%\MiControl\hardware_profile.json
/// and re-loaded on subsequent starts so the scan happens at most once a week.
///
/// Other hw modules call `global_profile()` to read the discovered paths.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{OnceLock, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(windows)]
use {
    std::ffi::OsStr,
    std::os::windows::ffi::OsStrExt,
    windows::{
        core::GUID,
        Win32::{
            Devices::{
                DeviceAndDriverInstallation::{
                    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces,
                    SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE,
                    DIGCF_PRESENT, SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
                },
                HumanInterfaceDevice::{
                    HidD_FreePreparsedData, HidD_GetPreparsedData, HidP_GetCaps, HIDP_CAPS,
                    PHIDP_PREPARSED_DATA,
                },
            },
            Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
            Storage::FileSystem::{
                CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE,
                OPEN_EXISTING,
            },
        },
    },
};

// ── Global singleton ─────────────────────────────────────────────────────────

static PROFILE: OnceLock<RwLock<HardwareProfile>> = OnceLock::new();

/// Access the global hardware profile.
/// Returns `None` only if `init()` has not been called yet.
pub fn global_profile() -> Option<HardwareProfile> {
    let lock = PROFILE.get()?;
    Some(lock.read().ok()?.clone())
}

/// Update the in-process global profile snapshot.
fn set_global_profile(profile: HardwareProfile) {
    if let Some(lock) = PROFILE.get() {
        if let Ok(mut guard) = lock.write() {
            *guard = profile;
        }
    } else {
        let _ = PROFILE.set(RwLock::new(profile));
    }
}

/// Call once from `lib.rs` setup().  Loads cached profile or runs discovery.
pub fn init(app_data_dir: Option<PathBuf>) {
    let profile = load_or_discover(app_data_dir.as_deref());
    set_global_profile(profile);
}

// ── Data structures ───────────────────────────────────────────────────────────

/// A driver that is expected but not currently installed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissingDriver {
    /// Short identifier, e.g. "VirtualControlHID"
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// Absolute path to the bundled .inf in the app's resources directory
    pub bundled_inf: Option<String>,
}

/// Derived set of boolean capability flags — what this hardware can actually do.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HardwareCapabilities {
    /// VHF performance device found — performance modes via DeviceIoControl
    pub has_vhf_performance: bool,
    /// Touchpad vendor HID channel found — haptics / sensitivity via HID
    pub has_touchpad_hid: bool,
    /// Touchscreen digitizer found (UsagePage=0x000D, Usage=0x0004)
    pub has_touchscreen: bool,
    /// Stylus / pen digitizer found (UsagePage=0x000D, Usage=0x0002/0x0022)
    pub has_stylus: bool,
    /// Intel IGCL (ControlLib.dll) available — AI brightness, advanced display
    pub has_igcl: bool,
    /// IoT charging service found — threshold control via IPC pipe
    pub has_iot_charging: bool,
    /// HKLM\SOFTWARE\MI present — Xiaomi-specific registry features
    pub has_mi_registry: bool,
}

/// Full snapshot of discovered hardware paths and capabilities.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HardwareProfile {
    /// Unix timestamp (seconds) of when discovery ran
    pub discovered_at: u64,
    /// WMI Win32_ComputerSystem.Model, e.g. "Xiaomi Book Pro 14 2026"
    pub device_model: Option<String>,
    /// VHF device interface path for DeviceIoControl (performance mode)
    pub vhf_device_path: Option<String>,
    /// Touchpad HID output-report interface path (vendor channel, 33 bytes)
    pub touchpad_hid_path: Option<String>,
    /// Touchscreen digitizer HID path
    pub touchscreen_hid_path: Option<String>,
    /// Stylus/pen digitizer HID path
    pub stylus_hid_path: Option<String>,
    /// IoTService named pipe path
    pub iot_pipe_path: Option<String>,
    /// Windows service name for IoTService ("IoTSvc" or similar)
    pub iot_service_name: Option<String>,
    /// Absolute path to Intel ControlLib.dll
    pub igcl_dll_path: Option<String>,
    /// Whether HKLM\SOFTWARE\MI exists on this machine
    pub mi_registry_present: bool,
    /// Drivers that need to be installed
    pub missing_drivers: Vec<MissingDriver>,
    /// Derived capability flags (computed from the fields above)
    pub capabilities: HardwareCapabilities,
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Force a fresh discovery, update the cache file, and return the result.
/// Also updates the in-process `PROFILE` so new paths/capabilities take effect
/// immediately without requiring app restart.
pub fn rediscover(app_data_dir: Option<PathBuf>) -> HardwareProfile {
    let profile = run_discovery();
    save_profile(&profile, app_data_dir.as_deref());
    set_global_profile(profile.clone());
    profile
}

/// Try to install a bundled driver using `pnputil /add-driver … /install`.
/// Requires administrator rights — returns a descriptive error if access is denied.
pub fn install_driver(inf_path: &str) -> Result<String> {
    let output = no_window_command("pnputil")
        .args(["/add-driver", inf_path, "/install"])
        .output()?;
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if output.status.success() {
        Ok(format!("Driver installed successfully.\n{stdout}"))
    } else if stderr.contains("Access") || stdout.contains("Access") {
        anyhow::bail!(
            "Administrator rights required to install drivers. \
             Please restart MiControl as Administrator."
        )
    } else {
        anyhow::bail!("pnputil failed: {stderr}{stdout}")
    }
}

/// Resolve a bundled driver `.inf` path by logical driver name.
///
/// Security checks:
/// - `driver_name` must be a simple token (no path separators / traversal chars)
/// - resolved `.inf` must exist
/// - canonicalized `.inf` path must remain inside the app resources directory
pub fn resolve_bundled_inf_by_name(driver_name: &str) -> Result<String> {
    let name = driver_name.trim();
    anyhow::ensure!(!name.is_empty(), "Driver name cannot be empty");
    let has_forbidden = name.chars().any(|c| c == '\\' || c == '/' || c == ':');
    anyhow::ensure!(
        !has_forbidden && !name.contains(".."),
        "Invalid driver name"
    );

    let resources = resources_dir();
    let resources_canon = std::fs::canonicalize(&resources)
        .with_context(|| format!("Cannot canonicalize resources dir: {}", resources.display()))?;

    let mut candidates: Vec<PathBuf> = Vec::new();

    // Known bundled drivers (default package layout).
    for (known_name, rel_inf) in [
        (
            "VirtualControlHID",
            "drivers/VirtualControlHID/virtualcontrolhid.inf",
        ),
        ("IoTDriver", "drivers/IoTDriver/iotdriver.inf"),
    ] {
        if known_name.eq_ignore_ascii_case(name) {
            candidates.push(resources.join(rel_inf));
            candidates.push(resources.join(format!("drivers/{name}/{name}.inf")));
            candidates.push(resources.join(format!("drivers/{name}/driver.inf")));
        }
    }

    // Discovery profile may provide additional bundled paths.
    if let Some(profile) = global_profile() {
        for missing in &profile.missing_drivers {
            if missing.name.eq_ignore_ascii_case(name) {
                if let Some(inf) = &missing.bundled_inf {
                    candidates.push(PathBuf::from(inf));
                }
            }
        }
    }

    anyhow::ensure!(
        !candidates.is_empty(),
        "Bundled .inf for driver '{}' is not registered.",
        name
    );

    for candidate in candidates {
        if !candidate.exists() {
            continue;
        }
        let canon = match std::fs::canonicalize(&candidate) {
            Ok(p) => p,
            Err(_) => continue,
        };
        if canon.starts_with(&resources_canon) {
            return Ok(canon.to_string_lossy().to_string());
        }
    }

    anyhow::bail!(
        "Bundled .inf for driver '{}' not found or outside resources directory.",
        name
    );
}

/// Locate the app's resources directory (works in both dev and installed modes).
pub fn resources_dir() -> PathBuf {
    // Installed: <exe_dir>\resources\
    if let Ok(exe) = std::env::current_exe() {
        let candidate = exe.parent().unwrap_or(Path::new(".")).join("resources");
        if candidate.exists() {
            return candidate;
        }
    }
    // Dev (cargo tauri dev from src-tauri/): ../resources/
    let dev = PathBuf::from("..").join("resources");
    if dev.exists() {
        return dev;
    }
    PathBuf::from("resources")
}

// ── Profile persistence ───────────────────────────────────────────────────────

fn load_or_discover(data_dir: Option<&Path>) -> HardwareProfile {
    if let Some(path) = profile_file_path(data_dir) {
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(p) = serde_json::from_str::<HardwareProfile>(&content) {
                    if !is_stale(p.discovered_at, 7) {
                        #[cfg(windows)]
                        {
                            if let Some(tp) = p.touchpad_hid_path.as_deref() {
                                if !is_touchpad_vendor_channel_path(tp) {
                                    log::warn!(
                                        "Cached touchpad_hid_path is no longer valid ({}). Re-discovering...",
                                        tp
                                    );
                                    let profile = run_discovery();
                                    save_profile(&profile, data_dir);
                                    return profile;
                                }
                            }
                        }
                        log::info!("Hardware profile loaded from {}", path.display());
                        return p;
                    }
                    log::info!("Hardware profile is stale, re-discovering...");
                }
            }
        }
    }
    let profile = run_discovery();
    save_profile(&profile, data_dir);
    profile
}

fn save_profile(profile: &HardwareProfile, data_dir: Option<&Path>) {
    if let Some(path) = profile_file_path(data_dir) {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(profile) {
            match std::fs::write(&path, json) {
                Ok(()) => log::info!("Hardware profile saved to {}", path.display()),
                Err(e) => log::warn!("Could not save hardware profile: {e}"),
            }
        }
    }
}

fn profile_file_path(data_dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(d) = data_dir {
        return Some(d.join("hardware_profile.json"));
    }
    if let Ok(appdata) = std::env::var("APPDATA") {
        return Some(
            PathBuf::from(appdata)
                .join("MiControl")
                .join("hardware_profile.json"),
        );
    }
    None
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn is_stale(timestamp: u64, max_age_days: u64) -> bool {
    now_unix().saturating_sub(timestamp) > max_age_days * 86_400
}

// ── Core discovery ────────────────────────────────────────────────────────────

fn run_discovery() -> HardwareProfile {
    log::info!("=== Hardware discovery started ===");
    let mut p = HardwareProfile::default();
    p.discovered_at = now_unix();

    p.device_model = probe_device_model();
    p.vhf_device_path = probe_vhf_device();
    p.touchpad_hid_path = probe_touchpad_hid();
    p.touchscreen_hid_path = probe_touchscreen_hid();
    p.stylus_hid_path = probe_stylus_hid();
    p.iot_pipe_path = probe_iot_pipe();
    p.iot_service_name = probe_iot_service();
    p.igcl_dll_path = probe_igcl_dll();
    p.mi_registry_present = probe_mi_registry();
    p.missing_drivers = probe_missing_drivers();

    // Derive capability flags from everything we found
    p.capabilities = HardwareCapabilities {
        has_vhf_performance: p.vhf_device_path.is_some(),
        has_touchpad_hid: p.touchpad_hid_path.is_some(),
        has_touchscreen: p.touchscreen_hid_path.is_some(),
        has_stylus: p.stylus_hid_path.is_some(),
        has_igcl: p.igcl_dll_path.is_some(),
        has_iot_charging: p.iot_service_name.is_some() || p.iot_pipe_path.is_some(),
        has_mi_registry: p.mi_registry_present,
    };

    log::info!(
        "Discovery complete: vhf={} touchpad={} touchscreen={} stylus={} igcl={} iot={} missing={}",
        p.capabilities.has_vhf_performance,
        p.capabilities.has_touchpad_hid,
        p.capabilities.has_touchscreen,
        p.capabilities.has_stylus,
        p.capabilities.has_igcl,
        p.capabilities.has_iot_charging,
        p.missing_drivers.len()
    );
    p
}

// ── Individual probes ─────────────────────────────────────────────────────────

fn probe_device_model() -> Option<String> {
    #[cfg(windows)]
    {
        use crate::hw::wmi_cache;
        wmi_cache::with_cimv2(|wmi| {
            let results: Vec<HashMap<String, wmi::Variant>> = wmi
                .raw_query("SELECT Model FROM Win32_ComputerSystem")
                .unwrap_or_default();
            if let Some(row) = results.into_iter().next() {
                if let Some(wmi::Variant::String(model)) = row.get("Model") {
                    return Ok(Some(model.clone()));
                }
            }
            Ok(None::<String>)
        })
        .ok()?
    }
    #[cfg(not(windows))]
    None
}

/// Enumerate the VHF custom interface GUID and return the first device path.
fn probe_vhf_device() -> Option<String> {
    #[cfg(windows)]
    {
        let guid = GUID {
            data1: 0x0CC99493,
            data2: 0xEB87,
            data3: 0x54F5,
            data4: [0xBB, 0x10, 0xC0, 0xD5, 0xEA, 0x4A, 0x4F, 0x4C],
        };
        return enumerate_device_interfaces(&guid).into_iter().next();
    }
    #[cfg(not(windows))]
    None
}

/// Enumerate standard HID devices and find the vendor-defined output-report
/// interface (Usage Page 0xFF00, OutputReportByteLength > 0) — the touchpad's
/// custom control channel.  Path is identified by HID caps, not by chip name,
/// so this works across BLTP7853, Elan, Synaptics, etc.
fn probe_touchpad_hid() -> Option<String> {
    #[cfg(windows)]
    {
        let hid_entries = enumerate_hid_paths_with_caps();
        let touchpad_roots: std::collections::HashSet<String> = hid_entries
            .iter()
            .filter(|entry| entry.caps.UsagePage == 0x000D && entry.caps.Usage == 0x0005)
            .filter_map(|entry| hid_instance_root(&entry.path))
            .collect();

        // Primary strategy: choose a vendor-defined output-report HID whose
        // instance root matches an actual touchpad digitizer collection.
        let mut candidates: Vec<(i32, String, u16, u16)> = hid_entries
            .iter()
            .filter(|entry| entry.caps.UsagePage >= 0xFF00 && entry.caps.OutputReportByteLength > 0)
            .filter_map(|entry| {
                let lower = entry.path.to_ascii_lowercase();
                if lower.contains("kbd")
                    || lower.contains("keyboard")
                    || lower.contains("mouse")
                    || lower.contains("col01")
                {
                    return None;
                }
                let root = hid_instance_root(&entry.path)?;
                if !touchpad_roots.contains(&root) {
                    return None;
                }
                let mut score = 0;
                if lower.contains("&col04#") {
                    score += 30;
                } else if lower.contains("&col05#") {
                    score += 20;
                }
                if lower.contains("bltp") {
                    score += 10;
                }
                score += entry.caps.OutputReportByteLength as i32;
                Some((
                    score,
                    entry.path.clone(),
                    entry.caps.UsagePage,
                    entry.caps.OutputReportByteLength,
                ))
            })
            .collect();

        candidates.sort_by(|a, b| b.0.cmp(&a.0));
        if let Some((_, path, usage_page, output_len)) = candidates.into_iter().next() {
            log::info!(
                "Touchpad HID found (touchpad-root matched): {} (UsagePage={:#X} OutputLen={})",
                path,
                usage_page,
                output_len
            );
            return Some(path);
        }

        // Fallback: keep old behavior when we could not correlate by root.
        for entry in &hid_entries {
            let path_lower = entry.path.to_ascii_lowercase();
            if path_lower.contains("kbd")
                || path_lower.contains("keyboard")
                || path_lower.contains("mouse")
                || path_lower.contains("col01")
            {
                continue;
            }
            if entry.caps.UsagePage >= 0xFF00 && entry.caps.OutputReportByteLength > 0 {
                log::info!(
                    "Touchpad HID found (fallback): {} (UsagePage={:#X} OutputLen={})",
                    entry.path,
                    entry.caps.UsagePage,
                    entry.caps.OutputReportByteLength
                );
                return Some(entry.path.clone());
            }
        }
        log::warn!("Touchpad vendor HID channel not found during discovery");
    }
    None
}

/// Probe for a touchscreen digitizer (HID UsagePage=0x000D, Usage=0x0004).
fn probe_touchscreen_hid() -> Option<String> {
    #[cfg(windows)]
    {
        let hid_guid = GUID {
            data1: 0x4D1E55B2,
            data2: 0xF16F,
            data3: 0x11CF,
            data4: [0x88, 0xCB, 0x00, 0x11, 0x11, 0x00, 0x00, 0x30],
        };
        let paths = enumerate_device_interfaces(&hid_guid);
        for path in &paths {
            if let Some(caps) = hid_caps_for_path(path) {
                // HID Digitizer (0x000D) — Touch Screen (0x0004)
                if caps.UsagePage == 0x000D && caps.Usage == 0x0004 {
                    log::info!("Touchscreen HID found: {}", path);
                    return Some(path.clone());
                }
            }
        }
        log::info!("Touchscreen HID not found (device may not have a touchscreen)");
    }
    None
}

/// Probe for a stylus / pen digitizer (HID UsagePage=0x000D, Usage=0x0002 or 0x0022).
fn probe_stylus_hid() -> Option<String> {
    #[cfg(windows)]
    {
        let hid_guid = GUID {
            data1: 0x4D1E55B2,
            data2: 0xF16F,
            data3: 0x11CF,
            data4: [0x88, 0xCB, 0x00, 0x11, 0x11, 0x00, 0x00, 0x30],
        };
        let paths = enumerate_device_interfaces(&hid_guid);
        for path in &paths {
            if let Some(caps) = hid_caps_for_path(path) {
                // HID Digitizer (0x000D) — Pen (0x0002) or Integrated Pen / Tablet PC (0x0022)
                if caps.UsagePage == 0x000D && (caps.Usage == 0x0002 || caps.Usage == 0x0022) {
                    log::info!("Stylus HID found: {}", path);
                    return Some(path.clone());
                }
            }
        }
        log::info!("Stylus HID not found (device may not have a pen)");
    }
    None
}

/// Try to open each known IoT pipe path and return the first that connects.
fn probe_iot_pipe() -> Option<String> {
    let candidates = [
        r"\\.\pipe\LOCAL\IoTService_IPC_Broker",
        r"\\.\pipe\IoTService",
        r"\\.\pipe\LOCAL\IoTDriver",
    ];
    for path in &candidates {
        if std::fs::metadata(path).is_ok() {
            log::info!("IoT pipe found: {}", path);
            return Some(path.to_string());
        }
    }
    log::warn!("IoT pipe not reachable — IoTSvc may not be running");
    None
}

/// Check which Windows service name IoTService is registered under.
fn probe_iot_service() -> Option<String> {
    let candidates = ["IoTSvc", "IoTDriver", "XiaomiIoT", "MiIoT"];
    for name in &candidates {
        if service_exists(name) {
            log::info!("IoT service found: {}", name);
            return Some(name.to_string());
        }
    }
    None
}

/// Find Intel ControlLib.dll (IGCL).
fn probe_igcl_dll() -> Option<String> {
    let candidates = [
        r"C:\Windows\System32\ControlLib.dll",
        r"C:\Windows\SysWOW64\ControlLib.dll",
    ];
    for path in &candidates {
        if std::path::Path::new(path).exists() {
            log::info!("IGCL DLL found: {}", path);
            return Some(path.to_string());
        }
    }
    None
}

/// Check whether HKLM\SOFTWARE\MI exists.
fn probe_mi_registry() -> bool {
    #[cfg(windows)]
    {
        use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};
        let hklm = RegKey::predef(HKEY_LOCAL_MACHINE);
        return hklm.open_subkey(r"SOFTWARE\MI").is_ok();
    }
    #[cfg(not(windows))]
    false
}

/// Use pnputil to see which of our required drivers are not installed.
fn probe_missing_drivers() -> Vec<MissingDriver> {
    let mut missing = Vec::new();

    let required = [
        (
            "VirtualControlHID",
            "Performance mode & HID control driver",
            "drivers/VirtualControlHID/virtualcontrolhid.inf",
        ),
        (
            "IoTDriver",
            "Battery charging threshold driver",
            "drivers/IoTDriver/iotdriver.inf",
        ),
    ];

    let installed_providers = installed_driver_providers();

    for (name, desc, inf_relative) in &required {
        let is_present = installed_providers
            .iter()
            .any(|p| p.to_lowercase().contains(&name.to_lowercase()));

        if !is_present {
            let bundled = resources_dir().join(inf_relative);
            missing.push(MissingDriver {
                name: name.to_string(),
                description: desc.to_string(),
                bundled_inf: if bundled.exists() {
                    Some(bundled.to_string_lossy().into_owned())
                } else {
                    None
                },
            });
            log::info!("Missing driver detected: {}", name);
        }
    }
    missing
}

/// Return a list of all published driver original names from pnputil.
fn installed_driver_providers() -> Vec<String> {
    let output = no_window_command("pnputil")
        .args(["/enum-drivers"])
        .output();
    match output {
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout);
            let mut names = Vec::new();
            for line in text.lines() {
                let line = line.trim();
                if let Some((key, val)) = line.split_once(':') {
                    let key = key.trim().to_lowercase();
                    if key == "original name" || key == "provider name" {
                        names.push(val.trim().to_string());
                    }
                }
            }
            names
        }
        Err(e) => {
            log::warn!("pnputil /enum-drivers failed: {e}");
            Vec::new()
        }
    }
}

// ── Windows helpers ───────────────────────────────────────────────────────────

#[cfg(windows)]
fn enumerate_device_interfaces(guid: &GUID) -> Vec<String> {
    let mut paths = Vec::new();
    unsafe {
        let Ok(dev_info) = SetupDiGetClassDevsW(
            Some(guid),
            None,
            None,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        ) else {
            return paths;
        };

        let mut idx = 0u32;
        loop {
            let mut iface = SP_DEVICE_INTERFACE_DATA {
                cbSize: std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
                ..std::mem::zeroed()
            };
            if SetupDiEnumDeviceInterfaces(dev_info, None, guid, idx, &mut iface).is_err() {
                break;
            }
            // First call: get required buffer size
            let mut required = 0u32;
            let _ = SetupDiGetDeviceInterfaceDetailW(
                dev_info,
                &iface,
                None,
                0,
                Some(&mut required),
                None,
            );
            if required > 0 && required <= 2048 {
                // Allocate a byte buffer large enough for the struct + path string
                let mut buf = vec![0u8; required as usize];
                let detail_ptr = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
                // cbSize must be set to the struct header size (not total buffer)
                (*detail_ptr).cbSize =
                    std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;
                if SetupDiGetDeviceInterfaceDetailW(
                    dev_info,
                    &iface,
                    Some(detail_ptr),
                    required,
                    None,
                    None,
                )
                .is_ok()
                {
                    // The device path is a null-terminated UTF-16 array after cbSize
                    let path_start = 4usize; // offset of DevicePath field (u32 cbSize = 4 bytes)
                    let wide_slice = std::slice::from_raw_parts(
                        buf.as_ptr().add(path_start) as *const u16,
                        (required as usize - path_start) / 2,
                    );
                    let null_pos = wide_slice
                        .iter()
                        .position(|&c| c == 0)
                        .unwrap_or(wide_slice.len());
                    if let Ok(s) = String::from_utf16(&wide_slice[..null_pos]) {
                        if !s.is_empty() {
                            paths.push(s);
                        }
                    }
                }
            }
            idx += 1;
        }
        let _ = SetupDiDestroyDeviceInfoList(dev_info);
    }
    paths
}

/// Open a HID device (read-only, shared) and return its capability snapshot.
#[cfg(windows)]
fn hid_caps_for_path(path: &str) -> Option<HIDP_CAPS> {
    unsafe {
        let path_w: Vec<u16> = OsStr::new(path).encode_wide().chain(Some(0)).collect();
        let handle = CreateFileW(
            windows::core::PCWSTR(path_w.as_ptr()),
            0, // no access flags — just open for caps query
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            HANDLE::default(),
        )
        .ok()?;

        if handle == INVALID_HANDLE_VALUE {
            return None;
        }

        let mut ppd = PHIDP_PREPARSED_DATA::default();
        let ok = HidD_GetPreparsedData(handle, &mut ppd);
        CloseHandle(handle).ok();
        if !ok.as_bool() || ppd.0 == 0 {
            return None;
        }

        let mut caps = HIDP_CAPS::default();
        let status = HidP_GetCaps(ppd, &mut caps);
        HidD_FreePreparsedData(ppd);

        // HIDP_STATUS_SUCCESS = 0x00110000
        if status.0 == 0x00110000_u32 as i32 {
            Some(caps)
        } else {
            None
        }
    }
}

/// Check if a Windows service with the given name exists.
fn service_exists(name: &str) -> bool {
    #[cfg(windows)]
    {
        use windows::Win32::System::Services::{
            CloseServiceHandle, OpenSCManagerW, OpenServiceW, SC_MANAGER_CONNECT,
            SERVICE_QUERY_STATUS,
        };
        unsafe {
            let Ok(scm) = OpenSCManagerW(None, None, SC_MANAGER_CONNECT) else {
                return false;
            };
            let name_w: Vec<u16> = OsStr::new(name).encode_wide().chain(Some(0)).collect();
            let svc = OpenServiceW(
                scm,
                windows::core::PCWSTR(name_w.as_ptr()),
                SERVICE_QUERY_STATUS,
            );
            let found = svc.is_ok();
            if let Ok(h) = svc {
                let _ = CloseServiceHandle(h);
            }
            let _ = CloseServiceHandle(scm);
            return found;
        }
    }
    #[cfg(not(windows))]
    false
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

#[cfg(windows)]
#[derive(Clone)]
struct HidPathCaps {
    path: String,
    caps: HIDP_CAPS,
}

#[cfg(windows)]
fn enumerate_hid_paths_with_caps() -> Vec<HidPathCaps> {
    let hid_guid = GUID {
        data1: 0x4D1E55B2,
        data2: 0xF16F,
        data3: 0x11CF,
        data4: [0x88, 0xCB, 0x00, 0x11, 0x11, 0x00, 0x00, 0x30],
    };
    enumerate_device_interfaces(&hid_guid)
        .into_iter()
        .filter_map(|path| hid_caps_for_path(&path).map(|caps| HidPathCaps { path, caps }))
        .collect()
}

#[cfg(windows)]
fn hid_instance_root(path: &str) -> Option<String> {
    let lower = path.to_ascii_lowercase();
    let mut parts = lower.split('#');
    let _prefix = parts.next()?;
    let _hardware = parts.next()?;
    let instance = parts.next()?.to_string();
    if instance.contains("&0&") {
        if let Some((root, _tail)) = instance.rsplit_once('&') {
            return Some(root.to_string());
        }
    }
    Some(instance)
}

#[cfg(windows)]
fn is_touchpad_vendor_channel_path(path: &str) -> bool {
    let target = path.to_ascii_lowercase();
    let entries = enumerate_hid_paths_with_caps();
    let touchpad_roots: std::collections::HashSet<String> = entries
        .iter()
        .filter(|entry| entry.caps.UsagePage == 0x000D && entry.caps.Usage == 0x0005)
        .filter_map(|entry| hid_instance_root(&entry.path))
        .collect();

    entries.iter().any(|entry| {
        if entry.path.to_ascii_lowercase() != target {
            return false;
        }
        if !(entry.caps.UsagePage >= 0xFF00 && entry.caps.OutputReportByteLength > 0) {
            return false;
        }
        if let Some(root) = hid_instance_root(&entry.path) {
            return touchpad_roots.contains(&root);
        }
        false
    })
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_stale_old_timestamp() {
        // A timestamp from 10 days ago should be stale with max_age=7
        let old = now_unix().saturating_sub(10 * 86_400);
        assert!(is_stale(old, 7));
    }

    #[test]
    fn test_is_stale_fresh_timestamp() {
        // A timestamp from 1 day ago should NOT be stale with max_age=7
        let fresh = now_unix().saturating_sub(86_400);
        assert!(!is_stale(fresh, 7));
    }

    #[test]
    fn test_resources_dir_does_not_panic() {
        let _ = resources_dir(); // just verify no panic
    }

    #[test]
    fn test_probe_igcl_returns_path_or_none() {
        let result = probe_igcl_dll();
        // On the dev machine ControlLib.dll should exist
        // In CI it may not — either way it must not panic
        let _ = result;
    }

    #[test]
    fn test_probe_mi_registry() {
        // On the Xiaomi Book this should be true; elsewhere it may be false
        let _ = probe_mi_registry();
    }

    #[test]
    fn test_profile_round_trip() {
        let p = HardwareProfile {
            discovered_at: 1_700_000_000,
            device_model: Some("Test Device".into()),
            vhf_device_path: Some(r"\\?\some\path".into()),
            ..Default::default()
        };
        let json = serde_json::to_string(&p).unwrap();
        let p2: HardwareProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(p2.device_model, Some("Test Device".into()));
        assert_eq!(p2.vhf_device_path, Some(r"\\?\some\path".into()));
    }

    #[cfg(windows)]
    #[test]
    fn test_hid_instance_root_parses_expected_segment() {
        let p = r"\\?\hid#vid_3151&pid_8888&mi_01&col05#7&5a6d3c2&0&0004#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        assert_eq!(hid_instance_root(p).as_deref(), Some("7&5a6d3c2&0"));
    }
}
