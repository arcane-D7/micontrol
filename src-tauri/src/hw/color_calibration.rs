//! ICC color profile management via Windows ICM API (mscms.dll).
//!
//! Provides:
//! - Listing installed ICC profiles
//! - Loading/unloading ICC profiles for the current display
//! - Querying the current display's color profile
//! - Launching Windows Color Calibration tool
//!
//! Uses raw FFI to `mscms.dll` since the `windows` crate 0.58 does not
//! expose `Win32_Graphics_ICM`. Current Windows 11 builds no longer export the
//! legacy HDC pair (`SetICMProfileW` / `GetICMProfileW`), so association goes
//! through `AssociateColorProfileWithDeviceW` /
//! `DisassociateColorProfileFromDeviceW` and the active profile is read with
//! `WcsGetDefaultColorProfile` — see the `wcs` module.

use crate::hw::display::AttachedDisplayDevice;
#[cfg(windows)]
use crate::hw::display::{attached_display_devices, HdrDisplayState};
use crate::hw::errors::{HardwareError, HardwareResult};
#[cfg(windows)]
use crate::hw::icc_catalog;
use crate::hw::icc_catalog::CatalogEntryView;
#[cfg(windows)]
use crate::hw::panel_edid::{self, MonitorIdentity};
use serde::{Deserialize, Serialize};
#[cfg(windows)]
use std::collections::HashMap;

// ── Data structures ──────────────────────────────────────────────────────────

/// EDID identity of the panel behind a GDI display.
///
/// Present only when the CCD monitor path joined to a `WmiMonitorID` row on a
/// machine where that join is unambiguous; otherwise the field is absent and
/// the UI shows “—”. Guessing here would make the catalogue match profiles to
/// the wrong panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PanelInfo {
    /// `EDO 0x4503`
    pub panel_id: String,
    /// Three-letter EDID vendor (`EDO`, `SDC`, `GSM`).
    pub manufacturer: String,
    /// EDID product code as hex text (`4503`).
    pub product_code: String,
    /// Windows monitor name when EDID carries one, else the panel id.
    pub name: String,
    /// `EDO4503` — what Panelook / NotebookCheck index.
    pub model_token: String,
    pub year: Option<u16>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorProfileInfo {
    /// Display device name (e.g. "\\\\.\\DISPLAY1")
    pub device_name: String,
    /// Monitor / adapter description, so the card is not just "Displays N".
    #[serde(default)]
    pub label: String,
    /// Currently active ICC profile path (if any)
    pub current_profile: Option<String>,
    /// All installed ICC profiles for this display
    pub installed_profiles: Vec<String>,
    /// The display exposes an advanced-color (HDR / wide colour gamut) pipeline.
    ///
    /// This replaces the old hardcoded `hardware_calibration: true`: Windows has
    /// no API for "monitor has an internal LUT", and while HDR is on the
    /// compositor bypasses ICC/gamma, so claiming hardware calibration for every
    /// screen was simply false. What is knowable per display is the advanced
    /// color capability, which is what decides whether a calibration is visible.
    #[serde(default)]
    pub advanced_color_supported: bool,
    /// HDR is currently on for this display (calibration gammas are ignored).
    #[serde(default)]
    pub hdr_enabled: bool,
    /// EDID panel identity, when the display could be joined to one monitor.
    #[serde(default)]
    pub panel: Option<PanelInfo>,
    /// Curated profiles applicable to this panel (panel-specific first).
    #[serde(default)]
    pub catalog: Vec<CatalogEntryView>,
}

/// Why the calibration wizard cannot be used right now.
///
/// Serialised as a stable code (`no-display`, `hdr-enabled`, `already-open`) so
/// the webview can translate it instead of echoing an English backend string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WizardBlock {
    /// Nothing is attached to the desktop — there is nothing to calibrate.
    NoDisplay,
    /// HDR is on: Windows ignores the calibration gamma ramp.
    HdrEnabled,
    /// A wizard instance is already running (our LUT is released meanwhile).
    AlreadyOpen,
}

impl WizardBlock {
    /// English sentence for error paths and logs (the UI maps the code itself).
    pub fn message(self) -> &'static str {
        match self {
            Self::NoDisplay => "no display is attached to the desktop — nothing to calibrate",
            Self::HdrEnabled => {
                "HDR (advanced color) is on — Windows ignores the calibration gamma ramp while \
                 HDR is enabled; turn HDR off first"
            }
            Self::AlreadyOpen => "the color calibration wizard is already open",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColorCalibrationStatus {
    /// All displays and their color profiles
    pub displays: Vec<ColorProfileInfo>,
    /// Whether the eye protection (blue light filter) is currently active
    pub eye_protection_active: bool,
    /// Current gamma ramp intensity (0-100, 0 = off)
    pub gamma_intensity: u32,
    /// Display `dccw.exe` will calibrate (the GDI primary), when there is one.
    #[serde(default)]
    pub wizard_target: Option<String>,
    /// Set when the wizard must stay disabled, with a translatable reason code.
    #[serde(default)]
    pub wizard_block: Option<WizardBlock>,
    /// Date the bundled ICC catalogue was last verified against its sources.
    #[serde(default)]
    pub catalog_updated: String,
}

// ── Public API ───────────────────────────────────────────────────────────────

/// Get color profile information for all displays.
pub fn get_color_status() -> HardwareResult<ColorCalibrationStatus> {
    #[cfg(windows)]
    {
        // One read-only enumeration of *real* displays (the colour tab used to
        // list DISPLAY1..DISPLAY8 unconditionally) plus one CCD HDR probe, since
        // Windows ignores the calibration gamma ramp while HDR is on.
        let devices = attached_display_devices();
        let hdr = crate::hw::display::hdr_displays();
        // One CCD pass for the monitor device paths (EDID join) — the GDI
        // display name alone cannot identify a panel.
        let monitor_paths = crate::hw::display::monitor_device_paths();
        let monitors = panel_edid::read_monitors();
        let displays = profiles_for_devices(&devices, &hdr, &monitors, &monitor_paths);
        let eye_protection_active = check_eye_protection();
        let gamma_intensity = read_gamma_intensity();
        let hdr_on = hdr.iter().any(|d| d.enabled);

        Ok(ColorCalibrationStatus {
            displays,
            eye_protection_active,
            gamma_intensity,
            wizard_target: wizard_target(&devices).map(|d| d.device.clone()),
            wizard_block: wizard_block(
                &devices,
                hdr_on,
                crate::hw::display::is_calibration_in_progress(),
            ),
            catalog_updated: icc_catalog::catalog()
                .map(|c| c.updated.clone())
                .unwrap_or_default(),
        })
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "Color calibration only available on Windows".into(),
    ))
}

/// Load an ICC profile for a specific display.
///
/// Associates the profile with the display device — the supported equivalent of
/// the old `SetICMProfile(HDC, ...)`, whose export this Windows build dropped.
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
        let canonical = std::fs::canonicalize(path)
            .map_err(|e| HardwareError::Other(format!("Cannot resolve ICC path: {e}")))?;
        let profiles_dir = get_color_profiles_dir();
        // `canonicalize` yields the extended-length form (`\\?\C:\...`), the
        // profiles dir does not — comparing one against the other always failed
        // even for a legitimate profile in the right folder. Canonicalize BOTH
        // sides for the containment check (symlink-safe), then hand mscms the
        // simplified form (mscms does not accept `\\?\` paths).
        let profiles_dir_canonical =
            std::fs::canonicalize(&profiles_dir).unwrap_or_else(|_| profiles_dir.clone());
        if !canonical.starts_with(&profiles_dir_canonical) {
            return Err(HardwareError::Other(format!(
                "ICC profile must be inside the system color profiles directory ({}), got {}",
                profiles_dir.display(),
                canonical.display()
            )));
        }
        let canonical = simplify_extended_path(&canonical);

        wcs::associate_profile(display, &canonical)
            .map_err(|e| HardwareError::Other(format!("Failed to load ICC profile: {e}")))?;
        log::info!("Loaded ICC profile {} for {}", canonical.display(), display);
        Ok(())
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
    icc_catalog::system_profiles_dir()
}

/// Unload (remove) the ICC profile for a display, reverting to sRGB.
///
/// The device's profile list is per user (`ICM\ProfileAssociations\Display`), and
/// associating a profile only moves it to the front of that list. Dropping the
/// association MiControl made keeps the list from growing a stale entry behind
/// the sRGB profile on every reset.
pub fn unload_icc_profile(display: &str) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        let srgb_path = get_srgb_profile_path()?;
        if let Some(current) = get_icm_profile_gdi(display) {
            if !current.eq_ignore_ascii_case(&srgb_path) {
                match wcs::disassociate_profile(display, std::path::Path::new(&current)) {
                    Ok(()) => log::info!("Removed ICC profile {current} from {display}"),
                    // Not fatal: associating sRGB below still takes over as the
                    // default; the stale entry is only untidy.
                    Err(e) => log::warn!("Could not disassociate {current} from {display}: {e}"),
                }
            }
        }
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
///
/// S32-003: While the wizard is open we set `CALIBRATION_IN_PROGRESS` so the
/// adaptive-brightness loop stops touching the backlight, and we temporarily
/// reset our eye-protection gamma ramp. Both would otherwise lock the display
/// LUT and make the wizard fail to save with "Access is denied / Close other
/// programs". When the wizard process exits, the flag is cleared and eye
/// protection is restored.
pub fn launch_color_calibration_wizard() -> HardwareResult<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        // Refuse instead of opening a wizard whose calibration Windows would
        // throw away (HDR on) or that has nothing to calibrate. The UI disables
        // the button with the same reason; this guard covers races and any
        // other caller.
        let devices = attached_display_devices();
        let hdr_on = crate::hw::display::hdr_displays().iter().any(|d| d.enabled);
        if let Some(block) = wizard_block(
            &devices,
            hdr_on,
            crate::hw::display::is_calibration_in_progress(),
        ) {
            log::info!("[color] calibration wizard refused: {block:?}");
            return Err(HardwareError::NotSupported(block.message().to_string()));
        }

        let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".to_string());
        let dccw = format!("{}\\System32\\dccw.exe", system_root);

        // Remember whether eye protection was on so we can restore it — with
        // its intensity: the old code dropped it (`let _ = eye_intensity`) and
        // the user's setting fell back to the default 50 after every wizard.
        let eye_was_on = crate::hw::eye_protection::get_eye_protection()
            .map(|s| s.enabled)
            .unwrap_or(false);
        let eye_intensity = crate::hw::eye_protection::get_eye_protection()
            .ok()
            .map(|s| s.intensity);

        // Pause our display control: stop the adaptive loop + release the LUT.
        crate::hw::display::CALIBRATION_IN_PROGRESS
            .store(true, std::sync::atomic::Ordering::SeqCst);
        if eye_was_on {
            // Temporarily disable eye protection so the LUT is free for dccw.
            let _ = crate::hw::eye_protection::set_eye_protection(false, None);
        }

        let spawned = std::process::Command::new(&dccw)
            .creation_flags(0x0800_0000)
            .spawn();

        let target = wizard_target(&devices)
            .map(|d| format!("{} ({})", d.device, d.label))
            .unwrap_or_else(|| "unknown".to_string());
        log::info!(
            "[color] calibration wizard launched — dccw calibrates the primary display {target}"
        );

        // Spawn a watcher thread that waits for the wizard to exit, then
        // restores our display state. Runs detached from the caller.
        std::thread::spawn(move || {
            let mut child = match spawned {
                Ok(c) => c,
                Err(e) => {
                    log::warn!("[color] failed to launch the calibration wizard: {e}");
                    restore_after_calibration(eye_was_on, eye_intensity);
                    return;
                }
            };
            // Block this thread until the wizard process exits.
            let _ = child.wait();
            restore_after_calibration(eye_was_on, eye_intensity);
        });

        Ok(())
    }
    #[cfg(not(windows))]
    Err(HardwareError::NotSupported(
        "Color calibration wizard only available on Windows".into(),
    ))
}

/// Why the wizard must stay disabled, if it must.
///
/// Pure — the three inputs are the only things that decide it, so the gating is
/// testable without a display. A disabled button with a reason replaces the old
/// behaviour where the button was always enabled and the action silently did
/// nothing useful (audit 2.9: 0 executions in 73 h).
fn wizard_block(
    devices: &[AttachedDisplayDevice],
    hdr_enabled: bool,
    already_open: bool,
) -> Option<WizardBlock> {
    if devices.is_empty() {
        return Some(WizardBlock::NoDisplay);
    }
    if already_open {
        return Some(WizardBlock::AlreadyOpen);
    }
    if hdr_enabled {
        return Some(WizardBlock::HdrEnabled);
    }
    None
}

/// Display `dccw.exe` calibrates: the GDI primary, else the first attached one.
fn wizard_target(devices: &[AttachedDisplayDevice]) -> Option<&AttachedDisplayDevice> {
    devices
        .iter()
        .find(|d| d.is_primary)
        .or_else(|| devices.first())
}

/// Clear the calibration-in-progress flag and restore eye protection with the
/// intensity it had before the wizard ran.
#[cfg(windows)]
fn restore_after_calibration(eye_was_on: bool, intensity: Option<u8>) {
    crate::hw::display::CALIBRATION_IN_PROGRESS.store(false, std::sync::atomic::Ordering::SeqCst);
    if eye_was_on {
        let _ = crate::hw::eye_protection::set_eye_protection(true, intensity);
    }
    log::info!("Color calibration wizard closed — display control restored");
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

// ── mscms.dll FFI ────────────────────────────────────────────────────────────

/// Colour-management entry points this Windows build actually exports.
///
/// `SetICMProfileW`, `SetICMProfileA`, `GetICMProfileW` and `GetICMProfile` were
/// removed from `mscms.dll` (GetProcAddress returns NULL for all four on build
/// 26200), which is why the old HDC-based FFI failed with "SetICMProfileW not
/// found" and the UI always showed "No ICC profile loaded". The calls below are
/// exported and are the supported replacement — signatures and constant values
/// come from the Windows SDK 10.0.26100 `um/Icm.h`:
///
/// ```c
/// BOOL WINAPI AssociateColorProfileWithDeviceW(
///     _In_opt_ PCWSTR pMachineName, _In_ PCWSTR pProfileName, _In_ PCWSTR pDeviceName);
/// BOOL WINAPI DisassociateColorProfileFromDeviceW(
///     _In_opt_ PCWSTR pMachineName, _In_ PCWSTR pProfileName, _In_ PCWSTR pDeviceName);
/// BOOL WINAPI WcsGetDefaultColorProfileSize(
///     _In_ WCS_PROFILE_MANAGEMENT_SCOPE scope, _In_opt_ PCWSTR pDeviceName,
///     _In_ COLORPROFILETYPE cptColorProfileType,
///     _In_ COLORPROFILESUBTYPE cpstColorProfileSubType,
///     _In_ DWORD dwProfileID, _Out_ PDWORD pcbProfileName);
/// BOOL WINAPI WcsGetDefaultColorProfile(
///     _In_ WCS_PROFILE_MANAGEMENT_SCOPE scope, _In_opt_ PCWSTR pDeviceName,
///     _In_ COLORPROFILETYPE cptColorProfileType,
///     _In_ COLORPROFILESUBTYPE cpstColorProfileSubType,
///     _In_ DWORD dwProfileID, _In_ DWORD cbProfileName,
///     _Out_writes_bytes_(cbProfileName) LPWSTR pProfileName);
/// ```
#[cfg(windows)]
mod wcs {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows::core::s;
    use windows::Win32::Foundation::{FreeLibrary, HMODULE};
    use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryA};

    /// `WCS_PROFILE_MANAGEMENT_SCOPE` (Icm.h): `SYSTEM_WIDE` = 0, `CURRENT_USER` = 1.
    const SCOPE_SYSTEM_WIDE: u32 = 0;
    const SCOPE_CURRENT_USER: u32 = 1;

    /// `COLORPROFILETYPE.CPT_ICC` — first enumerator of the enum, i.e. 0
    /// (`CPT_DMP` is 1, `CPT_CAMP` 2, `CPT_GMMP` 3).
    const CPT_ICC: u32 = 0;

    /// `COLORPROFILESUBTYPE.CPST_NONE` — the four `CPST_*` intent values take
    /// 0..=3, so "the device's own profile, no particular intent" is 4.
    const CPST_NONE: u32 = 4;

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn wide_os(text: &OsStr) -> Vec<u16> {
        text.encode_wide().chain(std::iter::once(0)).collect()
    }

    /// `mscms.dll` with its exports resolved lazily; freed on drop.
    struct Mscms(HMODULE);

    impl Mscms {
        fn load() -> Result<Self, String> {
            unsafe {
                LoadLibraryA(s!("mscms.dll"))
                    .map(Self)
                    .map_err(|e| format!("cannot load mscms.dll: {e}"))
            }
        }

        fn export(
            &self,
            name: &str,
            symbol: windows::core::PCSTR,
        ) -> Result<unsafe extern "system" fn() -> isize, String> {
            unsafe {
                // `windows-rs` returns the bare function pointer; NULL is what
                // "this build does not export it" looks like.
                GetProcAddress(self.0, symbol)
                    .ok_or_else(|| format!("mscms.dll does not export {name}"))
            }
        }
    }

    impl Drop for Mscms {
        fn drop(&mut self) {
            unsafe {
                let _ = FreeLibrary(self.0);
            }
        }
    }

    /// Candidate device names, most specific first.
    ///
    /// `mscms` resolves a display by its GDI name **without** the Win32 `\\.\`
    /// prefix (verified on build 26200: `DISPLAY1` succeeds while
    /// `\\.\DISPLAY1` returns FALSE). The prefixed form is kept as a fallback so
    /// the call keeps working if a future build expects it.
    fn device_names(device: &str) -> Vec<&str> {
        let bare = device.strip_prefix(r"\\.\").unwrap_or(device);
        if bare == device {
            vec![device]
        } else {
            vec![bare, device]
        }
    }

    /// `AssociateColorProfileWithDeviceW(NULL, profile, device)` — makes
    /// `profile` the default ICC profile of a display (the modern equivalent of
    /// `SetICMProfile`). Writes the current user's association list, so it needs
    /// no elevation for a display the user owns.
    pub fn associate_profile(device: &str, profile: &Path) -> Result<(), String> {
        let profile = wide_os(profile.as_os_str());
        let mut last_error = String::new();
        for name in device_names(device) {
            let device_w = wide(name);
            let result = (|| -> Result<(), String> {
                let mscms = Mscms::load()?;
                let proc = mscms.export(
                    "AssociateColorProfileWithDeviceW",
                    s!("AssociateColorProfileWithDeviceW"),
                )?;
                // BOOL WINAPI AssociateColorProfileWithDeviceW(PCWSTR, PCWSTR, PCWSTR)
                let associate: unsafe extern "system" fn(
                    *const u16,
                    *const u16,
                    *const u16,
                ) -> i32 = unsafe { std::mem::transmute(proc) };
                let ok =
                    unsafe { associate(std::ptr::null(), profile.as_ptr(), device_w.as_ptr()) };
                if ok != 0 {
                    Ok(())
                } else {
                    Err(format!(
                        "AssociateColorProfileWithDeviceW failed for {name} (error {})",
                        std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
                    ))
                }
            })();
            match result {
                Ok(()) => return Ok(()),
                Err(e) => last_error = e,
            }
        }
        Err(last_error)
    }

    /// `DisassociateColorProfileFromDeviceW(NULL, profile, device)` — removes one
    /// profile from the display's association list.
    pub fn disassociate_profile(device: &str, profile: &Path) -> Result<(), String> {
        let profile = wide_os(profile.as_os_str());
        let mut last_error = String::new();
        for name in device_names(device) {
            let device_w = wide(name);
            let result = (|| -> Result<(), String> {
                let mscms = Mscms::load()?;
                let proc = mscms.export(
                    "DisassociateColorProfileFromDeviceW",
                    s!("DisassociateColorProfileFromDeviceW"),
                )?;
                // BOOL WINAPI DisassociateColorProfileFromDeviceW(PCWSTR, PCWSTR, PCWSTR)
                let disassociate: unsafe extern "system" fn(
                    *const u16,
                    *const u16,
                    *const u16,
                ) -> i32 = unsafe { std::mem::transmute(proc) };
                let ok =
                    unsafe { disassociate(std::ptr::null(), profile.as_ptr(), device_w.as_ptr()) };
                if ok != 0 {
                    Ok(())
                } else {
                    Err(format!(
                        "DisassociateColorProfileFromDeviceW failed for {name} (error {})",
                        std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
                    ))
                }
            })();
            match result {
                Ok(()) => return Ok(()),
                Err(e) => last_error = e,
            }
        }
        Err(last_error)
    }

    /// Default ICC profile of `device`, as reported by
    /// `WcsGetDefaultColorProfile` — the profile name only, no directory.
    ///
    /// The user's association wins over the machine-wide one, which is where
    /// `AssociateColorProfileWithDeviceW` writes.
    pub fn default_profile_name(device: &str) -> Option<String> {
        for name in device_names(device) {
            for scope in [SCOPE_CURRENT_USER, SCOPE_SYSTEM_WIDE] {
                if let Some(found) = default_profile_name_in_scope(scope, name) {
                    return Some(found);
                }
            }
        }
        None
    }

    fn default_profile_name_in_scope(scope: u32, device: &str) -> Option<String> {
        let device_w = wide(device);
        // SIZE in bytes of the profile name, terminator included.
        let mut bytes = 0u32;
        let mut buffer: Vec<u16>;

        unsafe {
            let mscms = Mscms::load().ok()?;
            let get_proc = mscms
                .export("WcsGetDefaultColorProfile", s!("WcsGetDefaultColorProfile"))
                .ok()?;
            // BOOL WcsGetDefaultColorProfile(scope, device, cpt, cpst, id, cb, name)
            let get_profile: unsafe extern "system" fn(
                u32,
                *const u16,
                u32,
                u32,
                u32,
                u32,
                *mut u16,
            ) -> i32 = std::mem::transmute(get_proc);

            match mscms.export(
                "WcsGetDefaultColorProfileSize",
                s!("WcsGetDefaultColorProfileSize"),
            ) {
                Ok(size_proc) => {
                    // BOOL WcsGetDefaultColorProfileSize(scope, device, cpt, cpst, id, &bytes)
                    let get_size: unsafe extern "system" fn(
                        u32,
                        *const u16,
                        u32,
                        u32,
                        u32,
                        *mut u32,
                    ) -> i32 = std::mem::transmute(size_proc);
                    let sized =
                        get_size(scope, device_w.as_ptr(), CPT_ICC, CPST_NONE, 0, &mut bytes);
                    // FALSE + ERROR_FILE_NOT_FOUND means "this device has no
                    // default profile", i.e. the ordinary uncalibrated state.
                    if sized == 0 || bytes == 0 {
                        return None;
                    }
                    buffer = vec![0; (bytes as usize) / 2 + 1];
                }
                // Present on every Vista+ build; a fixed buffer keeps the reader
                // working anyway if it ever disappears.
                Err(_) => {
                    buffer = vec![0; 512];
                }
            }

            let cb = (buffer.len() * std::mem::size_of::<u16>()) as u32;
            let ok = get_profile(
                scope,
                device_w.as_ptr(),
                CPT_ICC,
                CPST_NONE,
                0,
                cb,
                buffer.as_mut_ptr(),
            );
            if ok == 0 {
                return None;
            }

            let name = String::from_utf16_lossy(&buffer);
            let name = name.trim_end_matches('\0').trim().to_string();
            (!name.is_empty()).then_some(name)
        }
    }
}

// ── Internal helpers ─────────────────────────────────────────────────────────

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
fn profiles_for_devices(
    devices: &[AttachedDisplayDevice],
    hdr: &[HdrDisplayState],
    monitors: &[MonitorIdentity],
    monitor_paths: &HashMap<String, String>,
) -> Vec<ColorProfileInfo> {
    if devices.is_empty() {
        log::warn!("[color] no display attached to the desktop — colour profile list is empty");
        return Vec::new();
    }

    let profiles_dir = icc_catalog::system_profiles_dir();
    devices
        .iter()
        .map(|dev| {
            let hdr_state = hdr.iter().find(|h| h.device == dev.device);
            // EDID panel: CCD monitor path → WmiMonitorID instance. `None` on
            // any ambiguity, and the UI then shows "—" instead of a guess.
            let monitor = panel_edid::monitor_for_display(
                monitor_paths.get(&dev.device).map(String::as_str),
                monitors,
                devices.len(),
            );
            let panel = monitor.map(|m| PanelInfo {
                panel_id: m.panel_id(),
                manufacturer: m.manufacturer.clone(),
                product_code: m.product_code.clone(),
                name: m.display_name(),
                model_token: m.model_token(),
                year: m.year,
            });
            ColorProfileInfo {
                device_name: dev.device.clone(),
                // Only the CCD monitor name is a *monitor* name; the GDI
                // DeviceString is the adapter ("Intel(R) Arc(TM) B390 GPU") and
                // would label every card identically, so it is dropped here and
                // the UI falls back to "Displays N".
                label: hdr_state
                    .map(|h| h.label.clone())
                    .filter(|label| !label.is_empty())
                    .unwrap_or_default(),
                current_profile: get_icm_profile_gdi(&dev.device),
                installed_profiles: get_installed_profiles_for_display(&dev.device),
                advanced_color_supported: hdr_state.is_some_and(|h| h.supported),
                hdr_enabled: hdr_state.is_some_and(|h| h.enabled),
                panel,
                catalog: icc_catalog::entries_for(monitor, &profiles_dir),
            }
        })
        .collect()
}

#[cfg(windows)]
fn get_icm_profile_gdi(device_name: &str) -> Option<String> {
    // `GetICMProfileW` — the HDC twin of `SetICMProfileW` — is gone from
    // mscms.dll on this Windows build too, so the active profile comes from
    // WCS. That is the same value Windows Settings shows: the first entry of
    // the device's `ICM\ProfileAssociations\Display` list.
    let profile = wcs::default_profile_name(device_name)?;
    Some(resolve_profile_path(&profile))
}

/// WCS reports a bare profile file name (`xiaomi_D65P3_base.icc`), while the
/// rest of the app talks in full paths (`GetICMProfileW` used to return one).
/// Resolve through the system colour directory so `current_profile` keeps
/// pointing at a real file the UI can name.
#[cfg(windows)]
fn resolve_profile_path(name: &str) -> String {
    let path = std::path::Path::new(name);
    if path.is_absolute() {
        return name.to_string();
    }
    get_color_profiles_dir()
        .join(path)
        .to_string_lossy()
        .into_owned()
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

/// Strip the Windows extended-length prefix (`\\?\`) so the path can be
/// handed to Win32 APIs that reject it (mscms among them).
/// `\\?\UNC\server\share` collapses to `\\server\share`.
fn simplify_extended_path(path: &std::path::Path) -> std::path::PathBuf {
    let s = path.as_os_str().to_string_lossy();
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        return std::path::PathBuf::from(format!(r"\\{rest}"));
    }
    if let Some(rest) = s.strip_prefix(r"\\?\") {
        return std::path::PathBuf::from(rest);
    }
    path.to_path_buf()
}

#[cfg(test)]
mod simplify_tests {
    use super::*;

    #[test]
    fn strips_the_extended_prefix_from_drive_paths() {
        assert_eq!(
            simplify_extended_path(std::path::Path::new(
                r"\\?\C:\WINDOWS\System32\spool\drivers\color\a.icc"
            )),
            std::path::PathBuf::from(r"C:\WINDOWS\System32\spool\drivers\color\a.icc")
        );
    }

    #[test]
    fn collapses_unc_extended_paths() {
        assert_eq!(
            simplify_extended_path(std::path::Path::new(r"\\?\UNC\server\share\p.icc")),
            std::path::PathBuf::from(r"\\server\share\p.icc")
        );
    }

    #[test]
    fn leaves_plain_paths_untouched() {
        let p = std::path::Path::new(r"C:\tmp\p.icc");
        assert_eq!(simplify_extended_path(p), p.to_path_buf());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(name: &str, primary: bool, internal: bool) -> AttachedDisplayDevice {
        AttachedDisplayDevice {
            device: name.to_string(),
            label: format!("{name} label"),
            is_internal: internal,
            is_primary: primary,
        }
    }

    #[test]
    fn test_get_color_status_does_not_panic() {
        let _ = get_color_status();
    }

    // ── calibration wizard gating (audit 2.9: the button always looked usable
    //    and was never used; with HDR on the calibration is discarded by the
    //    compositor) ─────────────────────────────────────────────────────────

    #[test]
    fn wizard_is_blocked_without_an_attached_display() {
        assert_eq!(
            wizard_block(&[], false, false),
            Some(WizardBlock::NoDisplay)
        );
        assert_eq!(
            wizard_block(&[], true, false),
            Some(WizardBlock::NoDisplay),
            "no display outranks the HDR reason: there is nothing to calibrate"
        );
    }

    #[test]
    fn wizard_is_blocked_while_hdr_is_on() {
        let devices = [device("DISPLAY1", true, true)];
        assert_eq!(
            wizard_block(&devices, true, false),
            Some(WizardBlock::HdrEnabled)
        );
        // ... and the reason survives into the error the command returns.
        let msg = wizard_block(&devices, true, false).unwrap().message();
        assert!(msg.contains("HDR"), "unexpected message: {msg}");
    }

    #[test]
    fn wizard_is_blocked_while_an_instance_is_running() {
        let devices = [device("DISPLAY1", true, true)];
        assert_eq!(
            wizard_block(&devices, false, true),
            Some(WizardBlock::AlreadyOpen)
        );
    }

    #[test]
    fn wizard_is_allowed_on_a_plain_sdr_display() {
        let devices = [
            device("DISPLAY1", false, true),
            device("DISPLAY3", true, false),
        ];
        assert_eq!(wizard_block(&devices, false, false), None);
        // dccw calibrates the monitor its window is on, i.e. the GDI primary.
        assert_eq!(
            wizard_target(&devices).map(|d| d.device.as_str()),
            Some("DISPLAY3")
        );
    }

    #[test]
    fn wizard_target_falls_back_to_the_first_display() {
        let devices = [
            device("DISPLAY1", false, true),
            device("DISPLAY2", false, false),
        ];
        assert_eq!(
            wizard_target(&devices).map(|d| d.device.as_str()),
            Some("DISPLAY1")
        );
        assert!(wizard_target(&[]).is_none());
    }

    #[test]
    fn wizard_block_serialises_as_a_stable_code() {
        // The webview switches on these exact strings.
        let codes: Vec<String> = [
            WizardBlock::NoDisplay,
            WizardBlock::HdrEnabled,
            WizardBlock::AlreadyOpen,
        ]
        .iter()
        .map(|b| serde_json::to_string(b).unwrap())
        .collect();
        assert_eq!(
            codes,
            vec![
                "\"no-display\"".to_string(),
                "\"hdr-enabled\"".to_string(),
                "\"already-open\"".to_string(),
            ]
        );
    }

    // ── per-device enumeration (audit 2.9: the page listed eight hardcoded
    //    displays when only three existed) ────────────────────────────────────

    #[cfg(windows)]
    #[test]
    fn profiles_are_one_per_attached_device_not_eight() {
        // Registries / DCs for these pseudo devices do not exist, so the ICC
        // lookups return None — exactly the "no profile loaded" case the UI
        // must show for a real display without a profile.
        let devices = [
            device("MICONTROL_TEST_DISPLAYA", true, true),
            device("MICONTROL_TEST_DISPLAYB", false, false),
        ];
        let hdr = [
            HdrDisplayState {
                device: "MICONTROL_TEST_DISPLAYA".to_string(),
                label: "A".to_string(),
                supported: true,
                enabled: false,
            },
            HdrDisplayState {
                device: "MICONTROL_TEST_DISPLAYB".to_string(),
                label: "B".to_string(),
                supported: false,
                enabled: false,
            },
        ];
        let profiles = profiles_for_devices(&devices, &hdr, &[], &HashMap::new());
        assert_eq!(
            profiles.len(),
            devices.len(),
            "one card per attached display, never a fixed 0..=7 list"
        );
        assert_eq!(profiles[0].device_name, "MICONTROL_TEST_DISPLAYA");
        // The monitor name comes from CCD, never from the GDI adapter string.
        assert_eq!(profiles[0].label, "A");
        assert!(profiles[0].advanced_color_supported);
        assert!(!profiles[1].advanced_color_supported);
        assert!(profiles.iter().all(|p| !p.hdr_enabled));
    }

    #[cfg(windows)]
    #[test]
    fn profile_label_is_empty_without_a_ccd_monitor_name() {
        // The adapter string must not leak into the card title: three displays
        // on one GPU would all read "Intel(R) Arc(TM) B390 GPU".
        let devices = [device("MICONTROL_TEST_DISPLAYC", true, false)];
        let profiles = profiles_for_devices(&devices, &[], &[], &HashMap::new());
        assert_eq!(profiles.len(), 1);
        assert!(
            profiles[0].label.is_empty(),
            "unexpected label: {:?}",
            profiles[0].label
        );
    }

    #[cfg(windows)]
    #[test]
    fn no_attached_display_yields_no_phantom_cards() {
        assert!(profiles_for_devices(&[], &[], &[], &HashMap::new()).is_empty());
    }

    /// Read-only smoke test against the live machine: the colour tab must list
    /// exactly the displays Windows reports (three here), never DISPLAY1..8.
    #[cfg(windows)]
    #[test]
    fn live_color_status_lists_only_attached_displays() {
        let attached = attached_display_devices();
        let status = get_color_status().expect("color status");
        println!(
            "attached={} listed={} wizard_target={:?} wizard_block={:?}",
            attached.len(),
            status.displays.len(),
            status.wizard_target,
            status.wizard_block
        );
        for d in &status.displays {
            println!(
                "  {} '{}' profile={:?} advanced_color={} hdr={} panel={:?} catalog={}",
                d.device_name,
                d.label,
                d.current_profile,
                d.advanced_color_supported,
                d.hdr_enabled,
                d.panel.as_ref().map(|p| p.panel_id.clone()),
                d.catalog.len()
            );
            for entry in &d.catalog {
                println!(
                    "    catalog: {} [{}] kind={:?} compatible={} local={}",
                    entry.display_name,
                    entry.file_name,
                    entry.kind,
                    entry.compatible,
                    entry.available_locally
                );
            }
        }
        assert_eq!(
            status.displays.len(),
            attached.len(),
            "one card per attached display (was a fixed 8)"
        );
        assert!(status
            .displays
            .iter()
            .all(|d| d.device_name.contains("DISPLAY")));
    }

    #[test]
    fn test_get_srgb_profile_path() {
        let _ = get_srgb_profile_path();
    }
}

/// Live tests that change the machine's colour-management state.
///
/// Ignored by default because they rewrite the default ICC association of
/// `\\.\DISPLAY1`. Run them explicitly:
/// `cargo test --lib live_profile -- --ignored --nocapture`.
#[cfg(all(test, windows))]
mod live_profile_tests {
    use super::*;
    use std::path::Path;

    const DISPLAY: &str = r"\\.\DISPLAY1";
    const XIAOMI: &str = r"C:\WINDOWS\System32\spool\drivers\color\xiaomi_D65P3_base.icc";
    const SRGB: &str = r"C:\WINDOWS\System32\spool\drivers\color\sRGB Color Space Profile.icm";

    fn file_name_of(path: &str) -> String {
        Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// Puts the display's default profile back the way the test found it, even
    /// when an assertion panics: a live test must never leave the panel on a
    /// profile the user did not choose.
    struct RestoreOnDrop {
        previous: Option<String>,
    }

    impl Drop for RestoreOnDrop {
        fn drop(&mut self) {
            match self.previous.clone() {
                Some(previous) => {
                    if let Err(e) = load_icc_profile(DISPLAY, &previous) {
                        eprintln!("live test: could not restore {previous}: {e:?}");
                    }
                }
                None => {
                    // Found with nothing associated — hand the display back to
                    // Windows' default instead of leaving sRGB pinned to it.
                    if let Some(current) = get_icm_profile_gdi(DISPLAY) {
                        if let Err(e) = wcs::disassociate_profile(DISPLAY, Path::new(&current)) {
                            eprintln!("live test: could not clear {current}: {e}");
                        }
                    }
                }
            }
        }
    }

    /// The whole point of this change: the Xiaomi profile really associates
    /// with a display, WCS really reports it back, and a reset really restores
    /// sRGB. Everything else in this module is unit-tested; this one talks to
    /// the live Windows colour stack.
    #[test]
    #[ignore = "live: rewrites the default ICC association of \\\\.\\DISPLAY1"]
    fn live_xiaomi_profile_round_trips_through_wcs() {
        assert!(Path::new(XIAOMI).is_file(), "{XIAOMI} is not installed");
        assert!(
            attached_display_devices()
                .iter()
                .any(|d| d.device == DISPLAY),
            "{DISPLAY} is not attached"
        );

        let previous = get_icm_profile_gdi(DISPLAY);
        println!("previous default profile of {DISPLAY}: {previous:?}");
        let _restore = RestoreOnDrop {
            previous: previous.clone(),
        };

        load_icc_profile(DISPLAY, XIAOMI).expect("AssociateColorProfileWithDeviceW must accept it");
        let read_back = get_icm_profile_gdi(DISPLAY)
            .expect("WcsGetDefaultColorProfile must report the profile just associated");
        println!("read back after load: {read_back}");
        assert_eq!(file_name_of(&read_back), file_name_of(XIAOMI));
        assert!(
            read_back.eq_ignore_ascii_case(XIAOMI),
            "expected the full path {XIAOMI}, got {read_back}"
        );

        // The UI reads the same value through `get_color_status`: it must agree.
        let status = get_color_status().expect("colour status");
        let listed = status
            .displays
            .iter()
            .find(|d| d.device_name == DISPLAY)
            .and_then(|d| d.current_profile.clone())
            .expect("get_color_status must report the profile the UI shows");
        println!("get_color_status reports: {listed}");
        assert_eq!(file_name_of(&listed), file_name_of(XIAOMI));

        unload_icc_profile(DISPLAY).expect("the reset to sRGB must succeed");
        let after_reset =
            get_icm_profile_gdi(DISPLAY).expect("a reset display must report the sRGB profile");
        println!("read back after reset: {after_reset}");
        assert_eq!(file_name_of(&after_reset), file_name_of(SRGB));

        // `_restore` puts back whatever was there before this test ran.
    }
}
