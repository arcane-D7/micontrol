//! HQWmiCommonInterface — BIOS/EC control via WMI.
//!
//! This module wraps the `HQWmiCommonInterface` WMI class (ROOT\WMI namespace),
//! which provides BIOS-level operations on Xiaomi Book devices.
//!
//! All methods take a `String` parameter named `req` and return a `String` named `ret`.
//! The WMI instance path is `ACPI\PNP0C14\0_0`.
//!
//! Methods implemented:
//! - `SetPerformanceMode(req)` — Set BIOS performance mode
//! - `ChangeBootOption(req)` — Change boot device order
//! - `LoadDefault(req)` — Load BIOS default settings
//! - `S5RTCWakeEnable(req)` — Enable/disable S5 RTC wake
//! - `EnablePXEBoot(req)` — Enable/disable PXE boot
//! - `LoadDefaultKey(req)` — Load default security key
//! - `ClearKey(req)` — Clear security key
//! - `SetOOBTestMode(req)` — Set OOB test mode
//! - `ClearOOBTestMode(req)` — Clear OOB test mode
//! - `WifiCountryCode(req)` — Set WiFi country code
//! - `ShippingCountryCode(req)` — Set shipping country code

use crate::hw::errors::{HardwareError, HardwareResult};
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;

/// Response from an HQWmiCommonInterface method call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HqWmiResponse {
    /// The method that was called.
    pub method: String,
    /// The request string sent to the method.
    pub req: String,
    /// The return string from the method.
    pub ret: String,
    /// Whether the call succeeded (ret is non-empty and not an error message).
    pub success: bool,
}

// ── Typed, allowlisted request layer (SEC-11) ──────────────────────────────
//
// The seven `hq_*` Tauri commands write to the BIOS through WMI
// (`HQWmiCommonInterface`). Before this layer they forwarded a caller-supplied
// `req: String` verbatim, so any JS in the webview could reorder the boot
// devices or reset the BIOS. Every request is now a `deny_unknown_fields`
// struct whose fields are enums or validated newtypes, and the WMI wire token
// is a fixed constant derived from the semantic variant - never from caller
// text. Both the command side (early rejection, before any elevation) and the
// elevated dispatcher (re-validation, never trusting the caller) derive the
// token from the same typed layer, so an arbitrary `req` is unrepresentable at
// every boundary.

/// Ground-truth BIOS performance profiles for `SetPerformanceMode`, per the
/// method wrapper ("1" performance, "2" balanced, "3" quiet). Any value
/// outside this set is rejected at the type boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HqPerformanceMode {
    Performance,
    Balanced,
    Quiet,
}

impl HqPerformanceMode {
    /// Fixed WMI wire token, never derived from caller text.
    pub const fn wire(self) -> &'static str {
        match self {
            Self::Performance => "1",
            Self::Balanced => "2",
            Self::Quiet => "3",
        }
    }
}

/// Boot devices the firmware will accept via `ChangeBootOption`. The wire
/// token is a fixed constant per variant; the firmware rejects unknown
/// devices ("Undefined Boot device!"), so an unlisted target can never reach
/// WMI even if a caller guessed a plausible string. Adding a boot target
/// means extending this enum with an explicit, reviewed constant - the only
/// path to a new device. The exact firmware strings are confirmed on real
/// hardware before any UI wires this command; until then a wrong constant is
/// refused by the firmware, never honoured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HqBootTarget {
    WindowsBootManager,
    UefiUsb,
    UefiNetwork,
}

impl HqBootTarget {
    pub const fn wire(self) -> &'static str {
        match self {
            Self::WindowsBootManager => "Windows Boot Manager",
            Self::UefiUsb => "UEFI USB",
            Self::UefiNetwork => "UEFI Network",
        }
    }
}

/// Destructive reset of the BIOS to firmware defaults. Requires an explicit
/// confirmation variant (no free-form token) plus elevation; the single
/// variant makes an accidental or arbitrary invocation unrepresentable, and
/// the wire constant is fixed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HqLoadDefaultConfirm {
    ResetToDefaults,
}

impl HqLoadDefaultConfirm {
    pub const fn wire(self) -> &'static str {
        match self {
            Self::ResetToDefaults => "1",
        }
    }
}

/// ISO/IEC 3166-1 alpha-2 codes accepted as WiFi/shipping regulatory domains.
/// Restricted to codes with a real broadcast-allowed country, not the full ISO
/// list, so a caller cannot smuggle an arbitrary two-letter string into the
/// firmware's radio domain setting.
pub const ALLOWED_COUNTRY_CODES: &[&str] = &[
    "AE", "AR", "AT", "AU", "BE", "BG", "BR", "CA", "CH", "CL", "CN", "CO", "CZ", "DE", "DK", "EE",
    "EG", "ES", "FI", "FR", "GB", "GR", "HK", "HU", "ID", "IE", "IL", "IN", "IS", "IT", "JP", "KR",
    "KZ", "LT", "LU", "LV", "MY", "MX", "NG", "NL", "NO", "NZ", "PH", "PL", "PT", "RO", "RU", "SE",
    "SG", "SI", "SK", "TH", "TR", "TW", "UA", "US", "ZA",
];

/// A validated ISO 3166-1 alpha-2 country code. The newtype keeps the
/// validation invariant: the only way to construct one is through
/// `Deserialize`, which enforces two uppercase ASCII letters and membership in
/// [`ALLOWED_COUNTRY_CODES`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HqCountryCode(String);

impl HqCountryCode {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    fn validate(s: &str) -> Result<(), HardwareError> {
        let bytes = s.as_bytes();
        if bytes.len() != 2 {
            return Err(HardwareError::Other(
                "country code must be an ISO 3166-1 alpha-2 pair".into(),
            ));
        }
        if !bytes.iter().all(u8::is_ascii_uppercase) {
            return Err(HardwareError::Other(
                "country code must be two uppercase ASCII letters".into(),
            ));
        }
        if !ALLOWED_COUNTRY_CODES.contains(&s) {
            return Err(HardwareError::Other(format!(
                "country code '{s}' is not an allowed regulatory domain"
            )));
        }
        Ok(())
    }
}

impl Serialize for HqCountryCode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for HqCountryCode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::validate(&s).map_err(D::Error::custom)?;
        Ok(Self(s))
    }
}

/// Re-validate a dispatcher `args` blob into a typed envelope. Shared by every
/// envelope's inherent `from_args`; it enforces `deny_unknown_fields` and the
/// enum/newtype invariants, so an arbitrary or smuggled `req` is rejected here
/// too (the elevated side never trusts the caller). The error is a
/// `HardwareError`, not a bare `String`, per the central error catalog.
fn parse_envelope<T: serde::de::DeserializeOwned>(args: &Value) -> Result<T, HardwareError> {
    serde_json::from_value::<T>(args.clone())
        .map_err(|e| HardwareError::Other(format!("invalid HQ BIOS request: {e}")))
}

/// Set the BIOS performance profile. `token` is a fixed "1"/"2"/"3".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct HqSetPerformanceModeReq {
    pub mode: HqPerformanceMode,
}
impl HqSetPerformanceModeReq {
    pub fn from_args(args: &Value) -> Result<Self, HardwareError> {
        parse_envelope(args)
    }
    pub fn token(&self) -> &str {
        self.mode.wire()
    }
}

/// Reorder the boot devices. `token` is a fixed firmware device string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct HqChangeBootOptionReq {
    pub target: HqBootTarget,
}
impl HqChangeBootOptionReq {
    pub fn from_args(args: &Value) -> Result<Self, HardwareError> {
        parse_envelope(args)
    }
    pub fn token(&self) -> &str {
        self.target.wire()
    }
}

/// Reset the BIOS to firmware defaults. Destructive; needs the explicit
/// confirm variant. `token` is the fixed confirmation constant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct HqLoadDefaultReq {
    pub confirm: HqLoadDefaultConfirm,
}
impl HqLoadDefaultReq {
    pub fn from_args(args: &Value) -> Result<Self, HardwareError> {
        parse_envelope(args)
    }
    pub fn token(&self) -> &str {
        self.confirm.wire()
    }
}

/// Enable/disable PXE (network) boot. `token` is "1"/"0".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct HqEnablePxeBootReq {
    pub enabled: bool,
}
impl HqEnablePxeBootReq {
    pub fn from_args(args: &Value) -> Result<Self, HardwareError> {
        parse_envelope(args)
    }
    pub fn token(&self) -> &str {
        if self.enabled {
            "1"
        } else {
            "0"
        }
    }
}

/// Enable/disable S5 RTC wake. `token` is "1"/"0".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct HqS5RtcWakeEnableReq {
    pub enabled: bool,
}
impl HqS5RtcWakeEnableReq {
    pub fn from_args(args: &Value) -> Result<Self, HardwareError> {
        parse_envelope(args)
    }
    pub fn token(&self) -> &str {
        if self.enabled {
            "1"
        } else {
            "0"
        }
    }
}

/// Set the WiFi regulatory country code. `token` is the validated ISO code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct HqSetWifiCountryCodeReq {
    pub country_code: HqCountryCode,
}
impl HqSetWifiCountryCodeReq {
    pub fn from_args(args: &Value) -> Result<Self, HardwareError> {
        parse_envelope(args)
    }
    pub fn token(&self) -> &str {
        self.country_code.as_str()
    }
}

/// Set the shipping country code. `token` is the validated ISO code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct HqSetShippingCountryCodeReq {
    pub country_code: HqCountryCode,
}
impl HqSetShippingCountryCodeReq {
    pub fn from_args(args: &Value) -> Result<Self, HardwareError> {
        parse_envelope(args)
    }
    pub fn token(&self) -> &str {
        self.country_code.as_str()
    }
}

// ── WMI implementation ─────────────────────────────────────────────────────

#[cfg(windows)]
mod imp {
    use super::*;
    use crate::hw::wmi_cache;
    use windows::core::{BSTR, HSTRING, PCWSTR, VARIANT};
    use windows::Win32::System::Wmi::{
        IWbemClassObject, WBEM_FLAG_FORWARD_ONLY, WBEM_FLAG_RETURN_IMMEDIATELY,
        WBEM_GENERIC_FLAG_TYPE,
    };

    const VT_BSTR_VAL: u16 = windows::core::imp::VT_BSTR;

    /// Helper to create a PCWSTR from a string.
    fn pcwstr(s: &str) -> PCWSTR {
        let h = HSTRING::from(s);
        PCWSTR::from_raw(h.as_ptr())
    }

    /// Call a method on HQWmiCommonInterface with a String parameter.
    pub fn call_method(method: &str, req: &str) -> HardwareResult<HqWmiResponse> {
        wmi_cache::with_wmi(|conn| {
            let svc = &conn.svc;

            // Get the HQWmiCommonInterface class to obtain method signature
            let class_path = BSTR::from("HQWmiCommonInterface");
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

            // Get method signature
            let method_name = pcwstr(method);
            let mut in_sig: Option<IWbemClassObject> = None;
            let mut out_sig: Option<IWbemClassObject> = None;
            unsafe {
                class_obj.GetMethod(method_name, 0, &mut in_sig, &mut out_sig)?;
            }

            // Spawn input parameters
            let in_params = if let Some(in_sig) = in_sig {
                let params = unsafe { in_sig.SpawnInstance(0)? };
                // Set the "req" parameter — build a VARIANT containing a BSTR
                let prop_name = pcwstr("req");
                let req_bstr = BSTR::from(req);
                let in_data_var = build_bstr_variant(&req_bstr)?;
                unsafe { params.Put(prop_name, 0, &in_data_var, 0)? };
                Some(params)
            } else {
                None
            };

            // Query for the active HQWmiCommonInterface instance to get its __Path
            let query = BSTR::from("SELECT * FROM HQWmiCommonInterface WHERE Active = TRUE");
            let wql = BSTR::from("WQL");
            let flags = WBEM_FLAG_FORWARD_ONLY | WBEM_FLAG_RETURN_IMMEDIATELY;
            let enumerator = unsafe { svc.ExecQuery(&wql, &query, flags, None)? };

            let mut objs: [Option<IWbemClassObject>; 1] = [None];
            let mut returned: u32 = 0;
            let hr = unsafe { enumerator.Next(-1, &mut objs, &mut returned) };
            hr.ok()?;

            let instance = objs[0].as_ref().ok_or_else(|| {
                anyhow::anyhow!(HardwareError::Wmi(
                    "No active HQWmiCommonInterface instance found".into()
                ))
            })?;

            // Get the instance path
            let path_name = pcwstr("__Path");
            let mut path_var = VARIANT::new();
            let mut cim_type: i32 = 0;
            unsafe {
                instance.Get(path_name, 0, &mut path_var, Some(&mut cim_type), None)?;
            }
            let path_str = variant_to_string(&path_var)?;

            // Call ExecMethod on IWbemServices
            let path_bstr = BSTR::from(path_str);
            let method_bstr = BSTR::from(method);
            let mut out_params: Option<IWbemClassObject> = None;
            unsafe {
                svc.ExecMethod(
                    &path_bstr,
                    &method_bstr,
                    WBEM_GENERIC_FLAG_TYPE(0),
                    None,
                    in_params.as_ref(),
                    Some(&mut out_params),
                    None,
                )?;
            }

            let out_params = out_params.ok_or_else(|| {
                anyhow::anyhow!(HardwareError::Wmi(
                    "ExecMethod returned null out_params".into()
                ))
            })?;

            // Read the "ret" return value
            let ret_name = pcwstr("ret");
            let mut ret_var = VARIANT::new();
            unsafe {
                out_params.Get(ret_name, 0, &mut ret_var, None, None)?;
            }

            let ret = variant_to_string(&ret_var)?;

            let success = !ret.is_empty()
                && !ret.to_lowercase().contains("fail")
                && !ret.to_lowercase().contains("error");

            Ok(HqWmiResponse {
                method: method.to_string(),
                req: req.to_string(),
                ret,
                success,
            })
        })
    }

    /// Build a VARIANT containing a BSTR value.
    fn build_bstr_variant(bstr: &BSTR) -> anyhow::Result<VARIANT> {
        let raw = windows::core::imp::VARIANT {
            Anonymous: windows::core::imp::VARIANT_0 {
                Anonymous: windows::core::imp::VARIANT_0_0 {
                    vt: VT_BSTR_VAL,
                    wReserved1: 0,
                    wReserved2: 0,
                    wReserved3: 0,
                    Anonymous: windows::core::imp::VARIANT_0_0_0 {
                        bstrVal: bstr.as_ptr(),
                    },
                },
            },
        };
        Ok(unsafe { VARIANT::from_raw(raw) })
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
            let bstr = unsafe { BSTR::from_raw(bstr_ptr) };
            Ok(bstr.to_string())
        } else {
            Err(anyhow::anyhow!(HardwareError::Wmi(format!(
                "ret has unexpected vt=0x{vt:04X}"
            ))))
        }
    }
}

// ── Public API ─────────────────────────────────────────────────────────────

/// Set BIOS performance mode via HQWmiCommonInterface.
///
/// `req` is typically a string like "1" (performance), "2" (balanced), "3" (quiet).
pub fn set_performance_mode(req: &str) -> HardwareResult<HqWmiResponse> {
    #[cfg(windows)]
    {
        imp::call_method("SetPerformanceMode", req)
    }
    #[cfg(not(windows))]
    {
        let _ = req;
        Err(HardwareError::NotSupported(
            "WMI only available on Windows".into(),
        ))
    }
}

/// Change boot device order via HQWmiCommonInterface.
pub fn change_boot_option(req: &str) -> HardwareResult<HqWmiResponse> {
    #[cfg(windows)]
    {
        imp::call_method("ChangeBootOption", req)
    }
    #[cfg(not(windows))]
    {
        let _ = req;
        Err(HardwareError::NotSupported(
            "WMI only available on Windows".into(),
        ))
    }
}

/// Load BIOS default settings.
pub fn load_default(req: &str) -> HardwareResult<HqWmiResponse> {
    #[cfg(windows)]
    {
        imp::call_method("LoadDefault", req)
    }
    #[cfg(not(windows))]
    {
        let _ = req;
        Err(HardwareError::NotSupported(
            "WMI only available on Windows".into(),
        ))
    }
}

/// Enable/disable S5 RTC wake.
pub fn s5_rtc_wake_enable(req: &str) -> HardwareResult<HqWmiResponse> {
    #[cfg(windows)]
    {
        imp::call_method("S5RTCWakeEnable", req)
    }
    #[cfg(not(windows))]
    {
        let _ = req;
        Err(HardwareError::NotSupported(
            "WMI only available on Windows".into(),
        ))
    }
}

/// Enable/disable PXE boot.
pub fn enable_pxe_boot(req: &str) -> HardwareResult<HqWmiResponse> {
    #[cfg(windows)]
    {
        imp::call_method("EnablePXEBoot", req)
    }
    #[cfg(not(windows))]
    {
        let _ = req;
        Err(HardwareError::NotSupported(
            "WMI only available on Windows".into(),
        ))
    }
}

/// Load default security key.
pub fn load_default_key(req: &str) -> HardwareResult<HqWmiResponse> {
    #[cfg(windows)]
    {
        imp::call_method("LoadDefaultKey", req)
    }
    #[cfg(not(windows))]
    {
        let _ = req;
        Err(HardwareError::NotSupported(
            "WMI only available on Windows".into(),
        ))
    }
}

/// Clear security key.
pub fn clear_key(req: &str) -> HardwareResult<HqWmiResponse> {
    #[cfg(windows)]
    {
        imp::call_method("ClearKey", req)
    }
    #[cfg(not(windows))]
    {
        let _ = req;
        Err(HardwareError::NotSupported(
            "WMI only available on Windows".into(),
        ))
    }
}

/// Set OOB test mode.
pub fn set_oob_test_mode(req: &str) -> HardwareResult<HqWmiResponse> {
    #[cfg(windows)]
    {
        imp::call_method("SetOOBTestMode", req)
    }
    #[cfg(not(windows))]
    {
        let _ = req;
        Err(HardwareError::NotSupported(
            "WMI only available on Windows".into(),
        ))
    }
}

/// Clear OOB test mode.
pub fn clear_oob_test_mode(req: &str) -> HardwareResult<HqWmiResponse> {
    #[cfg(windows)]
    {
        imp::call_method("ClearOOBTestMode", req)
    }
    #[cfg(not(windows))]
    {
        let _ = req;
        Err(HardwareError::NotSupported(
            "WMI only available on Windows".into(),
        ))
    }
}

/// Set WiFi country code.
pub fn set_wifi_country_code(req: &str) -> HardwareResult<HqWmiResponse> {
    #[cfg(windows)]
    {
        imp::call_method("WifiCountryCode", req)
    }
    #[cfg(not(windows))]
    {
        let _ = req;
        Err(HardwareError::NotSupported(
            "WMI only available on Windows".into(),
        ))
    }
}

/// Set shipping country code.
pub fn set_shipping_country_code(req: &str) -> HardwareResult<HqWmiResponse> {
    #[cfg(windows)]
    {
        imp::call_method("ShippingCountryCode", req)
    }
    #[cfg(not(windows))]
    {
        let _ = req;
        Err(HardwareError::NotSupported(
            "WMI only available on Windows".into(),
        ))
    }
}

// ── Tests (SEC-11 typed layer) ─────────────────────────────────────────────
//
// These exercise only the typed, allowlisted request envelope (serde parsing +
// fixed wire tokens), so they run on every platform - the WMI call itself is
// never reached, which is exactly the point: an invalid or arbitrary payload
// is rejected at the type boundary, before any BIOS write.
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── (1) invalid payload rejected before WMI ────────────────────────────

    #[test]
    fn performance_mode_rejects_unknown_variant() {
        // "turbo" is a valid *EC* mode but not a valid HQ BIOS profile.
        let bad = json!({ "mode": "turbo" });
        assert!(HqSetPerformanceModeReq::from_args(&bad).is_err());
    }

    #[test]
    fn boot_option_rejects_unknown_target_and_free_form_req() {
        // An arbitrary device string the caller invented is refused.
        assert!(HqChangeBootOptionReq::from_args(&json!({ "target": "My USB Stick" })).is_err());
        // The old raw `req` field no longer exists: deny_unknown_fields rejects it.
        assert!(
            HqChangeBootOptionReq::from_args(&json!({ "req": "Windows Boot Manager" })).is_err()
        );
    }

    #[test]
    fn load_default_requires_the_explicit_confirm_variant() {
        assert!(HqLoadDefaultReq::from_args(&json!({ "confirm": "yes_please" })).is_err());
        assert!(HqLoadDefaultReq::from_args(&json!({ "req": "1" })).is_err());
    }

    #[test]
    fn pxe_and_rtc_reject_non_bool_and_free_form_req() {
        assert!(HqEnablePxeBootReq::from_args(&json!({ "enabled": "1" })).is_err());
        assert!(HqS5RtcWakeEnableReq::from_args(&json!({ "enabled": 1 })).is_err());
        assert!(HqEnablePxeBootReq::from_args(&json!({ "req": "1" })).is_err());
    }

    #[test]
    fn country_code_rejects_out_of_allowlist_and_malformed() {
        // Lowercase, wrong length, unknown code and free-form req are all refused.
        assert!(HqSetWifiCountryCodeReq::from_args(&json!({ "countryCode": "pt" })).is_err());
        assert!(HqSetWifiCountryCodeReq::from_args(&json!({ "countryCode": "P" })).is_err());
        assert!(HqSetWifiCountryCodeReq::from_args(&json!({ "countryCode": "ZZ" })).is_err());
        assert!(HqSetShippingCountryCodeReq::from_args(&json!({ "req": "PT" })).is_err());
    }

    #[test]
    fn deny_unknown_fields_blocks_smuggled_extra_keys() {
        // A valid field plus a smuggled arbitrary `req` is rejected wholesale.
        let mixed = json!({ "mode": "balanced", "req": "rm -rf /" });
        assert!(HqSetPerformanceModeReq::from_args(&mixed).is_err());
    }

    // ── (2) valid payload yields a fixed token, never caller text ──────────

    #[test]
    fn valid_payloads_map_to_fixed_tokens() {
        let perf = HqSetPerformanceModeReq::from_args(&json!({ "mode": "quiet" })).unwrap();
        assert_eq!(perf.token(), "3");
        // Serialising back to the wire never carries the caller's words.
        assert_eq!(
            serde_json::to_value(&perf).unwrap(),
            json!({ "mode": "quiet" })
        );

        let boot =
            HqChangeBootOptionReq::from_args(&json!({ "target": "windows_boot_manager" })).unwrap();
        assert_eq!(boot.token(), "Windows Boot Manager");

        let ld = HqLoadDefaultReq::from_args(&json!({ "confirm": "reset_to_defaults" })).unwrap();
        assert_eq!(ld.token(), "1");

        let pxe_on = HqEnablePxeBootReq::from_args(&json!({ "enabled": true })).unwrap();
        assert_eq!(pxe_on.token(), "1");
        let pxe_off = HqEnablePxeBootReq::from_args(&json!({ "enabled": false })).unwrap();
        assert_eq!(pxe_off.token(), "0");

        let rtc = HqS5RtcWakeEnableReq::from_args(&json!({ "enabled": true })).unwrap();
        assert_eq!(rtc.token(), "1");

        let wifi = HqSetWifiCountryCodeReq::from_args(&json!({ "countryCode": "PT" })).unwrap();
        assert_eq!(wifi.token(), "PT");
        let ship = HqSetShippingCountryCodeReq::from_args(&json!({ "countryCode": "US" })).unwrap();
        assert_eq!(ship.token(), "US");
    }

    // ── (3) the command cannot accept an arbitrary `req` ───────────────────

    #[test]
    fn no_envelope_has_a_req_field() {
        // The whole attack class dies because `req` is not a field on any
        // envelope. Feed each envelope the legacy `{ "req": ... }` shape and
        // every one must reject it (deny_unknown_fields + missing required).
        let legacy = json!({ "req": "anything" });
        assert!(HqSetPerformanceModeReq::from_args(&legacy).is_err());
        assert!(HqChangeBootOptionReq::from_args(&legacy).is_err());
        assert!(HqLoadDefaultReq::from_args(&legacy).is_err());
        assert!(HqS5RtcWakeEnableReq::from_args(&legacy).is_err());
        assert!(HqEnablePxeBootReq::from_args(&legacy).is_err());
        assert!(HqSetWifiCountryCodeReq::from_args(&legacy).is_err());
        assert!(HqSetShippingCountryCodeReq::from_args(&legacy).is_err());
    }

    #[test]
    fn country_code_allowlist_is_iso_alpha2_and_nonempty() {
        // Sanity: every allowlisted code is exactly two uppercase ASCII letters.
        assert!(!ALLOWED_COUNTRY_CODES.is_empty());
        for code in ALLOWED_COUNTRY_CODES {
            let b = code.as_bytes();
            assert_eq!(b.len(), 2, "allowlist entry {code} is not alpha-2");
            assert!(
                b.iter().all(u8::is_ascii_uppercase),
                "allowlist entry {code} not uppercase"
            );
        }
        // Portugal and the US are the two codes the device actually uses.
        assert!(ALLOWED_COUNTRY_CODES.contains(&"PT"));
        assert!(ALLOWED_COUNTRY_CODES.contains(&"US"));
    }
}
