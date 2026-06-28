//! WMI MiInterface — direct EC/hardware access via ACPI WMAA method.
//!
//! This module provides access to hardware sensors and controls through the
//! WMI `MICommonInterface.MiInterface` method in the `ROOT\WMI` namespace.
//!
//! The method wraps the ACPI `WMAA` method (device `WMID`, `_UID = "MIFS"`)
//! and accepts a 32-byte input buffer with the following layout:
//!
//! ```text
//! Offset  Size  Field  Description
//!   0     word  FUN1   0xFA00 = read, 0xFB00 = write
//!   2     word  FUN2   Sub-command group (0x0800, 0x0A00, 0x0C00, 0x1000)
//!   4     word  FUN3   Parameter / sub-command ID
//!   6     dword FUN4   Additional data (for write commands)
//!  10-31        (padding to 32 bytes)
//! ```
//!
//! The output is a 30-byte array with the same layout:
//!
//! ```text
//! Offset  Size  Field  Description
//!   0     word  SGER   0x8000 = success, 0xE000 = error
//!   2     word  FUTR   Echoes FUN2
//!   4     word  FRD0   Echoes FUN3 (or result data)
//!   6     dword FRD1   Result data (primary return value)
//!  10     dword FRD2   Extended result data
//!  14     dword FRD3   Extended result data
//! ```

use crate::hw::errors::{HardwareError, HardwareResult};
use serde::{Deserialize, Serialize};

/// Required input buffer size (32 bytes).
const BUFFER_SIZE: usize = 32;

// FUN1 values
const FUN1_READ: u16 = 0xFA00;
const FUN1_WRITE: u16 = 0xFB00;

// FUN2 sub-command groups
const FUN2_EC_FUNC: u16 = 0x0800;
const FUN2_MI_INFO: u16 = 0x0A00;
const FUN2_MISC: u16 = 0x0C00;
const FUN2_SENSOR: u16 = 0x1000;

// SGER (status) values
const SGER_SUCCESS: u16 = 0x8000;

/// WMAA response buffer (30 bytes from WMI output).
#[derive(Debug, Clone)]
pub struct WmaaResponse {
    pub sger: u16,
    pub futr: u16,
    pub frd0: u16,
    pub frd1: u32,
    pub frd2: u32,
    pub frd3: u32,
    pub raw: Vec<u8>,
}

impl WmaaResponse {
    /// Returns `true` if the ACPI method reported success.
    pub fn is_success(&self) -> bool {
        self.sger == SGER_SUCCESS
    }

    /// Parse a raw byte array into a WmaaResponse.
    fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 10 {
            return None;
        }
        Some(Self {
            sger: u16::from_le_bytes([data[0], data[1]]),
            futr: u16::from_le_bytes([data[2], data[3]]),
            frd0: u16::from_le_bytes([data[4], data[5]]),
            frd1: u32::from_le_bytes([data[6], data[7], data[8], data[9]]),
            frd2: if data.len() >= 14 {
                u32::from_le_bytes([data[10], data[11], data[12], data[13]])
            } else {
                0
            },
            frd3: if data.len() >= 18 {
                u32::from_le_bytes([data[14], data[15], data[16], data[17]])
            } else {
                0
            },
            raw: data.to_vec(),
        })
    }
}

/// Build a 32-byte WMAA input buffer.
fn make_buffer(fun1: u16, fun2: u16, fun3: u16, fun4: u32) -> [u8; BUFFER_SIZE] {
    let mut buf = [0u8; BUFFER_SIZE];
    buf[0..2].copy_from_slice(&fun1.to_le_bytes());
    buf[2..4].copy_from_slice(&fun2.to_le_bytes());
    buf[4..6].copy_from_slice(&fun3.to_le_bytes());
    buf[6..10].copy_from_slice(&fun4.to_le_bytes());
    buf
}

/// Performance mode IDs (FUN3 for FUN2=0x0800 write commands).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u16)]
pub enum EcPerformanceMode {
    Performance = 5,
    Balanced = 6,
    Quiet = 7,
    SuperQuiet = 8,
    UltraPerformance = 9,
    Extreme = 0x0A,
}

impl EcPerformanceMode {
    pub fn from_raw(val: u16) -> Option<Self> {
        match val {
            5 => Some(Self::Performance),
            6 => Some(Self::Balanced),
            7 => Some(Self::Quiet),
            8 => Some(Self::SuperQuiet),
            9 => Some(Self::UltraPerformance),
            0x0A => Some(Self::Extreme),
            _ => None,
        }
    }
}

/// Sensor data read from the EC via WMI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EcSensorData {
    pub battery_health: u32,
    pub adapter_power: u32,
    pub mi_usage_type: u32,
    pub wmid_type: u32,
    pub lid_open_type: u32,
    pub removable_type: u32,
    pub current_mode: u16,
}

// ── WMI method call implementation ──────────────────────────────────────────

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::hw::wmi_cache;
    use windows::core::{BSTR, HSTRING, PCWSTR, VARIANT};
    use windows::Win32::System::Com::SAFEARRAY as Win32SAFEARRAY;
    use windows::Win32::System::Ole::{
        SafeArrayAccessData, SafeArrayCreateVector, SafeArrayGetLBound, SafeArrayGetUBound,
        SafeArrayPutElement, SafeArrayUnaccessData,
    };
    use windows::Win32::System::Variant::VT_UI1 as Win32_VT_UI1;
    use windows::Win32::System::Wmi::{
        IWbemClassObject, WBEM_FLAG_FORWARD_ONLY, WBEM_FLAG_RETURN_IMMEDIATELY,
        WBEM_GENERIC_FLAG_TYPE,
    };

    // VT_ARRAY is 0x2000 — not available in windows::core::imp, so we define it.
    const VT_ARRAY_VAL: u16 = 0x2000;
    // VT_UI1 from imp is u16 (17).
    const VT_UI1_VAL: u16 = windows::core::imp::VT_UI1;
    const VT_BSTR_VAL: u16 = windows::core::imp::VT_BSTR;
    const VT_I2_VAL: u16 = windows::core::imp::VT_I2;
    const VT_UI2_VAL: u16 = windows::core::imp::VT_UI2;
    const VT_I4_VAL: u16 = windows::core::imp::VT_I4;
    const VT_UI4_VAL: u16 = windows::core::imp::VT_UI4;

    /// Helper to create a PCWSTR from a string.
    fn pcwstr(s: &str) -> PCWSTR {
        let h = HSTRING::from(s);
        PCWSTR::from_raw(h.as_ptr())
    }

    /// Call the WMI MiInterface method with the given WMAA parameters.
    pub fn wmi_call(fun1: u16, fun2: u16, fun3: u16, fun4: u32) -> HardwareResult<WmaaResponse> {
        wmi_cache::with_wmi(|conn| {
            let svc = &conn.svc;

            // Build the input buffer
            let buf = make_buffer(fun1, fun2, fun3, fun4);

            // Create a SAFEARRAY of bytes for InData
            let psa = unsafe { SafeArrayCreateVector(Win32_VT_UI1, 0, BUFFER_SIZE as u32) };
            if psa.is_null() {
                return Err(anyhow::anyhow!(HardwareError::Wmi(
                    "SafeArrayCreateVector returned null".into()
                )));
            }

            // Fill the SAFEARRAY
            for (i, &byte) in buf.iter().enumerate() {
                let idx = i as i32;
                unsafe {
                    SafeArrayPutElement(psa, &idx, &byte as *const u8 as *const _)?;
                }
            }

            // Get the MICommonInterface class object
            let class_path = BSTR::from("MICommonInterface");
            let mut class_obj: Option<IWbemClassObject> = None;
            unsafe {
                svc.GetObject(
                    &class_path,
                    WBEM_GENERIC_FLAG_TYPE(0),
                    None,
                    Some(&mut class_obj),
                    None,
                )?;
            }
            let class_obj = class_obj.ok_or_else(|| {
                anyhow::anyhow!(HardwareError::Wmi("GetObject returned null".into()))
            })?;

            // Get method signature — use GetMethod
            let method_name = pcwstr("MiInterface");
            let mut in_sig: Option<IWbemClassObject> = None;
            let mut out_sig: Option<IWbemClassObject> = None;
            unsafe {
                class_obj.GetMethod(method_name, 0, &mut in_sig, &mut out_sig)?;
            }

            // Spawn input parameters instance
            let in_sig = in_sig.ok_or_else(|| {
                anyhow::anyhow!(HardwareError::Wmi("GetMethod returned null in_sig".into()))
            })?;
            let in_params = unsafe { in_sig.SpawnInstance(0)? };

            // Set InData property — construct a VARIANT containing a SAFEARRAY of VT_UI1
            let prop_name = pcwstr("InData");
            let in_data_var = build_byte_array_variant(&buf)?;

            unsafe { in_params.Put(prop_name, 0, &in_data_var, 0)? };

            // Query for the MICommonInterface instance
            let query = BSTR::from("SELECT * FROM MICommonInterface");
            let wql = BSTR::from("WQL");
            let flags = WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY;
            let enumerator = unsafe { svc.ExecQuery(&wql, &query, flags, None)? };

            // Get first instance — Next returns HRESULT, takes array + u32 count
            let mut objs: [Option<IWbemClassObject>; 1] = [None];
            let mut returned: u32 = 0;
            let hr = unsafe { enumerator.Next(-1, &mut objs, &mut returned) };
            hr.ok()?;

            let instance = objs[0].as_ref().ok_or_else(|| {
                anyhow::anyhow!(HardwareError::Wmi(
                    "No MICommonInterface instance found".into()
                ))
            })?;

            // Get the instance path
            let path_name = pcwstr("__Path");
            let mut path_var = VARIANT::new();
            let mut cim_type: i32 = 0;
            unsafe {
                instance.Get(path_name, 0, &mut path_var, Some(&mut cim_type), None)?;
            }

            // Convert path VARIANT to string
            let path_str = variant_to_string(&path_var)?;

            // Call ExecMethod on IWbemServices
            let path_bstr = BSTR::from(path_str);
            let method_bstr = BSTR::from("MiInterface");
            let mut out_params: Option<IWbemClassObject> = None;
            unsafe {
                svc.ExecMethod(
                    &path_bstr,
                    &method_bstr,
                    WBEM_GENERIC_FLAG_TYPE(0),
                    None,
                    &in_params,
                    Some(&mut out_params),
                    None,
                )?;
            }

            let out_params = out_params.ok_or_else(|| {
                anyhow::anyhow!(HardwareError::Wmi(
                    "ExecMethod returned null out_params".into()
                ))
            })?;

            // Read ReturnCode
            let rc_name = pcwstr("ReturnCode");
            let mut rc_var = VARIANT::new();
            unsafe {
                out_params.Get(rc_name, 0, &mut rc_var, None, None)?;
            }
            let return_code = variant_to_u16(&rc_var)?;
            if return_code != 0 {
                return Err(anyhow::anyhow!(HardwareError::Wmi(format!(
                    "MiInterface returned error code: {return_code}"
                ))));
            }

            // Read OutData
            let od_name = pcwstr("OutData");
            let mut od_var = VARIANT::new();
            unsafe {
                out_params.Get(od_name, 0, &mut od_var, None, None)?;
            }
            let raw = variant_to_bytes(&od_var)?;

            WmaaResponse::parse(&raw).ok_or_else(|| {
                anyhow::anyhow!(HardwareError::Wmi(format!(
                    "OutData too short: {} bytes",
                    raw.len()
                )))
            })
        })
    }

    /// Build a VARIANT containing a SAFEARRAY of bytes.
    fn build_byte_array_variant(data: &[u8]) -> anyhow::Result<VARIANT> {
        let psa = unsafe { SafeArrayCreateVector(Win32_VT_UI1, 0, data.len() as u32) };
        if psa.is_null() {
            return Err(anyhow::anyhow!(HardwareError::Wmi(
                "SafeArrayCreateVector returned null".into()
            )));
        }

        for (i, &byte) in data.iter().enumerate() {
            let idx = i as i32;
            unsafe {
                SafeArrayPutElement(psa, &idx, &byte as *const u8 as *const _)?;
            }
        }

        // Construct VARIANT from raw using from_raw.
        // The raw VARIANT struct uses imp::bindings::SAFEARRAY, which is a different
        // type than Win32::System::Com::SAFEARRAY but has the same layout.
        // We cast through *mut c_void to avoid naming the private bindings type.
        let parray_imp = psa as *mut std::ffi::c_void as *mut _;
        let raw = windows::core::imp::VARIANT {
            Anonymous: windows::core::imp::VARIANT_0 {
                Anonymous: windows::core::imp::VARIANT_0_0 {
                    vt: VT_UI1_VAL | VT_ARRAY_VAL,
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: windows::core::imp::VARIANT_0_0_0 { parray: parray_imp },
                },
            },
        };

        Ok(unsafe { VARIANT::from_raw(raw) })
    }

    /// Convert a VARIANT containing a SAFEARRAY of bytes into a Vec<u8>.
    fn variant_to_bytes(var: &VARIANT) -> anyhow::Result<Vec<u8>> {
        let raw = var.as_raw();
        let vt: u16 = unsafe { raw.Anonymous.Anonymous.vt };

        // Check if it's VT_ARRAY | VT_UI1
        if vt != VT_UI1_VAL | VT_ARRAY_VAL {
            // Maybe it's a single byte (VT_UI1)
            if vt == VT_UI1_VAL {
                let val = unsafe { raw.Anonymous.Anonymous.Anonymous.bVal };
                return Ok(vec![val]);
            }
            return Err(anyhow::anyhow!(HardwareError::Wmi(format!(
                "OutData is not a byte array (vt=0x{vt:04X})"
            ))));
        }

        let psa_imp = unsafe { raw.Anonymous.Anonymous.Anonymous.parray };
        if psa_imp.is_null() {
            return Err(anyhow::anyhow!(HardwareError::Wmi(
                "OutData SAFEARRAY is null".into()
            )));
        }

        // Cast to *const Win32 SAFEARRAY for the SafeArray functions.
        // The parray field is *mut imp::bindings::SAFEARRAY (private), but it has
        // the same layout as Win32::System::Com::SAFEARRAY. Cast through c_void.
        let psa_const = psa_imp as *mut std::ffi::c_void as *const Win32SAFEARRAY;

        unsafe {
            let lower = SafeArrayGetLBound(psa_const, 1)?;
            let upper = SafeArrayGetUBound(psa_const, 1)?;

            let count = (upper - lower + 1) as usize;
            let mut data: Vec<u8> = vec![0u8; count];

            let mut ptr: *mut std::ffi::c_void = std::ptr::null_mut();
            SafeArrayAccessData(psa_const, &mut ptr)?;

            std::ptr::copy_nonoverlapping(ptr as *const u8, data.as_mut_ptr(), count);
            SafeArrayUnaccessData(psa_const)?;

            Ok(data)
        }
    }

    /// Convert a VARIANT to a u16 value.
    fn variant_to_u16(var: &VARIANT) -> anyhow::Result<u16> {
        let raw = var.as_raw();
        let vt: u16 = unsafe { raw.Anonymous.Anonymous.vt };

        if vt == VT_UI1_VAL {
            let val = unsafe { raw.Anonymous.Anonymous.Anonymous.bVal };
            Ok(val as u16)
        } else if vt == VT_I2_VAL {
            let val = unsafe { raw.Anonymous.Anonymous.Anonymous.iVal };
            Ok(val as u16)
        } else if vt == VT_UI2_VAL {
            let val = unsafe { raw.Anonymous.Anonymous.Anonymous.uiVal };
            Ok(val)
        } else if vt == VT_I4_VAL {
            let val = unsafe { raw.Anonymous.Anonymous.Anonymous.lVal };
            Ok(val as u16)
        } else if vt == VT_UI4_VAL {
            let val = unsafe { raw.Anonymous.Anonymous.Anonymous.ulVal };
            Ok(val as u16)
        } else {
            Err(anyhow::anyhow!(HardwareError::Wmi(format!(
                "ReturnCode has unexpected vt=0x{vt:04X}"
            ))))
        }
    }

    /// Convert a VARIANT to a String.
    fn variant_to_string(var: &VARIANT) -> anyhow::Result<String> {
        let raw = var.as_raw();
        let vt: u16 = unsafe { raw.Anonymous.Anonymous.vt };

        if vt == VT_BSTR_VAL {
            let bstr_ptr = unsafe { raw.Anonymous.Anonymous.Anonymous.bstrVal };
            if bstr_ptr.is_null() {
                return Ok(String::new());
            }
            // bstr_val is *const u16 (raw BSTR), convert to public BSTR
            let bstr = unsafe { BSTR::from_raw(bstr_ptr) };
            Ok(bstr.to_string())
        } else {
            Err(anyhow::anyhow!(HardwareError::Wmi(format!(
                "__Path has unexpected vt=0x{vt:04X}"
            ))))
        }
    }
}

/// Read a WMAA register via WMI MiInterface.
#[cfg(windows)]
pub fn wmi_read(fun2: u16, fun3: u16) -> HardwareResult<WmaaResponse> {
    imp::wmi_call(FUN1_READ, fun2, fun3, 0)
}

/// Write a WMAA register via WMI MiInterface.
#[cfg(windows)]
pub fn wmi_write(fun2: u16, fun3: u16, fun4: u32) -> HardwareResult<WmaaResponse> {
    imp::wmi_call(FUN1_WRITE, fun2, fun3, fun4)
}

// ── Public API ─────────────────────────────────────────────────────────────

/// Read the current performance mode from the EC.
pub fn get_performance_mode() -> HardwareResult<EcPerformanceMode> {
    #[cfg(windows)]
    {
        let resp = wmi_read(FUN2_EC_FUNC, 0)?;
        if !resp.is_success() {
            return Err(HardwareError::Wmi(format!(
                "WMAA read failed: SGER=0x{:04X}",
                resp.sger
            )));
        }
        EcPerformanceMode::from_raw(resp.frd0)
            .ok_or_else(|| HardwareError::Wmi(format!("Unknown performance mode: {}", resp.frd0)))
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "WMI only available on Windows".into(),
    ))
}

/// Set the performance mode via the EC.
pub fn set_performance_mode(mode: EcPerformanceMode) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        let resp = wmi_write(FUN2_EC_FUNC, mode as u16, 0)?;
        if !resp.is_success() {
            return Err(HardwareError::Wmi(format!(
                "WMAA write failed: SGER=0x{:04X}",
                resp.sger
            )));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "WMI only available on Windows".into(),
    ))
}

/// Read all available sensor data from the EC.
pub fn read_sensor_data() -> HardwareResult<EcSensorData> {
    #[cfg(windows)]
    {
        let battery_health = wmi_read(FUN2_SENSOR, 0x01)?.frd1;
        let adapter_power = wmi_read(FUN2_SENSOR, 0x06)?.frd1;
        let mi_usage_type = wmi_read(FUN2_MI_INFO, 0x05)?.frd1;
        let wmid_type = wmi_read(FUN2_MI_INFO, 0x07)?.frd1;
        let lid_open_type = wmi_read(FUN2_MISC, 0x02)?.frd1;
        let removable_type = wmi_read(FUN2_MISC, 0x03)?.frd1;
        let current_mode = wmi_read(FUN2_EC_FUNC, 0x00)?.frd0;

        Ok(EcSensorData {
            battery_health,
            adapter_power,
            mi_usage_type,
            wmid_type,
            lid_open_type,
            removable_type,
            current_mode,
        })
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "WMI only available on Windows".into(),
    ))
}

/// Read the battery state of health (0-100).
pub fn read_battery_health() -> HardwareResult<u32> {
    #[cfg(windows)]
    {
        let resp = wmi_read(FUN2_SENSOR, 0x01)?;
        if !resp.is_success() {
            return Err(HardwareError::Wmi(format!(
                "WMAA read failed: SGER=0x{:04X}",
                resp.sger
            )));
        }
        Ok(resp.frd1)
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "WMI only available on Windows".into(),
    ))
}

/// Read the AC adapter power in watts.
pub fn read_adapter_power() -> HardwareResult<u32> {
    #[cfg(windows)]
    {
        let resp = wmi_read(FUN2_SENSOR, 0x06)?;
        if !resp.is_success() {
            return Err(HardwareError::Wmi(format!(
                "WMAA read failed: SGER=0x{:04X}",
                resp.sger
            )));
        }
        Ok(resp.frd1)
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "WMI only available on Windows".into(),
    ))
}

/// Set the hotkey brightness data (HBDA).
pub fn set_brightness_data(level: u32) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        let resp = wmi_write(FUN2_SENSOR, 0x02, level)?;
        if !resp.is_success() {
            return Err(HardwareError::Wmi(format!(
                "WMAA write failed: SGER=0x{:04X}",
                resp.sger
            )));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "WMI only available on Windows".into(),
    ))
}

/// Set the SAGV (System Agent Geyserville) mode.
pub fn set_sagv_mode(mode: u32) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        let resp = wmi_write(FUN2_MISC, 0x06, mode)?;
        if !resp.is_success() {
            return Err(HardwareError::Wmi(format!(
                "WMAA write failed: SGER=0x{:04X}",
                resp.sger
            )));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "WMI only available on Windows".into(),
    ))
}

/// Set the PL1 power limit flag (OD08).
pub fn set_pl1_flag(enabled: bool) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        let resp = wmi_write(FUN2_MISC, 0x04, if enabled { 1 } else { 0 })?;
        if !resp.is_success() {
            return Err(HardwareError::Wmi(format!(
                "WMAA write failed: SGER=0x{:04X}",
                resp.sger
            )));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "WMI only available on Windows".into(),
    ))
}

/// Set the EPOF (emergency power off) flag (OD09).
pub fn set_epof_flag(enabled: bool) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        let resp = wmi_write(FUN2_MISC, 0x05, if enabled { 1 } else { 0 })?;
        if !resp.is_success() {
            return Err(HardwareError::Wmi(format!(
                "WMAA write failed: SGER=0x{:04X}",
                resp.sger
            )));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "WMI only available on Windows".into(),
    ))
}

/// Set the MI usage type (MIUT).
pub fn set_mi_usage_type(enabled: bool) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        let resp = wmi_write(FUN2_MI_INFO, 0x05, if enabled { 1 } else { 0 })?;
        if !resp.is_success() {
            return Err(HardwareError::Wmi(format!(
                "WMAA write failed: SGER=0x{:04X}",
                resp.sger
            )));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "WMI only available on Windows".into(),
    ))
}

/// Set the WMID type (WMIT).
pub fn set_wmid_type(wmit_type: u32) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        let resp = wmi_write(FUN2_MI_INFO, 0x07, wmit_type)?;
        if !resp.is_success() {
            return Err(HardwareError::Wmi(format!(
                "WMAA write failed: SGER=0x{:04X}",
                resp.sger
            )));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "WMI only available on Windows".into(),
    ))
}

/// Set the lid open type (LOTS).
pub fn set_lid_open_type(lot: u32) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        let resp = wmi_write(FUN2_MISC, 0x02, lot)?;
        if !resp.is_success() {
            return Err(HardwareError::Wmi(format!(
                "WMAA write failed: SGER=0x{:04X}",
                resp.sger
            )));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "WMI only available on Windows".into(),
    ))
}

/// Set the removable type (RMTS).
pub fn set_removable_type(rmt: u32) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        let resp = wmi_write(FUN2_MISC, 0x03, rmt)?;
        if !resp.is_success() {
            return Err(HardwareError::Wmi(format!(
                "WMAA write failed: SGER=0x{:04X}",
                resp.sger
            )));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "WMI only available on Windows".into(),
    ))
}

/// Set the auto-adjustable illumination (AILM).
pub fn set_auto_illumination(enabled: bool) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        let resp = wmi_write(FUN2_MI_INFO, 0x08, if enabled { 1 } else { 0 })?;
        if !resp.is_success() {
            return Err(HardwareError::Wmi(format!(
                "WMAA write failed: SGER=0x{:04X}",
                resp.sger
            )));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "WMI only available on Windows".into(),
    ))
}

/// Set the label mode (LBLM).
pub fn set_label_mode(enabled: bool) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        let resp = wmi_write(FUN2_MI_INFO, 0x09, if enabled { 1 } else { 0 })?;
        if !resp.is_success() {
            return Err(HardwareError::Wmi(format!(
                "WMAA write failed: SGER=0x{:04X}",
                resp.sger
            )));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "WMI only available on Windows".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_make_buffer() {
        let buf = make_buffer(FUN1_READ, FUN2_SENSOR, 0x06, 0);
        assert_eq!(buf.len(), BUFFER_SIZE);
        assert_eq!(u16::from_le_bytes([buf[0], buf[1]]), FUN1_READ);
        assert_eq!(u16::from_le_bytes([buf[2], buf[3]]), FUN2_SENSOR);
        assert_eq!(u16::from_le_bytes([buf[4], buf[5]]), 0x06);
        assert_eq!(u32::from_le_bytes([buf[6], buf[7], buf[8], buf[9]]), 0);
    }

    #[test]
    fn test_parse_response() {
        let raw = vec![
            0x00, 0x80, // SGER = 0x8000 (success)
            0x00, 0x10, // FUTR = 0x1000
            0x06, 0x00, // FRD0 = 0x0006
            0x64, 0x00, 0x00, 0x00, // FRD1 = 100
            0, 0, 0, 0, // FRD2
            0, 0, 0, 0, // FRD3
        ];
        let resp = WmaaResponse::parse(&raw).unwrap();
        assert!(resp.is_success());
        assert_eq!(resp.futr, 0x1000);
        assert_eq!(resp.frd0, 0x0006);
        assert_eq!(resp.frd1, 100);
    }
}
