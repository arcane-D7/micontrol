//! ICC color profile management via Windows ICM API (mscms.dll).
//!
//! Provides:
//! - Listing installed ICC profiles
//! - Loading/unloading ICC profiles for the current display
//! - Querying the current display's color profile
//! - Launching Windows Color Calibration tool
//!
//! Uses raw FFI to `mscms.dll` since the `windows` crate 0.58 does not
//! expose `Win32_Graphics_ICM`.

use crate::hw::errors::{HardwareError, HardwareResult};
use serde::{Deserialize, Serialize};

// ── Data structures ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorProfileInfo {
    /// Display device name (e.g. "\\\\.\\DISPLAY1")
    pub device_name: String,
    /// Currently active ICC profile path (if any)
    pub current_profile: Option<String>,
    /// All installed ICC profiles for this display
    pub installed_profiles: Vec<String>,
    /// Whether hardware calibration is available
    pub hardware_calibration: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorCalibrationStatus {
    /// All displays and their color profiles
    pub displays: Vec<ColorProfileInfo>,
    /// Whether the eye protection (blue light filter) is currently active
    pub eye_protection_active: bool,
    /// Current gamma ramp intensity (0-100, 0 = off)
    pub gamma_intensity: u32,
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Get color profile information for all displays.
pub fn get_color_status() -> HardwareResult<ColorCalibrationStatus> {
    #[cfg(windows)]
    {
        let displays = enumerate_display_profiles();
        let eye_protection_active = check_eye_protection();
        let gamma_intensity = read_gamma_intensity();

        Ok(ColorCalibrationStatus {
            displays,
            eye_protection_active,
            gamma_intensity,
        })
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "Color calibration only available on Windows".into(),
    ))
}

/// Load an ICC profile for a specific display via mscms.dll FFI.
///
/// Uses `CreateDCW` to obtain a device context (HDC) for the display,
/// then calls `SetICMProfileW` with that HDC.
pub fn load_icc_profile(display: &str, profile_path: &str) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        // Validate the profile path
        let path = std::path::Path::new(profile_path);
        if !path.exists() {
            return Err(HardwareError::Other(format!(
                "ICC profile not found: {profile_path}"
            )));
        }
        if let Some(ext) = path.extension() {
            if ext != "icm" && ext != "icc" {
                return Err(HardwareError::Other(
                    "File is not an ICC profile (.icm/.icc)".to_string(),
                ));
            }
        } else {
            return Err(HardwareError::Other(
                "File has no extension — expected .icm/.icc".to_string(),
            ));
        }

        // S32-002: Canonicalize and contain the path to the system color
        // profiles directory. A crafted .icc path from a compromised webview
        // must not be able to load arbitrary files (or be used to probe the
        // filesystem) — restrict to the known profiles location.
        let canonical = std::fs::canonicalize(&path)
            .map_err(|e| HardwareError::Other(format!("Cannot resolve ICC path: {e}")))?;
        let profiles_dir = get_color_profiles_dir();
        if !canonical.starts_with(&profiles_dir) {
            return Err(HardwareError::Other(format!(
                "ICC profile must be inside the system color profiles directory ({}), got {}",
                profiles_dir.display(),
                canonical.display()
            )));
        }

        let device_w: Vec<u16> = display.encode_utf16().chain(std::iter::once(0)).collect();
        use std::os::windows::ffi::OsStrExt;
        let profile_w: Vec<u16> = canonical
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let result = unsafe {
            let mscms = windows::Win32::System::LibraryLoader::LoadLibraryA(windows::core::PCSTR(
                b"mscms.dll\0".as_ptr(),
            ))
            .map_err(|e| HardwareError::Other(format!("Failed to load mscms.dll: {e}")))?;

            let proc = windows::Win32::System::LibraryLoader::GetProcAddress(
                mscms,
                windows::core::PCSTR(b"SetICMProfileW\0".as_ptr()),
            )
            .ok_or_else(|| HardwareError::Other("SetICMProfileW not found".to_string()))?;

            // Create a DC for the display device
            let hdc = windows::Win32::Graphics::Gdi::CreateDCW(
                windows::core::PCWSTR(device_w.as_ptr()),
                windows::core::PCWSTR::null(),
                windows::core::PCWSTR::null(),
                None,
            );

            if hdc.is_invalid() {
                let _ = windows::Win32::Foundation::FreeLibrary(mscms);
                return Err(HardwareError::Other(format!(
                    "Failed to create DC for display: {display}"
                )));
            }

            // SetICMProfileW(HDC, LPCWSTR) -> BOOL
            let set_icm_profile: unsafe extern "system" fn(usize, *const u16) -> i32 =
                std::mem::transmute(proc);
            let r = set_icm_profile(hdc.0 as usize, profile_w.as_ptr());

            // Clean up
            let _ = windows::Win32::Graphics::Gdi::DeleteDC(hdc);
            let _ = windows::Win32::Foundation::FreeLibrary(mscms);
            r
        };

        if result != 0 {
            log::info!("Loaded ICC profile {} for {}", canonical.display(), display);
            Ok(())
        } else {
            Err(HardwareError::Other(format!(
                "Failed to load ICC profile (error code: {})",
                std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
            )))
        }
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "ICC profile loading only available on Windows".into(),
    ))
}

/// S32-002: Canonical path of the system color profiles directory
/// (`%SystemRoot%\System32\spool\drivers\color`). Used to contain
/// `load_icc_profile` paths.
#[cfg(windows)]
fn get_color_profiles_dir() -> std::path::PathBuf {
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
    std::path::PathBuf::from(&system_root)
        .join("System32")
        .join("spool")
        .join("drivers")
        .join("color")
}

/// Unload (remove) the ICC profile for a display, reverting to sRGB.
pub fn unload_icc_profile(display: &str) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        let srgb_path = get_srgb_profile_path()?;
        load_icc_profile(display, &srgb_path)
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "ICC profile unloading only available on Windows".into(),
    ))
}

/// Open the Windows Color Management settings page.
pub fn open_color_management_settings() -> HardwareResult<()> {
    #[cfg(windows)]
    {
        std::process::Command::new("cmd")
            .args(["/c", "start", "", "ms-settings:colormanagement"])
            .creation_flags(0x0800_0000)
            .spawn()
            .map_err(|e| {
                HardwareError::Other(format!("Failed to open color management settings: {e}"))
            })?;
        Ok(())
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "Color management settings only available on Windows".into(),
    ))
}

/// Launch Windows Display Color Calibration (dccw.exe).
pub fn launch_color_calibration_wizard() -> HardwareResult<()> {
    #[cfg(windows)]
    {
        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
        let dccw = format!("{}\\System32\\dccw.exe", system_root);
        std::process::Command::new(&dccw)
            .creation_flags(0x0800_0000)
            .spawn()
            .map_err(|e| {
                HardwareError::Other(format!("Failed to launch color calibration wizard: {e}"))
            })?;
        Ok(())
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "Color calibration wizard only available on Windows".into(),
    ))
}

/// Get the path to the system sRGB color profile.
fn get_srgb_profile_path() -> HardwareResult<String> {
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
    let path = format!(
        "{}\\System32\\spool\\drivers\\color\\sRGB Color Space Profile.icm",
        system_root
    );
    if std::path::Path::new(&path).exists() {
        Ok(path)
    } else {
        // Fallback: search common color profile directories
        let color_dirs = [
            format!("{}\\System32\\spool\\drivers\\color", system_root),
            format!("{}\\System32\\Color", system_root),
        ];

        for dir in &color_dirs {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.filter_map(|e| e.ok()) {
                    let path = entry.path();
                    if let Some(ext) = path.extension() {
                        if ext == "icm" || ext == "icc" {
                            let name = entry.file_name().to_string_lossy().to_lowercase();
                            if name.contains("srgb") || name.contains("standard") {
                                return Ok(path.to_string_lossy().to_string());
                            }
                        }
                    }
                }
            }
        }

        Err(HardwareError::Other(
            "sRGB color profile not found on this system".to_string(),
        ))
    }
}

// ── Internal helpers ─────────────────────────────────────────────────────────

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
fn enumerate_display_profiles() -> Vec<ColorProfileInfo> {
    let mut displays = Vec::new();

    for i in 0..=7 {
        let device_name = format!(r"\\.\DISPLAY{}", i + 1);
        let current_profile = get_display_icc_profile(&device_name);
        let installed_profiles = get_installed_profiles_for_display(&device_name);

        displays.push(ColorProfileInfo {
            device_name: device_name.clone(),
            current_profile,
            installed_profiles,
            hardware_calibration: true,
        });
    }

    if displays.is_empty() {
        displays.push(ColorProfileInfo {
            device_name: r"\\.\DISPLAY1".to_string(),
            current_profile: None,
            installed_profiles: vec![],
            hardware_calibration: false,
        });
    }

    displays
}

#[cfg(windows)]
fn get_display_icc_profile(device_name: &str) -> Option<String> {
    // Try the legacy GDI path first (works on some systems).
    if let Some(path) = get_icm_profile_gdi(device_name) {
        return Some(path);
    }
    // Fallback: WCS (Windows Color System) default profile for the display.
    // This is the API that Windows Settings actually uses for per-display
    // profile association on modern Windows 10/11.
    get_wcs_default_profile(device_name)
}

#[cfg(windows)]
fn get_icm_profile_gdi(device_name: &str) -> Option<String> {
    let device_w: Vec<u16> = device_name
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let mut profile_path = [0u16; 512];
    let mut size = profile_path.len() as u32;

    let result = unsafe {
        let mscms = windows::Win32::System::LibraryLoader::LoadLibraryA(windows::core::PCSTR(
            b"mscms.dll\0".as_ptr(),
        ))
        .ok()?;

        let proc = windows::Win32::System::LibraryLoader::GetProcAddress(
            mscms,
            windows::core::PCSTR(b"GetICMProfileW\0".as_ptr()),
        )?;

        let hdc = windows::Win32::Graphics::Gdi::CreateDCW(
            windows::core::PCWSTR(device_w.as_ptr()),
            windows::core::PCWSTR::null(),
            windows::core::PCWSTR::null(),
            None,
        );

        if hdc.is_invalid() {
            let _ = windows::Win32::Foundation::FreeLibrary(mscms);
            return None;
        }

        let get_icm_profile: unsafe extern "system" fn(usize, *mut u32, *mut u16) -> i32 =
            std::mem::transmute(proc);
        let r = get_icm_profile(hdc.0 as usize, &mut size, profile_path.as_mut_ptr());

        let _ = windows::Win32::Graphics::Gdi::DeleteDC(hdc);
        let _ = windows::Win32::Foundation::FreeLibrary(mscms);
        r
    };

    if result != 0 {
        let len = std::cmp::min(size as usize, profile_path.len());
        let path = String::from_utf16_lossy(&profile_path[..len]);
        let path = path.trim_end_matches('\0').to_string();
        if !path.is_empty() {
            return Some(path);
        }
    }

    None
}

#[cfg(windows)]
fn get_wcs_default_profile(device_name: &str) -> Option<String> {
    // WCS_PROFILE_MANAGEMENT_SCOPE_CURRENT_USER = 1
    const WCS_SCOPE_CURRENT_USER: u32 = 1;
    // COLORPROFILETYPE values from mscms.h
    const CPT_ICC: u32 = 1;
    // COLORPROFILESUBTYPE values from mscms.h; CPST_NONE asks for the device profile.
    const CPST_NONE: u32 = 0;

    // WCS uses the bare display name (e.g. "DISPLAY1") without the \\.\ prefix.
    let wcs_device = device_name
        .strip_prefix(r"\\.\")
        .unwrap_or(device_name)
        .to_ascii_uppercase();
    let device_w: Vec<u16> = wcs_device
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    unsafe {
        let mscms = windows::Win32::System::LibraryLoader::LoadLibraryA(windows::core::PCSTR(
            b"mscms.dll\0".as_ptr(),
        ))
        .ok()?;

        let size_proc = windows::Win32::System::LibraryLoader::GetProcAddress(
            mscms,
            windows::core::PCSTR(b"WcsGetDefaultColorProfileSize\0".as_ptr()),
        )?;
        let get_proc = windows::Win32::System::LibraryLoader::GetProcAddress(
            mscms,
            windows::core::PCSTR(b"WcsGetDefaultColorProfile\0".as_ptr()),
        )?;

        let wcs_get_size: unsafe extern "system" fn(
            u32,
            *const u16,
            u32,
            u32,
            u32,
            *mut u32,
        ) -> i32 = std::mem::transmute(size_proc);
        let wcs_get_profile: unsafe extern "system" fn(
            u32,
            *const u16,
            u32,
            u32,
            u32,
            u32,
            *mut u16,
        ) -> i32 = std::mem::transmute(get_proc);

        let mut size = 0u32;
        let r_size = wcs_get_size(
            WCS_SCOPE_CURRENT_USER,
            device_w.as_ptr(),
            CPT_ICC,
            CPST_NONE,
            0,
            &mut size,
        );
        if r_size == 0 || size == 0 || size > 4096 {
            let _ = windows::Win32::Foundation::FreeLibrary(mscms);
            return None;
        }

        let mut profile_name: Vec<u16> = vec![0; size as usize];
        let r = wcs_get_profile(
            WCS_SCOPE_CURRENT_USER,
            device_w.as_ptr(),
            CPT_ICC,
            CPST_NONE,
            0,
            size * std::mem::size_of::<u16>() as u32,
            profile_name.as_mut_ptr(),
        );
        let _ = windows::Win32::Foundation::FreeLibrary(mscms);

        if r != 0 {
            let name = String::from_utf16_lossy(&profile_name);
            let name = name.trim_end_matches('\0').to_string();
            if !name.is_empty() {
                return Some(name);
            }
        }
        None
    }
}

#[cfg(windows)]
fn get_installed_profiles_for_display(_device_name: &str) -> Vec<String> {
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
    let color_dir = format!("{}\\System32\\spool\\drivers\\color", system_root);

    let mut profiles = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&color_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if let Some(ext) = path.extension() {
                if ext == "icm" || ext == "icc" {
                    profiles.push(path.to_string_lossy().to_string());
                }
            }
        }
    }

    profiles
}

#[cfg(windows)]
fn check_eye_protection() -> bool {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    if let Ok(key) = hkcu.open_subkey(r"Software\MiControl\EyeProtection") {
        if let Ok(intensity) = key.get_value::<u32, _>("Intensity") {
            return intensity > 0;
        }
    }

    false
}

#[cfg(windows)]
fn read_gamma_intensity() -> u32 {
    use winreg::{enums::HKEY_CURRENT_USER, RegKey};
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    if let Ok(key) = hkcu.open_subkey(r"Software\MiControl\EyeProtection") {
        if let Ok(intensity) = key.get_value::<u32, _>("Intensity") {
            return intensity;
        }
    }

    0
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_color_status_does_not_panic() {
        let _ = get_color_status();
    }

    #[test]
    fn test_get_srgb_profile_path() {
        let _ = get_srgb_profile_path();
    }
}
