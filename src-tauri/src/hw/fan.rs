//! Fan control and temperature monitoring.
//!
//! Reads fan speed, CPU/GPU temperature, and power from WMI
//! and Intel ESIF/DPTF, with support for setting fan modes.

use crate::hw::errors::{HardwareError, HardwareResult};
use anyhow::Context;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FanInfo {
    pub mode: FanMode,
    /// Fan RPM from Win32_Fan.CurrentReading — 0 when the EC does not expose
    /// RPM via WMI (common on Xiaomi Book Pro 14 and similar Intel platforms).
    pub speed_rpm: u32,
    pub speed_percent: u8,
    /// GPU temperature in Celsius. None when no sensor is available
    /// (ESIF/DPTF driver absent and ACPI thermal zone unavailable).
    pub gpu_temp_celsius: Option<f32>,
    /// CPU temperature in Celsius. None when no sensor is available
    /// (ESIF/DPTF driver absent and ACPI thermal zone unavailable).
    pub cpu_temp_celsius: Option<f32>,
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
    cpu_temp: Option<f32>,
    gpu_temp: Option<f32>,
    tdp_watts: Option<f32>,
}

fn get_esif_readings() -> HardwareResult<EsifReadings> {
    #[cfg(windows)]
    {
        use crate::hw::wmi_cache;
        use crate::util::wmi_extract;
        use std::collections::HashMap;

        let results: Vec<HashMap<String, wmi::Variant>> = wmi_cache::with_wmi(|wmi| {
            Ok(wmi
                .raw_query("SELECT InstanceName, Temperature, Power FROM EsifDeviceInformation")
                .unwrap_or_default())
        })?;

        let extract_u32_temp = |row: &HashMap<String, wmi::Variant>, key: &str| -> Option<f32> {
            wmi_extract::extract_u32(row, key).map(|v| v as f32)
        };

        let instance_suffix = |row: &HashMap<String, wmi::Variant>, suffix: &str| -> bool {
            wmi_extract::extract_string(row, "InstanceName").is_some_and(|s| s.ends_with(suffix))
        };

        // CPU temp: max Temperature across participants (hotspot).
        // Zero is a valid reading (0°C idle), so we do NOT filter it out.
        let cpu_temp = results
            .iter()
            .filter_map(|r| extract_u32_temp(r, "Temperature"))
            .fold(f32::NEG_INFINITY, |acc, v| acc.max(v));
        let cpu_temp = if cpu_temp.is_finite() {
            Some(cpu_temp.clamp(0.0, 120.0))
        } else {
            None // No ESIF data — do NOT fabricate a value
        };

        // GPU temp: prefer participant _10 (GPU/secondary SoC domain on Panther Lake)
        let gpu_temp = results
            .iter()
            .find(|r| instance_suffix(r, "_10"))
            .and_then(|r| extract_u32_temp(r, "Temperature"))
            .map(|v| v.clamp(0.0, 120.0))
            .or_else(|| {
                // Fallback: package maximum (same die, valid under GPU load)
                let m = results
                    .iter()
                    .filter_map(|r| extract_u32_temp(r, "Temperature"))
                    .fold(f32::NEG_INFINITY, |acc, v| acc.max(v));
                if m.is_finite() {
                    Some(m.clamp(0.0, 120.0))
                } else {
                    None
                }
            });

        // TDP: participant _0 is the highest-level DPTF power domain
        let tdp_watts = results
            .iter()
            .find(|r| instance_suffix(r, "_0"))
            .and_then(|r| extract_u32_temp(r, "Power"))
            .map(|v| (v / 10.0).clamp(0.0, 150.0));

        Ok(EsifReadings {
            cpu_temp,
            gpu_temp,
            tdp_watts,
        })
    }
    #[cfg(not(windows))]
    {
        Ok(EsifReadings {
            cpu_temp: None,
            gpu_temp: None,
            tdp_watts: None,
        })
    }
}

pub fn get_fan_info() -> HardwareResult<FanInfo> {
    let speed_rpm = get_fan_rpm_wmi().unwrap_or(0);
    let esif = get_esif_readings().unwrap_or(EsifReadings {
        cpu_temp: None,
        gpu_temp: None,
        tdp_watts: None,
    });
    // If ESIF failed, try ACPI thermal zone as fallback (not a hardcoded value)
    let cpu_temp = esif
        .cpu_temp
        .or_else(|| match crate::hw::thermal::get_primary_thermal_zone() {
            Ok(zone) => Some(zone.current_temp_celsius as f32),
            Err(e) => {
                log::warn!(target: "hw::fan", "ESIF and ACPI thermal zone both unavailable: {e}");
                None
            }
        });
    let gpu_temp = esif.gpu_temp;
    let tdp_watts = esif.tdp_watts;
    let (mode, speed_percent) = get_fan_mode_registry().unwrap_or((FanMode::Auto, 50));

    // WORKING FORM — DO NOT MODIFY: EC performance mode is read via WMI
    // MiInterface (ACPI WMAA method), NOT via IoTDriver or ECRAM.
    // wmi_ec::get_performance_mode() calls wmi_read(FUN2_EC_FUNC, 0) which
    // goes through the MICommonInterface WMI class in root\WMI.
    // This works WITHOUT IoTDriver and WITHOUT elevation (when called from
    // the elevated bridge process).
    let ec_mode = crate::hw::wmi_ec::get_performance_mode().ok();
    if let Some(ec_mode) = ec_mode {
        log::debug!(target: "hw::fan", "EC performance mode via WMI: {:?}", ec_mode);
    }

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
        tdp_watts,
    })
}

pub fn set_fan_mode(mode: FanMode, speed_percent: u8) -> HardwareResult<()> {
    persist_fan_registry(&mode, speed_percent)?;

    // WORKING FORM — DO NOT MODIFY: EC performance mode is set via WMI
    // MiInterface (ACPI WMAA method), NOT via IoTDriver or ECRAM.
    // wmi_ec::set_performance_mode() calls wmi_write(FUN2_EC_FUNC, mode, 0)
    // which goes through the MICommonInterface WMI class in root\WMI.
    // FanMode mapping to EcPerformanceMode:
    //   Auto/Fixed >=80% → UltraPerformance (9)
    //   Auto/Fixed >=50% → Balanced (6)
    //   Auto/Fixed <50%  → Quiet (7)
    //   Off              → SuperQuiet (8)
    let ec_mode = match mode {
        FanMode::Auto => {
            // Map to EC balanced/performance based on speed
            if speed_percent >= 80 {
                crate::hw::wmi_ec::EcPerformanceMode::UltraPerformance
            } else if speed_percent >= 50 {
                crate::hw::wmi_ec::EcPerformanceMode::Balanced
            } else {
                crate::hw::wmi_ec::EcPerformanceMode::Quiet
            }
        }
        FanMode::Fixed => {
            if speed_percent >= 80 {
                crate::hw::wmi_ec::EcPerformanceMode::UltraPerformance
            } else if speed_percent >= 50 {
                crate::hw::wmi_ec::EcPerformanceMode::Balanced
            } else {
                crate::hw::wmi_ec::EcPerformanceMode::Quiet
            }
        }
        FanMode::Off => crate::hw::wmi_ec::EcPerformanceMode::SuperQuiet,
    };
    crate::hw::wmi_ec::set_performance_mode(ec_mode)
        .unwrap_or_else(|e| log::warn!("WMI EC set_performance_mode: {e}"));

    match mode {
        FanMode::Auto => set_fan_auto_igcl().unwrap_or_else(|e| log::warn!("IGCL fan auto: {e}")),
        FanMode::Fixed => {
            set_fan_fixed_igcl(speed_percent).unwrap_or_else(|e| log::warn!("IGCL fan fixed: {e}"))
        }
        FanMode::Off => log::warn!("Fan off mode not directly supported — using minimum speed"),
    }
    Ok(())
}

// ── WMI helpers ─────────────────────────────────────────────────────────────

fn get_fan_rpm_wmi() -> HardwareResult<u32> {
    #[cfg(windows)]
    {
        use crate::hw::wmi_cache;
        use crate::util::wmi_extract;
        use std::collections::HashMap;

        let results: Vec<HashMap<String, wmi::Variant>> = wmi_cache::with_cimv2(|wmi| {
            Ok(wmi
                .raw_query("SELECT CurrentReading FROM Win32_Fan")
                .unwrap_or_default())
        })?;
        if let Some(row) = results.first() {
            if let Some(rpm) = wmi_extract::extract_u32(row, "CurrentReading") {
                return Ok(rpm);
            }
        }
        Ok(0)
    }
    #[cfg(not(windows))]
    {
        Ok(0)
    }
}

// ── Registry persistence ─────────────────────────────────────────────────────

fn persist_fan_registry(mode: &FanMode, speed_percent: u8) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY_LOCAL_MACHINE, KEY_WRITE, REG_DWORD,
            REG_OPTION_NON_VOLATILE,
        };
        unsafe {
            // SAFETY: Null-terminated wide strings; MaybeUninit<HKEY> written by
            // RegCreateKeyExW before assume_init. Stack-local DWORD values have valid alignment.
            let key_w: Vec<u16> = OsStr::new(FAN_REG_KEY)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let mut hkey = std::mem::MaybeUninit::uninit();
            RegCreateKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(key_w.as_ptr()),
                0,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                hkey.as_mut_ptr(),
                None,
            )
            .ok()
            .context("Create fan reg key")?;
            let hkey = hkey.assume_init();

            let mode_val: u32 = match mode {
                FanMode::Auto => 0,
                FanMode::Fixed => 1,
                FanMode::Off => 2,
            };
            let mode_w: Vec<u16> = OsStr::new(FAN_REG_MODE)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let _ = RegSetValueExW(
                hkey,
                PCWSTR(mode_w.as_ptr()),
                0,
                REG_DWORD,
                Some(&mode_val.to_le_bytes()),
            )
            .ok();

            let speed_val = speed_percent as u32;
            let speed_w: Vec<u16> = OsStr::new(FAN_REG_SPEED)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let _ = RegSetValueExW(
                hkey,
                PCWSTR(speed_w.as_ptr()),
                0,
                REG_DWORD,
                Some(&speed_val.to_le_bytes()),
            )
            .ok();

            let _ = RegCloseKey(hkey).ok();
        }
    }
    Ok(())
}

fn get_fan_mode_registry() -> HardwareResult<(FanMode, u8)> {
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_LOCAL_MACHINE, REG_VALUE_TYPE,
        };
        unsafe {
            // SAFETY: Null-terminated wide strings; hkey is assume_init only after
            // RegOpenKeyExW succeeds. The u32 pointer cast is valid for DWORD-sized stack buffer.
            let key_w: Vec<u16> = OsStr::new(FAN_REG_KEY)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let mut hkey = std::mem::MaybeUninit::uninit();
            if RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(key_w.as_ptr()),
                0,
                windows::Win32::System::Registry::KEY_READ,
                hkey.as_mut_ptr(),
            )
            .is_err()
            {
                return Ok((FanMode::Auto, 50));
            }
            let hkey = hkey.assume_init();

            let mut mode_val: u32 = 0;
            let mut size = 4u32;
            let mut ty = REG_VALUE_TYPE::default();
            let mode_w: Vec<u16> = OsStr::new(FAN_REG_MODE)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let _ = RegQueryValueExW(
                hkey,
                PCWSTR(mode_w.as_ptr()),
                None,
                Some(&mut ty),
                Some((&mut mode_val as *mut u32).cast()),
                Some(&mut size),
            );

            let mut speed_val: u32 = 50;
            let speed_w: Vec<u16> = OsStr::new(FAN_REG_SPEED)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let _ = RegQueryValueExW(
                hkey,
                PCWSTR(speed_w.as_ptr()),
                None,
                Some(&mut ty),
                Some((&mut speed_val as *mut u32).cast()),
                Some(&mut size),
            );

            let _ = RegCloseKey(hkey).ok();
            let mode = match mode_val {
                1 => FanMode::Fixed,
                2 => FanMode::Off,
                _ => FanMode::Auto,
            };
            Ok((mode, speed_val.clamp(20, 100) as u8))
        }
    }
    #[cfg(not(windows))]
    {
        Ok((FanMode::Auto, 50))
    }
}

// ── IGCL fan control ─────────────────────────────────────────────────────────
//
// Intel IGCL (ControlLib.dll) exposes fan handles via ctlEnumFans.
// On laptops with only integrated graphics (no dGPU), ctlEnumFans typically
// returns 0 handles — the laptop fan is controlled by the EC/firmware and
// responds to performance mode changes, not IGCL.  The code below is real and
// will work on any machine where IGCL reports ≥1 fan handle.
//
// C layouts:
//   ctl_fan_speed_units_t: 0 = PERCENT, 1 = RPM
//   ctl_fan_speed_t { size:u32, version:u8, [3-pad], units:u32, value:i32 }

#[cfg(windows)]
mod igcl_fan {
    use std::ffi::c_void;

    pub type CtlDeviceHandle = *mut c_void;
    pub type CtlFanHandle = *mut c_void;
    pub type CtlResult = u32;

    #[repr(C)]
    pub struct CtlFanSpeed {
        pub size: u32,
        pub version: u8,
        pub _pad: [u8; 3],
        pub units: u32, // 0 = PERCENT, 1 = RPM
        pub value: i32,
    }

    pub type FnCtlEnumFans =
        unsafe extern "C" fn(CtlDeviceHandle, *mut u32, *mut CtlFanHandle) -> CtlResult;
    pub type FnCtlFanSetDefaultMode = unsafe extern "C" fn(CtlFanHandle) -> CtlResult;
    pub type FnCtlFanSetFixedSpeedMode =
        unsafe extern "C" fn(CtlFanHandle, *const CtlFanSpeed) -> CtlResult;
}

/// Run `f` for every IGCL fan handle on the first enumerated device.
/// Returns `Ok(0)` (no fans found, nothing done) if the device has no IGCL-
/// accessible fans — expected on integrated-only platforms where the EC
/// firmware manages the fan as a function of the active performance mode.
#[cfg(windows)]
fn with_igcl_fans<F>(f: F) -> HardwareResult<usize>
where
    F: Fn(igcl_fan::CtlFanHandle, &libloading::Library) -> HardwareResult<()>,
{
    use igcl_fan::*;

    let count = crate::hw::display::with_igcl_device_pub(|device, lib| unsafe {
        // SAFETY: device is a valid IGCL device handle from ctlEnumerateDevices. The
        // ctlEnumFans function returns fan handles owned by IGCL; we iterate them immediately
        // and do not retain handles after the closure ends.
        let ctl_enum_fans: libloading::Symbol<FnCtlEnumFans> =
            lib.get(b"ctlEnumFans\0").context("ctlEnumFans")?;

        let mut count: u32 = 0;
        ctl_enum_fans(device, &mut count, std::ptr::null_mut());
        if count == 0 {
            log::debug!("[fan] ctlEnumFans: no IGCL fan handles (EC-managed fan — OK)");
            return Ok(0usize);
        }

        let mut handles = vec![std::ptr::null_mut::<std::ffi::c_void>(); count as usize];
        let rc = ctl_enum_fans(device, &mut count, handles.as_mut_ptr());
        if rc != 0 {
            return Err(HardwareError::Display(format!(
                "ctlEnumFans failed: {rc:#x}"
            )));
        }

        for &fan in &handles[..count as usize] {
            f(fan, lib)?;
        }
        Ok(count as usize)
    })?;
    Ok(count)
}

#[cfg(not(windows))]
fn with_igcl_fans<F>(_f: F) -> HardwareResult<usize>
where
    F: Fn(*mut std::ffi::c_void, &libloading::Library) -> HardwareResult<()>,
{
    Ok(0)
}

fn set_fan_auto_igcl() -> HardwareResult<()> {
    #[cfg(windows)]
    {
        use igcl_fan::*;
        let n = with_igcl_fans(|fan, lib| unsafe {
            // SAFETY: fan is a valid IGCL fan handle from ctlEnumFans. The ctlFanSetDefaultMode
            // function pointer is loaded from the IGCL DLL which is still borrowed by the outer
            // with_igcl_device_pub closure.
            let set_default: libloading::Symbol<FnCtlFanSetDefaultMode> = lib
                .get(b"ctlFanSetDefaultMode\0")
                .context("ctlFanSetDefaultMode")?;
            let rc = set_default(fan);
            if rc != 0 {
                return Err(HardwareError::Display(format!(
                    "ctlFanSetDefaultMode: {rc:#x}"
                )));
            }
            log::info!("[fan] IGCL ctlFanSetDefaultMode OK");
            Ok(())
        })?;
        if n == 0 {
            log::debug!("[fan] Auto mode: no IGCL fans — performance mode controls EC fan");
        }
    }
    Ok(())
}

fn set_fan_fixed_igcl(speed_percent: u8) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        use igcl_fan::*;
        let clamped = speed_percent.clamp(20, 100) as i32;
        let n = with_igcl_fans(|fan, lib| unsafe {
            // SAFETY: fan is a valid IGCL fan handle. CtlFanSpeed is a POD struct with correct
            // layout (size, version, pad, units, value). The function pointer is loaded from
            // the IGCL DLL which is still borrowed by the outer with_igcl_device_pub closure.
            let set_fixed: libloading::Symbol<FnCtlFanSetFixedSpeedMode> = lib
                .get(b"ctlFanSetFixedSpeedMode\0")
                .context("ctlFanSetFixedSpeedMode")?;
            let speed = CtlFanSpeed {
                size: std::mem::size_of::<CtlFanSpeed>() as u32,
                version: 0,
                _pad: [0; 3],
                units: 0, // PERCENT
                value: clamped,
            };
            let rc = set_fixed(fan, &speed);
            if rc != 0 {
                return Err(HardwareError::Display(format!(
                    "ctlFanSetFixedSpeedMode {clamped}%: {rc:#x}"
                )));
            }
            log::info!("[fan] IGCL ctlFanSetFixedSpeedMode {clamped}% OK");
            Ok(())
        })?;
        if n == 0 {
            log::debug!("[fan] Fixed {clamped}%: no IGCL fans — only perf mode affects EC fan");
        }
    }
    Ok(())
}
