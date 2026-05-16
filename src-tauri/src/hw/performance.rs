use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use crate::state::PerformanceMode;

#[cfg(windows)]
use {
    windows::{
        core::GUID,
        Win32::{
            Devices::DeviceAndDriverInstallation::{
                SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces,
                SetupDiGetClassDevsW, SetupDiGetDeviceInterfaceDetailW,
                DIGCF_DEVICEINTERFACE, DIGCF_PRESENT,
                SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
                SP_DEVINFO_DATA,
            },
            Foundation::{CloseHandle, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE},
            Storage::FileSystem::{
                CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE,
                OPEN_EXISTING,
            },
            System::{
                IO::DeviceIoControl,
                Registry::{RegCloseKey, RegOpenKeyExW, RegSetValueExW, HKEY_LOCAL_MACHINE, KEY_WRITE, REG_DWORD},
            },
        },
    },
    std::ffi::OsStr,
    std::os::windows::ffi::OsStrExt,
};

/// Registry key for last-known performance mode (used as a fallback / state persistence).
const PERF_REG_KEY: &str = r"SOFTWARE\MI\PerformanceMode";
const PERF_REG_VALUE: &str = "LastLongBattery";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PerformanceResult {
    pub success: bool,
    pub method: String,
    pub mode: PerformanceMode,
}

/// Set the performance mode via VHF device + registry.
/// Falls back to registry-only if the VHF device cannot be opened.
pub fn set_performance_mode(mode: PerformanceMode) -> Result<PerformanceResult> {
    // Always persist to registry
    persist_to_registry(mode)?;

    // Attempt VHF device path
    match send_via_vhf(mode) {
        Ok(()) => Ok(PerformanceResult {
            success: true,
            method: "vhf+registry".to_string(),
            mode,
        }),
        Err(e) => {
            log::warn!("VHF send failed (using registry fallback): {e}");
            Ok(PerformanceResult {
                success: true,
                method: "registry".to_string(),
                mode,
            })
        }
    }
}

/// Read current performance mode from registry.
pub fn get_performance_mode() -> Result<PerformanceMode> {
    #[cfg(windows)]
    {
        use windows::Win32::System::Registry::{RegQueryValueExW, REG_VALUE_TYPE};
        use windows::core::PCWSTR;
        unsafe {
            let key_w: Vec<u16> = OsStr::new(PERF_REG_KEY).encode_wide().chain(Some(0)).collect();
            let mut hkey = std::mem::zeroed();
            let res = RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(key_w.as_ptr()),
                0,
                windows::Win32::System::Registry::KEY_READ,
                &mut hkey,
            );
            if res.is_err() {
                return Ok(PerformanceMode::Balance);
            }
            let val_w: Vec<u16> = OsStr::new(PERF_REG_VALUE).encode_wide().chain(Some(0)).collect();
            let mut data: u32 = 0;
            let mut data_size = 4u32;
            let mut ty = REG_VALUE_TYPE::default();
            let _ = RegQueryValueExW(
                hkey,
                PCWSTR(val_w.as_ptr()),
                None,
                Some(&mut ty),
                Some((&mut data as *mut u32).cast()),
                Some(&mut data_size),
            );
            let _ = RegCloseKey(hkey).ok();
            Ok(match data {
                0 => PerformanceMode::Silence,
                1 => PerformanceMode::Balance,
                2 => PerformanceMode::Turbo,
                3 => PerformanceMode::Decepticon,
                10 => PerformanceMode::Smart,
                11 => PerformanceMode::LongBattery,
                14 => PerformanceMode::SmartAcceleration,
                _ => PerformanceMode::Balance,
            })
        }
    }
    #[cfg(not(windows))]
    { Ok(PerformanceMode::Balance) }
}

// ── Private helpers ──────────────────────────────────────────────────────────

fn persist_to_registry(mode: PerformanceMode) -> Result<()> {
    #[cfg(windows)]
    {
        use windows::core::PCWSTR;
        unsafe {
            let key_w: Vec<u16> = OsStr::new(PERF_REG_KEY).encode_wide().chain(Some(0)).collect();
            let mut hkey = std::mem::zeroed();
            RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(key_w.as_ptr()),
                0,
                KEY_WRITE,
                &mut hkey,
            ).ok().context("Open HKLM\\SOFTWARE\\MI\\PerformanceMode")?;

            let val_w: Vec<u16> = OsStr::new(PERF_REG_VALUE).encode_wide().chain(Some(0)).collect();
            let hw_val = mode.to_hw_value();
            RegSetValueExW(
                hkey,
                PCWSTR(val_w.as_ptr()),
                0,
                REG_DWORD,
                Some(&hw_val.to_le_bytes()),
            ).ok().context("Write DWORD to registry")?;
            let _ = RegCloseKey(hkey).ok();
        }
    }
    Ok(())
}

#[cfg(windows)]
fn send_via_vhf(mode: PerformanceMode) -> Result<()> {
    // Try the path cached during startup discovery; avoids repeated SetupDi enumeration.
    let device_path = crate::hw::discovery::global_profile()
        .and_then(|p| p.vhf_device_path.clone())
        .map(Ok)
        .unwrap_or_else(find_vhf_device_path)?;
    let hw_val = mode.to_hw_value();

    unsafe {
        let path_w: Vec<u16> = OsStr::new(&device_path).encode_wide().chain(Some(0)).collect();
        let handle = CreateFileW(
            windows::core::PCWSTR(path_w.as_ptr()),
            GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            HANDLE::default(),
        ).context("Open VHF device")?;

        if handle == INVALID_HANDLE_VALUE {
            anyhow::bail!("INVALID_HANDLE_VALUE for VHF device");
        }

        // IOCTL_HID_SET_FEATURE = 0x000B0191
        const IOCTL_HID_SET_FEATURE: u32 = 0x000B0191;
        let payload = hw_val.to_le_bytes();
        let mut bytes_ret = 0u32;
        let ok = DeviceIoControl(
            handle,
            IOCTL_HID_SET_FEATURE,
            Some(payload.as_ptr().cast()),
            payload.len() as u32,
            None,
            0,
            Some(&mut bytes_ret),
            None,
        );

        CloseHandle(handle).ok();
        ok.context("DeviceIoControl to VHF device")?;
    }
    Ok(())
}

#[cfg(not(windows))]
fn send_via_vhf(_mode: PerformanceMode) -> Result<()> {
    anyhow::bail!("VHF not available on non-Windows")
}

#[cfg(windows)]
fn find_vhf_device_path() -> Result<String> {
    // VHF_PERF_GUID = "0CC99493-EB87-54F5-BB10-C0D5EA4A4F4C"
    let guid = GUID {
        data1: 0x0CC99493,
        data2: 0xEB87,
        data3: 0x54F5,
        data4: [0xBB, 0x10, 0xC0, 0xD5, 0xEA, 0x4A, 0x4F, 0x4C],
    };

    unsafe {
        let dev_info = SetupDiGetClassDevsW(
            Some(&guid),
            None,
            None,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        ).context("SetupDiGetClassDevsW")?;

        let mut iface_data = SP_DEVICE_INTERFACE_DATA {
            cbSize: std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
            ..std::mem::zeroed()
        };

        let result = if SetupDiEnumDeviceInterfaces(
            dev_info,
            None,
            &guid,
            0,
            &mut iface_data,
        ).is_ok() {
            let mut required_size = 0u32;
            // First call: get required size
            let _ = SetupDiGetDeviceInterfaceDetailW(
                dev_info,
                &iface_data,
                None,
                0,
                Some(&mut required_size),
                None,
            );

            let buf_size = required_size as usize;
            let mut buf = vec![0u8; buf_size];
            let detail = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail).cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;

            let mut devinfo_data = SP_DEVINFO_DATA {
                cbSize: std::mem::size_of::<SP_DEVINFO_DATA>() as u32,
                ..std::mem::zeroed()
            };

            SetupDiGetDeviceInterfaceDetailW(
                dev_info,
                &iface_data,
                Some(detail),
                buf_size as u32,
                None,
                Some(&mut devinfo_data),
            ).context("SetupDiGetDeviceInterfaceDetailW")?;

            // DevicePath is at offset 4 in SP_DEVICE_INTERFACE_DETAIL_DATA_W
            let path_ptr = buf.as_ptr().add(4) as *const u16;
            let len = (0..).take_while(|&i| *path_ptr.add(i) != 0).count();
            let path_slice = std::slice::from_raw_parts(path_ptr, len);
            Ok(String::from_utf16_lossy(path_slice))
        } else {
            Err(anyhow::anyhow!("No VHF device interface found"))
        };

        SetupDiDestroyDeviceInfoList(dev_info).ok();
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::PerformanceMode;

    #[test]
    fn hw_values_are_correct() {
        assert_eq!(PerformanceMode::Silence.to_hw_value(), 0);
        assert_eq!(PerformanceMode::Balance.to_hw_value(), 1);
        assert_eq!(PerformanceMode::Turbo.to_hw_value(), 2);
        assert_eq!(PerformanceMode::Decepticon.to_hw_value(), 3);
        assert_eq!(PerformanceMode::Smart.to_hw_value(), 10);
        assert_eq!(PerformanceMode::LongBattery.to_hw_value(), 11);
        assert_eq!(PerformanceMode::SmartAcceleration.to_hw_value(), 14);
    }

    #[test]
    fn serialization_roundtrip() {
        for mode in [
            PerformanceMode::Silence,
            PerformanceMode::Balance,
            PerformanceMode::Turbo,
            PerformanceMode::Smart,
            PerformanceMode::LongBattery,
            PerformanceMode::SmartAcceleration,
        ] {
            let json = serde_json::to_string(&mode).expect("serialize");
            let back: PerformanceMode = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(mode, back, "Roundtrip failed for {mode:?}");
        }
    }

    #[test]
    fn get_performance_mode_returns_valid() {
        let mode = get_performance_mode().expect("should return a mode");
        let json = serde_json::to_string(&mode).expect("should serialize");
        assert!(!json.is_empty());
    }
}
