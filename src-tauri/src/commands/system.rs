//! Tauri commands for system-level operations (display, fan, battery, etc.).
//!
//! Delegates to the `hw` module for each hardware domain and wraps
//! results in Tauri-compatible response types.

use crate::elev_bridge;
use crate::hw::audio::{get_playback_volume as hw_get_audio, AudioVolumeResult};
use crate::hw::battery::{get_battery_info as hw_get_battery, BatteryInfo};
use crate::hw::charging::get_charging_threshold as hw_get_charge;
use crate::hw::discovery::{global_profile, HardwareProfile};
use crate::hw::display::{
    get_ai_brightness_config as hw_get_ai_cfg, get_available_refresh_rates as hw_get_refresh_rates,
    get_display_info as hw_get_display, set_hdr as hw_set_hdr, AiBrightnessConfig, DisplayInfo,
};
use crate::hw::errors::{ErrorResponse, HardwareError};
use crate::hw::fan::{get_fan_info as hw_get_fan, FanInfo, FanMode};
use crate::hw::performance::get_performance_mode as hw_get_perf;
use crate::hw::processes::{get_process_list as hw_get_processes, ProcessInfo};
use crate::hw::startup::{get_autostart as hw_get_autostart, set_autostart as hw_set_autostart};
use crate::hw::system_info::{get_system_info as hw_get_sysinfo, SystemInfo};
use crate::hw::touchpad::{
    get_touchpad_info as hw_get_touchpad, set_touchpad_edge_slide as hw_set_touchpad_edge_slide,
    set_touchpad_gesture_screenshot as hw_set_touchpad_gesture_screenshot,
    set_touchpad_haptics as hw_set_touchpad_haptics,
    set_touchpad_haptics_intensity as hw_set_touchpad_haptics_intensity,
    set_touchpad_repress as hw_set_touchpad_repress,
    set_touchpad_sensitivity as hw_set_touchpad_sensitivity, TouchpadInfo, TouchpadSensitivity,
};
use crate::hw::update::{get_update_status as hw_get_update_status, UpdateStatus};
use crate::state::PerformanceMode;
use crate::util::blocking::run_blocking;

#[tauri::command]
pub async fn get_battery_info() -> Result<BatteryInfo, ErrorResponse> {
    let started = std::time::Instant::now();
    log::debug!(target: "cmd::system", "get_battery_info: start");
    let result = run_blocking(hw_get_battery)
        .await
        .map_err(ErrorResponse::from);
    match &result {
        Ok(info) => log::debug!(
            target: "cmd::system",
            "get_battery_info: ok plugged={} charging={} voltage_mv={} charge_rate_mw={} ac_input_power_mw={:?} elapsed_ms={}",
            info.is_plugged,
            info.is_charging,
            info.voltage_mv,
            info.charge_rate_mw,
            info.ac_input_power_mw,
            started.elapsed().as_millis()
        ),
        Err(error) => log::warn!(
            target: "cmd::system",
            "get_battery_info: failed after {} ms: {}",
            started.elapsed().as_millis(),
            error.message
        ),
    }
    result
}

#[tauri::command]
pub async fn get_display_info() -> Result<DisplayInfo, ErrorResponse> {
    run_blocking(hw_get_display)
        .await
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn set_brightness(level: u8) -> Result<(), ErrorResponse> {
    // If auto-brightness is active, record the delta so the adaptive loop
    // uses the user's chosen value as the new shifted baseline rather than
    // reverting to the pure lux-based calculation.
    // S26-007: Wrap in run_blocking — hw_get_ai_cfg() does sync registry I/O.
    let cfg = run_blocking(move || Ok(hw_get_ai_cfg()))
        .await
        .map_err(ErrorResponse::from)?;
    if cfg.enabled {
        crate::hw::display::record_user_brightness_override(level);
    }
    elev_bridge::run_elevated("set_brightness", serde_json::json!({ "level": level }))
        .await
        .map(|_| ())
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn set_hdr(enabled: bool) -> Result<(), ErrorResponse> {
    // DisplayConfigSetDeviceInfo operates on the current user's interactive
    // session and does NOT require administrator privileges — call directly.
    run_blocking(move || hw_set_hdr(enabled))
        .await
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn set_ai_brightness(enabled: bool) -> Result<(), ErrorResponse> {
    // Always reset the user override when toggling auto-brightness so the
    // loop starts fresh with no inherited delta.
    crate::hw::display::clear_user_brightness_override();
    elev_bridge::run_elevated(
        "set_ai_brightness",
        serde_json::json!({ "enabled": enabled }),
    )
    .await
    .map(|_| ())
    .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn get_ai_brightness_config() -> Result<AiBrightnessConfig, ErrorResponse> {
    // S24-013: Wrap in run_blocking — hw_get_ai_cfg() does sync registry I/O.
    run_blocking(move || Ok(hw_get_ai_cfg()))
        .await
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn set_ai_brightness_config(config: AiBrightnessConfig) -> Result<(), ErrorResponse> {
    // Config change invalidates the old offset (different curve parameters).
    crate::hw::display::clear_user_brightness_override();
    elev_bridge::run_elevated(
        "set_ai_brightness_config",
        serde_json::json!({ "config": config }),
    )
    .await
    .map(|_| ())
    .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn get_fan_info() -> Result<FanInfo, ErrorResponse> {
    // S36-038: The unprivileged process is DENIED access to the thermal WMI
    // classes (EsifDeviceInformation / MSAcpi_ThermalZoneTemperature →
    // 0x80041003 "Access to a CIM resource was not available to the client").
    //
    // First try the in-process read (fast, no round-trip). If CPU/GPU
    // temperatures come back empty AND the elevated bridge is available,
    // re-run with readings fetched from the bridge service (SYSTEM process,
    // which CAN read those WMI classes).
    let local = run_blocking(crate::hw::fan::get_fan_info)
        .await
        .map_err(ErrorResponse::from)?;

    if local.cpu_temp_celsius.is_none() && local.gpu_temp_celsius.is_none() {
        let elevated = crate::hw::fan::get_elevated_thermal_readings().await;
        let has_thermal = elevated.cpu_temp.is_some() || elevated.gpu_temp.is_some();
        if has_thermal {
            return run_blocking(move || crate::hw::fan::get_fan_info_seeded(elevated))
                .await
                .map_err(ErrorResponse::from);
        }
    }

    Ok(local)
}

#[tauri::command]
pub async fn set_fan_mode(mode: FanMode, speed_percent: u8) -> Result<(), ErrorResponse> {
    elev_bridge::run_elevated(
        "set_fan_mode",
        serde_json::json!({ "mode": mode, "speed_percent": speed_percent }),
    )
    .await
    .map(|_| ())
    .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn get_touchpad_info() -> Result<TouchpadInfo, ErrorResponse> {
    let started = std::time::Instant::now();
    log::debug!(target: "cmd::system", "get_touchpad_info: start");
    let result = run_blocking(hw_get_touchpad)
        .await
        .map_err(ErrorResponse::from);
    match &result {
        Ok(info) => log::debug!(
            target: "cmd::system",
            "get_touchpad_info: ok sensitivity={:?} haptics={} gesture_screenshot={} repress={} edge_slide={} elapsed_ms={}",
            info.sensitivity,
            info.haptics_enabled,
            info.gesture_screenshot,
            info.trackpad_repress,
            info.edge_slide,
            started.elapsed().as_millis()
        ),
        Err(error) => log::warn!(
            target: "cmd::system",
            "get_touchpad_info: failed after {} ms: {}",
            started.elapsed().as_millis(),
            error.message
        ),
    }
    result
}

#[tauri::command]
pub async fn set_touchpad_sensitivity(
    sensitivity: TouchpadSensitivity,
) -> Result<(), ErrorResponse> {
    run_blocking(move || hw_set_touchpad_sensitivity(sensitivity))
        .await
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn set_touchpad_haptics(enabled: bool) -> Result<(), ErrorResponse> {
    run_blocking(move || hw_set_touchpad_haptics(enabled))
        .await
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn set_touchpad_haptics_intensity(
    intensity: crate::hw::touchpad::HapticsIntensity,
) -> Result<(), ErrorResponse> {
    run_blocking(move || hw_set_touchpad_haptics_intensity(intensity))
        .await
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn set_touchpad_gesture_screenshot(enabled: bool) -> Result<(), ErrorResponse> {
    run_blocking(move || hw_set_touchpad_gesture_screenshot(enabled))
        .await
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn set_touchpad_repress(enabled: bool) -> Result<(), ErrorResponse> {
    run_blocking(move || hw_set_touchpad_repress(enabled))
        .await
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn set_touchpad_edge_slide(enabled: bool) -> Result<(), ErrorResponse> {
    run_blocking(move || hw_set_touchpad_edge_slide(enabled))
        .await
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn get_system_info() -> Result<SystemInfo, ErrorResponse> {
    run_blocking(hw_get_sysinfo)
        .await
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn get_process_list() -> Result<Vec<ProcessInfo>, ErrorResponse> {
    run_blocking(move || Ok(hw_get_processes()))
        .await
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn get_available_refresh_rates() -> Result<Vec<u32>, ErrorResponse> {
    // S25-009: Propagate errors instead of silently returning empty vec.
    run_blocking(hw_get_refresh_rates)
        .await
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn set_refresh_rate(hz: u32) -> Result<(), ErrorResponse> {
    elev_bridge::run_elevated("set_refresh_rate", serde_json::json!({ "hz": hz }))
        .await
        .map(|_| ())
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn set_adaptive_refresh_rate(enabled: bool) -> Result<(), ErrorResponse> {
    // Writes HKLM registry key — requires elevation.
    // The UI should inform the user that a driver restart / reboot is needed.
    elev_bridge::run_elevated(
        "set_adaptive_refresh_rate",
        serde_json::json!({ "enabled": enabled }),
    )
    .await
    .map(|_| ())
    .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn get_autostart() -> Result<bool, ErrorResponse> {
    run_blocking(hw_get_autostart)
        .await
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn set_autostart(enabled: bool) -> Result<(), ErrorResponse> {
    run_blocking(move || hw_set_autostart(enabled))
        .await
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn get_update_status() -> Result<UpdateStatus, ErrorResponse> {
    run_blocking(hw_get_update_status)
        .await
        .map_err(ErrorResponse::from)
}

/// Trigger a driver scan (pnputil /scan-devices).
///
/// `pnputil` requires administrator rights, so this runs through the
/// elevated bridge (autonomous service → scheduled task → UAC last resort)
/// exactly like `install_driver`. Previously it invoked the scan directly in
/// the app process, which failed as a normal user (access denied) and
/// surfaced the raw error in the UI.
#[tauri::command]
pub async fn trigger_driver_scan() -> Result<String, ErrorResponse> {
    let raw = elev_bridge::run_elevated("trigger_driver_scan", serde_json::json!({})).await?;
    Ok(raw.as_str().unwrap_or("scan triggered").to_string())
}

// ── Hardware Discovery (Phase 10) ────────────────────────────────────────────

#[tauri::command]
pub async fn get_hardware_profile() -> Option<HardwareProfile> {
    // A-L06: Log when profile is None so missing hardware discovery is visible.
    let profile = global_profile();
    if profile.is_none() {
        log::warn!(
            "get_hardware_profile: hardware profile not available (discovery not initialized)"
        );
    }
    profile
}

#[tauri::command]
pub async fn run_hardware_discovery() -> Result<HardwareProfile, ErrorResponse> {
    // No UAC: discovery runs at startup (auto-discovery when no cached
    // profile exists) and must never pop a consent prompt.
    let raw =
        elev_bridge::run_elevated_no_prompt("run_hardware_discovery", serde_json::Value::Null)
            .await?;
    serde_json::from_value(raw)
        .map_err(|e| ErrorResponse::from(anyhow::anyhow!("Unexpected profile result: {e}")))
}

/// Install a specific driver by name.  The bundled .inf must exist in resources.
/// Runs through the elevated scheduled task (no UAC prompt during install).
#[tauri::command]
pub async fn install_driver(driver_name: String) -> Result<String, ErrorResponse> {
    let raw = elev_bridge::run_elevated(
        "install_driver",
        serde_json::json!({ "driver_name": driver_name }),
    )
    .await?;
    Ok(raw.as_str().unwrap_or("installed").to_string())
}

/// Read raw ECRAM bytes for debugging.
/// Returns a hex dump string (one line per 16 bytes) of the EC's known data ranges.
/// Use this to identify which byte offset corresponds to charger wattage:
/// plug/unplug the charger and call this command to see which bytes change.
#[tauri::command]
pub async fn debug_ecram_dump() -> Result<String, ErrorResponse> {
    crate::hw::ecram::debug_ecram_hex().map_err(ErrorResponse::from)
}

// ── Batched hardware state (S4-002) ──────────────────────────────────────────

/// Consolidated snapshot of all polled hardware properties returned in a single
/// IPC call. Each field is `Option<T>` so partial failures don't block the whole batch.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HardwareState {
    pub battery: Option<BatteryInfo>,
    pub display: Option<DisplayInfo>,
    pub fan: Option<FanInfo>,
    pub touchpad: Option<TouchpadInfo>,
    pub system_info: Option<SystemInfo>,
    pub performance_mode: Option<PerformanceMode>,
    pub charging_threshold: Option<u8>,
    pub audio: Option<AudioVolumeResult>,
}

/// Poll all hardware state at once with parallel queries via rayon.
///
/// `rayon::join` runs closures in parallel using the global rayon thread pool.
/// Since WMI connections are thread-local (see `wmi_cache`), each closure
/// lazily creates its own WMI connection on the first query, making shared
/// rayon threads safe for concurrent WMI access.
///
/// Each subsystem query is wrapped in `ok()` so a transient WMI/pipe failure
/// on one sensor doesn't prevent the rest from returning.
#[tauri::command]
pub async fn get_hardware_state_batch() -> Result<HardwareState, ErrorResponse> {
    run_blocking(|| {
        // Wave 1: battery, display, fan, touchpad in parallel
        let ((battery, display), (fan, touchpad)) = rayon::join(
            || rayon::join(|| hw_get_battery().ok(), || hw_get_display().ok()),
            || rayon::join(|| hw_get_fan().ok(), || hw_get_touchpad().ok()),
        );

        // Wave 2: system_info, performance_mode, charging_threshold, audio in parallel
        let ((system_info, performance_mode), (charging_threshold, audio)) = rayon::join(
            || rayon::join(|| hw_get_sysinfo().ok(), || hw_get_perf().ok()),
            || rayon::join(|| hw_get_charge().ok(), || hw_get_audio().ok()),
        );

        Ok(HardwareState {
            battery,
            display,
            fan,
            touchpad,
            system_info,
            performance_mode,
            charging_threshold,
            audio,
        })
    })
    .await
    .map_err(ErrorResponse::from)
}

// ── Eye Protection ──────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_eye_protection(
) -> Result<crate::hw::eye_protection::EyeProtectionStatus, ErrorResponse> {
    run_blocking(crate::hw::eye_protection::get_eye_protection)
        .await
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn set_eye_protection(enabled: bool, intensity: Option<u8>) -> Result<(), ErrorResponse> {
    elev_bridge::run_elevated(
        "set_eye_protection",
        serde_json::json!({ "enabled": enabled, "intensity": intensity }),
    )
    .await?;
    Ok(())
}

// ── OS Turbo ─────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_os_turbo() -> Result<crate::hw::os_turbo::OsTurboStatus, ErrorResponse> {
    run_blocking(crate::hw::os_turbo::get_os_turbo)
        .await
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn set_os_turbo(
    enabled: bool,
) -> Result<crate::hw::os_turbo::OsTurboStatus, ErrorResponse> {
    let raw = elev_bridge::run_elevated("set_os_turbo", serde_json::json!({ "enabled": enabled }))
        .await?;
    let result: crate::hw::os_turbo::OsTurboStatus =
        serde_json::from_value(raw).map_err(|e| format!("Unexpected elevated result: {e}"))?;
    Ok(result)
}

// ── Crash Recovery ───────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_crash_recovery_status(
) -> Result<crate::hw::crash_recovery::CrashRecoveryStatus, ErrorResponse> {
    run_blocking(crate::hw::crash_recovery::init_crash_recovery)
        .await
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn mark_clean_exit() -> Result<(), ErrorResponse> {
    run_blocking(|| {
        crate::hw::crash_recovery::mark_clean_exit();
        Ok::<(), crate::hw::errors::HardwareError>(())
    })
    .await
    .map_err(ErrorResponse::from)
}

// ── Driver Details ───────────────────────────────────────────────────────────

/// Get detailed driver information via Win32_PnPSignedDriver WMI class.
#[tauri::command]
pub async fn get_drivers_detail() -> Result<Vec<crate::hw::update::DriverDetail>, ErrorResponse> {
    run_blocking(crate::hw::update::get_drivers_detail)
        .await
        .map_err(ErrorResponse::from)
}

// ── Official Driver Update Check ──────────────────────────────────────────────

/// Check installed drivers against Xiaomi's official driver portal.
#[tauri::command]
pub async fn check_official_driver_updates(
) -> Result<crate::hw::driver_update::DriverUpdateCheck, ErrorResponse> {
    crate::hw::driver_update::check_driver_updates()
        .await
        .map_err(ErrorResponse::from)
}

/// Fetch the list of official drivers for a specific model code.
#[tauri::command]
pub async fn fetch_official_drivers(
    model_code: Option<String>,
) -> Result<Vec<crate::hw::driver_update::OfficialDriver>, ErrorResponse> {
    let code = match model_code {
        Some(c) => c,
        None => crate::hw::driver_update::detect_model_code().map_err(ErrorResponse::from)?,
    };
    crate::hw::driver_update::fetch_official_drivers(&code)
        .await
        .map_err(ErrorResponse::from)
}

/// Detect the laptop model code (e.g. "TM2424").
#[tauri::command]
pub async fn get_model_code() -> Result<String, ErrorResponse> {
    crate::hw::driver_update::detect_model_code().map_err(ErrorResponse::from)
}

/// Download a driver package from Xiaomi's CDN.
#[tauri::command]
pub async fn download_driver_package(url: String) -> Result<String, ErrorResponse> {
    let path = crate::hw::driver_update::download_driver_package(&url)
        .await
        .map_err(ErrorResponse::from)?;
    Ok(path.to_string_lossy().to_string())
}

// ── Security Scan ────────────────────────────────────────────────────────────

/// Run a quick security scan via Windows Defender.
#[tauri::command]
pub async fn quick_security_scan(
) -> Result<crate::hw::security_scan::SecurityScanResult, ErrorResponse> {
    run_blocking(crate::hw::security_scan::quick_scan)
        .await
        .map_err(ErrorResponse::from)
}

/// Run a full system security scan.
#[tauri::command]
pub async fn full_security_scan(
) -> Result<crate::hw::security_scan::SecurityScanResult, ErrorResponse> {
    run_blocking(crate::hw::security_scan::full_scan)
        .await
        .map_err(ErrorResponse::from)
}

/// Run a custom security scan on a specific path.
#[tauri::command]
pub async fn custom_security_scan(
    path: String,
) -> Result<crate::hw::security_scan::SecurityScanResult, ErrorResponse> {
    run_blocking(move || crate::hw::security_scan::custom_scan(&path))
        .await
        .map_err(ErrorResponse::from)
}

/// Update Windows Defender signatures.
#[tauri::command]
pub async fn update_defender_signatures(
) -> Result<crate::hw::security_scan::SecurityScanResult, ErrorResponse> {
    run_blocking(crate::hw::security_scan::update_signatures)
        .await
        .map_err(ErrorResponse::from)
}

/// Get Windows Defender status.
#[tauri::command]
pub async fn get_defender_status() -> Result<crate::hw::security_scan::DefenderStatus, ErrorResponse>
{
    run_blocking(crate::hw::security_scan::get_defender_status)
        .await
        .map_err(ErrorResponse::from)
}

/// Get threat detection history.
#[tauri::command]
pub async fn get_threat_history() -> Result<crate::hw::security_scan::ThreatHistory, ErrorResponse>
{
    run_blocking(crate::hw::security_scan::get_threat_history)
        .await
        .map_err(ErrorResponse::from)
}

/// Open Windows Security app (Windows Defender).
#[tauri::command]
pub async fn open_windows_security() -> Result<(), ErrorResponse> {
    run_blocking(|| {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            std::process::Command::new("powershell")
                .args(["-NoProfile", "-Command", "Start-Process 'windowsdefender:'"])
                .creation_flags(CREATE_NO_WINDOW)
                .status()
                .map_err(|e| {
                    HardwareError::Other(format!("Failed to open Windows Security: {e}"))
                })?;
        }
        #[cfg(not(windows))]
        {
            return Err(HardwareError::Other(
                "Windows Security is only available on Windows".to_string(),
            ));
        }
        Ok(())
    })
    .await
    .map_err(ErrorResponse::from)
}

// ── Phone Link ────────────────────────────────────────────────────────────────

/// Get Phone Link status.
#[tauri::command]
pub async fn get_phone_link_status() -> Result<crate::hw::phone_link::PhoneLinkStatus, ErrorResponse>
{
    run_blocking(|| Ok(crate::hw::phone_link::get_phone_link_status()))
        .await
        .map_err(ErrorResponse::from)
}

/// Launch Phone Link app.
#[tauri::command]
pub async fn launch_phone_link() -> Result<(), ErrorResponse> {
    run_blocking(crate::hw::phone_link::launch_phone_link)
        .await
        .map_err(ErrorResponse::from)
}

/// Launch a specific Phone Link feature.
#[tauri::command]
pub async fn launch_phone_link_feature(feature: String) -> Result<(), ErrorResponse> {
    run_blocking(move || crate::hw::phone_link::launch_phone_link_feature(&feature))
        .await
        .map_err(ErrorResponse::from)
}

/// Open Phone Link settings in Windows Settings.
#[tauri::command]
pub async fn open_phone_link_settings() -> Result<(), ErrorResponse> {
    run_blocking(crate::hw::phone_link::open_phone_link_settings)
        .await
        .map_err(ErrorResponse::from)
}

// ── Color Calibration ─────────────────────────────────────────────────────────

/// Get color profile information for all displays.
#[tauri::command]
pub async fn get_color_status(
) -> Result<crate::hw::color_calibration::ColorCalibrationStatus, ErrorResponse> {
    run_blocking(crate::hw::color_calibration::get_color_status)
        .await
        .map_err(ErrorResponse::from)
}

/// Load an ICC profile for a display.
#[tauri::command]
pub async fn load_icc_profile(display: String, profile_path: String) -> Result<(), ErrorResponse> {
    run_blocking(move || crate::hw::color_calibration::load_icc_profile(&display, &profile_path))
        .await
        .map_err(ErrorResponse::from)
}

/// Unload ICC profile (revert to sRGB).
#[tauri::command]
pub async fn unload_icc_profile(display: String) -> Result<(), ErrorResponse> {
    run_blocking(move || crate::hw::color_calibration::unload_icc_profile(&display))
        .await
        .map_err(ErrorResponse::from)
}

/// Open Windows Color Management settings.
#[tauri::command]
pub async fn open_color_management_settings() -> Result<(), ErrorResponse> {
    run_blocking(crate::hw::color_calibration::open_color_management_settings)
        .await
        .map_err(ErrorResponse::from)
}

/// Launch Windows Display Color Calibration wizard.
#[tauri::command]
pub async fn launch_color_calibration_wizard() -> Result<(), ErrorResponse> {
    run_blocking(crate::hw::color_calibration::launch_color_calibration_wizard)
        .await
        .map_err(ErrorResponse::from)
}

// ── AI Noise Cancellation ────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_audio_effects(
) -> Result<crate::hw::audio_effects::AudioEffectsStatus, ErrorResponse> {
    run_blocking(crate::hw::audio_effects::get_audio_effects)
        .await
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn set_mic_noise_canceling(enabled: bool) -> Result<(), ErrorResponse> {
    elev_bridge::run_elevated(
        "set_mic_noise_canceling",
        serde_json::json!({ "enabled": enabled }),
    )
    .await?;
    Ok(())
}

#[tauri::command]
pub async fn set_speaker_noise_canceling(enabled: bool) -> Result<(), ErrorResponse> {
    elev_bridge::run_elevated(
        "set_speaker_noise_canceling",
        serde_json::json!({ "enabled": enabled }),
    )
    .await?;
    Ok(())
}

#[tauri::command]
pub async fn set_voice_focus(enabled: bool) -> Result<(), ErrorResponse> {
    elev_bridge::run_elevated("set_voice_focus", serde_json::json!({ "enabled": enabled })).await?;
    Ok(())
}

// ── System Cleanup ───────────────────────────────────────────────────────────

#[tauri::command]
pub async fn scan_junk_files() -> Result<Vec<crate::hw::cleanup::CleanupItem>, ErrorResponse> {
    run_blocking(crate::hw::cleanup::scan_junk_files)
        .await
        .map_err(ErrorResponse::from)
}

#[tauri::command]
pub async fn clean_junk_files(
    categories: Vec<crate::hw::cleanup::CleanupCategory>,
) -> Result<Vec<crate::hw::cleanup::CleanupResult>, ErrorResponse> {
    let result = elev_bridge::run_elevated(
        "clean_junk_files",
        serde_json::json!({ "categories": categories }),
    )
    .await?;
    // The elevated dispatch returns the CleanupResult array as a JSON value
    match serde_json::from_value::<Vec<crate::hw::cleanup::CleanupResult>>(result) {
        Ok(results) => Ok(results),
        Err(_) => Ok(Vec::new()),
    }
}

// ── Error Logging ────────────────────────────────────────────────────────────

/// Get the error logging configuration.
#[tauri::command]
pub async fn get_error_log_config() -> Result<crate::util::error_log::ErrorLogConfig, ErrorResponse>
{
    Ok(crate::util::error_log::get_config())
}

/// Enable or disable error logging.
#[tauri::command]
pub async fn set_error_logging_enabled(enabled: bool) -> Result<(), ErrorResponse> {
    crate::util::error_log::set_enabled(enabled);
    Ok(())
}

/// Read the error log (last N lines).
#[tauri::command]
pub async fn read_error_log(max_lines: Option<usize>) -> Result<String, ErrorResponse> {
    Ok(crate::util::error_log::read_log(max_lines.unwrap_or(500)))
}

/// Clear the error log file.
#[tauri::command]
pub async fn clear_error_log() -> Result<(), ErrorResponse> {
    crate::util::error_log::clear_log();
    Ok(())
}

/// Log a frontend error to the error log file.
#[tauri::command]
pub async fn log_frontend_error(target: String, message: String) -> Result<(), ErrorResponse> {
    crate::util::error_log::log_error(&target, &message);
    Ok(())
}
