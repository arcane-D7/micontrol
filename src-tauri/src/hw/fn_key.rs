//! Fn-Key customization — Fn-lock toggle and function key behavior.
//!
//! Provides access to the Fn-lock state via EC RAM register 0x4A.
//! When Fn-lock is enabled, the F1-F12 keys act as standard function keys
//! (F1, F2, ...) instead of multimedia keys (brightness, volume, etc.).
//!
//! This mirrors XPM's `get_function_key` / `set_function_key` feature.
//!
//! EC Register 0x4A:
//!   Bit 0: Fn-lock state (0 = multimedia keys default, 1 = F1-F12 default)
//!
//! The offset 0x4A is in the safe-write allowlist, so single-byte writes
//! don't require the raw-write env var override.

use crate::hw::ecram::{get_eram_base, read_ecram, write_ecram};
use crate::hw::errors::{HardwareError, HardwareResult};
use serde::{Deserialize, Serialize};

/// EC RAM offset for Fn-lock state.
const FN_LOCK_ERAM_OFFSET: usize = 0x4A;

/// Registry key for persisting Fn-lock state.
#[cfg(windows)]
const FN_KEY_REG_KEY: &str = r"SOFTWARE\MiControl\FnKey";

/// Registry value for Fn-lock enabled state.
const FN_LOCK_ENABLED_VALUE: &str = "FnLockEnabled";

/// Fn-key mode.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FnKeyMode {
    /// F1-F12 act as multimedia keys (brightness, volume, etc.)
    Multimedia,
    /// F1-F12 act as standard function keys
    FunctionKey,
}

impl FnKeyMode {
    /// Convert to EC register bit value.
    fn to_ec_value(self) -> u8 {
        match self {
            FnKeyMode::Multimedia => 0,
            FnKeyMode::FunctionKey => 1,
        }
    }

    /// Convert from EC register bit value.
    fn from_ec_value(val: u8) -> Self {
        if val & 0x01 != 0 {
            FnKeyMode::FunctionKey
        } else {
            FnKeyMode::Multimedia
        }
    }
}

/// Fn-key status information.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FnKeyStatus {
    /// Current Fn-lock mode.
    pub mode: FnKeyMode,
    /// Whether Fn-lock is enabled (true = FunctionKey mode).
    pub fn_lock_enabled: bool,
}

/// Get the current Fn-key mode.
///
/// Reads EC register 0x4A to determine the Fn-lock state.
/// Falls back to registry if EC read fails.
pub fn get_function_key() -> HardwareResult<FnKeyStatus> {
    // Try EC read first
    #[cfg(windows)]
    {
        match read_ecram(get_eram_base() + FN_LOCK_ERAM_OFFSET as u64, 1) {
            Ok(data) if !data.is_empty() => {
                let mode = FnKeyMode::from_ec_value(data[0]);
                log::debug!(
                    target: "hw::fn_key",
                    "Fn-lock read from EC 0x{:02X}: mode={:?}",
                    FN_LOCK_ERAM_OFFSET,
                    mode
                );
                return Ok(FnKeyStatus {
                    mode,
                    fn_lock_enabled: matches!(mode, FnKeyMode::FunctionKey),
                });
            }
            Ok(_) => {
                log::warn!(
                    target: "hw::fn_key",
                    "EC read returned empty data for Fn-lock, falling back to registry"
                );
            }
            Err(e) => {
                log::warn!(
                    target: "hw::fn_key",
                    "EC read failed for Fn-lock: {e}, falling back to registry"
                );
            }
        }

        // Fallback: read from registry
        let mode = get_fn_lock_registry()
            .map(FnKeyMode::from_ec_value)
            .unwrap_or(FnKeyMode::Multimedia);

        Ok(FnKeyStatus {
            mode,
            fn_lock_enabled: matches!(mode, FnKeyMode::FunctionKey),
        })
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "Fn-key only available on Windows".into(),
    ))
}

/// Set the Fn-key mode.
///
/// Writes to EC register 0x4A to toggle Fn-lock state.
/// Also persists to registry for fallback reads.
///
/// S41-001: EC write is now best-effort (mirrors `set_battery_care` S38-001).
/// The IoTDriver security check requires the calling process to live under
/// the IoTDriver.sys DriverStore directory — the elevated scheduled-task
/// helper does NOT satisfy that, so the IOCTL fails with STATUS_ACCESS_DENIED
/// even when elevated. We persist the requested state to the registry (which
/// drives re-assertion + what the UI reports) and only surface an error when
/// the registry write itself fails.
pub fn set_function_key(mode: FnKeyMode) -> HardwareResult<()> {
    let ec_val = mode.to_ec_value();

    // Best-effort write to EC. When running from the DriverStore the write
    // succeeds; otherwise it is expected to fail with access denied, which we
    // log and continue so the registry still reflects the user's intent.
    match write_ecram(get_eram_base() + FN_LOCK_ERAM_OFFSET as u64, &[ec_val]) {
        Ok(()) => {
            // Read back to verify
            match read_ecram(get_eram_base() + FN_LOCK_ERAM_OFFSET as u64, 1) {
                Ok(data) if !data.is_empty() => {
                    let actual = FnKeyMode::from_ec_value(data[0]);
                    if actual != mode {
                        log::warn!(
                            target: "hw::fn_key",
                            "Fn-lock verification mismatch: wrote {:?}, read back {:?}",
                            mode,
                            actual
                        );
                    } else {
                        log::info!(
                            target: "hw::fn_key",
                            "Fn-lock set to {:?} (EC 0x{:02X} = 0x{:02X})",
                            mode,
                            FN_LOCK_ERAM_OFFSET,
                            ec_val
                        );
                    }
                }
                _ => {
                    log::warn!(
                        target: "hw::fn_key",
                        "Fn-lock write verification failed — could not read back EC 0x{:02X}",
                        FN_LOCK_ERAM_OFFSET
                    );
                }
            }
        }
        Err(e) => {
            // Expected when running as the scheduled-task helper (not in
            // DriverStore). Persist the state anyway so the UI stays
            // consistent and re-assertion can happen from a privileged path.
            log::warn!(
                target: "hw::fn_key",
                "Fn-lock EC write unavailable ({e}) — persisting registry state"
            );
        }
    }

    // Persist to registry
    persist_fn_lock_registry(ec_val)?;

    Ok(())
}

/// Persist Fn-lock state to registry.
#[cfg(windows)]
fn persist_fn_lock_registry(value: u8) -> HardwareResult<()> {
    use crate::util::registry::RegKeyGuard;
    use windows::Win32::System::Registry::HKEY_CURRENT_USER;

    let key = RegKeyGuard::create_write(HKEY_CURRENT_USER, FN_KEY_REG_KEY)
        .map_err(|e| HardwareError::Registry(format!("Create FnKey key: {e}")))?;

    key.write_u32(FN_LOCK_ENABLED_VALUE, value as u32)
        .map_err(|e| HardwareError::Registry(format!("Write FnLock enabled: {e}")))?;

    Ok(())
}

/// Read Fn-lock state from registry.
#[cfg(windows)]
fn get_fn_lock_registry() -> Option<u8> {
    use crate::util::registry::RegKeyGuard;
    use windows::Win32::System::Registry::HKEY_CURRENT_USER;

    let key = RegKeyGuard::open_read(HKEY_CURRENT_USER, FN_KEY_REG_KEY).ok()??;
    key.read_u32(FN_LOCK_ENABLED_VALUE).ok()?.map(|v| v as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fn_key_mode_conversion() {
        assert_eq!(FnKeyMode::Multimedia.to_ec_value(), 0);
        assert_eq!(FnKeyMode::FunctionKey.to_ec_value(), 1);

        assert_eq!(FnKeyMode::from_ec_value(0), FnKeyMode::Multimedia);
        assert_eq!(FnKeyMode::from_ec_value(1), FnKeyMode::FunctionKey);
        assert_eq!(FnKeyMode::from_ec_value(0x01), FnKeyMode::FunctionKey);
        assert_eq!(FnKeyMode::from_ec_value(0x00), FnKeyMode::Multimedia);
    }
}
