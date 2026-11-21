//! Eye protection / blue light filter.
//!
//! Provides a functional equivalent to XPM's "Eye Protection" feature using
//! Windows `SetDeviceGammaRamp` to reduce blue channel intensity, warming
//! the display color temperature.
//!
//! XPM uses proprietary `.m3d` 3D LUT calibration files and ICC profiles from
//! `icc-client.pc.mi.com`. We skip those and use the standard Win32 gamma ramp
//! API instead, which is reversible and requires no proprietary data.

use crate::hw::errors::{HardwareError, HardwareResult};
use serde::{Deserialize, Serialize};

/// Registry key for persisting eye protection state.
#[cfg(windows)]
const EYE_PROTECTION_REG_KEY: &str = r"SOFTWARE\MiControl\EyeProtection";

/// Registry value for enabled state.
const EYE_PROTECTION_ENABLED_VALUE: &str = "Enabled";

/// Registry value for intensity level (0-100).
const EYE_PROTECTION_INTENSITY_VALUE: &str = "Intensity";

/// Eye protection status.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct EyeProtectionStatus {
    pub enabled: bool,
    /// 0-100, where 0 = no filter, 100 = maximum warm filter.
    pub intensity: u8,
}

/// Default intensity when eye protection is enabled.
const DEFAULT_INTENSITY: u8 = 50;

// ── FFI for SetDeviceGammaRamp / GetDC / ReleaseDC ──────────────────────────
//
// The windows 0.58 crate does not expose `SetDeviceGammaRamp` or `GAMMA_RAMP`
// in the `Win32_Graphics_Gdi` feature, so we declare the FFI signatures
// directly.  The gamma ramp is a 3×256 array of `u16` (WORD) values.

#[cfg(windows)]
#[repr(C)]
#[derive(Clone, Copy)]
struct GammaRamp {
    red: [u16; 256],
    green: [u16; 256],
    blue: [u16; 256],
}

#[cfg(windows)]
impl Default for GammaRamp {
    fn default() -> Self {
        GammaRamp {
            red: [0u16; 256],
            green: [0u16; 256],
            blue: [0u16; 256],
        }
    }
}

#[cfg(windows)]
windows_targets::link!("gdi32.dll" "system" fn SetDeviceGammaRamp(hdc: *mut std::ffi::c_void, ramp: *mut GammaRamp) -> i32);

#[cfg(windows)]
windows_targets::link!("user32.dll" "system" fn GetDC(hwnd: *mut std::ffi::c_void) -> *mut std::ffi::c_void);

#[cfg(windows)]
windows_targets::link!("user32.dll" "system" fn ReleaseDC(hwnd: *mut std::ffi::c_void, hdc: *mut std::ffi::c_void) -> i32);

/// Get the current eye protection state: the state store first, with a
/// one-time read-only import of the legacy HKCU values on the first miss.
/// Corrupt or unavailable state degrades to the legacy registry read.
pub fn get_eye_protection() -> HardwareResult<EyeProtectionStatus> {
    #[cfg(windows)]
    {
        let store = crate::util::state_store::eye_protection_store();
        let legacy = legacy_eye_protection_state();
        let loaded = load_eye_from(&store, legacy);
        match loaded {
            Some(status) => Ok(status),
            None => get_eye_protection_registry(),
        }
    }
    #[cfg(not(windows))]
    {
        Ok(EyeProtectionStatus {
            enabled: false,
            intensity: DEFAULT_INTENSITY,
        })
    }
}

/// Shared load path (explicit store + injected legacy state for tests).
/// `Ok(Some(status))` from the store wins; the legacy import runs once on a
/// store miss; anything unreadable returns `None` and the caller falls back.
#[cfg(any(windows, test))]
fn load_eye_from(
    store: &Result<
        crate::util::state_store::Store<crate::util::state_store::EyeProtectionState>,
        crate::util::state_store::StoreError,
    >,
    legacy: Option<crate::util::state_store::EyeProtectionState>,
) -> Option<EyeProtectionStatus> {
    let Ok(store) = store else {
        return legacy.map(EyeProtectionStatus::from_state);
    };
    match store.load() {
        Ok(Some(state)) => Some(EyeProtectionStatus::from_state(state)),
        Ok(None) => {
            if let Some(state) = legacy {
                let _ = store.save(&state);
                return Some(EyeProtectionStatus::from_state(state));
            }
            None
        }
        Err(e) => {
            log::warn!(
                target: "hw::eye_protection",
                "eye protection store unreadable ({e}); falling back to the registry values"
            );
            legacy.map(EyeProtectionStatus::from_state)
        }
    }
}

#[cfg(any(windows, test))]
impl EyeProtectionStatus {
    fn from_state(state: crate::util::state_store::EyeProtectionState) -> Self {
        Self {
            enabled: state.enabled,
            intensity: state.intensity,
        }
    }
}

/// Legacy HKCU values written by pre-store builds (read-only import source).
#[cfg(windows)]
fn legacy_eye_protection_state() -> Option<crate::util::state_store::EyeProtectionState> {
    use crate::util::registry::RegKeyGuard;
    use windows::Win32::System::Registry::HKEY_CURRENT_USER;

    let key = RegKeyGuard::open_read(HKEY_CURRENT_USER, EYE_PROTECTION_REG_KEY)
        .ok()
        .flatten()?;
    let enabled = key
        .read_u32(EYE_PROTECTION_ENABLED_VALUE)
        .ok()
        .flatten()
        .map(|v| v != 0)
        .unwrap_or(false);
    let intensity = key
        .read_u32(EYE_PROTECTION_INTENSITY_VALUE)
        .ok()
        .flatten()
        .map(|v| v.clamp(0, 100) as u8)
        .unwrap_or(DEFAULT_INTENSITY);
    Some(crate::util::state_store::EyeProtectionState { enabled, intensity })
}

/// Enable or disable eye protection.
///
/// When enabled, applies a gamma ramp that reduces the blue channel
/// proportional to the intensity level. When disabled, restores the
/// default linear gamma ramp.
pub fn set_eye_protection(enabled: bool, intensity: Option<u8>) -> HardwareResult<()> {
    let intensity = intensity.unwrap_or(DEFAULT_INTENSITY).clamp(0, 100);

    if enabled {
        apply_gamma_ramp(intensity)?;
    } else {
        reset_gamma_ramp()?;
    }

    // Persist state
    persist_eye_protection(enabled, intensity)?;
    log::info!(
        target: "hw::eye_protection",
        "Eye protection {} (intensity={})",
        if enabled { "enabled" } else { "disabled" },
        intensity
    );

    Ok(())
}

/// Apply a warm gamma ramp to reduce blue light.
///
/// The ramp reduces the blue channel proportionally to the intensity.
/// At intensity=0, the ramp is linear (no change). At intensity=100,
/// the blue channel is reduced by ~50%.
fn apply_gamma_ramp(intensity: u8) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        // Calculate the blue channel reduction factor.
        // intensity=0 → factor=1.0 (no reduction)
        // intensity=100 → factor=0.5 (50% blue reduction)
        let blue_factor = 1.0 - (intensity as f64 / 100.0) * 0.5;
        // Red and green are slightly reduced too for a warmer tone
        let warm_factor = 1.0 - (intensity as f64 / 100.0) * 0.1;

        let mut ramp = GammaRamp::default();

        for i in 0..256 {
            let val = i as f64;
            // Linear ramp with channel-specific scaling
            let red = (val * warm_factor).clamp(0.0, 255.0) as u16;
            let green = (val * warm_factor).clamp(0.0, 255.0) as u16;
            let blue = (val * blue_factor).clamp(0.0, 255.0) as u16;

            // SetDeviceGammaRamp expects values in range 0-65535 (WORD)
            // but most monitors only use the lower 8 bits meaningfully.
            // Scale to 16-bit range.
            ramp.red[i] = red * 257;
            ramp.green[i] = green * 257;
            ramp.blue[i] = blue * 257;
        }

        apply_ramp_via_ffi(&mut ramp)
    }
    #[cfg(not(windows))]
    {
        let _ = intensity;
        Ok(())
    }
}

/// Reset the gamma ramp to default (linear).
fn reset_gamma_ramp() -> HardwareResult<()> {
    #[cfg(windows)]
    {
        let mut ramp = GammaRamp::default();

        // Linear ramp: output = input
        for i in 0..256 {
            let val = (i * 257) as u16;
            ramp.red[i] = val;
            ramp.green[i] = val;
            ramp.blue[i] = val;
        }

        apply_ramp_via_ffi(&mut ramp)
    }
    #[cfg(not(windows))]
    {
        Ok(())
    }
}

/// Shared helper: get DC, call `SetDeviceGammaRamp`, release DC.
#[cfg(windows)]
fn apply_ramp_via_ffi(ramp: &mut GammaRamp) -> HardwareResult<()> {
    let hdc = unsafe { GetDC(std::ptr::null_mut()) };
    if hdc.is_null() {
        return Err(HardwareError::Display(
            "Failed to get screen DC for gamma ramp".into(),
        ));
    }

    let ok = unsafe { SetDeviceGammaRamp(hdc, ramp) };

    let _ = unsafe { ReleaseDC(std::ptr::null_mut(), hdc) };

    if ok != 0 {
        Ok(())
    } else {
        // S42-001: This fails on Intel panels where the display driver (IGCL /
        // PSR2) owns the gamma pipeline and does not expose the classic
        // SetDeviceGammaRamp API. Surface a clear message so the UI can tell
        // the user why the toggle cannot take effect instead of a cryptic
        // Win32-style failure.
        Err(HardwareError::Display(
            "SetDeviceGammaRamp failed — this display driver (Intel IGCL) manages color itself; try Windows Night Light instead"
                .into(),
        ))
    }
}

/// Persist eye protection state through the state store. The legacy HKCU
/// values are imported read-only on the next get and left in place (nothing
/// reads them once the store exists; deleting registry values the app does
/// not own is riskier than leaving a dead DWORD).
fn persist_eye_protection(enabled: bool, intensity: u8) -> HardwareResult<()> {
    let store = crate::util::state_store::eye_protection_store()
        .map_err(|e| state_store_error("eye protection store", e))?;
    let state = crate::util::state_store::EyeProtectionState { enabled, intensity };
    store
        .save(&state)
        .map_err(|e| state_store_error("eye protection save", e))
}

/// Map a state-store failure into the catalog-classified IO error.
fn state_store_error(context: &str, e: crate::util::state_store::StoreError) -> HardwareError {
    HardwareError::Io(std::io::Error::other(format!("{context}: {e}")))
}

/// Registry-only read used as the degradation path when the store is
/// unreadable (and for the not-yet-imported legacy values).
#[cfg(windows)]
fn get_eye_protection_registry() -> HardwareResult<EyeProtectionStatus> {
    use crate::util::registry::RegKeyGuard;
    use windows::Win32::System::Registry::HKEY_CURRENT_USER;

    let key = RegKeyGuard::open_read(HKEY_CURRENT_USER, EYE_PROTECTION_REG_KEY)
        .ok()
        .flatten();

    let (enabled, intensity) = if let Some(k) = key {
        let enabled = k
            .read_u32(EYE_PROTECTION_ENABLED_VALUE)
            .ok()
            .flatten()
            .map(|v| v != 0)
            .unwrap_or(false);
        let intensity = k
            .read_u32(EYE_PROTECTION_INTENSITY_VALUE)
            .ok()
            .flatten()
            .map(|v| v.clamp(0, 100) as u8)
            .unwrap_or(DEFAULT_INTENSITY);
        (enabled, intensity)
    } else {
        (false, DEFAULT_INTENSITY)
    };

    Ok(EyeProtectionStatus { enabled, intensity })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Unique per-test scratch state root.
    fn scratch_root(tag: &str) -> std::path::PathBuf {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "micontrol-eye-protection-{}-{}-{}",
            tag,
            std::process::id(),
            seq
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch root");
        dir
    }

    #[test]
    fn eye_state_roundtrips_through_the_store() {
        let root = scratch_root("roundtrip");
        let store = crate::util::state_store::eye_protection_store_in(&root);
        store
            .save(&crate::util::state_store::EyeProtectionState {
                enabled: true,
                intensity: 65,
            })
            .expect("save");
        let holder = Ok(store);
        let status = load_eye_from(&holder, None).expect("stored state");
        assert!(status.enabled);
        assert_eq!(status.intensity, 65);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn legacy_registry_values_are_imported_once() {
        let root = scratch_root("import");
        let store = crate::util::state_store::eye_protection_store_in(&root);
        let legacy = crate::util::state_store::EyeProtectionState {
            enabled: true,
            intensity: 40,
        };
        let holder = Ok(store);
        let status = load_eye_from(&holder, Some(legacy)).expect("legacy import");
        assert!(status.enabled);
        assert_eq!(status.intensity, 40);
        let reloaded = crate::util::state_store::eye_protection_store_in(&root);
        let state = reloaded.load().expect("load").expect("imported state");
        assert_eq!(state.intensity, 40);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn corrupt_store_degrades_to_legacy_state() {
        let root = scratch_root("corrupt");
        let spec = crate::util::state_store::eye_protection_store_in(&root);
        std::fs::create_dir_all(spec.path().parent().expect("parent")).expect("dir");
        std::fs::write(spec.path(), "{corrupt").expect("corrupt fixture");
        let store = crate::util::state_store::eye_protection_store_in(&root);
        let legacy = crate::util::state_store::EyeProtectionState {
            enabled: false,
            intensity: 30,
        };
        let holder = Ok(store);
        let status = load_eye_from(&holder, Some(legacy)).expect("legacy fallback");
        assert!(!status.enabled);
        assert_eq!(status.intensity, 30);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_everywhere_maps_to_none() {
        let root = scratch_root("absent");
        let store = crate::util::state_store::eye_protection_store_in(&root);
        let holder = Ok(store);
        assert!(load_eye_from(&holder, None).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
