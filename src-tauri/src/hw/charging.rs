//! Charging limit ("Battery Care") — firmware-authoritative path.
//!
//! # What actually controls the limit on this platform
//!
//! The limit lives in two EC registers: `ERAM 0xA4` (`LONL`, bit0 = care on)
//! and `ERAM 0xA7` (`HBDA`, the limit in percent). Both sit inside the
//! `SystemMemory 0xFE0B0300` region that `IoTDriver.sys` **refuses** to expose
//! through its ECRAM IOCTLs (`STATUS_ACCESS_DENIED` / `0x80070005`, verified
//! live even for the SYSTEM `IoTService.exe`). Every ERAM write path this file
//! used to attempt failed and silently fell back to the registry, so the UI
//! showed a limit the hardware never received.
//!
//! The firmware exposes the same two registers through the ACPI `WMAA` method
//! (`MICommonInterface.MiInterface`, `\_SB.PC00.WMID`, `_HID` `PNP0C14`,
//! `_UID` `"MIFS"`). Decoded from `ssdt24.dsl`, Method `WMAA`, `FUN1=0xFB00`,
//! `FUN2=0x1000`, `FUN3=0x02`:
//!
//! | FUN4 | effect                                        |
//! |------|-----------------------------------------------|
//! | 0    | `LONL &= ~1`, `HBDA = 0x64` → care OFF (100%) |
//! | 1    | `LONL |= 1`,  `HBDA = 0x50` → 80%             |
//! | 4    | `LONL |= 1`,  `HBDA = 0x5A` → 90%             |
//! | 5    | `LONL |= 1`,  `HBDA = 0x46` → 70%             |
//! | 6    | `LONL |= 1`,  `HBDA = 0x3C` → 60%             |
//! | 7    | `LONL |= 1`,  `HBDA = 0x32` → 50%             |
//! | 8    | `LONL |= 1`,  `HBDA = 0x28` → 40%             |
//!
//! The read branch (`FUN1=0xFA00`, same FUN2/FUN3) returns the same code in
//! `FRD1`: 0 = care off, otherwise the active level. The code set is the OEM
//! enum the original PC Manager UI sends (`gp.One=1 … gp.Eight=8`), i.e. the
//! payload is a **level code, not a percentage**.
//!
//! This module therefore writes and verifies through WMAA and never claims
//! success without a read-back. The registry (`HKLM\SOFTWARE\MI\IoTDriver`
//! `ChargingThreshold`) is only the user's *requested* value and for
//! re-assertion; it is never reported as applied hardware state.

use crate::hw::errors::{HardwareError, HardwareResult};
use serde::{Deserialize, Serialize};

/// Registry key shared with the original Xiaomi IoT software.
#[cfg(windows)]
const CHARGE_REG_KEY: &str = r"SOFTWARE\MI\IoTDriver";
#[cfg(windows)]
const CHARGE_REG_VALUE: &str = "ChargingThreshold";

/// WMAA sub-command group used by the charging-protection method.
pub const FUN2_CHARGE: u16 = 0x1000;
/// WMAA action id inside `FUN2_CHARGE` (read and write share it).
pub const FUN3_CHARGE: u16 = 0x02;
/// `SGER` value the ACPI method returns on success.
pub const SGER_SUCCESS: u16 = 0x8000;

/// Percentages the firmware can express. 100 means "no limit" (care off).
pub const VALID_THRESHOLDS: [u8; 7] = [40, 50, 60, 70, 80, 90, 100];

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct ChargingResult {
    /// True only when the firmware accepted **and** read back the new value.
    pub success: bool,
    /// `wmi_wmaa` when the value is on the hardware, `unverified` otherwise.
    pub method: String,
    /// The value the caller asked for.
    pub threshold: u8,
    /// The value the firmware reported after the write (`None` = unreadable).
    pub hardware_threshold: Option<u8>,
    /// Human-readable reason when `success` is false.
    pub detail: Option<String>,
}

/// The truth about the charging limit: what the user asked for, what the
/// firmware actually holds, and whether they agree.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct ChargeLimitState {
    /// Last requested value (registry). 100 = no limit.
    pub requested_threshold: u8,
    /// Value read back from the firmware; `None` when the read failed.
    pub hardware_threshold: Option<u8>,
    /// Raw firmware level code (0,1,4,5,6,7,8); `None` when unreadable.
    pub hardware_code: Option<u8>,
    /// True only when the firmware read succeeded and equals the request.
    pub applied: bool,
    /// `firmware` | `registry` (read failed) — how much of this is real.
    pub source: String,
    /// Why the hardware value is unknown, when it is.
    pub detail: Option<String>,
}

/// Map a user-facing percentage to the firmware level code.
///
/// Returns `None` for values the firmware cannot express.
pub fn percent_to_code(percent: u8) -> Option<u8> {
    match percent {
        40 => Some(8),
        50 => Some(7),
        60 => Some(6),
        70 => Some(5),
        80 => Some(1),
        90 => Some(4),
        100 => Some(0),
        _ => None,
    }
}

/// Map a firmware level code back to a percentage.
pub fn code_to_percent(code: u8) -> Option<u8> {
    match code {
        8 => Some(40),
        7 => Some(50),
        6 => Some(60),
        5 => Some(70),
        1 => Some(80),
        4 => Some(90),
        0 => Some(100),
        _ => None,
    }
}

/// Parse the `FRD1` value of a `FUN2=0x1000 / FUN3=0x02` read into a code.
///
/// `FRD1` is a DWORD; anything outside the OEM code set is a firmware
/// surprise and is rejected instead of being coerced into a percentage.
pub fn parse_level_code(frd1: u32) -> Option<u8> {
    let code = u8::try_from(frd1).ok()?;
    code_to_percent(code).map(|_| code)
}

fn validate(threshold: u8) -> HardwareResult<u8> {
    percent_to_code(threshold).ok_or_else(|| {
        HardwareError::InvalidConfig(format!(
            "Invalid threshold {threshold}. Must be one of: 40,50,60,70,80,90,100"
        ))
    })
}

/// Read the limit straight from the firmware.
///
/// Returns `Err` when the WMAA read fails (no admin, provider stall, WMI
/// blocked). Callers must not translate that error into a value.
pub fn read_hardware_threshold() -> HardwareResult<u8> {
    #[cfg(windows)]
    {
        let resp = crate::hw::wmi_ec::wmi_read(FUN2_CHARGE, FUN3_CHARGE)?;
        if resp.sger != SGER_SUCCESS {
            return Err(HardwareError::Wmi(format!(
                "WMAA 0x1000/0x02 read rejected: SGER=0x{:04X}",
                resp.sger
            )));
        }
        let code = parse_level_code(resp.frd1).ok_or_else(|| {
            HardwareError::Wmi(format!(
                "WMAA 0x1000/0x02 returned unknown level code {:#X}",
                resp.frd1
            ))
        })?;
        Ok(code_to_percent(code).expect("parse_level_code validated the code"))
    }
    #[cfg(not(windows))]
    {
        Err(HardwareError::NotSupported(
            "WMI only available on Windows".into(),
        ))
    }
}

/// Set the charging limit in the firmware and verify it by reading back.
///
/// There is deliberately **no silent fallback**: a failed write is an error,
/// because reporting success while the EC still charges to 100% is exactly
/// the bug this function exists to fix.
pub fn set_charging_threshold(threshold: u8) -> HardwareResult<ChargingResult> {
    let code = validate(threshold)?;

    #[cfg(windows)]
    {
        let resp = crate::hw::wmi_ec::wmi_write(FUN2_CHARGE, FUN3_CHARGE, code as u32)?;
        if resp.sger != SGER_SUCCESS {
            return Err(HardwareError::Wmi(format!(
                "WMAA 0x1000/0x02 write rejected: SGER=0x{:04X}",
                resp.sger
            )));
        }

        // The ACPI method sets SGER before touching the EC and never reports
        // a failure of the ECWT itself, so SGER alone is not proof. Read back.
        match read_hardware_threshold() {
            Ok(actual) if actual == threshold => {
                persist_threshold_registry(threshold).ok();
                log::info!(
                    target: "hw::charging",
                    "charging limit {threshold}% applied on firmware (WMAA 0x1000/0x02, code {code})"
                );
                Ok(ChargingResult {
                    success: true,
                    method: "wmi_wmaa".to_string(),
                    threshold,
                    hardware_threshold: Some(actual),
                    detail: None,
                })
            }
            Ok(actual) => {
                log::warn!(
                    target: "hw::charging",
                    "charging limit write acknowledged but firmware reads {actual}% (wanted {threshold}%)"
                );
                Err(HardwareError::Wmi(format!(
                    "firmware accepted the write but still reports {actual}% (wanted {threshold}%)"
                )))
            }
            Err(e) => {
                log::warn!(
                    target: "hw::charging",
                    "charging limit written but read-back failed: {e}"
                );
                Err(HardwareError::Wmi(format!(
                    "limit written to firmware but the read-back failed: {e}"
                )))
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = code;
        Err(HardwareError::NotSupported(
            "WMI only available on Windows".into(),
        ))
    }
}

/// Combined view: requested value (registry) + firmware value.
///
/// Never fails: an unreadable firmware is reported as
/// `hardware_threshold: None, applied: false, detail: Some(..)` so the UI can
/// say "not confirmed on the hardware" instead of inventing a value.
pub fn get_charge_limit_state() -> ChargeLimitState {
    let requested = get_requested_threshold();
    match read_hardware_threshold() {
        Ok(hardware) => ChargeLimitState {
            requested_threshold: requested,
            hardware_threshold: Some(hardware),
            hardware_code: percent_to_code(hardware),
            applied: hardware == requested,
            source: "firmware".to_string(),
            detail: None,
        },
        Err(e) => ChargeLimitState {
            requested_threshold: requested,
            hardware_threshold: None,
            hardware_code: None,
            applied: false,
            source: "registry".to_string(),
            detail: Some(e.to_string()),
        },
    }
}

/// The user's requested limit (registry), 100 when unset.
///
/// This is **not** hardware state — call it what it is.
pub fn get_requested_threshold() -> u8 {
    #[cfg(windows)]
    {
        read_threshold_registry()
            .and_then(|r| r.ok())
            .unwrap_or(100)
    }
    #[cfg(not(windows))]
    {
        100
    }
}

/// Backwards-compatible accessor: the requested limit.
///
/// Prefer [`get_charge_limit_state`] when the UI needs to tell the truth about
/// whether the hardware accepted it.
pub fn get_charging_threshold() -> HardwareResult<u8> {
    Ok(get_requested_threshold())
}

// ── Battery Care toggle ─────────────────────────────────────────────────────
//
// "Care" and "limit" are the same firmware field: code 0 means care off
// (charges to 100%), any other code means care on at that level. The toggle is
// therefore expressed through the same WMAA write.

/// Is charging protection currently active on the firmware?
pub fn get_battery_care() -> HardwareResult<bool> {
    match read_hardware_threshold() {
        Ok(percent) => Ok(percent != 100),
        Err(e) => {
            log::warn!(
                target: "hw::charging",
                "battery care state unreadable from firmware: {e}"
            );
            Err(e)
        }
    }
}

/// Turn charging protection on (at `limit`) or off (charges to 100%).
pub fn set_battery_care(enabled: bool, limit: u8) -> HardwareResult<ChargingResult> {
    let target = if enabled {
        if limit >= 100 || percent_to_code(limit).is_none() {
            80
        } else {
            limit
        }
    } else {
        100
    };
    set_charging_threshold(target)
}

// ── Registry persistence (requested value only) ─────────────────────────────

fn persist_threshold_registry(threshold: u8) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
        match RegKeyGuard::create_write(HKEY_LOCAL_MACHINE, CHARGE_REG_KEY) {
            Ok(key) => {
                key.write_u32(CHARGE_REG_VALUE, threshold as u32)
                    .map_err(|e| {
                        HardwareError::Registry(format!(
                            "Write charging threshold to registry: {e}"
                        ))
                    })?;
            }
            Err(e) => {
                log::debug!("Cannot open charging registry key for write: {e}");
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn read_threshold_registry() -> Option<HardwareResult<u8>> {
    use crate::util::registry::RegKeyGuard;
    use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
    let key = RegKeyGuard::open_read(HKEY_LOCAL_MACHINE, CHARGE_REG_KEY).ok()??;
    match key.read_u32(CHARGE_REG_VALUE) {
        Ok(Some(data)) => Some(Ok(data.clamp(40, 100) as u8)),
        Ok(None) => None,
        Err(e) => Some(Err(HardwareError::Registry(format!(
            "Read charging threshold: {e}"
        )))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_to_code_matches_oem_enum_and_ssdt() {
        // OEM PC Manager enum gp: One=1 (80%), Four=4 (90%), Five=5 (70%),
        // Six=6 (60%), Seven=7 (50%), Eight=8 (40%); 0 = care off.
        assert_eq!(percent_to_code(80), Some(1));
        assert_eq!(percent_to_code(90), Some(4));
        assert_eq!(percent_to_code(70), Some(5));
        assert_eq!(percent_to_code(60), Some(6));
        assert_eq!(percent_to_code(50), Some(7));
        assert_eq!(percent_to_code(40), Some(8));
        assert_eq!(percent_to_code(100), Some(0));
    }

    #[test]
    fn percent_to_code_rejects_values_the_firmware_cannot_express() {
        for bad in [0u8, 1, 79, 81, 95, 99, 101, 255] {
            assert_eq!(percent_to_code(bad), None, "{bad} must not be writable");
        }
    }

    #[test]
    fn code_roundtrip_is_identity() {
        for p in VALID_THRESHOLDS {
            let c = percent_to_code(p).expect("valid");
            assert_eq!(code_to_percent(c), Some(p), "roundtrip for {p}%");
        }
    }

    #[test]
    fn parse_level_code_accepts_only_the_oem_code_set() {
        for code in [0u32, 1, 4, 5, 6, 7, 8] {
            assert_eq!(parse_level_code(code), Some(code as u8));
        }
        for bad in [2u32, 3, 9, 80, 0x50, 0xFFFF_FFFF] {
            assert_eq!(
                parse_level_code(bad),
                None,
                "{bad:#X} is not a firmware level code"
            );
        }
    }

    #[test]
    fn threshold_is_a_code_not_a_percentage() {
        // The historical bug: the MCPI 0x1003 payload carried the percentage
        // (80) where the firmware expects the level code (1). Pin the
        // distinction so it cannot regress silently.
        assert_ne!(percent_to_code(80).unwrap(), 80);
        assert_eq!(parse_level_code(80), None);
    }

    #[test]
    fn charging_result_serializes_the_honesty_fields() {
        let r = ChargingResult {
            success: false,
            method: "unverified".into(),
            threshold: 80,
            hardware_threshold: None,
            detail: Some("access denied".into()),
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["success"], serde_json::json!(false));
        assert_eq!(v["threshold"], serde_json::json!(80));
        assert_eq!(v["hardware_threshold"], serde_json::Value::Null);
        assert_eq!(v["detail"], serde_json::json!("access denied"));
    }

    #[test]
    fn state_never_claims_applied_without_hardware() {
        let s = ChargeLimitState {
            requested_threshold: 80,
            hardware_threshold: None,
            hardware_code: None,
            applied: false,
            source: "registry".into(),
            detail: Some("ECRAM blocked".into()),
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["applied"], serde_json::json!(false));
        assert_eq!(v["source"], serde_json::json!("registry"));
    }
}
