use crate::state::PerformanceMode;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[cfg(windows)]
use {
    std::ffi::OsStr,
    std::os::windows::ffi::OsStrExt,
    windows::{
        core::GUID,
        Win32::{
            Devices::DeviceAndDriverInstallation::{
                SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
                SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT,
                SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W, SP_DEVINFO_DATA,
            },
            Foundation::{CloseHandle, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE},
            Storage::FileSystem::{
                CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE,
                OPEN_EXISTING,
            },
            System::{
                Registry::{
                    RegCloseKey, RegCreateKeyExW, RegOpenKeyExW, RegSetValueExW,
                    HKEY_LOCAL_MACHINE, KEY_WRITE, REG_CREATE_KEY_DISPOSITION, REG_DWORD,
                    REG_OPTION_NON_VOLATILE,
                },
                IO::DeviceIoControl,
            },
        },
    },
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

/// Set the performance mode via WMI (HQWmiCommonInterface) + registry + Windows power overlay.
/// Falls back to VHF, then registry-only if WMI is unavailable.
pub fn set_performance_mode(mode: PerformanceMode) -> Result<PerformanceResult> {
    // Always persist to registry
    persist_to_registry(mode)?;

    // Also sync the Windows 11 power overlay so the native power settings
    // slider reflects the change (requires this process to be elevated).
    set_windows_power_overlay(mode);

    // Attempt WMI HQWmiCommonInterface first (real TDP control on Xiaomi Book Pro 14)
    #[cfg(windows)]
    match send_via_hq_wmi(mode) {
        Ok(()) => {
            return Ok(PerformanceResult {
                success: true,
                method: "hq_wmi+registry+overlay".to_string(),
                mode,
            })
        }
        Err(e) => log::warn!("HQ WMI SetPerformanceMode failed, trying VHF: {e}"),
    }

    // Fall back to VHF device path
    match send_via_vhf(mode) {
        Ok(()) => Ok(PerformanceResult {
            success: true,
            method: "vhf+registry+overlay".to_string(),
            mode,
        }),
        Err(e) => {
            log::warn!("VHF send failed (using registry+overlay fallback): {e}");
            Ok(PerformanceResult {
                success: true,
                method: "registry+overlay".to_string(),
                mode,
            })
        }
    }
}

/// Read current performance mode from registry.
///
/// Priority:
///  1. Our own MI registry key — written by `set_performance_mode`, stores the
///     **exact** mode including Silence (0), Smart (10), and SmartAcceleration (14)
///     which all three Windows overlay GUIDs cannot distinguish on their own.
///  2. Windows power overlay GUID — used as fallback to reflect external changes
///     made by XiaomiPcManager or the Windows Settings power slider.
///
/// The previous approach (overlay first) meant that polling every 2 s would
/// silently revert Silence → LongBattery, Smart → Balance, and
/// SmartAcceleration → Turbo (because they share overlay GUIDs).
pub fn get_performance_mode() -> Result<PerformanceMode> {
    #[cfg(windows)]
    {
        // Prefer our own registry — contains exact mode value (0–14).
        if let Some(mode) = read_mi_registry_mode() {
            return Ok(mode);
        }
        // Fallback: Windows power overlay (set by external apps).
        if let Some(mode) = read_windows_power_overlay() {
            return Ok(mode);
        }
        Ok(PerformanceMode::Balance)
    }
    #[cfg(not(windows))]
    {
        Ok(PerformanceMode::Balance)
    }
}

#[cfg(windows)]
fn read_mi_registry_mode() -> Option<PerformanceMode> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{RegQueryValueExW, KEY_READ, REG_VALUE_TYPE};
    unsafe {
        // SAFETY: Wide strings are null-terminated; MaybeUninit<HKEY> is zero-sized before
        // RegOpenKeyExW writes to it. The hkey is assume_init only after the call succeeds.
        // The cast `(&mut data as *mut u32).cast()` is valid because u32 has no alignment
        // requirements stricter than the byte buffer RegQueryValueExW expects and data lives
        // on the stack.
        let key_w: Vec<u16> = OsStr::new(PERF_REG_KEY)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let mut hkey = std::mem::MaybeUninit::uninit();
        if RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(key_w.as_ptr()),
            0,
            KEY_READ,
            hkey.as_mut_ptr(),
        )
        .is_err()
        {
            return None;
        }
        let hkey = hkey.assume_init();
        let val_w: Vec<u16> = OsStr::new(PERF_REG_VALUE)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let mut data: u32 = 0;
        let mut data_size = 4u32;
        let mut ty = REG_VALUE_TYPE::default();
        let ok = RegQueryValueExW(
            hkey,
            PCWSTR(val_w.as_ptr()),
            None,
            Some(&mut ty),
            Some((&mut data as *mut u32).cast()),
            Some(&mut data_size),
        );
        let _ = RegCloseKey(hkey).ok();
        if ok.is_err() {
            return None;
        }
        Some(match data {
            0 => PerformanceMode::Silence,
            1 => PerformanceMode::Balance,
            2 => PerformanceMode::Turbo,
            3 => PerformanceMode::Decepticon,
            4 => PerformanceMode::Overdrive,
            5 => PerformanceMode::OverdriveHigh,
            6 => PerformanceMode::OverdriveMax,
            9 => PerformanceMode::SmartAdaptive,
            10 => PerformanceMode::Smart,
            11 => PerformanceMode::LongBattery,
            14 => PerformanceMode::SmartAcceleration,
            _ => return None,
        })
    }
}

// ── Windows power overlay sync ───────────────────────────────────────────────
//
// Windows 11 stores the active "power mode" overlay as a GUID in:
//   HKLM\SYSTEM\CurrentControlSet\Control\Power\User\PowerSchemes
//   Value: ActiveOverlayAcPowerScheme (REG_SZ, braces-formatted GUID)
//
// Three overlay GUIDs:
//   Best Power Efficiency : 961cc777-2547-4f9d-8174-7d86181b8a7a
//   Balanced              : 3af9b8d9-7c97-431d-ad78-34a8bfea439f
//   Best Performance      : ded574b5-45a0-4f42-8737-46345c09c238
//
// We set this via PowerSetActiveOverlayScheme (requires elevation).
// We read it via the registry (no elevation needed).

const OVERLAY_REG_KEY: &str = r"SYSTEM\CurrentControlSet\Control\Power\User\PowerSchemes";

const GUID_BEST_POWER_EFFICIENCY: &str = "961cc777-2547-4f9d-8174-7d86181b8a7a";
const GUID_BALANCED: &str = "3af9b8d9-7c97-431d-ad78-34a8bfea439f";
const GUID_BEST_PERFORMANCE: &str = "ded574b5-45a0-4f42-8737-46345c09c238";

#[cfg(windows)]
fn read_windows_power_overlay() -> Option<PerformanceMode> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
    use windows::Win32::System::Registry::{RegQueryValueExW, REG_VALUE_TYPE};

    // Determine current power source: AC (plugged in) = 1, DC (battery) = 0
    let on_ac = unsafe {
        // SAFETY: SYSTEM_POWER_STATUS is a POD struct; zero-initialization is valid for all
        // fields. GetSystemPowerStatus writes the actual values before we read ACLineStatus.
        let mut sps = std::mem::zeroed::<SYSTEM_POWER_STATUS>();
        GetSystemPowerStatus(&mut sps).is_ok() && sps.ACLineStatus == 1
    };
    let reg_value = if on_ac {
        "ActiveOverlayAcPowerScheme"
    } else {
        "ActiveOverlayDcPowerScheme"
    };

    unsafe {
        // SAFETY: Null-terminated wide strings, stack buffer aligned for u16 (REG_SZ data).
        // hkey is assume_init only after RegOpenKeyExW succeeds. buf is a fixed [u16; 64] on
        // the stack — well within the typical REG_SZ size for a GUID string.
        let key_w: Vec<u16> = OsStr::new(OVERLAY_REG_KEY)
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
            return None;
        }
        let hkey = hkey.assume_init();
        let val_w: Vec<u16> = OsStr::new(reg_value).encode_wide().chain(Some(0)).collect();
        let mut buf = [0u16; 64];
        let mut size = (buf.len() * 2) as u32;
        let mut ty = REG_VALUE_TYPE::default();
        if RegQueryValueExW(
            hkey,
            PCWSTR(val_w.as_ptr()),
            None,
            Some(&mut ty),
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
        .is_err()
        {
            let _ = RegCloseKey(hkey).ok();
            return None;
        }
        let _ = RegCloseKey(hkey).ok();
        let len = (size / 2).saturating_sub(1) as usize;
        let guid_str = String::from_utf16_lossy(&buf[..len]).to_lowercase();
        Some(if guid_str.contains("961cc777") {
            PerformanceMode::LongBattery
        } else if guid_str.contains("ded574b5") {
            PerformanceMode::Turbo
        } else {
            PerformanceMode::Balance
        })
    }
}

/// Called by the elevated process to also update the Windows power overlay.
/// 1. Calls PowerSetActiveOverlayScheme (powrprof.dll) — updates the LIVE
///    Windows power state so Settings / Task Manager reflect the change immediately.
/// 2. Writes the GUID to BOTH ActiveOverlayAcPowerScheme and
///    ActiveOverlayDcPowerScheme so the registry reflects the choice regardless
///    of whether the device is on AC or DC power.
#[cfg(windows)]
pub fn set_windows_power_overlay(mode: PerformanceMode) {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegSetValueExW, HKEY_LOCAL_MACHINE, KEY_SET_VALUE, REG_SZ,
    };

    let guid_str = match mode {
        PerformanceMode::Silence | PerformanceMode::LongBattery => GUID_BEST_POWER_EFFICIENCY,
        PerformanceMode::Turbo
        | PerformanceMode::Decepticon
        | PerformanceMode::SmartAcceleration
        | PerformanceMode::Overdrive
        | PerformanceMode::OverdriveHigh
        | PerformanceMode::OverdriveMax => GUID_BEST_PERFORMANCE,
        _ => GUID_BALANCED,
    };

    // ── 1. Call PowerSetActiveOverlayScheme via powrprof.dll ─────────────────
    // This updates the live Windows power state that Settings and Task Manager read.
    // The function is not bound in windows-rs 0.58, so we load it dynamically.
    #[repr(C)]
    struct WinGuid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    let guid_bytes: Option<WinGuid> = if guid_str.contains("961cc777") {
        Some(WinGuid {
            data1: 0x961cc777,
            data2: 0x2547,
            data3: 0x4f9d,
            data4: [0x81, 0x74, 0x7d, 0x86, 0x18, 0x1b, 0x8a, 0x7a],
        })
    } else if guid_str.contains("ded574b5") {
        Some(WinGuid {
            data1: 0xded574b5,
            data2: 0x45a0,
            data3: 0x4f42,
            data4: [0x87, 0x37, 0x46, 0x34, 0x5c, 0x09, 0xc2, 0x38],
        })
    } else {
        Some(WinGuid {
            data1: 0x3af9b8d9,
            data2: 0x7c97,
            data3: 0x431d,
            data4: [0xad, 0x78, 0x34, 0xa8, 0xbf, 0xea, 0x43, 0x9f],
        })
    };

    if let Some(guid) = guid_bytes {
        unsafe {
            // SAFETY: WinGuid matches the native GUID layout (u32/u16/u16/[u8;8]) used by
            // powrprof.dll. The Library reference lives on the stack for the duration of the call.
            // PowerSetActiveOverlayScheme only reads the GUID and does not retain the pointer.
            use libloading::Library;
            type FnSetOverlay = unsafe extern "system" fn(*const WinGuid) -> u32;
            if let Ok(lib) = Library::new("powrprof.dll") {
                if let Ok(f) = lib.get::<FnSetOverlay>(b"PowerSetActiveOverlayScheme\0") {
                    let ret = f(&guid as *const WinGuid);
                    log::debug!("PowerSetActiveOverlayScheme({mode:?}) → {ret}");
                } else {
                    log::warn!("PowerSetActiveOverlayScheme not found in powrprof.dll");
                }
            }
        }
    }

    // ── 2. Persist to registry (both AC and DC) ──────────────────────────────
    // Ensures get_performance_mode() reads the correct value regardless of
    // whether the device is currently on AC or DC power.
    let guid_with_braces = guid_str.to_string();
    let val_utf16: Vec<u16> = guid_with_braces.encode_utf16().chain(Some(0)).collect();
    let val_bytes: &[u8] =
        // SAFETY: val_utf16 is a Vec<u16> owned by this function; from_raw_parts creates a
        // byte slice over its backing memory for the exact byte length. No aliasing occurs.
        unsafe { std::slice::from_raw_parts(val_utf16.as_ptr().cast(), val_utf16.len() * 2) };

    let reg_values = ["ActiveOverlayAcPowerScheme", "ActiveOverlayDcPowerScheme"];

    unsafe {
        // SAFETY: Null-terminated wide strings; hkey is assume_init only after RegOpenKeyExW
        // succeeds. RegSetValueExW reads from the byte slice without retaining the pointer.
        let key_w: Vec<u16> = OsStr::new(OVERLAY_REG_KEY)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let mut hkey = std::mem::MaybeUninit::uninit();
        if RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(key_w.as_ptr()),
            0,
            KEY_SET_VALUE,
            hkey.as_mut_ptr(),
        )
        .is_err()
        {
            log::warn!("set_windows_power_overlay: cannot open registry key (needs elevation)");
            return;
        }
        let hkey = hkey.assume_init();
        for val_name_str in &reg_values {
            let val_name: Vec<u16> = OsStr::new(val_name_str)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let _ = RegSetValueExW(hkey, PCWSTR(val_name.as_ptr()), 0, REG_SZ, Some(val_bytes));
        }
        let _ = RegCloseKey(hkey).ok();
    }
    log::debug!("set_windows_power_overlay({mode:?}) → {guid_str} (AC+DC)");
}

#[cfg(not(windows))]
pub fn set_windows_power_overlay(_mode: PerformanceMode) {}

fn persist_to_registry(mode: PerformanceMode) -> Result<()> {
    #[cfg(windows)]
    {
        use windows::core::PCWSTR;
        unsafe {
            // SAFETY: Null-terminated wide strings; MaybeUninit<HKEY> written by RegCreateKeyExW
            // before assume_init. The DWORD value is a stack-local byte array with valid
            // alignment for RegSetValueExW.
            let key_w: Vec<u16> = OsStr::new(PERF_REG_KEY)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let mut hkey = std::mem::MaybeUninit::uninit();
            let mut disposition = REG_CREATE_KEY_DISPOSITION::default();
            // Use RegCreateKeyExW so the key is created if it does not yet exist
            RegCreateKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(key_w.as_ptr()),
                0,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                hkey.as_mut_ptr(),
                Some(&mut disposition),
            )
            .ok()
            .context("Create/open HKLM\\SOFTWARE\\MI\\PerformanceMode")?;
            let hkey = hkey.assume_init();

            let val_w: Vec<u16> = OsStr::new(PERF_REG_VALUE)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let hw_val = mode.to_hw_value();
            RegSetValueExW(
                hkey,
                PCWSTR(val_w.as_ptr()),
                0,
                REG_DWORD,
                Some(&hw_val.to_le_bytes()),
            )
            .ok()
            .context("Write DWORD to registry")?;
            let _ = RegCloseKey(hkey).ok();
        }
    }
    Ok(())
}

/// Send performance mode via `HQWmiCommonInterface.SetPerformanceMode` in root\WMI.
/// Query ROOT\WMI for the first active HQWmiCommonInterface instance name.
/// Returns the raw InstanceName string (e.g. "ACPI\\PNP0C14\\0_0").
#[cfg(windows)]
fn find_hq_wmi_instance_name(wmi: &wmi::WMIConnection) -> Result<String> {
    use std::collections::HashMap;
    let rows: Vec<HashMap<String, wmi::Variant>> = wmi
        .raw_query("SELECT InstanceName FROM HQWmiCommonInterface WHERE Active = TRUE")
        .context("WMI query HQWmiCommonInterface")?;
    rows.into_iter()
        .next()
        .and_then(|row| match row.get("InstanceName") {
            Some(wmi::Variant::String(s)) => Some(s.clone()),
            _ => None,
        })
        .context("No active HQWmiCommonInterface instance found in ROOT\\WMI")
}

/// This is the primary TDP-level control channel on Xiaomi Book Pro 14
/// (uses ACPI\PNP0C14\0 device, WmiMethodId=9).
#[cfg(windows)]
fn send_via_hq_wmi(mode: PerformanceMode) -> Result<()> {
    use windows::core::{BSTR, VARIANT};
    use windows::Win32::System::Wmi::{WBEM_FLAG_RETURN_WBEM_COMPLETE, WBEM_GENERIC_FLAG_TYPE};
    use wmi::{COMLibrary, WMIConnection};

    let com = COMLibrary::without_security().context("HQ WMI: COM init")?;
    let wmi = WMIConnection::with_namespace_path("ROOT\\WMI", com)
        .context("HQ WMI: connect root\\WMI")?;

    // Dynamically discover the active HQWmiCommonInterface instance name instead
    // of hardcoding "ACPI\PNP0C14\0_0" — firmware revisions (Panther Lake, future
    // Xiaomi hardware) may use a different ACPI path.
    let instance_name = find_hq_wmi_instance_name(&wmi)
        .context("HQ WMI: cannot find active HQWmiCommonInterface instance")?;

    let mode_str = mode.to_hw_value().to_string();

    // WMI object-path format requires backslashes in key values to be escaped as \\.
    // e.g. InstanceName="ACPI\PNP0C14\0_0"  →  "ACPI\\PNP0C14\\0_0" in the path string.
    let escaped = instance_name.replace('\\', "\\\\");
    let instance_path = BSTR::from(format!("HQWmiCommonInterface.InstanceName=\"{escaped}\""));
    let method_name = BSTR::from("SetPerformanceMode");

    unsafe {
        // SAFETY: WMI COM pointers (class_obj, in_sig, in_params, out_params) are ref-counted
        // and returned by the WMI infrastructure. All BSTRs and VARIANTs are stack-local and
        // valid for the duration of each call. The raw pointers to in_sig are only used as
        // output parameters for GetMethod and not dereferenced outside that call.
        // Get the class definition to spawn a parameter object
        let mut class_obj = None;
        wmi.svc
            .GetObject(
                &BSTR::from("HQWmiCommonInterface"),
                WBEM_FLAG_RETURN_WBEM_COMPLETE,
                None,
                Some(&mut class_obj),
                None,
            )
            .context("HQ WMI: GetObject class")?;
        let class_obj = class_obj.context("HQ WMI: class object is None")?;

        // Get the in-params class for SetPerformanceMode
        let mut in_sig: Option<windows::Win32::System::Wmi::IWbemClassObject> = None;
        class_obj
            .GetMethod(&method_name, 0, &mut in_sig as *mut _, std::ptr::null_mut())
            .context("HQ WMI: GetMethod")?;
        let in_sig = in_sig.context("HQ WMI: in-params class is None")?;

        // Spawn an instance of the in-params class
        let in_params = in_sig.SpawnInstance(0).context("HQ WMI: SpawnInstance")?;

        // Set req = mode string (e.g. "1" for Balance)
        let req_variant = VARIANT::from(BSTR::from(mode_str.as_str()));
        in_params
            .Put(&BSTR::from("req"), 0, &req_variant, 0)
            .context("HQ WMI: Put req")?;

        // Execute the method on the specific instance
        let mut out_params = None;
        wmi.svc
            .ExecMethod(
                &instance_path,
                &method_name,
                WBEM_GENERIC_FLAG_TYPE(0),
                None,
                Some(&in_params),
                Some(&mut out_params),
                None,
            )
            .context("HQ WMI: ExecMethod")?;

        if let Some(out) = out_params {
            let mut ret_v = VARIANT::default();
            let _ = out.Get(&BSTR::from("ret"), 0, &mut ret_v, None, None);
            let ret_str = BSTR::try_from(&ret_v)
                .map(|b| b.to_string())
                .unwrap_or_default();
            log::debug!("HQ WMI SetPerformanceMode({mode:?}) → {ret_str}");
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
        let path_w: Vec<u16> = OsStr::new(&device_path)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let handle = CreateFileW(
            windows::core::PCWSTR(path_w.as_ptr()),
            GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            HANDLE::default(),
        )
        .context("Open VHF device")?;

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
        // SAFETY: SP_DEVICE_INTERFACE_DATA is POD; zeroed is valid. SetupDiGetClassDevsW
        // returns a valid HDEVINFO; SetupDiEnumDeviceInterfaces fills iface_data. The device
        // path pointer arithmetic (offset 4 in the detail struct) matches the
        // SP_DEVICE_INTERFACE_DETAIL_DATA_W layout documented by Microsoft.
        let dev_info = SetupDiGetClassDevsW(
            Some(&guid),
            None,
            None,
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
        .context("SetupDiGetClassDevsW")?;

        let mut iface_data = SP_DEVICE_INTERFACE_DATA {
            cbSize: std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
            ..std::mem::zeroed()
        };

        let result =
            if SetupDiEnumDeviceInterfaces(dev_info, None, &guid, 0, &mut iface_data).is_ok() {
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
                )
                .context("SetupDiGetDeviceInterfaceDetailW")?;

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

// ── Debug / diagnostic ────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PerfDebugInfo {
    /// Name discovered from HQWmiCommonInterface (null = class not found)
    pub hq_wmi_instance: Option<String>,
    /// Whether SetPerformanceMode("1") returned "Success" in a live test
    pub hq_wmi_works: bool,
    /// Return string from the test call (empty on failure)
    pub hq_wmi_test_ret: String,
    /// VHF device path from discovery cache (null = not found)
    pub vhf_device_path: Option<String>,
    /// Current mode as stored in our registry key
    pub registry_mode: String,
    /// Current Windows power overlay GUID (truncated)
    pub overlay_mode: String,
}

pub fn get_perf_debug() -> PerfDebugInfo {
    #[cfg(windows)]
    {
        use windows::core::{BSTR, VARIANT};
        use windows::Win32::System::Wmi::WBEM_GENERIC_FLAG_TYPE;
        use wmi::{COMLibrary, WMIConnection};

        let mut info = PerfDebugInfo {
            hq_wmi_instance: None,
            hq_wmi_works: false,
            hq_wmi_test_ret: String::new(),
            vhf_device_path: None,
            registry_mode: String::from("unknown"),
            overlay_mode: String::from("unknown"),
        };

        // Registry mode
        if let Some(mode) = read_mi_registry_mode() {
            info.registry_mode = format!("{mode:?}");
        }
        // Overlay mode
        if let Some(mode) = read_windows_power_overlay() {
            info.overlay_mode = format!("{mode:?}");
        }
        // VHF path
        info.vhf_device_path =
            crate::hw::discovery::global_profile().and_then(|p| p.vhf_device_path.clone());

        // WMI check
        let Ok(com) = COMLibrary::without_security() else {
            return info;
        };
        let Ok(wmi) = WMIConnection::with_namespace_path("ROOT\\WMI", com) else {
            return info;
        };

        if let Ok(name) = find_hq_wmi_instance_name(&wmi) {
            info.hq_wmi_instance = Some(name.clone());

            // Live test: call SetPerformanceMode("1") and capture return
            let escaped = name.replace('\\', "\\\\");
            let instance_path =
                BSTR::from(format!("HQWmiCommonInterface.InstanceName=\"{escaped}\""));
            let method_name = BSTR::from("SetPerformanceMode");

            // Wrap in a closure so `?` propagates into anyhow::Result correctly.
            let test_ok: anyhow::Result<String> = (|| -> anyhow::Result<String> {
                unsafe {
                    // SAFETY: Same WMI COM pattern as send_via_hq_wmi — the class objects and
                    // in/out params are ref-counted COM pointers. All BSTR/VARIANT temporaries
                    // live on the stack for the duration of each call.
                    let mut class_obj = None;
                    wmi.svc
                        .GetObject(
                            &BSTR::from("HQWmiCommonInterface"),
                            windows::Win32::System::Wmi::WBEM_FLAG_RETURN_WBEM_COMPLETE,
                            None,
                            Some(&mut class_obj),
                            None,
                        )
                        .context("GetObject")?;
                    let class_obj = class_obj.context("class obj none")?;

                    let mut in_sig: Option<windows::Win32::System::Wmi::IWbemClassObject> = None;
                    class_obj
                        .GetMethod(&method_name, 0, &mut in_sig as *mut _, std::ptr::null_mut())
                        .context("GetMethod")?;
                    let in_sig = in_sig.context("in_sig none")?;
                    let in_params = in_sig.SpawnInstance(0).context("SpawnInstance")?;

                    let req_v = VARIANT::from(BSTR::from("1"));
                    in_params
                        .Put(&BSTR::from("req"), 0, &req_v, 0)
                        .context("Put req")?;

                    let mut out_params = None;
                    wmi.svc
                        .ExecMethod(
                            &instance_path,
                            &method_name,
                            WBEM_GENERIC_FLAG_TYPE(0),
                            None,
                            Some(&in_params),
                            Some(&mut out_params),
                            None,
                        )
                        .context("ExecMethod")?;

                    let ret = out_params
                        .and_then(|out| {
                            let mut v = VARIANT::default();
                            out.Get(&BSTR::from("ret"), 0, &mut v, None, None).ok()?;
                            BSTR::try_from(&v).ok().map(|b| b.to_string())
                        })
                        .unwrap_or_default();
                    Ok(ret)
                }
            })();

            match test_ok {
                Ok(ret) => {
                    info.hq_wmi_works = ret.to_lowercase().contains("success");
                    info.hq_wmi_test_ret = ret;
                }
                Err(e) => {
                    info.hq_wmi_test_ret = format!("ERROR: {e}");
                }
            }
        }

        info
    }
    #[cfg(not(windows))]
    {
        PerfDebugInfo {
            hq_wmi_instance: None,
            hq_wmi_works: false,
            hq_wmi_test_ret: String::from("not Windows"),
            vhf_device_path: None,
            registry_mode: String::from("n/a"),
            overlay_mode: String::from("n/a"),
        }
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
        assert_eq!(PerformanceMode::Overdrive.to_hw_value(), 4);
        assert_eq!(PerformanceMode::OverdriveHigh.to_hw_value(), 5);
        assert_eq!(PerformanceMode::OverdriveMax.to_hw_value(), 6);
        assert_eq!(PerformanceMode::SmartAdaptive.to_hw_value(), 9);
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
            PerformanceMode::Overdrive,
            PerformanceMode::OverdriveHigh,
            PerformanceMode::OverdriveMax,
            PerformanceMode::SmartAdaptive,
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
