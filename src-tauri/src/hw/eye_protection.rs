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

/// Get the current eye protection state from registry.
pub fn get_eye_protection() -> HardwareResult<EyeProtectionStatus> {
    #[cfg(windows)]
    {
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
    #[cfg(not(windows))]
    {
        Ok(EyeProtectionStatus {
            enabled: false,
            intensity: DEFAULT_INTENSITY,
        })
    }
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

/// Persist eye protection state to registry.
fn persist_eye_protection(enabled: bool, intensity: u8) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_CURRENT_USER;

        let key = RegKeyGuard::create_write(HKEY_CURRENT_USER, EYE_PROTECTION_REG_KEY)
            .map_err(|e| HardwareError::Registry(format!("Create eye protection key: {e}")))?;

        key.write_u32(EYE_PROTECTION_ENABLED_VALUE, if enabled { 1 } else { 0 })
            .map_err(|e| HardwareError::Registry(format!("Write eye protection enabled: {e}")))?;

        key.write_u32(EYE_PROTECTION_INTENSITY_VALUE, intensity as u32)
            .map_err(|e| HardwareError::Registry(format!("Write eye protection intensity: {e}")))?;

        Ok(())
    }
    #[cfg(not(windows))]
    {
        Ok(())
    }
}
