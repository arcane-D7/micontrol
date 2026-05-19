use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FanInfo {
    pub mode: FanMode,
    /// Fan RPM from Win32_Fan.CurrentReading — 0 when the EC does not expose
    /// RPM via WMI (common on Xiaomi Book Pro 14 and similar Intel platforms).
    pub speed_rpm: u32,
    pub speed_percent: u8,
    pub gpu_temp_celsius: f32,
    pub cpu_temp_celsius: f32,
    /// CPU package power from Intel ESIF/DPTF (EsifDeviceInformation._0 Power
    /// field), in watts. The raw WMI value is in deciwatts (×0.1 W). None when
    /// the DPTF driver is absent or reports zero.
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

// ── ESIF thermal readings ─────────────────────────────────────────────────
//
// EsifDeviceInformation (ROOT\WMI) is populated by the Intel DPTF/ESIF driver.
// Participants _0/_1/_2 track the CPU hotspot; _10 tracks the GPU/secondary SoC
// domain. The Power field is in deciwatts (×0.1 W); Temperature is Celsius.
// One WMI query returns all participants, so we read temps and TDP together.

struct EsifReadings {
    cpu_temp: f32,
    gpu_temp: f32,
    tdp_watts: Option<f32>,
}

fn get_esif_readings() -> Result<EsifReadings> {
    #[cfg(windows)]
    {
        use wmi::{COMLibrary, WMIConnection};
        use std::collections::HashMap;
        let com = COMLibrary::new().context("COM")?;
        let wmi = WMIConnection::with_namespace_path("ROOT\\WMI", com.into()).context("WMI")?;
        let results: Vec<HashMap<String, wmi::Variant>> = wmi
            .raw_query("SELECT InstanceName, Temperature, Power FROM EsifDeviceInformation")
            .unwrap_or_default();

        let extract_int = |row: &HashMap<String, wmi::Variant>, key: &str| -> Option<i64> {
            match row.get(key) {
                Some(wmi::Variant::I4(v)) if *v > 0 => Some(*v as i64),
                Some(wmi::Variant::UI4(v)) if *v > 0 => Some(*v as i64),
                _ => None,
            }
        };

        let instance_suffix = |row: &HashMap<String, wmi::Variant>, suffix: &str| -> bool {
            matches!(row.get("InstanceName"), Some(wmi::Variant::String(s)) if s.ends_with(suffix))
        };

        // CPU temp: max non-zero Temperature (participants _0/_1/_2 are hotspot)
        let cpu_temp = results.iter()
            .filter_map(|r| extract_int(r, "Temperature"))
            .fold(f32::NEG_INFINITY, |acc, v| acc.max(v as f32));
        let cpu_temp = if cpu_temp > 0.0 && cpu_temp.is_finite() {
            cpu_temp.clamp(0.0, 120.0)
        } else {
            50.0
        };

        // GPU temp: prefer participant _10 (GPU/secondary SoC domain on Panther Lake)
        let gpu_temp = results.iter()
            .find(|r| instance_suffix(r, "_10"))
            .and_then(|r| extract_int(r, "Temperature"))
            .map(|v| (v as f32).clamp(0.0, 120.0))
            .unwrap_or_else(|| {
                // Fallback: package maximum (same die, valid under GPU load)
                let m = results.iter()
                    .filter_map(|r| extract_int(r, "Temperature"))
                    .fold(f32::NEG_INFINITY, |acc, v| acc.max(v as f32));
                if m > 0.0 && m.is_finite() { m.clamp(0.0, 120.0) } else { 45.0 }
            });

        // TDP: participant _0 is the highest-level DPTF power domain (CPU package/
        // platform RAPL). Power is in deciwatts — divide by 10 to get watts.
        let tdp_watts = results.iter()
            .find(|r| instance_suffix(r, "_0"))
            .and_then(|r| extract_int(r, "Power"))
            .map(|v| (v as f32 / 10.0).clamp(0.0, 150.0));

        return Ok(EsifReadings { cpu_temp, gpu_temp, tdp_watts });
    }
    #[cfg(not(windows))]
    {
        Ok(EsifReadings { cpu_temp: 50.0, gpu_temp: 45.0, tdp_watts: None })
    }
}

pub fn get_fan_info() -> Result<FanInfo> {
    let speed_rpm = get_fan_rpm_wmi().unwrap_or(0);
    let esif = get_esif_readings().unwrap_or(EsifReadings { cpu_temp: 50.0, gpu_temp: 45.0, tdp_watts: None });
    let (mode, speed_percent) = get_fan_mode_registry().unwrap_or((FanMode::Auto, 50));

    let speed_percent_actual = if speed_rpm > 0 {
        ((speed_rpm as f32 / 5000.0) * 100.0).clamp(0.0, 100.0) as u8
    } else {
        speed_percent
    };

    Ok(FanInfo {
        mode,
        speed_rpm,
        speed_percent: speed_percent_actual,
        gpu_temp_celsius: esif.gpu_temp,
        cpu_temp_celsius: esif.cpu_temp,
        tdp_watts: esif.tdp_watts,
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
