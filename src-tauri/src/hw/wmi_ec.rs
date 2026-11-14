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

use crate::hw::adapter_power::AdapterPowerReading;
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// WMAA `FUN2=0x0800` write-profile IDs (`FUN3`).
///
/// PITFALL — these names are the ones this app has always used, but they are
/// **not** the EC's mode register values. The ACPI write branch treats them
/// differently (decoded from `ssdt24.dsl`):
///
/// * `2/3/4/9/0x0A` write `QFAN` (EC RAM `0x68`) — the register the read path
///   reports. These are the only values whose effect is readable back.
/// * `5..=8` write `SMMT`/`SMMD` (EC RAM `0x4A`/`0x4B`) plus a DPTF OEM
///   variable and never touch `QFAN`. On this firmware they are DPTF power
///   profiles, **not** the fan/performance mode: writing `6` ("Balanced" here)
///   while the EC sits at `9` leaves it at `9`, and `SGER` still says success.
///
/// The EC's own numbering (SSDB `SetPerformanceMode`, EC RAM `0x68`) is
/// 0=Silence, 1=Balance, 2=Turbo, 3=Decepticon, 4..8=hidden profiles,
/// 9=SmartAdaptive, 10=Smart, 11=LongBattery, 14=SmartAcceleration. See
/// [`EcActiveMode`] for the read-back side.
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

/// The EC's **active** performance mode, as the ACPI read path can report it.
///
/// This is deliberately *not* [`EcPerformanceMode`]. The firmware's WMAA read
/// (`FUN2=0x0800`, decoded from `ssdt24.dsl` / `dsdt.dsl` on this platform)
/// returns EC RAM byte `0x68` (`QFAN`) through `FUNR(0x16)`, and the AML switch
/// only has branches for five values:
///
/// ```text
/// FNKN = Q_EC.FUNR(0x16)
/// If (FNKN == 0x02) FRD0 = 0x02      // Turbo
/// If (FNKN == 0x03) FRD0 = 0x03      // Decepticon
/// If (FNKN == 0x09) FRD0 = 0x09      // SmartAdaptive
/// If (FNKN == 0x04) FRD0 = 0x04      // Overdrive (hidden profile)
/// If (FNKN == 0x0A) FRD0 = 0x0A      // Smart
/// // no Else: every other QFAN value leaves FRD0 = 0
/// ```
///
/// So a raw `0` does **not** mean "Silence": it means *the EC is in a mode the
/// ACPI read path cannot name* (Silence, Balance, the hidden 5/6/7/8 profiles,
/// LongBattery, …). Reporting it as a real mode would be a lie in the other
/// direction, so it gets [`EcActiveMode::Unreported`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EcActiveMode {
    /// QFAN 0x02 — the SSDB "Turbo" profile.
    Turbo,
    /// QFAN 0x03 — the SSDB "Decepticon" profile.
    Decepticon,
    /// QFAN 0x04 — hidden DPTF profile ODV1=4.
    Overdrive,
    /// QFAN 0x09 — SMMT-driven adaptive profile (the EC feeds NTDP dynamically
    /// while on AC). This is the value the owner's machine sat at.
    SmartAdaptive,
    /// QFAN 0x0A — the SSDB "Smart" profile.
    Smart,
    /// Any other QFAN value: the read path collapses them all to 0, so the
    /// exact mode is unknown from this channel alone.
    Unreported,
}

impl EcActiveMode {
    /// Map a raw `FRD0` byte from `FUN2=0x0800 / FUN3=0` to a readable mode.
    pub fn from_raw(val: u16) -> Self {
        match val {
            2 => Self::Turbo,
            3 => Self::Decepticon,
            4 => Self::Overdrive,
            9 => Self::SmartAdaptive,
            0x0A => Self::Smart,
            _ => Self::Unreported,
        }
    }

    /// Stable machine-readable label (also used by the TS types).
    pub fn label(self) -> &'static str {
        match self {
            Self::Turbo => "turbo",
            Self::Decepticon => "decepticon",
            Self::Overdrive => "overdrive",
            Self::SmartAdaptive => "smart_adaptive",
            Self::Smart => "smart",
            Self::Unreported => "unreported",
        }
    }

    /// `false` only for [`EcActiveMode::Unreported`] — the case where the EC is
    /// in a mode the WMAA read cannot name.
    pub fn is_reported(self) -> bool {
        self != Self::Unreported
    }
}

/// Did a WMAA profile write really latch the EC mode?
///
/// The write path and the read path do **not** address the same thing for
/// every mode. Decoded from `ssdt24.dsl` (WMAA `FUN1=0xFB00`):
///
/// * `FUN3 = 2, 3, 4, 9, 0x0A` → `ECWT(value, QFAN)` + `QV20(1,0x16)`, i.e. the
///   value lands in the register the read path reports (EC RAM `0x68`).
/// * `FUN3 = 5, 6, 7, 8` → `ECWT(0, SMMT)` + `ECWT(mode, SMMD)` (SMMT/SMMD are
///   EC RAM `0x4A`/`0x4B`) + `NTDP(mode)`, i.e. a DPTF/OEM-variable profile.
///   **QFAN is never written**, so the readback cannot change and the UI must
///   not claim the mode was applied.
/// * Any other `FUN3` → no EC write at all (the AML only sets `SGER = 0x8000`).
///
/// `SGER = 0x8000` is set unconditionally at the end of that AML branch, which
/// is why `set_performance_mode(6)` used to report success while nothing moved.
///
/// Verification therefore depends on whether the requested value is one the
/// read path can report:
/// * reportable (`2/3/4/9/0x0A`) → the readback must equal the request;
/// * unreportable (`5..=8`, and everything else) → the only evidence available
///   is a *transition* to the unreported bucket (`0`), so a readback of `0`
///   after a non-zero `before` proves the register moved; `0 → 0` proves
///   nothing and stays unverified.
pub fn qfan_write_verified(requested: u16, before: u16, after: u16) -> bool {
    if EcActiveMode::from_raw(requested).is_reported() {
        after == requested
    } else {
        after == 0 && before != 0
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
        IWbemClassObject, WBEM_FLAG_RETURN_IMMEDIATELY, WBEM_FLAG_RETURN_WBEM_COMPLETE,
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

            // Get the MICommonInterface class object (for method signature).
            // We need the class definition to call GetMethod and spawn input params.
            //
            // GetObject with just "MICommonInterface" fails with WBEM_E_NOT_FOUND
            // (0x80041002) on this WMI provider. The fix is to first query for the
            // instance, get its __Path, and then use GetObject with the full path
            // to get the class definition.
            //
            // Step 1: Query for the active MICommonInterface instance.
            // Use WBEM_FLAG_RETURN_IMMEDIATELY (without WBEM_FLAG_FORWARD_ONLY)
            // to ensure all system properties (__Path, __RELPATH) are available.
            let query = BSTR::from("SELECT * FROM MICommonInterface WHERE Active = TRUE");
            let wql = BSTR::from("WQL");
            let flags = WBEM_FLAG_RETURN_IMMEDIATELY;
            let enumerator = unsafe { svc.ExecQuery(&wql, &query, flags, None)? };

            // Get first instance
            let mut objs: [Option<IWbemClassObject>; 1] = [None];
            let mut returned: u32 = 0;
            let hr = unsafe { enumerator.Next(-1, &mut objs, &mut returned) };
            hr.ok()?;

            let instance = objs[0].as_ref().ok_or_else(|| {
                anyhow::anyhow!(HardwareError::Wmi(
                    "No MICommonInterface instance found".into()
                ))
            })?;

            // Get the instance path. Try __Path first, then __RELPATH as fallback.
            // With WBEM_FLAG_FORWARD_ONLY, system properties like __Path may not
            // always be available (WBEM_E_NOT_FOUND / 0x80041002).
            // Get the instance path. Try __Path first, then __RELPATH as fallback.
            // With WBEM_FLAG_FORWARD_ONLY, system properties like __Path may not
            // always be available (WBEM_E_NOT_FOUND / 0x80041002).
            let path_str = {
                let mut path_var = VARIANT::new();
                let mut cim_type: i32 = 0;
                let path_name = pcwstr("__Path");
                let get_result =
                    unsafe { instance.Get(path_name, 0, &mut path_var, Some(&mut cim_type), None) };
                if get_result.is_err() {
                    // __Path not available — try __RELPATH
                    let mut relpath_var = VARIANT::new();
                    let mut cim_type2: i32 = 0;
                    let relpath_name = pcwstr("__RELPATH");
                    let get_result2 = unsafe {
                        instance.Get(
                            relpath_name,
                            0,
                            &mut relpath_var,
                            Some(&mut cim_type2),
                            None,
                        )
                    };
                    if get_result2.is_err() {
                        // __RELPATH also not available — construct from InstanceName
                        let mut inst_var = VARIANT::new();
                        let mut cim_type3: i32 = 0;
                        let inst_name = pcwstr("InstanceName");
                        unsafe {
                            instance.Get(
                                inst_name,
                                0,
                                &mut inst_var,
                                Some(&mut cim_type3),
                                None,
                            )?;
                        }
                        let inst_str = variant_to_string(&inst_var)?;
                        // Escape backslashes for WMI path
                        let escaped = inst_str.replace('\\', "\\\\");
                        format!("MICommonInterface.InstanceName=\"{escaped}\"")
                    } else {
                        variant_to_string(&relpath_var)?
                    }
                } else {
                    variant_to_string(&path_var)?
                }
            };

            // Step 2: Construct input parameters for ExecMethod.
            //
            // Standard WMI approach: GetObject(class) → GetMethod →
            // SpawnInstance on in_sig → Put("InData").
            //
            // On this WMI provider, the Put("InData") call sometimes fails
            // with WBEM_E_NOT_FOUND (0x80041002) when the WMI provider is
            // in a degraded state (correlated with __Path being unavailable).
            // When Put fails, we fall back to using the instance directly
            // and setting InData on it via SpawnInstance.
            let class_path = BSTR::from("MICommonInterface");
            let mut class_obj: Option<IWbemClassObject> = None;
            let get_obj_result = unsafe {
                svc.GetObject(
                    &class_path,
                    WBEM_FLAG_RETURN_WBEM_COMPLETE,
                    None,
                    Some(&mut class_obj),
                    None,
                )
            };

            let in_params: IWbemClassObject = match get_obj_result {
                Ok(()) => {
                    let class_obj = class_obj.ok_or_else(|| {
                        anyhow::anyhow!(HardwareError::Wmi("GetObject returned null".into()))
                    })?;

                    // Get method signature
                    let method_name = pcwstr("MiInterface");
                    let mut in_sig: Option<IWbemClassObject> = None;
                    let mut out_sig: Option<IWbemClassObject> = None;
                    unsafe {
                        if let Err(e) =
                            class_obj.GetMethod(method_name, 0, &mut in_sig, &mut out_sig)
                        {
                            log::debug!(target: "hw::wmi_ec", "GetMethod(MiInterface) failed: {e}");
                            return Err(e.into());
                        }
                    }

                    let in_sig = in_sig.ok_or_else(|| {
                        anyhow::anyhow!(HardwareError::Wmi("GetMethod returned null in_sig".into()))
                    })?;

                    // SpawnInstance on in_sig to create input parameter object
                    let params = unsafe { in_sig.SpawnInstance(0)? };

                    // Set InData property
                    let prop_name = pcwstr("InData");
                    let in_data_var = build_byte_array_variant(&buf)?;
                    unsafe {
                        if let Err(e) = params.Put(prop_name, 0, &in_data_var, 0) {
                            // Put fails intermittently with WBEM_E_NOT_FOUND (0x80041002)
                            // when the WMI provider is in a degraded state (correlated
                            // with __Path being unavailable). This is a WMI provider
                            // bug, not our code. Return the error so the caller can
                            // handle it gracefully (most callers use .ok()).
                            log::trace!(target: "hw::wmi_ec", "params.Put(InData) failed: {e}");
                            return Err(e.into());
                        }
                    }
                    params
                }
                Err(ref e) => {
                    // GetObject failed — use fallback: spawn instance from the
                    // queried instance and set InData directly.
                    log::debug!(target: "hw::wmi_ec", "GetObject(MICommonInterface) failed: {e}, using SpawnInstance fallback");
                    let params = unsafe { instance.SpawnInstance(0)? };
                    let prop_name = pcwstr("InData");
                    let in_data_var = build_byte_array_variant(&buf)?;
                    unsafe { params.Put(prop_name, 0, &in_data_var, 0)? };
                    params
                }
            };

            // Call ExecMethod on IWbemServices
            let path_bstr = BSTR::from(path_str.as_str());
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

// ═══════════════════════════════════════════════════════════════════════
// WORKING FORM — DO NOT MODIFY
//
// wmi_read and wmi_write are the core WMI MiInterface callers. They use
// wmi_cache::with_wmi() which holds a RefCell borrow for the duration of
// the WMI call. Callers MUST NOT nest these calls inside another
// with_wmi() closure — doing so will panic with "RefCell already borrowed".
//
// The WMAA buffer format:
//   FUN1=0xFA00 (read) / 0xFB00 (write)
//   FUN2: sub-command group (0x0800=EC, 0x0A00=MI_INFO, 0x0C00=MISC, 0x1000=SENSOR)
//   FUN3: parameter/sub-command ID
//   FUN4: additional data (write only)
//
// Response: SGER=0x8000 means success. FRD0/FRD1 contain the result data.
// ═══════════════════════════════════════════════════════════════════════

/// Attempts for a transient provider stall (`0x80041002`) on a WMAA read.
///
/// Measured on this host: 1 716 elevated reads in 53 h, 129 of them failed
/// with `WBEM_E_NOT_FOUND` (7.5 %), and every retry in the historical log
/// returned a value moments later. Three attempts cover the stall without
/// making a genuinely broken provider slow (each failed call returns in ~5 ms,
/// so the extra cost is the 200 + 400 ms backoff).
pub(crate) const TRANSIENT_READ_ATTEMPTS: u32 = 3;

/// Is this error the provider stall that must be retried?
///
/// ONLY `0x80041002` (WBEM_E_NOT_FOUND) — on this provider that HRESULT is
/// raised while it rebuilds its method parameters and the next call succeeds.
/// `0x80041010` (WBEM_E_INVALID_CLASS) is the opposite: a class that is really
/// absent, where retrying only wastes time. Never widen this predicate to
/// "any WMI error" — a permission failure must fail fast.
pub(crate) fn is_transient_provider_stall(message: &str) -> bool {
    message.contains("0x80041002")
}

/// Run `call`, retrying ONLY the transient provider stall.
///
/// Split out (with the sleep injected) so the retry policy is unit-tested
/// without touching WMI: the production caller passes
/// `std::thread::sleep`, tests count the sleeps instead.
fn with_transient_retry<T, F, S>(mut call: F, mut sleep: S) -> HardwareResult<T>
where
    F: FnMut() -> HardwareResult<T>,
    S: FnMut(u64),
{
    let mut attempt = 0u32;
    loop {
        match call() {
            Ok(value) => return Ok(value),
            Err(e) => {
                attempt += 1;
                if attempt >= TRANSIENT_READ_ATTEMPTS
                    || !is_transient_provider_stall(&e.to_string())
                {
                    return Err(e);
                }
                // The provider needs a moment to rebuild the method parameters;
                // 200 ms was enough in the measured recovery, 400 ms covers the
                // slower stalls. Linear, not exponential: the whole budget is
                // 600 ms and the caller's timeout is seconds.
                let backoff_ms = 200 * u64::from(attempt);
                log::debug!(
                    target: "hw::wmi_ec",
                    "WMAA read hit a degraded WMI provider ({e}); retry {attempt}/{} after {backoff_ms} ms",
                    TRANSIENT_READ_ATTEMPTS - 1
                );
                sleep(backoff_ms);
            }
        }
    }
}

/// Read a WMAA register via WMI MiInterface.
///
/// Retries the transient provider stall in-process (see
/// [`is_transient_provider_stall`]). Before this, a stall answered `Err` to
/// `run_elevated_impl`, which could not tell "the command failed" from "the
/// pipe failed" and therefore re-dispatched a SECOND elevation through the
/// scheduled task: two process spawns and two failures for one 5 ms hiccup
/// (129 failed helper runs in 53 h, each matching an app-side failure at the
/// same second). A stall also blanked the charge-limit read-back, which is
/// what made the hold supervisor flap to `phase=unknown`.
#[cfg(windows)]
pub fn wmi_read(fun2: u16, fun3: u16) -> HardwareResult<WmaaResponse> {
    with_transient_retry(
        || imp::wmi_call(FUN1_READ, fun2, fun3, 0),
        |ms| std::thread::sleep(std::time::Duration::from_millis(ms)),
    )
}

/// Write a WMAA register via WMI MiInterface.
#[cfg(windows)]
pub fn wmi_write(fun2: u16, fun3: u16, fun4: u32) -> HardwareResult<WmaaResponse> {
    imp::wmi_call(FUN1_WRITE, fun2, fun3, fun4)
}

// ── Public API ─────────────────────────────────────────────────────────────

/// Read the current performance mode from the EC.
///
/// WORKING FORM — DO NOT MODIFY: Uses wmi_read(FUN2_EC_FUNC=0x0800, FUN3=0).
/// This reads the EC performance mode via the ACPI WMAA method through
/// the MICommonInterface WMI class. Does NOT require IoTDriver.
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

/// Read the EC's active mode as `(raw, decoded)`.
///
/// `raw` is the firmware's `FRD0` verbatim (QFAN for the five reportable
/// values, `0` otherwise) so callers can show the truth even when it is not a
/// name we know.
pub fn read_active_mode() -> HardwareResult<(u16, EcActiveMode)> {
    #[cfg(windows)]
    {
        let resp = wmi_read(FUN2_EC_FUNC, 0)?;
        if !resp.is_success() {
            return Err(HardwareError::Wmi(format!(
                "WMAA read failed: SGER=0x{:04X}",
                resp.sger
            )));
        }
        Ok((resp.frd0, EcActiveMode::from_raw(resp.frd0)))
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "WMI only available on Windows".into(),
    ))
}

/// Set the performance mode via the EC.
///
/// WORKING FORM — DO NOT MODIFY: Uses wmi_write(FUN2_EC_FUNC=0x0800, mode, 0).
/// This sets the EC performance mode via the ACPI WMAA method through
/// the MICommonInterface WMI class. Does NOT require IoTDriver.
/// Modes: Performance=5, Balanced=6, Quiet=7, SuperQuiet=8, UltraPerformance=9, Extreme=0x0A
///
/// IMPORTANT (verified live 2026-11-10): `SGER=0x8000` is returned for every
/// `FUN3` in this branch, including values the AML never acts on. Modes
/// `5..=8` write SMMT/SMMD + a DPTF variable and **leave QFAN untouched**, so
/// a readback cannot confirm them; only `2/3/4/9/0x0A` land in QFAN and are
/// readable back. Use [`qfan_write_verified`] — never trust `Ok(())` here.
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
///
/// WORKING FORM — DO NOT MODIFY: Uses wmi_read(FUN2_SENSOR=0x1000, FUN3=0x01).
/// Reads battery SoH via ACPI WMAA method. Does NOT require IoTDriver.
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

/// Read the AC adapter power in watts (ADPW).
///
/// WORKING FORM — DO NOT MODIFY: Uses wmi_read(FUN2_SENSOR=0x1000, FUN3=0x06).
/// Reads AC adapter power via ACPI WMAA method. Does NOT require IoTDriver.
/// IMPORTANT: This function calls wmi_cache::with_wmi() internally, so it
/// MUST NOT be called from inside another with_wmi() closure.
///
/// `0x80041002` (`WBEM_E_NOT_FOUND`) is retried once, immediately. On this
/// provider that HRESULT is raised while building the method parameters
/// (`params.Put(InData)`) when the provider is in a degraded state — the
/// production log shows it interleaved with successes for the *same* call
/// (18 451 `ok=true` vs 1 337 failures on 2026-11-02…06), and the next attempt
/// moments later always returned a value. A single retry therefore recovers
/// most of the samples that used to be lost.
pub fn read_adapter_power() -> HardwareResult<u32> {
    #[cfg(windows)]
    {
        // Measured on this machine (2026-11-08, bridge as SYSTEM): the first
        // WMAA call of a fresh WMI connection returns 0x80041002, and every
        // call after it returns the real wattage (65 W). So attempt a few
        // times before giving up — the answer is there, the provider just
        // needs a warm-up call.
        const ATTEMPTS: u32 = 3;
        let mut last = None;
        for attempt in 0..ATTEMPTS {
            match read_adapter_power_once() {
                Ok(watts) => return Ok(watts),
                Err(e) => {
                    use crate::hw::adapter_power::{classify_failure, AdapterPowerFailure};
                    let degraded =
                        classify_failure(&e.to_string()) == AdapterPowerFailure::ProviderDegraded;
                    if !degraded || attempt + 1 == ATTEMPTS {
                        return Err(e);
                    }
                    log::debug!(
                        target: "hw::wmi_ec",
                        "adapter power read hit a degraded WMI provider ({e}); retry {}/{}",
                        attempt + 1,
                        ATTEMPTS - 1
                    );
                    // The provider needs a moment to rebuild its method
                    // parameters; a short backoff is enough (measured: the
                    // second call ~0.4 s later returns the real wattage).
                    std::thread::sleep(std::time::Duration::from_millis(
                        200 * u64::from(attempt + 1),
                    ));
                    last = Some(e);
                }
            }
        }
        match last {
            Some(e) => Err(e),
            None => Err(HardwareError::Wmi("adapter power read failed".into())),
        }
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "WMI only available on Windows".into(),
    ))
}

/// One WMAA `0x1000 / 0x06` read, no retry.
#[cfg(windows)]
fn read_adapter_power_once() -> HardwareResult<u32> {
    let resp = wmi_read(FUN2_SENSOR, 0x06)?;
    if !resp.is_success() {
        return Err(HardwareError::Wmi(format!(
            "WMAA read failed: SGER=0x{:04X}",
            resp.sger
        )));
    }
    Ok(resp.frd1)
}

/// Read the AC adapter power as a structured result.
///
/// Only `WBEM_E_INVALID_CLASS` / invalid namespace — a real capability fact —
/// is reported as `Ok(supported: false)`. `WBEM_E_NOT_FOUND` (`0x80041002`) is
/// a degraded-provider hiccup on this machine and stays an `Err`, so the
/// caller retries on the next poll instead of hiding the field forever.
///
/// The "once per process" log lives here because the dispatch may run inside
/// the long-lived bridge service (many requests, one process).
pub fn read_adapter_power_status() -> HardwareResult<AdapterPowerReading> {
    use crate::hw::adapter_power::{classify_failure, AdapterPowerFailure};
    use std::sync::Once;

    match read_adapter_power() {
        Ok(watts) => Ok(AdapterPowerReading::watts(watts)),
        Err(e) => {
            let message = e.to_string();
            if classify_failure(&message) == AdapterPowerFailure::HardwareUnsupported {
                static LOGGED: Once = Once::new();
                LOGGED.call_once(|| {
                    log::info!(
                        target: "hw::wmi_ec",
                        "AC adapter power class absent on this hardware ({message}) — \
                         returning a structured 'unsupported' result instead of an error"
                    );
                });
                Ok(AdapterPowerReading::unsupported(message))
            } else {
                Err(e)
            }
        }
    }
}

// REMOVED — this pair is the charging-protection setter.
//
// WMAA `FUN2=0x1000, FUN3=0x02` writes `LONL`/`HBDA` (the battery limit), not
// display brightness (`DBLL` lives at ERAM 0xAE and is not reachable through
// WMAA). Any caller of this function would silently rewrite the charge limit:
// FUN4=1 → 80%, 4 → 90%, 5 → 70%, 6 → 60%, 7 → 50%, 8 → 40%, 0 → no limit.
// Use `crate::hw::charging::set_charging_threshold` instead.

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

    #[test]
    fn active_mode_only_names_the_five_aml_branches() {
        // The AML switch has cases for exactly these QFAN values.
        assert_eq!(EcActiveMode::from_raw(2), EcActiveMode::Turbo);
        assert_eq!(EcActiveMode::from_raw(3), EcActiveMode::Decepticon);
        assert_eq!(EcActiveMode::from_raw(4), EcActiveMode::Overdrive);
        assert_eq!(EcActiveMode::from_raw(9), EcActiveMode::SmartAdaptive);
        assert_eq!(EcActiveMode::from_raw(0x0A), EcActiveMode::Smart);
        // Everything else falls through to FRD0 = 0: the mode is NOT "Silence",
        // it is simply not reportable through this channel.
        for raw in [0u16, 1, 5, 6, 7, 8, 11, 12, 13, 14, 0xFFFF] {
            assert_eq!(
                EcActiveMode::from_raw(raw),
                EcActiveMode::Unreported,
                "raw {raw} must not be named"
            );
            assert!(!EcActiveMode::from_raw(raw).is_reported());
        }
    }

    #[test]
    fn active_mode_labels_are_stable() {
        assert_eq!(EcActiveMode::Turbo.label(), "turbo");
        assert_eq!(EcActiveMode::Decepticon.label(), "decepticon");
        assert_eq!(EcActiveMode::Overdrive.label(), "overdrive");
        assert_eq!(EcActiveMode::SmartAdaptive.label(), "smart_adaptive");
        assert_eq!(EcActiveMode::Smart.label(), "smart");
        assert_eq!(EcActiveMode::Unreported.label(), "unreported");
    }

    #[test]
    fn reportable_writes_need_an_exact_readback() {
        // Live-verified: FUN3=0x0A latched QFAN=0x0A and the readback followed.
        assert!(qfan_write_verified(0x0A, 9, 0x0A));
        assert!(qfan_write_verified(9, 0x0A, 9));
        assert!(qfan_write_verified(2, 9, 2));
        // A write the AML ignores must never verify.
        assert!(!qfan_write_verified(0x0A, 9, 9));
        assert!(!qfan_write_verified(2, 0x0A, 0x0A));
    }

    #[test]
    fn smm_profile_writes_only_verify_on_a_real_transition() {
        // Live-verified: FUN3=6 left QFAN at 9, so the readback is unchanged.
        assert!(!qfan_write_verified(6, 9, 9));
        assert!(!qfan_write_verified(5, 9, 9));
        assert!(!qfan_write_verified(7, 9, 9));
        assert!(!qfan_write_verified(8, 9, 9));
        // A move into the unreported bucket is evidence the register changed…
        assert!(qfan_write_verified(6, 9, 0));
        assert!(qfan_write_verified(8, 2, 0));
        // …but 0 → 0 proves nothing and must stay unverified.
        assert!(!qfan_write_verified(6, 0, 0));
    }

    /// Production shape: 62 app-visible + 129 helper-level stalls in 53 h, all
    /// the same `WBEM_E_NOT_FOUND` while the provider rebuilt its parameters.
    #[test]
    fn only_the_provider_stall_is_retried() {
        assert!(is_transient_provider_stall("Hardware error: 0x80041002"));
        assert!(is_transient_provider_stall(
            "WMI query failed: provider returned 0x80041002 (WBEM_E_NOT_FOUND)"
        ));
        // A truly absent class, an access denial and an IO error must fail fast.
        assert!(!is_transient_provider_stall(
            "Hardware error: 0x80041010 (WBEM_E_INVALID_CLASS)"
        ));
        assert!(!is_transient_provider_stall(
            "Hardware error: 0x80070005 (Access is denied)"
        ));
        assert!(!is_transient_provider_stall(
            "Timeout: blocking hardware operation exceeded 5 seconds"
        ));
        assert!(!is_transient_provider_stall(""));
    }

    #[test]
    fn a_stall_recovered_on_the_second_call_never_surfaces() {
        let mut calls = 0u32;
        let mut sleeps = Vec::new();
        let result: HardwareResult<u16> = with_transient_retry(
            || {
                calls += 1;
                if calls < 2 {
                    Err(HardwareError::Wmi("0x80041002".into()))
                } else {
                    Ok(65)
                }
            },
            |ms| sleeps.push(ms),
        );
        assert_eq!(result.unwrap(), 65);
        assert_eq!(calls, 2, "one retry is enough for a single stall");
        assert_eq!(sleeps, vec![200], "first backoff is 200 ms");
    }

    #[test]
    fn a_persistent_stall_gives_up_after_the_declared_attempts() {
        let mut calls = 0u32;
        let mut sleeps = Vec::new();
        let result: HardwareResult<u16> = with_transient_retry(
            || {
                calls += 1;
                Err(HardwareError::Wmi("0x80041002".into()))
            },
            |ms| sleeps.push(ms),
        );
        assert!(result.is_err());
        assert_eq!(calls, TRANSIENT_READ_ATTEMPTS);
        // Never a third sleep: the last failure is returned, not retried.
        assert_eq!(sleeps.len() as u32, TRANSIENT_READ_ATTEMPTS - 1);
        assert_eq!(sleeps, vec![200, 400]);
    }

    #[test]
    fn a_non_transient_error_is_not_retried() {
        let mut calls = 0u32;
        let result: HardwareResult<u16> = with_transient_retry(
            || {
                calls += 1;
                Err(HardwareError::Wmi("0x80070005".into()))
            },
            |_| panic!("a hard failure must not back off"),
        );
        assert!(result.is_err());
        assert_eq!(calls, 1, "permission failures fail fast");
    }
}
