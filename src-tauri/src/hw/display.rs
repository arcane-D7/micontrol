use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DisplayInfo {
    pub brightness: u8,
    pub hdr_enabled: bool,
    pub refresh_rate_hz: u32,
    pub ai_brightness: bool,
    pub ai_brightness_config: AiBrightnessConfig,
}

const IGCL_DLL: &str = r"C:\Windows\System32\ControlLib.dll";
const AI_BRIGHTNESS_REG_KEY: &str = r"SOFTWARE\MI\DisplaySettings";
const AI_BRIGHTNESS_REG_VALUE: &str = "AiAdaptiveBrightness";
const AI_BRIGHTNESS_MIN_VALUE:  &str = "AiBrightnessMin";
const AI_BRIGHTNESS_MAX_VALUE:  &str = "AiBrightnessMax";
const AI_BRIGHTNESS_SENS_VALUE: &str = "AiBrightnessSensitivity";
const AI_BRIGHTNESS_SMTH_VALUE: &str = "AiBrightnessSmoothing";

/// Sensitivity / range configuration for our own adaptive brightness loop.
///
/// Formula per iteration (every 2 s):
///   max_lux  = 2000 / (sensitivity / 100)   — lux where ceiling is reached
///   target   = clamp(min + (lux / max_lux) * (max - min), min, max)
///   smoothed = current + (target - current) * (1 - smoothing/100)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiBrightnessConfig {
    /// Whether our adaptive loop should run.
    pub enabled: bool,
    /// Floor: brightness will never drop below this % (5-80, default 10).
    pub min_brightness: u8,
    /// Ceiling: brightness will never exceed this % (20-100, default 100).
    pub max_brightness: u8,
    /// Reactivity: 100 = full range at 2000 lux, 200 = at 1000 lux (more), 50 = at 4000 lux (less).
    pub sensitivity: u8,
    /// Transition smoothing 0-90: 0 = instant, 30 = default (fast), 90 = very gradual.
    pub smoothing: u8,
}

pub fn get_display_info() -> Result<DisplayInfo> {
    let brightness = get_brightness_igcl().unwrap_or_else(|_| get_brightness_wmi().unwrap_or(80));
    let hdr_enabled = false;
    let refresh_rate_hz = get_refresh_rate().unwrap_or(120);
    let ai_brightness_config = get_ai_brightness_config();
    let ai_brightness = ai_brightness_config.enabled;
    Ok(DisplayInfo { brightness, hdr_enabled, refresh_rate_hz, ai_brightness, ai_brightness_config })
}

pub fn set_brightness(level: u8) -> Result<()> {
    let level = level.clamp(10, 100);
    if let Err(e) = set_brightness_igcl(level) {
        log::warn!("IGCL brightness failed: {e}, using WMI");
        set_brightness_wmi(level)?;
    }
    Ok(())
}

pub fn set_hdr(enabled: bool) -> Result<()> {
    // IGCL ctlSetHDRSetting — not yet implemented, log for now
    log::info!("HDR set to {enabled} (stub)");
    Ok(())
}

pub fn set_ai_brightness(enabled: bool) -> Result<()> {
    // Toggle the enabled flag while preserving all other config values.
    let mut cfg = get_ai_brightness_config();
    cfg.enabled = enabled;
    set_ai_brightness_config(cfg)
}

// ── Adaptive brightness config ────────────────────────────────────────────────

fn read_display_dword(name: &str, default: u32) -> u32 {
    #[cfg(windows)]
    {
        use winreg::{RegKey, enums::HKEY_LOCAL_MACHINE};
        if let Ok(key) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(AI_BRIGHTNESS_REG_KEY) {
            if let Ok(v) = key.get_value::<u32, _>(name) {
                return v;
            }
        }
    }
    default
}

fn write_display_dword(name: &str, value: u32) -> Result<()> {
    #[cfg(windows)]
    {
        use winreg::{RegKey, enums::HKEY_LOCAL_MACHINE};
        let (key, _) = RegKey::predef(HKEY_LOCAL_MACHINE)
            .create_subkey(AI_BRIGHTNESS_REG_KEY)
            .context("create display settings key")?;
        key.set_value(name, &value).context("write dword")?;
    }
    Ok(())
}

pub fn get_ai_brightness_config() -> AiBrightnessConfig {
    let enabled = get_ai_brightness_registry().unwrap_or(false);
    let min_b = (read_display_dword(AI_BRIGHTNESS_MIN_VALUE, 10) as u8).clamp(5, 80);
    let max_b = (read_display_dword(AI_BRIGHTNESS_MAX_VALUE, 100) as u8).clamp(min_b + 5, 100);
    AiBrightnessConfig {
        enabled,
        min_brightness: min_b,
        max_brightness: max_b,
        sensitivity: (read_display_dword(AI_BRIGHTNESS_SENS_VALUE, 100) as u8).clamp(10, 200),
        smoothing:   (read_display_dword(AI_BRIGHTNESS_SMTH_VALUE, 30) as u8).min(90),
    }
}

pub fn set_ai_brightness_config(config: AiBrightnessConfig) -> Result<()> {
    persist_ai_brightness_registry(config.enabled)?;
    write_display_dword(AI_BRIGHTNESS_MIN_VALUE,  config.min_brightness as u32)?;
    write_display_dword(AI_BRIGHTNESS_MAX_VALUE,  config.max_brightness as u32)?;
    write_display_dword(AI_BRIGHTNESS_SENS_VALUE, config.sensitivity    as u32)?;
    write_display_dword(AI_BRIGHTNESS_SMTH_VALUE, config.smoothing      as u32)?;
    Ok(())
}

// ── Ambient light sensor ──────────────────────────────────────────────────────

#[cfg(windows)]
fn get_ambient_lux() -> Option<f32> {
    use windows::Devices::Sensors::LightSensor;
    let sensor = LightSensor::GetDefault().ok()?;
    let reading = sensor.GetCurrentReading().ok()?;
    reading.IlluminanceInLux().ok()
}

#[cfg(not(windows))]
fn get_ambient_lux() -> Option<f32> { None }

// ── Adaptive brightness background loop ──────────────────────────────────────

/// Spawned once at startup. Every 2 s it reads the ambient light sensor and
/// adjusts screen brightness according to the user-configured sensitivity curve.
/// Config changes are picked up automatically on each iteration.
pub async fn adaptive_brightness_loop() {
    let mut smoothed: Option<f32> = None;
    let mut no_sensor_warned = false;
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
        let cfg = get_ai_brightness_config();
        if !cfg.enabled {
            smoothed = None;
            continue;
        }
        let lux = match get_ambient_lux() {
            Some(v) => v,
            None => {
                if !no_sensor_warned {
                    log::warn!("adaptive_brightness: no ambient light sensor found — loop idle");
                    no_sensor_warned = true;
                }
                continue;
            }
        };
        no_sensor_warned = false;
        // sensitivity=100 → reaches ceiling at 2000 lux
        // sensitivity=200 → reaches ceiling at 1000 lux  (more reactive)
        // sensitivity=50  → reaches ceiling at 4000 lux  (less reactive)
        let max_lux = 2000.0_f32 * (100.0 / cfg.sensitivity.max(1) as f32);
        let range   = cfg.max_brightness as f32 - cfg.min_brightness as f32;
        let target  = (cfg.min_brightness as f32 + (lux / max_lux) * range)
            .clamp(cfg.min_brightness as f32, cfg.max_brightness as f32);
        let current = smoothed.unwrap_or(target);
        let sf      = cfg.smoothing.min(95) as f32 / 100.0;
        let next    = current + (target - current) * (1.0 - sf);
        smoothed = Some(next);
        if let Err(e) = set_brightness(next.round() as u8) {
            log::warn!("adaptive_brightness: set_brightness error: {e}");
        }
    }
}

// ── IGCL FFI ────────────────────────────────────────────────────────────────

#[cfg(windows)]
mod igcl {
    use std::ffi::c_void;

    #[repr(C)]
    pub struct CtlInitArgs {
        pub size: u32,
        pub app_version: u64,
        pub flags: u32,
    }

    #[repr(C)]
    pub struct CtlBrightnessArgs {
        pub size: u32,
        pub brightness_setting: f64,
        pub brightness_type: u32, // 0 = absolute
    }

    pub type CtlApiHandle = *mut c_void;
    pub type CtlDeviceHandle = *mut c_void;
    pub type CtlResult = u32; // 0 = success

    // Function pointer types
    pub type FnCtlInit = unsafe extern "C" fn(*mut CtlInitArgs, *mut CtlApiHandle) -> CtlResult;
    pub type FnCtlClose = unsafe extern "C" fn(CtlApiHandle) -> CtlResult;
    pub type FnCtlEnumerateDevices =
        unsafe extern "C" fn(CtlApiHandle, *mut u32, *mut CtlDeviceHandle) -> CtlResult;
    pub type FnCtlGetBrightnessSetting =
        unsafe extern "C" fn(CtlDeviceHandle, *mut CtlBrightnessArgs) -> CtlResult;
    pub type FnCtlSetBrightnessSetting =
        unsafe extern "C" fn(CtlDeviceHandle, *mut CtlBrightnessArgs) -> CtlResult;
}

#[cfg(windows)]
fn with_igcl_device<F, T>(f: F) -> Result<T>
where
    F: FnOnce(*mut std::ffi::c_void, &libloading::Library) -> Result<T>,
{
    use libloading::Library;
    use igcl::*;

    unsafe {
        // Use the IGCL DLL path found during startup discovery; fall back to the system default.
        let igcl_path = crate::hw::discovery::global_profile()
            .and_then(|p| p.igcl_dll_path.as_deref())
            .unwrap_or(IGCL_DLL)
            .to_string();
        let lib = Library::new(&igcl_path).context("Load ControlLib.dll")?;

        let ctl_init: libloading::Symbol<FnCtlInit> =
            lib.get(b"ctlInit\0").context("ctlInit")?;
        let ctl_enumerate: libloading::Symbol<FnCtlEnumerateDevices> =
            lib.get(b"ctlEnumerateDevices\0").context("ctlEnumerateDevices")?;
        let ctl_close: libloading::Symbol<FnCtlClose> =
            lib.get(b"ctlClose\0").context("ctlClose")?;

        let mut init_args = CtlInitArgs {
            size: std::mem::size_of::<CtlInitArgs>() as u32,
            app_version: 1,
            flags: 0,
        };
        let mut api_handle: CtlApiHandle = std::ptr::null_mut();
        let rc = ctl_init(&mut init_args, &mut api_handle);
        if rc != 0 {
            anyhow::bail!("ctlInit failed: {rc}");
        }

        let mut count: u32 = 0;
        ctl_enumerate(api_handle, &mut count, std::ptr::null_mut());
        if count == 0 {
            ctl_close(api_handle);
            anyhow::bail!("No IGCL devices found");
        }
        let mut devices = vec![std::ptr::null_mut::<std::ffi::c_void>(); count as usize];
        ctl_enumerate(api_handle, &mut count, devices.as_mut_ptr());

        let device = devices[0];
        let result = f(device, &lib);
        ctl_close(api_handle);
        result
    }
}

#[cfg(windows)]
fn get_brightness_igcl() -> Result<u8> {
    use igcl::*;
    with_igcl_device(|device, lib| unsafe {
        let get_brightness: libloading::Symbol<FnCtlGetBrightnessSetting> =
            lib.get(b"ctlGetBrightnessSetting\0").context("ctlGetBrightnessSetting")?;
        let mut args = CtlBrightnessArgs {
            size: std::mem::size_of::<CtlBrightnessArgs>() as u32,
            brightness_setting: 0.0,
            brightness_type: 0,
        };
        let rc = get_brightness(device as CtlDeviceHandle, &mut args);
        if rc != 0 {
            anyhow::bail!("ctlGetBrightnessSetting failed: {rc}");
        }
        Ok(args.brightness_setting.clamp(0.0, 100.0) as u8)
    })
}

#[cfg(not(windows))]
fn get_brightness_igcl() -> Result<u8> { anyhow::bail!("IGCL not on non-Windows") }

#[cfg(windows)]
fn set_brightness_igcl(level: u8) -> Result<()> {
    use igcl::*;
    with_igcl_device(|device, lib| unsafe {
        let set_brightness: libloading::Symbol<FnCtlSetBrightnessSetting> =
            lib.get(b"ctlSetBrightnessSetting\0").context("ctlSetBrightnessSetting")?;
        let mut args = CtlBrightnessArgs {
            size: std::mem::size_of::<CtlBrightnessArgs>() as u32,
            brightness_setting: level as f64,
            brightness_type: 0,
        };
        let rc = set_brightness(device as CtlDeviceHandle, &mut args);
        if rc != 0 { anyhow::bail!("ctlSetBrightnessSetting failed: {rc}"); }
        Ok(())
    })
}

#[cfg(not(windows))]
fn set_brightness_igcl(_level: u8) -> Result<()> { anyhow::bail!("IGCL not on non-Windows") }

// ── WMI fallback ────────────────────────────────────────────────────────────

fn get_brightness_wmi() -> Result<u8> {
    #[cfg(windows)]
    {
        use wmi::{COMLibrary, WMIConnection};
        use std::collections::HashMap;

        let com = COMLibrary::new().context("COM")?;
        let wmi = WMIConnection::with_namespace_path("root\\WMI", com.into()).context("WMI")?;
        let results: Vec<HashMap<String, wmi::Variant>> = wmi
            .raw_query("SELECT CurrentBrightness FROM WmiMonitorBrightness")
            .context("WmiMonitorBrightness")?;
        let first = results.first().context("No monitor")?;
        match first.get("CurrentBrightness") {
            Some(wmi::Variant::UI1(v)) => Ok(*v),
            _ => Ok(80),
        }
    }
    #[cfg(not(windows))]
    { Ok(80) }
}

fn set_brightness_wmi(level: u8) -> Result<()> {
    #[cfg(windows)]
    {
        use wmi::{COMLibrary, WMIConnection};
        use std::collections::HashMap;

        let com = COMLibrary::new().context("COM")?;
        let wmi = WMIConnection::with_namespace_path("root\\WMI", com.into()).context("WMI")?;
        let results: Vec<HashMap<String, wmi::Variant>> = wmi
            .raw_query("SELECT * FROM WmiMonitorBrightnessMethods")
            .context("WmiMonitorBrightnessMethods")?;
        if let Some(_inst) = results.first() {
            // Execute WMI method WmiMonitorBrightnessMethods.WmiSetBrightness
            // Using exec_method_with_params would need wmi crate v0.13+ method support
            // For now invoke via powershell as fallback
            let _ = std::process::Command::new("powershell")
                .args(["-NoProfile", "-Command", &format!(
                    "(Get-WmiObject -Namespace root/WMI -Class WmiMonitorBrightnessMethods).WmiSetBrightness(1,{})",
                    level
                )])
                .output();
        }
    }
    Ok(())
}

fn get_refresh_rate() -> Result<u32> {
    #[cfg(windows)]
    {
        use wmi::{COMLibrary, WMIConnection};
        use std::collections::HashMap;
        if let Ok(com) = COMLibrary::new() {
            if let Ok(wmi) = WMIConnection::new(com.into()) {
                let results: Vec<HashMap<String, wmi::Variant>> = wmi
                    .raw_query("SELECT CurrentRefreshRate FROM Win32_VideoController")
                    .unwrap_or_default();
                if let Some(row) = results.first() {
                    match row.get("CurrentRefreshRate") {
                        Some(wmi::Variant::UI4(v)) => return Ok(*v),
                        _ => {}
                    }
                }
            }
        }
    }
    Ok(120)
}

fn get_ai_brightness_registry() -> Result<bool> {
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_LOCAL_MACHINE, REG_VALUE_TYPE,
        };
        use windows::core::PCWSTR;
        unsafe {
            let key_w: Vec<u16> = OsStr::new(AI_BRIGHTNESS_REG_KEY).encode_wide().chain(Some(0)).collect();
            let mut hkey = std::mem::zeroed();
            let res = RegOpenKeyExW(
                HKEY_LOCAL_MACHINE, PCWSTR(key_w.as_ptr()), 0,
                windows::Win32::System::Registry::KEY_READ, &mut hkey,
            );
            if res.is_err() { return Ok(false); }
            let val_w: Vec<u16> = OsStr::new(AI_BRIGHTNESS_REG_VALUE).encode_wide().chain(Some(0)).collect();
            let mut data: u32 = 0;
            let mut data_size = 4u32;
            let mut ty = REG_VALUE_TYPE::default();
            let _ = RegQueryValueExW(hkey, PCWSTR(val_w.as_ptr()), None, Some(&mut ty),
                Some((&mut data as *mut u32).cast()), Some(&mut data_size));
            let _ = RegCloseKey(hkey).ok();
            Ok(data != 0)
        }
    }
    #[cfg(not(windows))]
    { Ok(false) }
}

fn persist_ai_brightness_registry(enabled: bool) -> Result<()> {
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
            let key_w: Vec<u16> = OsStr::new(AI_BRIGHTNESS_REG_KEY).encode_wide().chain(Some(0)).collect();
            let mut hkey = std::mem::zeroed();
            RegCreateKeyExW(
                HKEY_LOCAL_MACHINE, PCWSTR(key_w.as_ptr()), 0, None,
                REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut hkey, None,
            ).ok().context("Create display settings key")?;
            let val_w: Vec<u16> = OsStr::new(AI_BRIGHTNESS_REG_VALUE).encode_wide().chain(Some(0)).collect();
            let val: u32 = if enabled { 1 } else { 0 };
            RegSetValueExW(hkey, PCWSTR(val_w.as_ptr()), 0, REG_DWORD, Some(&val.to_le_bytes()))
                .ok().context("Write AI brightness")?;
            let _ = RegCloseKey(hkey).ok();
        }
    }
    Ok(())
}
