use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FanInfo {
    pub mode: FanMode,
    /// Fan RPM from Win32_Fan.CurrentReading — 0 when the EC does not expose
    /// RPM via WMI (common on Xiaomi Book Pro 14 and similar Intel platforms).
    pub speed_rpm: u32,
    pub speed_percent: u8,
    pub gpu_temp_celsius: f32,
    pub cpu_temp_celsius: f32,
    /// System package power from the RAPL/ACPI Power Meter PDH counter
    /// (\Power Meter(_Total)\Power), in watts.  None until the background
    /// poller has completed its first successful sample (~1.5 s after launch).
    pub tdp_watts: Option<f32>,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FanMode {
    Auto,
    Fixed,
    Off,
}

const FAN_REG_KEY: &str = r"SOFTWARE\MI\FanControl";
const FAN_REG_MODE: &str = "FanMode";
const FAN_REG_SPEED: &str = "FixedSpeed";

// ── TDP — RAPL via PDH \Power Meter(_Total)\Power ────────────────────────
//
// Win32_Fan does NOT expose CurrentReading on this hardware (Xiaomi Book Pro 14).
// CPU load comes from Win32_PerfFormattedData_PerfOS_Processor in system_info.rs
// and should be read from SystemInfo in the frontend to avoid duplication.
// TDP is from the Windows ACPI Power Meter interface (RAPL), updated every ~1.5 s.

static TDP_WATTS_CACHE: OnceLock<Mutex<Option<f32>>> = OnceLock::new();
static TDP_POLLER_STARTED: OnceLock<()> = OnceLock::new();

fn ensure_tdp_poller() {
    TDP_WATTS_CACHE.get_or_init(|| Mutex::new(None));
    TDP_POLLER_STARTED.get_or_init(|| {
        std::thread::Builder::new()
            .name("tdp-pdh-poller".into())
            .spawn(tdp_pdh_poller_thread)
            .ok();
    });
}

fn read_tdp_watts() -> Option<f32> {
    TDP_WATTS_CACHE
        .get()
        .and_then(|m| m.lock().ok())
        .and_then(|g| *g)
}

fn tdp_pdh_poller_thread() {
    #[cfg(windows)]
    unsafe {
        use libloading::{Library, Symbol};
        type FnOpenQuery  = unsafe extern "system" fn(*const std::ffi::c_void, usize, *mut isize) -> u32;
        type FnAddCounter = unsafe extern "system" fn(isize, *const u16, usize, *mut isize) -> u32;
        type FnCollect    = unsafe extern "system" fn(isize) -> u32;
        type FnGetValue   = unsafe extern "system" fn(isize, u32, *mut u32, *mut u8) -> u32;
        type FnClose      = unsafe extern "system" fn(isize) -> u32;

        let lib: &'static Library = match Library::new("pdh.dll") {
            Ok(l) => Box::leak(Box::new(l)),
            Err(_) => return,
        };
        let open_q:  Symbol<'static, FnOpenQuery>  = match lib.get(b"PdhOpenQueryW\0")               { Ok(f) => f, Err(_) => return };
        let add_c:   Symbol<'static, FnAddCounter> = match lib.get(b"PdhAddEnglishCounterW\0")       { Ok(f) => f, Err(_) => return };
        let collect: Symbol<'static, FnCollect>    = match lib.get(b"PdhCollectQueryData\0")         { Ok(f) => f, Err(_) => return };
        let get_val: Symbol<'static, FnGetValue>   = match lib.get(b"PdhGetFormattedCounterValue\0") { Ok(f) => f, Err(_) => return };
        let close_q: Symbol<'static, FnClose>      = match lib.get(b"PdhCloseQuery\0")               { Ok(f) => f, Err(_) => return };

        let mut query: isize = 0;
        if open_q(std::ptr::null(), 0, &mut query) != 0 { return; }

        // _Total aggregates all Power Meter instances (value is in milliwatts)
        let path: Vec<u16> = "\\Power Meter(_Total)\\Power\0".encode_utf16().collect();
        let mut counter: isize = 0;
        if add_c(query, path.as_ptr(), 0, &mut counter) != 0 {
            close_q(query);
            return;
        }

        collect(query); // baseline

        // PDH_FMT_COUNTERVALUE layout (x64):
        //   offset 0 : CStatus  u32  (4 bytes)
        //   offset 4 : padding       (4 bytes)
        //   offset 8 : doubleValue f64 (8 bytes)
        const PDH_FMT_DOUBLE: u32 = 0x00000200;
        let mut val_buf = [0u8; 16];
        let mut dummy_type: u32 = 0;

        loop {
            std::thread::sleep(std::time::Duration::from_millis(1500));
            collect(query);
            if get_val(counter, PDH_FMT_DOUBLE, &mut dummy_type, val_buf.as_mut_ptr()) != 0 { continue; }
            let c_status = u32::from_ne_bytes(val_buf[0..4].try_into().unwrap_or([1; 4]));
            if c_status > 1 { continue; }
            let milliwatts = f64::from_ne_bytes(val_buf[8..16].try_into().unwrap_or([0; 8]));
            if milliwatts.is_finite() && milliwatts > 0.0 {
                if let Some(cache) = TDP_WATTS_CACHE.get() {
                    if let Ok(mut g) = cache.lock() { *g = Some((milliwatts / 1000.0) as f32); }
                }
            }
        }
    }
}

pub fn get_fan_info() -> Result<FanInfo> {
    ensure_tdp_poller();
    let speed_rpm = get_fan_rpm_wmi().unwrap_or(0);
    let gpu_temp = get_gpu_temp_wmi().unwrap_or(45.0);
    let cpu_temp = get_cpu_temp_wmi().unwrap_or(50.0);
    let (mode, speed_percent) = get_fan_mode_registry().unwrap_or((FanMode::Auto, 50));

    // Estimate speed percent from rpm (max ~5000 rpm for this model)
    let speed_percent_actual = if speed_rpm > 0 {
        ((speed_rpm as f32 / 5000.0) * 100.0).clamp(0.0, 100.0) as u8
    } else {
        speed_percent
    };

    Ok(FanInfo {
        mode,
        speed_rpm,
        speed_percent: speed_percent_actual,
        gpu_temp_celsius: gpu_temp,
        cpu_temp_celsius: cpu_temp,
        tdp_watts: read_tdp_watts(),
    })
}

pub fn set_fan_mode(mode: FanMode, speed_percent: u8) -> Result<()> {
    persist_fan_registry(&mode, speed_percent)?;

    match mode {
        FanMode::Auto => set_fan_auto_igcl().unwrap_or_else(|e| log::warn!("IGCL fan auto: {e}")),
        FanMode::Fixed => set_fan_fixed_igcl(speed_percent).unwrap_or_else(|e| log::warn!("IGCL fan fixed: {e}")),
        FanMode::Off => log::warn!("Fan off mode not directly supported — using minimum speed"),
    }
    Ok(())
}

// ── WMI helpers ─────────────────────────────────────────────────────────────

fn get_fan_rpm_wmi() -> Result<u32> {
    #[cfg(windows)]
    {
        use wmi::{COMLibrary, WMIConnection};
        use std::collections::HashMap;
        let com = COMLibrary::new().context("COM")?;
        let wmi = WMIConnection::new(com.into()).context("WMI")?;
        let results: Vec<HashMap<String, wmi::Variant>> = wmi
            .raw_query("SELECT CurrentReading FROM Win32_Fan")
            .unwrap_or_default();
        if let Some(row) = results.first() {
            match row.get("CurrentReading") {
                Some(wmi::Variant::UI4(v)) => return Ok(*v),
                Some(wmi::Variant::I4(v)) => return Ok(*v as u32),
                _ => {}
            }
        }
        Ok(0)
    }
    #[cfg(not(windows))]
    { Ok(0) }
}

fn get_cpu_temp_wmi() -> Result<f32> {
    #[cfg(windows)]
    {
        use wmi::{COMLibrary, WMIConnection};
        use std::collections::HashMap;
        let com = COMLibrary::new().context("COM")?;
        let wmi = WMIConnection::with_namespace_path("ROOT\\WMI", com.into()).context("WMI")?;
        let results: Vec<HashMap<String, wmi::Variant>> = wmi
            .raw_query("SELECT CurrentTemperature FROM MSAcpi_ThermalZoneTemperature")
            .unwrap_or_default();
        // All thermal zones — take the maximum (CPU package is hottest on Intel)
        let max_temp = results.iter()
            .filter_map(|row| {
                match row.get("CurrentTemperature") {
                    Some(wmi::Variant::UI4(v)) => Some((*v as f32 / 10.0) - 273.15),
                    _ => None,
                }
            })
            .fold(f32::NEG_INFINITY, f32::max);
        if max_temp > f32::NEG_INFINITY {
            return Ok(max_temp.clamp(0.0, 120.0));
        }
        Ok(50.0)
    }
    #[cfg(not(windows))]
    { Ok(50.0) }
}

fn get_gpu_temp_wmi() -> Result<f32> {
    #[cfg(windows)]
    {
        use wmi::{COMLibrary, WMIConnection};
        use std::collections::HashMap;
        let com = COMLibrary::new().context("COM")?;
        let wmi = WMIConnection::with_namespace_path("ROOT\\WMI", com.into()).context("WMI")?;
        // Try MSAcpi_ThermalZoneTemperature first
        let results: Vec<HashMap<String, wmi::Variant>> = wmi
            .raw_query("SELECT CurrentTemperature FROM MSAcpi_ThermalZoneTemperature")
            .unwrap_or_default();
        if let Some(row) = results.first() {
            match row.get("CurrentTemperature") {
                Some(wmi::Variant::UI4(v)) => {
                    // Kelvin * 10 -> Celsius
                    return Ok((*v as f32 / 10.0) - 273.15);
                }
                _ => {}
            }
        }
        Ok(45.0)
    }
    #[cfg(not(windows))]
    { Ok(45.0) }
}

// ── Registry persistence ─────────────────────────────────────────────────────

fn persist_fan_registry(mode: &FanMode, speed_percent: u8) -> Result<()> {
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY_LOCAL_MACHINE, KEY_WRITE, REG_DWORD,
            REG_OPTION_NON_VOLATILE,
        };
        use windows::core::PCWSTR;
        unsafe {
            let key_w: Vec<u16> = OsStr::new(FAN_REG_KEY).encode_wide().chain(Some(0)).collect();
            let mut hkey = std::mem::zeroed();
            RegCreateKeyExW(
                HKEY_LOCAL_MACHINE, PCWSTR(key_w.as_ptr()), 0, None,
                REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut hkey, None,
            ).ok().context("Create fan reg key")?;

            let mode_val: u32 = match mode { FanMode::Auto => 0, FanMode::Fixed => 1, FanMode::Off => 2 };
            let mode_w: Vec<u16> = OsStr::new(FAN_REG_MODE).encode_wide().chain(Some(0)).collect();
            let _ = RegSetValueExW(hkey, PCWSTR(mode_w.as_ptr()), 0, REG_DWORD, Some(&mode_val.to_le_bytes())).ok();

            let speed_val = speed_percent as u32;
            let speed_w: Vec<u16> = OsStr::new(FAN_REG_SPEED).encode_wide().chain(Some(0)).collect();
            let _ = RegSetValueExW(hkey, PCWSTR(speed_w.as_ptr()), 0, REG_DWORD, Some(&speed_val.to_le_bytes())).ok();

            let _ = RegCloseKey(hkey).ok();
        }
    }
    Ok(())
}

fn get_fan_mode_registry() -> Result<(FanMode, u8)> {
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_LOCAL_MACHINE, REG_VALUE_TYPE,
        };
        use windows::core::PCWSTR;
        unsafe {
            let key_w: Vec<u16> = OsStr::new(FAN_REG_KEY).encode_wide().chain(Some(0)).collect();
            let mut hkey = std::mem::zeroed();
            if RegOpenKeyExW(HKEY_LOCAL_MACHINE, PCWSTR(key_w.as_ptr()), 0,
                windows::Win32::System::Registry::KEY_READ, &mut hkey).is_err() {
                return Ok((FanMode::Auto, 50));
            }

            let mut mode_val: u32 = 0;
            let mut size = 4u32;
            let mut ty = REG_VALUE_TYPE::default();
            let mode_w: Vec<u16> = OsStr::new(FAN_REG_MODE).encode_wide().chain(Some(0)).collect();
            let _ = RegQueryValueExW(hkey, PCWSTR(mode_w.as_ptr()), None, Some(&mut ty),
                Some((&mut mode_val as *mut u32).cast()), Some(&mut size));

            let mut speed_val: u32 = 50;
            let speed_w: Vec<u16> = OsStr::new(FAN_REG_SPEED).encode_wide().chain(Some(0)).collect();
            let _ = RegQueryValueExW(hkey, PCWSTR(speed_w.as_ptr()), None, Some(&mut ty),
                Some((&mut speed_val as *mut u32).cast()), Some(&mut size));

            let _ = RegCloseKey(hkey).ok();
            let mode = match mode_val { 1 => FanMode::Fixed, 2 => FanMode::Off, _ => FanMode::Auto };
            Ok((mode, speed_val.clamp(20, 100) as u8))
        }
    }
    #[cfg(not(windows))]
    { Ok((FanMode::Auto, 50)) }
}

// ── IGCL stubs ───────────────────────────────────────────────────────────────

fn set_fan_auto_igcl() -> Result<()> {
    // IGCL ctlFanSetDefaultMode — requires ctlEnumFans first.
    // Stubbed: actual IGCL fan API requires ctl_fan_handle_t enumeration.
    log::info!("Fan auto mode via IGCL (stub)");
    Ok(())
}

fn set_fan_fixed_igcl(speed_percent: u8) -> Result<()> {
    log::info!("Fan fixed speed {speed_percent}% via IGCL (stub)");
    Ok(())
}
