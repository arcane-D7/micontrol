use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TouchpadInfo {
    pub sensitivity: TouchpadSensitivity,
    pub haptics_enabled: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TouchpadSensitivity {
    Low,
    Medium,
    High,
}

/// Fallback HID path used when hardware discovery has not yet found the touchpad.
/// The discovery module enumerates HID devices at first launch and replaces this.
const TOUCHPAD_HID_PATH_DEFAULT: &str =
    r"\\?\hid#bltp7853&col04#5&37166779&0&0003#{4d1e55b2-f16f-11cf-88cb-001111000030}";

/// Return the active touchpad HID path.
/// Uses the discovery profile when available; falls back to the BLTP7853 default.
fn touchpad_hid_path() -> String {
    crate::hw::discovery::global_profile()
        .and_then(|p| p.touchpad_hid_path.clone())
        .unwrap_or_else(|| TOUCHPAD_HID_PATH_DEFAULT.to_string())
}

const TP_REG_KEY: &str = r"SOFTWARE\MI\Touchpad";
const TP_REG_SENSITIVITY: &str = "Sensitivity";
const TP_REG_HAPTICS: &str = "HapticsEnabled";

pub fn get_touchpad_info() -> Result<TouchpadInfo> {
    let (sensitivity, haptics) = read_touchpad_registry().unwrap_or((TouchpadSensitivity::Medium, true));
    Ok(TouchpadInfo { sensitivity, haptics_enabled: haptics })
}

pub fn set_touchpad_sensitivity(sensitivity: TouchpadSensitivity) -> Result<()> {
    let report = build_sensitivity_report(&sensitivity);
    if let Err(e) = send_hid_output_report(&report) {
        log::warn!("HID sensitivity report failed: {e}");
    }
    persist_touchpad_registry(Some(sensitivity), None)
}

pub fn set_touchpad_haptics(enabled: bool) -> Result<()> {
    let report = build_haptics_report(enabled);
    if let Err(e) = send_hid_output_report(&report) {
        log::warn!("HID haptics report failed: {e}");
    }
    persist_touchpad_registry(None, Some(enabled))
}

// ── HID output report ────────────────────────────────────────────────────────

/// Build a 33-byte sensitivity output report for BLTP7853 COL04.
/// Report byte layout reverse-engineered from XiaomiPCManager HID writes:
/// [0]=report_id, [1]=cmd, [2]=param, [3..32]=0
fn build_sensitivity_report(sensitivity: &TouchpadSensitivity) -> [u8; 33] {
    let mut report = [0u8; 33];
    report[0] = 0x01; // report ID
    report[1] = 0x07; // command: set sensitivity
    report[2] = match sensitivity {
        TouchpadSensitivity::Low => 0x01,
        TouchpadSensitivity::Medium => 0x02,
        TouchpadSensitivity::High => 0x03,
    };
    report
}

fn build_haptics_report(enabled: bool) -> [u8; 33] {
    let mut report = [0u8; 33];
    report[0] = 0x01;
    report[1] = 0x08; // command: set haptics
    report[2] = if enabled { 0x01 } else { 0x00 };
    report
}

fn send_hid_output_report(report: &[u8; 33]) -> Result<()> {
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::{
            Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, GENERIC_WRITE},
            Storage::FileSystem::{
                CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE,
                OPEN_EXISTING,
            },
            Devices::HumanInterfaceDevice::HidD_SetOutputReport,
        };

        unsafe {
            let hid_path = touchpad_hid_path();
            let path_w: Vec<u16> = OsStr::new(&hid_path).encode_wide().chain(Some(0)).collect();
            let handle = CreateFileW(
                windows::core::PCWSTR(path_w.as_ptr()),
                GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                HANDLE::default(),
            ).context("Open touchpad HID device")?;

            if handle == INVALID_HANDLE_VALUE {
                anyhow::bail!("INVALID_HANDLE_VALUE for touchpad");
            }

            let ok = HidD_SetOutputReport(
                handle,
                report.as_ptr() as *mut _,
                report.len() as u32,
            );
            CloseHandle(handle).ok();

            if !ok.as_bool() {
                anyhow::bail!("HidD_SetOutputReport failed");
            }
        }
    }
    #[cfg(not(windows))]
    { let _ = report; }
    Ok(())
}

// ── Registry persistence ─────────────────────────────────────────────────────

fn persist_touchpad_registry(sensitivity: Option<TouchpadSensitivity>, haptics: Option<bool>) -> Result<()> {
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
            let key_w: Vec<u16> = OsStr::new(TP_REG_KEY).encode_wide().chain(Some(0)).collect();
            let mut hkey = std::mem::zeroed();
            RegCreateKeyExW(
                HKEY_LOCAL_MACHINE, PCWSTR(key_w.as_ptr()), 0, None,
                REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut hkey, None,
            ).ok().context("Create touchpad reg key")?;

            if let Some(s) = sensitivity {
                let v: u32 = match s { TouchpadSensitivity::Low => 1, TouchpadSensitivity::Medium => 2, TouchpadSensitivity::High => 3 };
                let val_w: Vec<u16> = OsStr::new(TP_REG_SENSITIVITY).encode_wide().chain(Some(0)).collect();
                let _ = RegSetValueExW(hkey, PCWSTR(val_w.as_ptr()), 0, REG_DWORD, Some(&v.to_le_bytes())).ok();
            }
            if let Some(h) = haptics {
                let v: u32 = if h { 1 } else { 0 };
                let val_w: Vec<u16> = OsStr::new(TP_REG_HAPTICS).encode_wide().chain(Some(0)).collect();
                let _ = RegSetValueExW(hkey, PCWSTR(val_w.as_ptr()), 0, REG_DWORD, Some(&v.to_le_bytes())).ok();
            }
            let _ = RegCloseKey(hkey).ok();
        }
    }
    Ok(())
}

fn read_touchpad_registry() -> Result<(TouchpadSensitivity, bool)> {
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_LOCAL_MACHINE, REG_VALUE_TYPE,
        };
        use windows::core::PCWSTR;
        unsafe {
            let key_w: Vec<u16> = OsStr::new(TP_REG_KEY).encode_wide().chain(Some(0)).collect();
            let mut hkey = std::mem::zeroed();
            if RegOpenKeyExW(HKEY_LOCAL_MACHINE, PCWSTR(key_w.as_ptr()), 0,
                windows::Win32::System::Registry::KEY_READ, &mut hkey).is_err() {
                return Ok((TouchpadSensitivity::Medium, true));
            }
            let mut ty = REG_VALUE_TYPE::default();

            let mut sens: u32 = 2;
            let mut size = 4u32;
            let sv_w: Vec<u16> = OsStr::new(TP_REG_SENSITIVITY).encode_wide().chain(Some(0)).collect();
            let _ = RegQueryValueExW(hkey, PCWSTR(sv_w.as_ptr()), None, Some(&mut ty),
                Some((&mut sens as *mut u32).cast()), Some(&mut size));

            let mut haptics: u32 = 1;
            let hv_w: Vec<u16> = OsStr::new(TP_REG_HAPTICS).encode_wide().chain(Some(0)).collect();
            let _ = RegQueryValueExW(hkey, PCWSTR(hv_w.as_ptr()), None, Some(&mut ty),
                Some((&mut haptics as *mut u32).cast()), Some(&mut size));

            let _ = RegCloseKey(hkey).ok();
            let sensitivity = match sens { 1 => TouchpadSensitivity::Low, 3 => TouchpadSensitivity::High, _ => TouchpadSensitivity::Medium };
            Ok((sensitivity, haptics != 0))
        }
    }
    #[cfg(not(windows))]
    { Ok((TouchpadSensitivity::Medium, true)) }
}
