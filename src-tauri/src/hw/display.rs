//! Display brightness, HDR, and refresh rate control.
//!
//! Uses IGCL (Intel Graphics Command Library) for brightness and WMI
//! for display info queries on Windows.

use crate::hw::errors::{HardwareError, HardwareResult};
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicI16, AtomicU8, Ordering};

/// Set to `false` after the first `set_brightness_igcl` failure so we never
/// retry a DLL that cannot load — avoids a WARN log on every brightness change.
static IGCL_SET_AVAILABLE: AtomicBool = AtomicBool::new(true);

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DisplayInfo {
    pub brightness: u8,
    pub hdr_enabled: bool,
    pub refresh_rate_hz: u32,
    /// All Hz values supported by the primary display at its current resolution.
    pub available_refresh_rates: Vec<u32>,
    /// True when the user has selected the max available refresh rate.
    pub dynamic_refresh_rate_capable: bool,
    /// Intel PSR2 DRRS (Panel Self Refresh 2 Display Refresh Rate Switching).
    /// Controlled via the Intel Arc driver registry key Psr2DrrsEnable.
    pub adaptive_refresh_rate: bool,
    pub ai_brightness: bool,
    pub ai_brightness_config: AiBrightnessConfig,
    /// Current ambient illuminance from the light sensor (lux). None when unavailable.
    pub ambient_lux: Option<f32>,
}

const IGCL_DLL: &str = r"C:\Windows\System32\ControlLib.dll";
const AI_BRIGHTNESS_REG_KEY: &str = r"SOFTWARE\MI\DisplaySettings";
const AI_BRIGHTNESS_REG_VALUE: &str = "AiAdaptiveBrightness";
const AI_BRIGHTNESS_MIN_VALUE: &str = "AiBrightnessMin";
const AI_BRIGHTNESS_MAX_VALUE: &str = "AiBrightnessMax";
const AI_BRIGHTNESS_SENS_VALUE: &str = "AiBrightnessSensitivity";
const AI_BRIGHTNESS_SMTH_VALUE: &str = "AiBrightnessSmoothing";

/// Sensitivity / range configuration for our own adaptive brightness loop.
///
/// Formula per iteration (every 2 s):
///   max_lux  = 2000 / (sensitivity / 100)   — lux where ceiling is reached
///   target   = clamp(min + (lux / max_lux) * (max - min), min, max)
///   smoothed = current + (target - current) * (1 - smoothing/100)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiBrightnessConfig {
    /// Whether our adaptive loop should run.
    pub enabled: bool,
    /// Floor: brightness will never drop below this % (5-80, default 10).
    pub min_brightness: u8,
    /// Ceiling: brightness will never exceed this % (20-100, default 100).
    pub max_brightness: u8,
    /// Reactivity: 100 = full range at 2000 lux, 200 = at 1000 lux (more), 50 = at 4000 lux (less).
    pub sensitivity: u8,
    /// Transition smoothing 0-90: 0 = instant, 30 = default (fast), 90 = very gradual.
    pub smoothing: u8,
}

// ── User-override offset for the adaptive loop ────────────────────────────────
//
// When the user manually adjusts brightness while auto-brightness is active,
// we compute the delta between their chosen value and what the loop would have
// produced at the current lux level.  This offset is added to every future
// loop iteration so the curve shifts to match the user's preference without
// disabling automation entirely.
//
// The offset is:
//   • stored as a signed integer in the range -100..=100
//   • applied before the final clamp(min, max)
//   • reset whenever the user disables auto-brightness or changes its config

/// Last lux-based target (before offset) stored so we can compute the delta.
static AUTO_LAST_TARGET: AtomicU8 = AtomicU8::new(50);
/// Signed offset (percentage points) to add to the loop's raw target.
static AUTO_OFFSET: AtomicI16 = AtomicI16::new(0);
/// Whether the offset was explicitly set by the user (false = use 0).
static AUTO_OFFSET_ACTIVE: AtomicBool = AtomicBool::new(false);

/// Called by the `set_brightness` Tauri command when auto-brightness is on.
/// Records the delta so future loop iterations honour the user's preference.
pub fn record_user_brightness_override(user_value: u8) {
    let last_target = AUTO_LAST_TARGET.load(Ordering::Relaxed);
    let offset = user_value as i16 - last_target as i16;
    AUTO_OFFSET.store(offset, Ordering::Relaxed);
    AUTO_OFFSET_ACTIVE.store(true, Ordering::Relaxed);
    log::debug!(
        "auto_brightness: user override {user_value}% \
         (last_target={last_target}%, offset={offset:+})"
    );
}

/// Reset the offset — call when auto-brightness is toggled or config changes.
pub fn clear_user_brightness_override() {
    AUTO_OFFSET.store(0, Ordering::Relaxed);
    AUTO_OFFSET_ACTIVE.store(false, Ordering::Relaxed);
}

/// Read the current display brightness from WMI (ground truth) or IGCL.
/// WmiMonitorBrightness.CurrentBrightness is what Windows Display Settings
/// reads, so it is the correct source for "what Windows thinks the brightness is".
fn read_current_brightness() -> Option<u8> {
    get_brightness_wmi().or_else(|_| get_brightness_igcl()).ok()
}

/// Lightweight brightness read (no full DisplayInfo) for the gesture thread.
pub fn current_brightness() -> u8 {
    read_current_brightness().unwrap_or(80)
}

pub fn get_display_info() -> HardwareResult<DisplayInfo> {
    // WMI brightness = what Windows Display Settings slider shows (ground truth).
    let brightness = get_brightness_wmi().unwrap_or_else(|_| get_brightness_igcl().unwrap_or(80));
    let hdr_enabled = get_hdr_state();
    let refresh_rate_hz = get_refresh_rate().unwrap_or(120);
    // S25-009: Propagate error from get_available_refresh_rates.
    let available_refresh_rates = get_available_refresh_rates()?;
    // DRR is active when the display is set to its highest supported refresh rate.
    let dynamic_refresh_rate_capable = available_refresh_rates
        .last()
        .map(|&max| max == refresh_rate_hz)
        .unwrap_or(false);
    let adaptive_refresh_rate = get_intel_drrs();
    let ai_brightness_config = get_ai_brightness_config();
    let ai_brightness = ai_brightness_config.enabled;
    let ambient_lux = get_ambient_lux().filter(|&v| v > 0.5);
    Ok(DisplayInfo {
        brightness,
        hdr_enabled,
        refresh_rate_hz,
        available_refresh_rates,
        dynamic_refresh_rate_capable,
        adaptive_refresh_rate,
        ai_brightness,
        ai_brightness_config,
        ambient_lux,
    })
}

pub fn set_brightness(level: u8) -> HardwareResult<()> {
    let level = level.clamp(10, 100);
    // Only try IGCL if it has not already failed permanently.
    if IGCL_SET_AVAILABLE.load(Ordering::Relaxed) {
        if let Err(e) = set_brightness_igcl(level) {
            log::warn!("IGCL brightness failed: {e}, falling back to WMI permanently");
            IGCL_SET_AVAILABLE.store(false, Ordering::Relaxed);
            set_brightness_wmi(level)?;
        }
    } else {
        set_brightness_wmi(level)?;
    }
    Ok(())
}

pub fn set_hdr(enabled: bool) -> HardwareResult<()> {
    set_hdr_ccd(enabled)
}

pub fn set_ai_brightness(enabled: bool) -> HardwareResult<()> {
    // Toggle the enabled flag while preserving all other config values.
    let mut cfg = get_ai_brightness_config();
    cfg.enabled = enabled;
    set_ai_brightness_config(cfg).map_err(|e| {
        log::error!("[display] set_ai_brightness_config failed: {e}");
        e
    })?;
    if enabled {
        // Windows has its own ALS-based adaptive brightness (ADAPTBRIGHT power plan setting).
        // If both are active they fight over the same backlight knob, causing the 90% cap
        // symptom. Disable Windows adaptive brightness while our loop is in charge.
        disable_windows_adaptive_brightness();
    }
    log::info!(
        "[display] AI brightness {}",
        if enabled { "enabled" } else { "disabled" }
    );
    Ok(())
}

// ── Adaptive brightness config ────────────────────────────────────────────────

fn read_display_dword(name: &str, default: u32) -> u32 {
    #[cfg(windows)]
    {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
        if let Ok(Some(key)) = RegKeyGuard::open_read(HKEY_LOCAL_MACHINE, AI_BRIGHTNESS_REG_KEY) {
            if let Ok(Some(v)) = key.read_u32(name) {
                return v;
            }
        }
    }
    default
}

fn write_display_dword(name: &str, value: u32) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
        let key = RegKeyGuard::create_write(HKEY_LOCAL_MACHINE, AI_BRIGHTNESS_REG_KEY)
            .map_err(|e| HardwareError::Registry(format!("create display settings key: {e}")))?;
        key.write_u32(name, value)
            .map_err(HardwareError::Registry)?;
    }
    Ok(())
}

pub fn get_ai_brightness_config() -> AiBrightnessConfig {
    let enabled = get_ai_brightness_registry().unwrap_or(false);
    let min_b = (read_display_dword(AI_BRIGHTNESS_MIN_VALUE, 10) as u8).clamp(5, 80);
    let max_b = (read_display_dword(AI_BRIGHTNESS_MAX_VALUE, 100) as u8).clamp(min_b + 5, 100);
    AiBrightnessConfig {
        enabled,
        min_brightness: min_b,
        max_brightness: max_b,
        sensitivity: (read_display_dword(AI_BRIGHTNESS_SENS_VALUE, 100) as u8).clamp(10, 200),
        smoothing: (read_display_dword(AI_BRIGHTNESS_SMTH_VALUE, 30) as u8).min(90),
    }
}

pub fn set_ai_brightness_config(config: AiBrightnessConfig) -> HardwareResult<()> {
    persist_ai_brightness_registry(config.enabled)?;
    write_display_dword(AI_BRIGHTNESS_MIN_VALUE, config.min_brightness as u32)?;
    write_display_dword(AI_BRIGHTNESS_MAX_VALUE, config.max_brightness as u32)?;
    write_display_dword(AI_BRIGHTNESS_SENS_VALUE, config.sensitivity as u32)?;
    write_display_dword(AI_BRIGHTNESS_SMTH_VALUE, config.smoothing as u32)?;
    Ok(())
}

// ── Ambient light sensor ──────────────────────────────────────────────────────

#[cfg(windows)]
fn get_ambient_lux() -> Option<f32> {
    use windows::Devices::Sensors::LightSensor;
    let sensor = LightSensor::GetDefault().ok()?;
    log::debug!("[display] Ambient light sensor found");
    let reading = sensor.GetCurrentReading().ok()?;
    let lux = reading.IlluminanceInLux().ok()?;
    log::debug!("[display] Ambient lux: {lux}");
    Some(lux)
}

#[cfg(not(windows))]
fn get_ambient_lux() -> Option<f32> {
    None
}

// ── Adaptive brightness background loop ──────────────────────────────────────

/// Returns `true` when the monitor is powered on.
///
/// Uses `GetSystemMetrics(SM_MONITORPOWER)`:
/// -1 = display is on
///  1 = display is going to power-off
///  2 = display is off
///
/// On non-Windows or if the call fails, we assume the display is on.
#[cfg(windows)]
fn is_display_on() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SYSTEM_METRICS_INDEX};
    // SM_MONITORPOWER is not exposed by the windows 0.58 crate; the raw value is 112.
    const SM_MONITORPOWER: i32 = 112;
    // SAFETY: GetSystemMetrics is a thread-safe Win32 API call.
    let val = unsafe { GetSystemMetrics(SYSTEM_METRICS_INDEX(SM_MONITORPOWER)) };
    // -1 means the display is on; anything else means off or transitioning.
    val == -1
}

#[cfg(not(windows))]
fn is_display_on() -> bool {
    true
}

/// Spawned once at startup. Every 2 s it reads the ambient light sensor and
/// adjusts screen brightness according to the user-configured sensitivity curve.
/// Config changes are picked up automatically on each iteration.
pub async fn adaptive_brightness_loop() {
    let mut smoothed: Option<f32> = None;
    let mut no_sensor_warned = false;
    // Last value we applied so we can detect external changes (Fn keys, OS).
    let mut last_set: Option<u8> = None;
    // Track whether we have already disabled Windows ADAPTBRIGHT for the
    // current "enabled session".  Reset when adaptive brightness is turned off
    // so we re-disable it if the user re-enables.
    let mut adaptbright_suppressed = false;
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // Skip the iteration when the display is off (lid closed, sleep, etc.)
        // to avoid wasting CPU and fighting with the OS power manager.
        let display_on = tokio::task::spawn_blocking(is_display_on)
            .await
            .unwrap_or(true);
        if !display_on {
            continue;
        }

        // Run blocking hardware calls on the blocking thread pool to avoid
        // starving the tokio runtime worker threads.
        let Ok((cfg, brightness_actual)) = tokio::task::spawn_blocking(|| {
            let cfg = get_ai_brightness_config();
            let brightness_actual = if cfg.enabled {
                read_current_brightness()
            } else {
                None
            };
            (cfg, brightness_actual)
        })
        .await
        else {
            log::warn!("adaptive_brightness: spawn_blocking(config) panicked");
            continue;
        };

        if !cfg.enabled {
            smoothed = None;
            last_set = None;
            adaptbright_suppressed = false;
            continue;
        }

        // Ensure Windows' own ADAPTBRIGHT (power-plan adaptive brightness) is
        // off — if both run simultaneously they fight over the backlight,
        // causing the brightness-near-zero oscillation symptom.
        if !adaptbright_suppressed {
            let _ = tokio::task::spawn_blocking(disable_windows_adaptive_brightness).await;
            adaptbright_suppressed = true;
        }

        // ── Detect external brightness changes (Fn keys, Windows sliders) ──
        // If the actual brightness differs from what we last set by ≥ 2 pp,
        // someone else changed it.  Treat it as a user preference shift:
        // compute a new offset so the loop keeps the adjusted baseline.
        if let (Some(prev), Some(actual)) = (last_set, brightness_actual) {
            let diff = (actual as i16 - prev as i16).abs();
            if diff >= 2 {
                let raw = AUTO_LAST_TARGET.load(Ordering::Relaxed);
                let new_offset = actual as i16 - raw as i16;
                AUTO_OFFSET.store(new_offset, Ordering::Relaxed);
                AUTO_OFFSET_ACTIVE.store(true, Ordering::Relaxed);
                // Snap smoothed to actual so we don't animate back.
                smoothed = Some(actual as f32);
                log::debug!(
                    "auto_brightness: external change detected \
                     prev={prev}% actual={actual}% → offset={new_offset:+}"
                );
            }
        }

        let lux = match tokio::task::spawn_blocking(get_ambient_lux).await {
            // A reading ≤ 0 lux is physically impossible with the screen on; it
            // means the sensor returned an invalid/uninitialised value (common at
            // process startup on this hardware).  Treat it the same as "no sensor"
            // so we never drive brightness to the floor from a bad initial read.
            Ok(Some(v)) if v > 0.5 => v,
            Ok(Some(_)) => continue,
            Ok(None) => {
                if !no_sensor_warned {
                    log::warn!(
                        "adaptive_brightness: no ambient light sensor found — loop idle. \
                         LightSensor::GetDefault() returned None. \
                         Check: (1) sensor driver installed, (2) sensor enabled in Device Manager, \
                         (3) Devices_Sensors feature in Cargo.toml (confirmed present)."
                    );
                    no_sensor_warned = true;
                }
                continue;
            }
            Err(e) => {
                log::warn!("adaptive_brightness: spawn_blocking(get_ambient_lux) panicked: {e}");
                continue;
            }
        };
        no_sensor_warned = false;
        // sensitivity=100 → reaches ceiling at 2000 lux
        // sensitivity=200 → reaches ceiling at 1000 lux  (more reactive)
        // sensitivity=50  → reaches ceiling at 4000 lux  (less reactive)
        let max_lux = 2000.0_f32 * (100.0 / cfg.sensitivity.max(1) as f32);
        let range = cfg.max_brightness as f32 - cfg.min_brightness as f32;
        // CURVE_BOOST lifts the entire brightness curve by this many percentage
        // points without changing the slope or the user-configurable min/max.
        const CURVE_BOOST: f32 = 20.0;
        let raw_target = (cfg.min_brightness as f32 + (lux / max_lux) * range + CURVE_BOOST)
            .clamp(cfg.min_brightness as f32, cfg.max_brightness as f32);

        // Persist raw target so set_brightness can compute the correct offset.
        AUTO_LAST_TARGET.store(raw_target.round() as u8, Ordering::Relaxed);

        // Apply user-override offset: shifts the entire curve up/down so that
        // when the user manually sets brightness the automation respects that
        // preference and only adjusts relative to it as light changes.
        let offset = if AUTO_OFFSET_ACTIVE.load(Ordering::Relaxed) {
            AUTO_OFFSET.load(Ordering::Relaxed) as f32
        } else {
            0.0
        };
        let target =
            (raw_target + offset).clamp(cfg.min_brightness as f32, cfg.max_brightness as f32);

        let current = match smoothed {
            Some(s) => s,
            None => {
                // First valid lux reading: seed the smoother from actual current
                // brightness so we never jump immediately to the computed target.
                // read_current_brightness → get_brightness_wmi is a blocking COM
                // call, so it must run on the blocking thread pool.
                tokio::task::spawn_blocking(move || {
                    read_current_brightness()
                        .map(|b| b as f32)
                        .unwrap_or(target)
                })
                .await
                .unwrap_or(target)
            }
        };
        let sf = cfg.smoothing.min(95) as f32 / 100.0;
        let next = current + (target - current) * (1.0 - sf);
        smoothed = Some(next);
        let value = next.round() as u8;
        // Hysteresis: skip the write if the new value is the same as last
        // (or within 1 pp) to avoid constant low-amplitude oscillations.
        if last_set.is_some_and(|prev| (value as i16 - prev as i16).abs() < 2) {
            continue;
        }
        let set_value = value;
        match tokio::task::spawn_blocking(move || set_brightness(set_value)).await {
            Ok(Ok(())) => {
                last_set = Some(set_value);
            }
            Ok(Err(e)) => {
                log::warn!("adaptive_brightness: set_brightness error: {e}");
            }
            Err(e) => {
                log::warn!("adaptive_brightness: set_brightness task panicked: {e}");
            }
        }
    }
}

// ── IGCL FFI ────────────────────────────────────────────────────────────────

#[cfg(windows)]
mod igcl {
    use std::ffi::c_void;

    #[repr(C)]
    pub struct CtlInitArgs {
        pub size: u32,
        pub app_version: u64,
        pub flags: u32,
    }

    /// Matches Intel IGCL `ctl_brightness_settings_t` (sizeof = 32).
    /// Fields: Size(4) | Version(1) + 3-pad | TargetBrightness(8) |
    ///         SmoothTransitionTargetBrightness(8) | SmoothTransitionTime(4) + 4-pad
    #[repr(C)]
    pub struct CtlBrightnessArgs {
        pub size: u32,
        pub version: u8,
        // [3 bytes C-alignment padding before f64]
        pub target_brightness: f64,
        pub smooth_target_brightness: f64,
        pub smooth_time_ms: u32,
        // [4 bytes C-alignment trailing padding]
    }

    pub type CtlApiHandle = *mut c_void;
    pub type CtlDeviceHandle = *mut c_void;
    pub type CtlResult = u32; // 0 = success

    // Function pointer types
    pub type FnCtlInit = unsafe extern "C" fn(*mut CtlInitArgs, *mut CtlApiHandle) -> CtlResult;
    pub type FnCtlClose = unsafe extern "C" fn(CtlApiHandle) -> CtlResult;
    pub type FnCtlEnumerateDevices =
        unsafe extern "C" fn(CtlApiHandle, *mut u32, *mut CtlDeviceHandle) -> CtlResult;
    pub type FnCtlGetBrightnessSetting =
        unsafe extern "C" fn(CtlDeviceHandle, *mut CtlBrightnessArgs) -> CtlResult;
    pub type FnCtlSetBrightnessSetting =
        unsafe extern "C" fn(CtlDeviceHandle, *mut CtlBrightnessArgs) -> CtlResult;
}

#[cfg(windows)]
pub fn with_igcl_device_pub<F, T>(f: F) -> HardwareResult<T>
where
    F: FnOnce(*mut std::ffi::c_void, &libloading::Library) -> HardwareResult<T>,
{
    use igcl::*;
    use libloading::Library;

    unsafe {
        // SAFETY: IGCL DLL function pointers (ctlInit, ctlEnumerateDevices, ctlClose) are
        // loaded from the dynamically-linked ControlLib.dll — the Symbol objects guard the
        // function lifetimes. CtlInitArgs is a POD struct with correct size/alignment.
        // The api_handle and device pointers are opaque handles managed by IGCL.
        // Use the IGCL DLL path found during startup discovery; fall back to the system default.
        let igcl_path = crate::hw::discovery::global_profile()
            .and_then(|p| p.igcl_dll_path)
            .unwrap_or_else(|| IGCL_DLL.to_string());
        let lib = Library::new(&igcl_path).context("Load ControlLib.dll")?;

        let ctl_init: libloading::Symbol<FnCtlInit> = lib.get(b"ctlInit\0").context("ctlInit")?;
        let ctl_enumerate: libloading::Symbol<FnCtlEnumerateDevices> = lib
            .get(b"ctlEnumerateDevices\0")
            .context("ctlEnumerateDevices")?;
        let ctl_close: libloading::Symbol<FnCtlClose> =
            lib.get(b"ctlClose\0").context("ctlClose")?;

        let mut init_args = CtlInitArgs {
            size: std::mem::size_of::<CtlInitArgs>() as u32,
            app_version: 1,
            flags: 0,
        };
        let mut api_handle: CtlApiHandle = std::ptr::null_mut();
        let rc = ctl_init(&mut init_args, &mut api_handle);
        if rc != 0 {
            return Err(HardwareError::Display(format!("ctlInit failed: {rc}")));
        }

        let mut count: u32 = 0;
        ctl_enumerate(api_handle, &mut count, std::ptr::null_mut());
        if count == 0 {
            ctl_close(api_handle);
            return Err(HardwareError::Display("No IGCL devices found".to_string()));
        }
        let mut devices = vec![std::ptr::null_mut::<std::ffi::c_void>(); count as usize];
        ctl_enumerate(api_handle, &mut count, devices.as_mut_ptr());

        let device = devices[0];
        let result = f(device, &lib);
        ctl_close(api_handle);
        result
    }
}

#[cfg(windows)]
fn get_brightness_igcl() -> HardwareResult<u8> {
    use igcl::*;
    with_igcl_device_pub(|device, lib| unsafe {
        // SAFETY: device is a valid IGCL device handle obtained from ctlEnumerateDevices.
        // CtlBrightnessArgs is POD with correctly set size field; the IGCL function only reads
        // args and writes back the brightness value.
        let get_brightness: libloading::Symbol<FnCtlGetBrightnessSetting> = lib
            .get(b"ctlGetBrightnessSetting\0")
            .context("ctlGetBrightnessSetting")?;
        let mut args = CtlBrightnessArgs {
            size: std::mem::size_of::<CtlBrightnessArgs>() as u32,
            version: 0,
            target_brightness: 0.0,
            smooth_target_brightness: 0.0,
            smooth_time_ms: 0,
        };
        let rc = get_brightness(device as CtlDeviceHandle, &mut args);
        if rc != 0 {
            return Err(HardwareError::Display(format!(
                "ctlGetBrightnessSetting failed: {rc:#x}"
            )));
        }
        Ok(args.target_brightness.clamp(0.0, 100.0) as u8)
    })
}

#[cfg(not(windows))]
fn get_brightness_igcl() -> HardwareResult<u8> {
    Err(HardwareError::NotSupported(
        "IGCL not available on non-Windows".to_string(),
    ))
}

#[cfg(windows)]
fn set_brightness_igcl(level: u8) -> HardwareResult<()> {
    use igcl::*;
    with_igcl_device_pub(|device, lib| unsafe {
        // SAFETY: device is a valid IGCL device handle. The brightness args struct is sized
        // correctly and both target_brightness and smooth_target_brightness are initialized.
        let set_brightness: libloading::Symbol<FnCtlSetBrightnessSetting> = lib
            .get(b"ctlSetBrightnessSetting\0")
            .context("ctlSetBrightnessSetting")?;
        let mut args = CtlBrightnessArgs {
            size: std::mem::size_of::<CtlBrightnessArgs>() as u32,
            version: 0,
            target_brightness: level as f64,
            smooth_target_brightness: level as f64,
            smooth_time_ms: 0,
        };
        let rc = set_brightness(device as CtlDeviceHandle, &mut args);
        if rc != 0 {
            return Err(HardwareError::Display(format!(
                "ctlSetBrightnessSetting failed: {rc:#x}"
            )));
        }
        Ok(())
    })
}

#[cfg(not(windows))]
fn set_brightness_igcl(_level: u8) -> HardwareResult<()> {
    Err(HardwareError::NotSupported(
        "IGCL not available on non-Windows".to_string(),
    ))
}

// ── WMI fallback ────────────────────────────────────────────────────────────

fn get_brightness_wmi() -> HardwareResult<u8> {
    #[cfg(windows)]
    {
        use crate::hw::wmi_cache;
        use crate::util::wmi_extract;
        use std::collections::HashMap;

        let res = wmi_cache::with_wmi(|wmi| {
            let results: Vec<HashMap<String, wmi::Variant>> = wmi
                .raw_query("SELECT CurrentBrightness FROM WmiMonitorBrightness")
                .context("WmiMonitorBrightness")?;
            let first = results.first().context("No monitor")?;
            Ok(wmi_extract::extract_u32(first, "CurrentBrightness")
                .map(|v| v as u8)
                .unwrap_or(80))
        });
        res
    }
    #[cfg(not(windows))]
    {
        Ok(80)
    }
}

fn set_brightness_wmi(level: u8) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        // WmiSetBrightness requires:
        //  1. Targeting a specific CIM *instance* (not just the class name)
        //  2. Brightness typed as [byte] (UInt8), Timeout as [uint32]
        // Using -ClassName without -InputObject returns "Invalid method Parameter(s)".
        let cmd = format!(
            "$i=Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightnessMethods; \
             Invoke-CimMethod -InputObject $i -MethodName WmiSetBrightness \
             -Arguments @{{Timeout=[uint32]1;Brightness=[byte]{}}}",
            level
        );
        let status = std::process::Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &cmd])
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .context("PowerShell spawn for WmiSetBrightness")?;
        if !status.success() {
            return Err(HardwareError::Display(format!(
                "WmiSetBrightness exited with {status}"
            )));
        }
    }
    Ok(())
}

// ── Windows built-in adaptive brightness (ADAPTBRIGHT) ───────────────────────
//
// Windows has its own ALS-based adaptive brightness in the active power plan
// (power setting ADAPTBRIGHT = fbd9aa66-9553-4097-ba44-ed6e9d65eab8).
// When it is enabled it intercepts every brightness request and adjusts the
// value based on its own sensor reading, producing the well-known "caps at 90%"
// symptom where the user sets 100% but Windows immediately dials it back.
// MiControl provides its own, better-calibrated loop, so the two must not run
// concurrently.  This function silently disables ADAPTBRIGHT for the current
// power scheme on both AC and DC.  It is best-effort (no error returned) — if
// powercfg is unavailable the loop still works, just with occasional fighting.
fn disable_windows_adaptive_brightness() {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let scheme = "SCHEME_CURRENT";
        let sub = "SUB_VIDEO";
        let guid = "ADAPTBRIGHT";
        for flag in ["/SETACVALUEINDEX", "/SETDCVALUEINDEX"] {
            let _ = std::process::Command::new("powercfg")
                .args([flag, scheme, sub, guid, "0"])
                .creation_flags(0x08000000) // CREATE_NO_WINDOW
                .output();
        }
        // Activate the scheme so the change takes effect immediately.
        let _ = std::process::Command::new("powercfg")
            .args(["/setactive", scheme])
            .creation_flags(0x08000000)
            .output();
        log::info!("adaptive_brightness: disabled Windows ADAPTBRIGHT (power plan)");
    }
}

// ── HDR state via Windows CCD API ────────────────────────────────────────────
//
// Windows stores HDR (Advanced Color / Wide Color Gamut) state per-display
// in the Connected Displays API (CCD).  We use the windows crate's typed
// bindings for type-safety and correct struct layout.
//
// GetDisplayConfigBufferSizes → sizes → QueryDisplayConfig → paths[] →
// DisplayConfigGetDeviceInfo(GET_ADVANCED_COLOR_INFO) → read bit 1 for HDR on
// DisplayConfigSetDeviceInfo(SET_ADVANCED_COLOR_STATE) → write bit 0 to toggle
//
// None of these calls require administrator privileges.
// A retry loop handles the rare race where display config changes between the
// GetDisplayConfigBufferSizes and QueryDisplayConfig calls (ERROR_INSUFFICIENT_BUFFER).

#[cfg(windows)]
use windows::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, DisplayConfigSetDeviceInfo, GetDisplayConfigBufferSizes,
    QueryDisplayConfig, DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
    DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_DEVICE_INFO_SET_ADVANCED_COLOR_STATE,
    DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SET_ADVANCED_COLOR_STATE, QDC_ONLY_ACTIVE_PATHS,
};
#[cfg(windows)]
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS};

/// Call GetDisplayConfigBufferSizes + QueryDisplayConfig with retry on
/// ERROR_INSUFFICIENT_BUFFER (display config may change between the two calls).
///
/// # Safety
///
/// This function calls raw Win32 CCD display API functions. The caller must ensure that
/// the returned path/mode vectors are not modified while the underlying display config
/// handle (which does not exist here — this is a one-shot query) remains in use.
#[cfg(windows)]
unsafe fn query_display_config_retry() -> HardwareResult<(
    u32,
    u32,
    Vec<DISPLAYCONFIG_PATH_INFO>,
    Vec<DISPLAYCONFIG_MODE_INFO>,
)> {
    for _ in 0..5 {
        let mut np = 0u32;
        let mut nm = 0u32;
        let rc = GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm);
        if rc != ERROR_SUCCESS {
            return Err(HardwareError::Display(format!(
                "GetDisplayConfigBufferSizes failed: {}",
                rc.0
            )));
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        let rc = QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut np,
            paths.as_mut_ptr(),
            &mut nm,
            modes.as_mut_ptr(),
            None,
        );
        if rc == ERROR_INSUFFICIENT_BUFFER {
            continue; // retry with fresh buffer sizes
        }
        if rc != ERROR_SUCCESS {
            return Err(HardwareError::Display(format!(
                "QueryDisplayConfig failed: {}",
                rc.0
            )));
        }
        return Ok((np, nm, paths, modes));
    }
    Err(HardwareError::Display(
        "QueryDisplayConfig: too many retries (display config keeps changing)".to_string(),
    ))
}

/// Read the real HDR (Advanced Color) enabled state for the primary display.
pub fn get_hdr_state() -> bool {
    #[cfg(windows)]
    unsafe {
        // SAFETY: query_display_config_retry() returns valid paths with adapterId/id populated
        // from the active display topology. The DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO struct
        // is correctly initialized and its header points to its own base address.
        let (np, _nm, paths, _modes) = match query_display_config_retry() {
            Ok(x) => x,
            Err(_) => return false,
        };
        for path in paths.iter().take(np as usize) {
            let mut info = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
                    size: std::mem::size_of::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>() as u32,
                    adapterId: path.targetInfo.adapterId,
                    id: path.targetInfo.id,
                },
                ..Default::default()
            };
            // Pass pointer to the header (= base of struct, same address since header is field 0)
            let rc = DisplayConfigGetDeviceInfo(&mut info.header as *mut _);
            if rc == 0 {
                // Anonymous union: value field holds the bitfield
                // bit 0 = advancedColorSupported, bit 1 = advancedColorEnabled
                if info.Anonymous.value & 0x2 != 0 {
                    return true;
                }
            }
        }
    }
    false
}

/// Enable or disable HDR (Advanced Color) on the primary display.
///
/// Uses `DisplayConfigSetDeviceInfo` — operates on the current user's
/// interactive session and does NOT require administrator privileges.
fn set_hdr_ccd(enabled: bool) -> HardwareResult<()> {
    #[cfg(windows)]
    unsafe {
        // SAFETY: Same as get_hdr_state — paths are valid CCD topology data.
        // DISPLAYCONFIG_SET_ADVANCED_COLOR_STATE is correctly initialized with
        // the enableAdvancedColor bit set. DisplayConfigSetDeviceInfo only reads the
        // struct during the call and does not retain the pointer.
        let (np, _nm, paths, _modes) =
            query_display_config_retry().context("query display config")?;
        let mut last_err = 0i32;
        for path in paths.iter().take(np as usize) {
            let mut state = DISPLAYCONFIG_SET_ADVANCED_COLOR_STATE {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_SET_ADVANCED_COLOR_STATE,
                    size: std::mem::size_of::<DISPLAYCONFIG_SET_ADVANCED_COLOR_STATE>() as u32,
                    adapterId: path.targetInfo.adapterId,
                    id: path.targetInfo.id,
                },
                ..Default::default()
            };
            // bit 0 = enableAdvancedColor
            state.Anonymous.value = enabled as u32;
            let rc = DisplayConfigSetDeviceInfo(&state.header as *const _);
            if rc != 0 {
                last_err = rc;
            }
        }
        if last_err != 0 {
            return Err(HardwareError::Display(format!(
                "DisplayConfigSetDeviceInfo failed: {last_err:#x}"
            )));
        }
    }
    #[cfg(not(windows))]
    {
        log::info!("set_hdr({enabled}) — stub on non-Windows");
    }
    Ok(())
}

// ── Intel PSR2 DRRS (Display Refresh Rate Switching) ─────────────────────────
//
// Intel's PSR2 DRRS is a driver-level feature distinct from the Windows 11
// "Dynamic Refresh Rate" (DRR) API.  It lets the Intel Arc GPU driver
// automatically switch the panel between 60 Hz (idle) and the max rate
// (active content) without Windows involvement.
//
// The Xiaomi laptop BIOS/firmware marks this feature as supported.
// Windows says "Variable refresh rate: Not Supported" because that refers to
// the hardware VRR (FreeSync/G-Sync) capability — a different, faster mechanism.
// PSR2 DRRS works on fixed-rate panels by switching between pre-defined modes.
//
// Controlled via the Intel Arc driver registry key:
// HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-...}\####\Psr2DrrsEnable
//
// Writing requires elevation. Changes take effect after driver restart (brief
// screen flash) or system reboot.

const INTEL_GPU_CLASS: &str =
    r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}";
const DRRS_REG_VALUE: &str = "Psr2DrrsEnable";

#[cfg(windows)]
fn find_intel_arc_driver_key() -> Option<String> {
    use crate::util::registry::RegKeyGuard;
    use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
    // Check that the GPU class key exists before iterating subkeys.
    let _ = RegKeyGuard::open_read(HKEY_LOCAL_MACHINE, INTEL_GPU_CLASS).ok()?;
    for i in 0..=9u32 {
        let name = format!("{:04}", i);
        if let Ok(Some(sub)) =
            RegKeyGuard::open_read(HKEY_LOCAL_MACHINE, &format!("{INTEL_GPU_CLASS}\\{name}"))
        {
            if let Ok(Some(desc)) = sub.read_string("DriverDesc") {
                let dl = desc.to_lowercase();
                if dl.contains("intel")
                    && (dl.contains("arc") || dl.contains("uhd") || dl.contains("iris"))
                {
                    return Some(format!("{}\\{}", INTEL_GPU_CLASS, name));
                }
            }
        }
    }
    None
}

/// Read Intel PSR2 DRRS state from the Arc driver registry key.
pub fn get_intel_drrs() -> bool {
    #[cfg(windows)]
    {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
        if let Some(path) = find_intel_arc_driver_key() {
            if let Ok(Some(key)) = RegKeyGuard::open_read(HKEY_LOCAL_MACHINE, &path) {
                if let Ok(Some(v)) = key.read_u32(DRRS_REG_VALUE) {
                    return v != 0;
                }
            }
        }
    }
    true // default: assume enabled when registry is unreadable
}

/// Write Intel PSR2 DRRS state to the Arc driver registry key.
/// Requires an elevated (admin) process — called from elevated.rs.
/// Changes take effect after the display driver restarts or system reboots.
pub fn set_intel_drrs(enabled: bool) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
        let path = find_intel_arc_driver_key().ok_or_else(|| {
            HardwareError::Display("Intel Arc driver registry key not found".to_string())
        })?;
        // RegKeyGuard::create_write opens with KEY_ALL_ACCESS which includes KEY_WRITE.
        let key = RegKeyGuard::create_write(HKEY_LOCAL_MACHINE, &path).map_err(|e| {
            HardwareError::Registry(format!("open Intel Arc driver key for write: {e}"))
        })?;
        key.write_u32(DRRS_REG_VALUE, enabled as u32)
            .map_err(|e| HardwareError::Registry(format!("set Psr2DrrsEnable: {e}")))?;
    }
    Ok(())
}

// ── Refresh rate ──────────────────────────────────────────────────────────────
///
/// Uses `EnumDisplaySettingsExW` (Win32 GDI) which is the same source the
/// Windows Display Settings page uses when building the "Choose a refresh
/// rate" dropdown.
pub fn get_available_refresh_rates() -> HardwareResult<Vec<u32>> {
    #[cfg(windows)]
    {
        use std::collections::HashSet;
        use windows::Win32::Graphics::Gdi::{
            EnumDisplaySettingsExW, DEVMODEW, ENUM_CURRENT_SETTINGS, ENUM_DISPLAY_SETTINGS_FLAGS,
            ENUM_DISPLAY_SETTINGS_MODE,
        };

        unsafe {
            // SAFETY: DEVMODEW is a POD struct with dmSize correctly set. EnumDisplaySettingsExW
            // writes to the struct and does not retain the pointer. Comparison of dmPelsWidth,
            // dmPelsHeight, dmBitsPerPel, and dmDisplayFrequency reads the fields Windows
            // populated — no uninitialized data is read.
            let mut cur = DEVMODEW {
                dmSize: std::mem::size_of::<DEVMODEW>() as u16,
                ..Default::default()
            };
            // Query current mode to know the active resolution.
            let _ = EnumDisplaySettingsExW(
                None,
                ENUM_CURRENT_SETTINGS,
                &mut cur,
                ENUM_DISPLAY_SETTINGS_FLAGS(0),
            );
            let (w, h, bpp) = (cur.dmPelsWidth, cur.dmPelsHeight, cur.dmBitsPerPel);

            let mut seen: HashSet<u32> = HashSet::new();
            let mut idx = 0u32;
            loop {
                let mut m = DEVMODEW {
                    dmSize: std::mem::size_of::<DEVMODEW>() as u16,
                    ..Default::default()
                };
                if !EnumDisplaySettingsExW(
                    None,
                    ENUM_DISPLAY_SETTINGS_MODE(idx),
                    &mut m,
                    ENUM_DISPLAY_SETTINGS_FLAGS(0),
                )
                .as_bool()
                {
                    break;
                }
                if m.dmPelsWidth == w
                    && m.dmPelsHeight == h
                    && m.dmBitsPerPel == bpp
                    && m.dmDisplayFrequency > 0
                {
                    seen.insert(m.dmDisplayFrequency);
                }
                idx += 1;
            }
            let mut rates: Vec<u32> = seen.into_iter().collect();
            rates.sort_unstable();
            // S25-009: Return error if no refresh rates were found.
            if rates.is_empty() {
                return Err(HardwareError::Display(
                    "No refresh rates found for current display mode".to_string(),
                ));
            }
            Ok(rates)
        }
    }
    #[cfg(not(windows))]
    {
        Ok(vec![60, 120])
    }
}

/// Change the primary display's refresh rate.
///
/// `hz` must be one of the values returned by `get_available_refresh_rates()`.
/// The change is persisted to the registry (`CDS_UPDATEREGISTRY`) so it
/// survives reboots.  Returns an error if the rate is not supported or if
/// Windows rejects the mode change.
pub fn set_refresh_rate(hz: u32) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        use windows::Win32::Graphics::Gdi::{
            ChangeDisplaySettingsExW, EnumDisplaySettingsExW, CDS_TYPE, DEVMODEW,
            DEVMODE_FIELD_FLAGS, DISP_CHANGE, ENUM_CURRENT_SETTINGS, ENUM_DISPLAY_SETTINGS_FLAGS,
        };

        const DM_DISPLAYFREQUENCY: u32 = 0x00400000;
        const CDS_UPDATEREGISTRY_VAL: u32 = 0x00000001;

        unsafe {
            // SAFETY: DEVMODEW is POD with dmSize set. EnumDisplaySettingsExW populates the
            // current mode; we modify dmDisplayFrequency and dmFields before passing it to
            // ChangeDisplaySettingsExW which does not retain the pointer.
            let mut mode = DEVMODEW {
                dmSize: std::mem::size_of::<DEVMODEW>() as u16,
                ..Default::default()
            };
            if !EnumDisplaySettingsExW(
                None,
                ENUM_CURRENT_SETTINGS,
                &mut mode,
                ENUM_DISPLAY_SETTINGS_FLAGS(0),
            )
            .as_bool()
            {
                return Err(HardwareError::Display(
                    "EnumDisplaySettingsExW(CURRENT) failed".to_string(),
                ));
            }
            mode.dmDisplayFrequency = hz;
            // Tell Windows we're only changing the refresh rate field.
            mode.dmFields = DEVMODE_FIELD_FLAGS(DM_DISPLAYFREQUENCY);

            let result = ChangeDisplaySettingsExW(
                None,
                Some(&mode),
                None,
                CDS_TYPE(CDS_UPDATEREGISTRY_VAL),
                None,
            );
            if result == DISP_CHANGE(0) {
                Ok(())
            } else {
                Err(HardwareError::Display(format!("ChangeDisplaySettingsExW failed ({result:?}); requested {hz} Hz may not be supported")))
            }
        }
    }
    #[cfg(not(windows))]
    {
        log::info!("set_refresh_rate({hz}) — stub on non-Windows");
        Ok(())
    }
}

fn get_refresh_rate() -> HardwareResult<u32> {
    #[cfg(windows)]
    {
        use crate::hw::wmi_cache;
        use std::collections::HashMap;

        if let Ok(Some(hz)) = wmi_cache::with_cimv2(|wmi| {
            let results: Vec<HashMap<String, wmi::Variant>> = wmi
                .raw_query("SELECT CurrentRefreshRate FROM Win32_VideoController")
                .unwrap_or_default();
            if let Some(row) = results.first() {
                match row.get("CurrentRefreshRate") {
                    Some(wmi::Variant::UI4(v)) => Ok(Some(*v)),
                    _ => Ok(None),
                }
            } else {
                Ok(None)
            }
        }) {
            return Ok(hz);
        }
    }
    Ok(120)
}

fn get_ai_brightness_registry() -> HardwareResult<bool> {
    #[cfg(windows)]
    {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
        if let Ok(Some(key)) = RegKeyGuard::open_read(HKEY_LOCAL_MACHINE, AI_BRIGHTNESS_REG_KEY) {
            if let Ok(Some(v)) = key.read_u32(AI_BRIGHTNESS_REG_VALUE) {
                return Ok(v != 0);
            }
        }
        Ok(false)
    }
    #[cfg(not(windows))]
    {
        Ok(false)
    }
}

fn persist_ai_brightness_registry(enabled: bool) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
        let key = RegKeyGuard::create_write(HKEY_LOCAL_MACHINE, AI_BRIGHTNESS_REG_KEY)
            .map_err(|e| HardwareError::Registry(format!("Create display settings key: {e}")))?;
        let val: u32 = if enabled { 1 } else { 0 };
        key.write_u32(AI_BRIGHTNESS_REG_VALUE, val)
            .map_err(|e| HardwareError::Registry(format!("Write AI brightness: {e}")))?;
    }
    Ok(())
}
