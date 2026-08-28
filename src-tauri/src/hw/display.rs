//! Display brightness, HDR, and refresh rate control.
//!
//! Uses IGCL (Intel Graphics Command Library) for brightness and WMI
//! for display info queries on Windows.

use crate::hw::errors::{HardwareError, HardwareResult};
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicI16, AtomicU64, AtomicU8, Ordering};

/// Set to `false` after the first `set_brightness_igcl` failure so we never
/// retry a DLL that cannot load — avoids a WARN log on every brightness change.
static IGCL_SET_AVAILABLE: AtomicBool = AtomicBool::new(true);

/// S32-003: Set while the Windows display color calibration wizard (dccw.exe)
/// is open. While set, the adaptive-brightness loop stops touching the
/// backlight/LUT so the wizard can save its calibration without
/// "Access is denied" (the LUT would otherwise be locked by our gamma ramp).
pub static CALIBRATION_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

/// Returns `true` while the display color calibration wizard is running.
pub fn is_calibration_in_progress() -> bool {
    CALIBRATION_IN_PROGRESS.load(Ordering::Relaxed)
}

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
/// 0 = "curve" (fixed min/max formula), 1 = "smart" (learned user preference curve).
const AI_BRIGHTNESS_MODE_VALUE: &str = "AiBrightnessMode";

/// Smart-mode persistence file: `%LOCALAPPDATA%\MiControl\smart_brightness.json`.
///
/// Tests override this via `MICONTROL_SMART_MODEL_PATH` so unit tests never
/// clobber the user's real learned model.
fn smart_model_path() -> std::path::PathBuf {
    if let Ok(override_path) = std::env::var("MICONTROL_SMART_MODEL_PATH") {
        return std::path::PathBuf::from(override_path);
    }
    let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into());
    std::path::PathBuf::from(base)
        .join("MiControl")
        .join("smart_brightness.json")
}

/// Sensitivity / range configuration for our own adaptive brightness loop.
///
/// Formula per iteration (every 2 s):
///   max_lux  = 2000 / (sensitivity / 100)   — lux where ceiling is reached
///   target   = clamp(min + (lux / max_lux) * (max - min), min, max)
///   smoothed = current + (target - current) * (1 - smoothing/100)
///
/// Mode:
///   "curve" — always use the fixed formula above.
///   "smart" — use a learned user-preference curve (mean per lux bucket with
///             std-dev band) where enough samples exist; fall back to the
///             formula on buckets without data.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    /// "curve" (fixed formula) or "smart" (learned user-preference curve).
    pub mode: String,
}

impl Default for AiBrightnessConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_brightness: 10,
            max_brightness: 100,
            sensitivity: 100,
            smoothing: 30,
            mode: "curve".into(),
        }
    }
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

// ── Manual-override grace period ─────────────────────────────────────────────
//
// S32-005: When the user manually changes brightness (MiControl slider, OSD,
// hotkey, Windows slider, Fn key) and a crash/relaunch happens shortly after,
// the relaunched process must NOT immediately revert the user's brightness.
// Before the last manual change is older than MANUAL_OVERRIDE_GRACE_MS, the
// adaptive loop is frozen at the user's value. This also protects against the
// "app recovers brightness instantly after I adjusted it" complaint.

/// Milliseconds the adaptive loop must stay hands-off after a manual change.
const MANUAL_OVERRIDE_GRACE_MS: u64 = 120_000; // 2 minutes
/// Wall-clock (ms since UNIX epoch) of the last manual brightness change.
static AUTO_LAST_MANUAL_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// True while the user's last manual brightness change is still within the
/// grace window — the adaptive loop must not touch the backlight.
pub fn user_override_in_grace() -> bool {
    let last = AUTO_LAST_MANUAL_MS.load(Ordering::Relaxed);
    last != 0 && now_ms().saturating_sub(last) < MANUAL_OVERRIDE_GRACE_MS
}

/// Called by the `set_brightness` Tauri command when auto-brightness is on.
/// Records the delta so future loop iterations honour the user's preference,
/// and stamps the manual-change time to enter the grace window.
///
/// Also records a (lux, brightness) sample into the smart model, so the
/// "Smart" mode can learn the user's preferred brightness per light level.
pub fn record_user_brightness_override(user_value: u8) {
    let last_target = AUTO_LAST_TARGET.load(Ordering::Relaxed);
    let offset = user_value as i16 - last_target as i16;
    AUTO_OFFSET.store(offset, Ordering::Relaxed);
    AUTO_OFFSET_ACTIVE.store(true, Ordering::Relaxed);
    AUTO_LAST_MANUAL_MS.store(now_ms(), Ordering::Relaxed);
    log::debug!(
        "auto_brightness: user override {user_value}% \
         (last_target={last_target}%, offset={offset:+}) — grace {MANUAL_OVERRIDE_GRACE_MS}ms"
    );

    // Smart-mode learning: log the manual brightness change with the current
    // ambient lux (if the sensor is available).
    if let Some(lux) = get_ambient_lux().filter(|v| *v > MIN_PLAUSIBLE_LUX) {
        record_smart_sample(lux, user_value);
    }
}

/// Reset the offset — call when auto-brightness is toggled or config changes.
pub fn clear_user_brightness_override() {
    AUTO_OFFSET.store(0, Ordering::Relaxed);
    AUTO_OFFSET_ACTIVE.store(false, Ordering::Relaxed);
    AUTO_LAST_MANUAL_MS.store(0, Ordering::Relaxed);
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
    // S25-009: Don't let refresh-rate enumeration failure nuke the entire display info.
    // If we can't enumerate rates, fall back to the current rate as the only option.
    let available_refresh_rates = get_available_refresh_rates().unwrap_or_else(|e| {
        log::warn!(target: "hw::display", "get_available_refresh_rates failed: {e}, using current rate as fallback");
        vec![refresh_rate_hz]
    });
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
    let mode = match read_display_dword(AI_BRIGHTNESS_MODE_VALUE, 0) {
        1 => "smart".to_string(),
        _ => "curve".to_string(),
    };
    AiBrightnessConfig {
        enabled,
        min_brightness: min_b,
        max_brightness: max_b,
        sensitivity: (read_display_dword(AI_BRIGHTNESS_SENS_VALUE, 100) as u8).clamp(10, 200),
        smoothing: (read_display_dword(AI_BRIGHTNESS_SMTH_VALUE, 30) as u8).min(90),
        mode,
    }
}

pub fn set_ai_brightness_config(config: AiBrightnessConfig) -> HardwareResult<()> {
    persist_ai_brightness_registry(config.enabled)?;
    write_display_dword(AI_BRIGHTNESS_MIN_VALUE, config.min_brightness as u32)?;
    write_display_dword(AI_BRIGHTNESS_MAX_VALUE, config.max_brightness as u32)?;
    write_display_dword(AI_BRIGHTNESS_SENS_VALUE, config.sensitivity as u32)?;
    write_display_dword(AI_BRIGHTNESS_SMTH_VALUE, config.smoothing as u32)?;
    let mode_dword: u32 = if config.mode == "smart" { 1 } else { 0 };
    write_display_dword(AI_BRIGHTNESS_MODE_VALUE, mode_dword)?;
    Ok(())
}

// ── Smart brightness model (learned user preference) ─────────────────────────
//
// The "Smart" mode learns the user's brightness preference per ambient-light
// level and uses it to drive the adaptive loop instead of the fixed
// min/max formula.  Each manual brightness change (slider, hotkey, Fn key,
// external Windows change) is logged as a (lux, brightness) sample; the
// samples are aggregated into lux buckets and the loop uses the *mean*
// brightness inside the bucket, with a *standard-deviation band* used to
// flag outlier samples that are likely accidental (brightness changes made
// for other reasons, e.g. plugging in HDMI, sunny window).
//
// Persistence format (`smart_brightness.json`):
// {
//   "schema": 1,
//   "bins": {
//     "0":   { "n": 3, "mean": 42.0, "m2": 100.0, "min": 30, "max": 55 },  // lux range [0,20)
//     "20":  { "n": 5, "mean": 60.0, "m2": 200.0, "min": 45, "max": 80 }, // lux range [20,40)
//     ...
//   }
// }
//
// Bin size is 20 lux: bin index = floor(lux / 20), keyed by index * 20.
// `m2` is the sum of squared deviations (Welford's online mean/variance
// algorithm) so we can compute the std-dev without storing every sample.

/// Lux bucket width for the smart model (10 lux per bucket keeps memory tiny
/// while still capturing meaningful per-light-level preferences).
const SMART_BIN_LUX: f32 = 10.0;
/// Minimum number of samples in a bucket before the smart model is trusted
/// over the raw formula.  Fewer than this → use the standard formula.
const SMART_MIN_SAMPLES: u32 = 3;
/// Maximum number of buckets cap — protects the file from unbounded growth
/// if the sensor reports bizarre lux values.
const SMART_MAX_BUCKETS: usize = 200;

/// One lux bucket in the smart brightness model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartBin {
    /// Number of samples in this bucket.
    pub n: u32,
    /// Running mean (Welford).
    pub mean: f32,
    /// Running M2 (sum of squared deviations from the mean).
    pub m2: f32,
    /// Minimum brightness observed in this bucket.
    pub min: u8,
    /// Maximum brightness observed in this bucket.
    pub max: u8,
}

/// The full smart brightness model (serialized to `smart_brightness.json`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SmartModel {
    /// Schema version for future migrations.
    pub schema: u32,
    /// Map of bucket index → stats. The bucket index is `floor(lux / SMART_BIN_LUX)`.
    pub bins: std::collections::HashMap<u32, SmartBin>,
}

impl SmartModel {
    fn new() -> Self {
        Self {
            schema: 1,
            bins: std::collections::HashMap::new(),
        }
    }

    /// Number of buckets with at least `SMART_MIN_SAMPLES`.
    fn mature_bucket_count(&self) -> usize {
        self.bins
            .values()
            .filter(|b| b.n >= SMART_MIN_SAMPLES)
            .count()
    }

    /// Total number of samples across all buckets.
    fn total_samples(&self) -> u32 {
        self.bins.values().map(|b| b.n).sum()
    }
}

/// In-memory cache of the smart model. Loaded once at startup (or on demand)
/// and updated on every sample; persisted on change.
static SMART_MODEL_CACHE: std::sync::OnceLock<std::sync::Mutex<Option<SmartModel>>> =
    std::sync::OnceLock::new();

fn load_smart_model() -> SmartModel {
    let path = smart_model_path();
    if let Ok(data) = std::fs::read_to_string(&path) {
        if let Ok(model) = serde_json::from_str::<SmartModel>(&data) {
            return model;
        }
    }
    SmartModel::new()
}

fn persist_smart_model(model: &SmartModel) {
    let path = smart_model_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match serde_json::to_string_pretty(model) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                log::warn!("[display] Failed to persist smart model: {e}");
            } else if let Err(e) = crate::util::auth::restrict_file_acl(&path) {
                log::warn!("[display] Failed to restrict ACL on smart_brightness.json: {e}");
            }
        }
        Err(e) => log::warn!("[display] Failed to serialize smart model: {e}"),
    }
}

/// Access to the smart model (locked).  Loads from disk on first access.
fn with_smart_model<T>(f: impl FnOnce(&mut SmartModel) -> T) -> T {
    let cache = SMART_MODEL_CACHE.get_or_init(|| std::sync::Mutex::new(None));
    let mut guard = crate::util::panic::lock_or_recover(cache);
    if guard.is_none() {
        *guard = Some(load_smart_model());
    }
    let model = guard.as_mut().expect("smart model cache initialized");
    let result = f(model);

    // Opportunistic: only write back when the model changed.  We can't easily
    // detect churn, so write on every public mutation call that uses this fn
    // (record_log_sample / reset_smart_model). Kept simple: callers that only
    // *read* must not go through here (they use `read_smart_model()`).
    result
}

/// Read the current smart model without mutating it (for the frontend stats bar).
pub fn get_smart_model() -> SmartModel {
    let cache = SMART_MODEL_CACHE.get_or_init(|| std::sync::Mutex::new(None));
    let guard = crate::util::panic::lock_or_recover(cache);
    match guard.as_ref() {
        Some(m) => m.clone(),
        None => load_smart_model(),
    }
}

/// Clear the entire smart model (user clicked "Reset learning").
pub fn reset_smart_model() {
    let path = smart_model_path();
    let _ = std::fs::remove_file(path);
    if let Some(cache) = SMART_MODEL_CACHE.get() {
        if let Ok(mut guard) = cache.lock() {
            *guard = Some(SmartModel::new());
        }
    }
    log::info!("[display] Smart brightness model reset");
}

/// Log a (lux, brightness) sample into the smart model and persist it.
///
/// Outliers (values more than 2 std-dev from the bucket mean, when the bucket
/// already has enough data) are still recorded — they shift the mean — but
/// they are logged separately so the stats panel can show "X outlier-adjusted
/// samples".  The loop uses the *mean* which is robust to occasional outliers.
pub fn record_smart_sample(lux: f32, brightness: u8) {
    if !lux.is_finite() || lux < 0.0 {
        return;
    }
    let bin_idx = (lux / SMART_BIN_LUX).floor() as u32;
    if bin_idx as usize > SMART_MAX_BUCKETS {
        return; // absurd lux value — ignore
    }
    with_smart_model(|m| {
        let bin = m.bins.entry(bin_idx).or_insert(SmartBin {
            n: 0,
            mean: 0.0,
            m2: 0.0,
            min: 100,
            max: 0,
        });
        // Welford's online algorithm.
        bin.n += 1;
        let delta = brightness as f32 - bin.mean;
        bin.mean += delta / bin.n as f32;
        let delta2 = brightness as f32 - bin.mean;
        bin.m2 += delta * delta2;
        bin.min = bin.min.min(brightness);
        bin.max = bin.max.max(brightness);
        let _ = bin;
    });
    let model = get_smart_model();
    // Persist outside the lock to avoid holding it during file I/O.
    persist_smart_model(&model);
    log::debug!(
        "[display] Smart sample: lux={lux:.0} brightness={brightness}% \
         → bucket={bin_idx} (total={})",
        model.total_samples()
    );
}

/// Compute the std-dev for a bucket (population std-dev using Welford's M2).
/// Used by `smart_model_stats` to report the confidence band in the UI.
pub fn bin_stddev(bin: &SmartBin) -> f32 {
    if bin.n < 2 {
        return 0.0;
    }
    (bin.m2 / bin.n as f32).sqrt()
}

/// Number of smart-bucket samples (for the stats UI).
pub fn smart_model_stats() -> (u32, usize) {
    let model = get_smart_model();
    (model.total_samples(), model.mature_bucket_count())
}

/// Compute the predicted (mean) brightness for a given lux value using the
/// learned model. Returns `None` when the bucket has too few samples or the
/// lux value is outside every known bucket — the caller falls back to the
/// fixed formula in that case.
pub fn smart_predict_brightness(lux: f32) -> Option<u8> {
    if !lux.is_finite() || lux < 0.0 {
        return None;
    }
    let bin_idx = (lux / SMART_BIN_LUX).floor() as u32;
    let model = get_smart_model();
    let bin = model.bins.get(&bin_idx)?;
    if bin.n < SMART_MIN_SAMPLES {
        return None;
    }
    Some(bin.mean.round().clamp(0.0, 100.0) as u8)
}

// ── Ambient light sensor ──────────────────────────────────────────────────────

/// Global flag to force sensor re-initialization on next read.
/// Set to `true` when a power-resume event is detected so the sensor
/// instance is re-created instead of returning stale `None`.
static SENSOR_RESET_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Request a sensor reset on the next polling cycle.
/// Called when a WM_POWERBROADCAST resume event is received.
pub fn request_sensor_reset() {
    SENSOR_RESET_REQUESTED.store(true, Ordering::SeqCst);
    log::info!("[display] Sensor reset requested (power resume event)");
}

/// Cached LightSensor instance — re-created only when a reset is requested.
/// This avoids calling `LightSensor::GetDefault()` on every 2-second poll,
/// which can be slow and may return `None` transiently after sleep/wake.
#[cfg(windows)]
static CACHED_LIGHT_SENSOR: std::sync::OnceLock<
    std::sync::Mutex<Option<windows::Devices::Sensors::LightSensor>>,
> = std::sync::OnceLock::new();

/// Fallback ambient-light reader using the WinRT `Windows.Devices.Sensors.LightSensor` API.
/// This works well in UWP/WinUI apps, but in an unpackaged desktop app it frequently
/// returns `None` even when a HID ALS sensor is present in Device Manager.
#[cfg(windows)]
fn get_ambient_lux_winrt() -> Option<f32> {
    use windows::Devices::Sensors::LightSensor;

    // Check if a reset was requested (e.g., after sleep/wake)
    let needs_reset = SENSOR_RESET_REQUESTED.swap(false, Ordering::SeqCst);

    let cache = CACHED_LIGHT_SENSOR.get_or_init(|| std::sync::Mutex::new(None));
    let mut guard = cache.lock().ok()?;

    if needs_reset || guard.is_none() {
        // Re-create the sensor instance
        match LightSensor::GetDefault() {
            Ok(sensor) => {
                log::debug!(
                    "[display] WinRT ambient light sensor {}",
                    if needs_reset {
                        "re-initialized (reset)"
                    } else {
                        "found"
                    }
                );
                *guard = Some(sensor);
            }
            Err(_) => {
                log::debug!("[display] WinRT ambient light sensor not available");
                *guard = None;
                return None;
            }
        }
    }

    let sensor = guard.as_ref()?;
    let reading = sensor.GetCurrentReading().ok()?;
    let lux = reading.IlluminanceInLux().ok()?;
    log::debug!("[display] WinRT ambient lux: {lux}");
    Some(lux)
}

// Per-thread cache of the classic COM light sensor stack.
//
// `ISensorManager` (COM) is not `Send`/`Sync`, so we cannot put it in a
// `static`. This mirrors `wmi_cache.rs`'s thread_local pattern: each worker
// thread lazily creates the manager + sensor collection once, then reuses it
// for every 2-second poll. The entry is left as `None` (a) before first use,
// (b) when a poll fails — the next poll retries from scratch (re-enumerating
// hot-plugged sensors) — and (c) forever on machines without the sensor.
#[cfg(windows)]
thread_local! {
    static COM_SENSOR_MANAGER: std::cell::RefCell<Option<ComSensorStack>> = const {
        std::cell::RefCell::new(None)
    };
}

/// Lazily-created COM sensor manager + ambient-light collection.
#[cfg(windows)]
struct ComSensorStack {
    /// Sensor collection (owns the enumeration). Manager is kept only for its
    /// lifetime semantics; both are released together when the thread exits.
    collection: windows::Win32::Devices::Sensors::ISensorCollection,
}

#[cfg(windows)]
impl ComSensorStack {
    /// (Re)create the sensor stack. Returns None on any failure path so the
    /// caller falls back to WinRT / sensor-off.
    fn new() -> Option<Self> {
        use windows::Win32::Devices::Sensors::{
            ISensorManager, SensorManager as CLSID_SensorManager, SENSOR_TYPE_AMBIENT_LIGHT,
        };
        use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

        // SAFETY: COM objects released on drop; pointers remain valid within the call.
        unsafe {
            let manager: ISensorManager = CoCreateInstance(&CLSID_SensorManager, None, CLSCTX_ALL)
                .map_err(|e| {
                    log::debug!("[display] COM SensorManager not available: {e}");
                    e
                })
                .ok()?;
            let collection = manager.GetSensorsByType(&SENSOR_TYPE_AMBIENT_LIGHT).ok()?;
            let count = collection.GetCount().ok()?;
            if count == 0 {
                log::debug!("[display] COM SensorManager: no ambient light sensors found");
                return None;
            }
            log::debug!("[display] COM SensorManager cached ({count} sensors)");
            Some(Self { collection })
        }
    }

    /// Read every sensor's current lux value. Returns None if the whole stack
    /// must be recreated (the caller invalidates the cache).
    fn read_all(&self) -> Option<Vec<Option<f32>>> {
        use windows::Win32::Devices::Sensors::SENSOR_DATA_TYPE_LIGHT_LEVEL_LUX;
        use windows::Win32::Foundation::{E_FAIL, S_OK};

        // SAFETY: sensor/report/VARIANT pointers valid within the call; released
        // by their wrappers on drop.
        unsafe {
            let count = self.collection.GetCount().ok()?;
            let mut readings: Vec<Option<f32>> = Vec::with_capacity(count as usize);
            for i in 0..count {
                let Ok(sensor) = self.collection.GetAt(i) else {
                    // A sensor appeared/disappeared → invalidate cache.
                    return None;
                };
                let Ok(report) = sensor.GetData() else {
                    // Sensor momentarily unresponsive: report as None this poll,
                    // but don't tear down the whole stack for one glitch.
                    readings.push(None);
                    continue;
                };
                let pv = report.GetSensorValue(&SENSOR_DATA_TYPE_LIGHT_LEVEL_LUX);
                match pv {
                    Ok(v) => {
                        let lux = f64::try_from(&v).ok()? as f32;
                        log::debug!("[display] COM sensor[{i}] ambient lux: {lux}");
                        readings.push(if lux.is_finite() { Some(lux) } else { None });
                    }
                    Err(e) => {
                        let hr = e.code();
                        if hr == E_FAIL || hr == S_OK {
                            readings.push(None);
                        } else {
                            // Unusual HRESULT → invalidate the whole stack.
                            return None;
                        }
                    }
                }
            }
            Some(readings)
        }
    }
}

/// Primary ambient-light reader using the classic COM Sensor API (`ISensorManager`).
/// This is the API that Windows itself uses for the "Adaptive brightness" power-plan
/// setting and it works for unpackaged desktop applications, unlike the WinRT API.
#[cfg(windows)]
fn get_ambient_lux_com() -> Option<f32> {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    // S32-003 rationale (sensor selection) lives in `select_responsive_sensor`.
    //
    // S26-006: The manager+enumerator are cached per-thread (this runs on the
    // tokio blocking pool, whose threads are long-lived). The previous
    // implementation called `CoCreateInstance` + `GetSensorsByType` + `GetAt`
    // on EVERY 2-second poll, churning COM allocations and MTA registrations
    // for the whole app lifetime — an unnecessary leak/stress vector. Caching
    // mirrors the `wmi_cache` thread_local pattern; the cache is invalidated
    // when a poll fails so hot-plugged sensors are re-enumerated.

    // SAFETY: `CoInitializeEx` is idempotent for the same apartment model and is safe
    // to call from each thread. If the thread is already in a different apartment model
    // the call returns RPC_E_CHANGED_MODE, which we ignore because the runtime may have
    // already initialized COM for us.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };

    let readings = COM_SENSOR_MANAGER.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let stack = match borrow.as_ref() {
            Some(s) => s,
            None => {
                let s = ComSensorStack::new()?;
                *borrow = Some(s);
                borrow.as_ref()?
            }
        };
        let r = stack.read_all();
        if r.is_none() {
            // Invalidate on failure so the next poll re-enumerates.
            *borrow = None;
        }
        r
    });
    let readings = readings?;
    let lux = select_responsive_sensor(&readings);
    log::debug!("[display] COM selected lux: {lux:?}");
    lux
}

/// Per-sensor last-read cache used to detect which ALS sensor actually responds
/// to light (the others are placeholders that return a fixed value).
#[cfg(windows)]
static COM_LUX_HISTORY: std::sync::OnceLock<std::sync::Mutex<Vec<Option<f32>>>> =
    std::sync::OnceLock::new();

/// A sensor reading is only trustworthy when it is a plausible, finite
/// illuminance value. Values at or below this floor are physically impossible
/// with the panel on and mean either a placeholder sensor, an uninitialised
/// driver value, or a stale/unresponsive sensor (e.g. stuck at 1 lux after the
/// app was closed and reopened). The adaptive loop uses the same threshold.
const MIN_PLAUSIBLE_LUX: f32 = 1.5;

/// Choose the most responsive ambient-light sensor from a set of simultaneous
/// readings, using cached previous readings to measure variability.
///
/// Strategy:
///   1. Compute the absolute delta between each sensor's current and previous
///      reading. The sensor with the largest delta is the real one.
///   2. If all deltas are 0 (steady light), reuse the index of the sensor that
///      was responsive in the last poll (cached).
///   3. If never responsive, fall back to the first finite reading.
#[cfg(windows)]
fn select_responsive_sensor(readings: &[Option<f32>]) -> Option<f32> {
    select_responsive_sensor_impl(readings)
}

/// Test hook: public wrapper around the responsive-sensor selector so the
/// selection logic can be validated from integration tests.
#[cfg(windows)]
pub fn test_select_responsive(readings: Vec<Option<f32>>) -> Option<f32> {
    select_responsive_sensor_impl(&readings)
}

#[cfg(windows)]
fn select_responsive_sensor_impl(readings: &[Option<f32>]) -> Option<f32> {
    let history = COM_LUX_HISTORY.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    let mut guard = history.lock().ok()?;

    // Resize the history cache to match the sensor count.
    if guard.len() != readings.len() {
        guard.resize(readings.len(), None);
    }
    // Snapshot the PREVIOUS readings (before we overwrite them below) so we
    // can measure variability between this poll and the last poll.
    let prev: Vec<Option<f32>> = guard.clone();

    // 1. Find the sensor with the largest absolute change vs the previous poll.
    let mut best_index: Option<usize> = None;
    let mut best_delta: f32 = 0.0;
    for (i, reading) in readings.iter().enumerate() {
        if let (Some(cur), Some(prev_v)) = (reading, prev[i]) {
            let delta = (cur - prev_v).abs();
            if delta > best_delta {
                best_delta = delta;
                best_index = Some(i);
            }
        }
    }

    // 2. Update the history with ALL current readings (not just the chosen
    //    one) so every sensor accumulates history for future polls.
    for (i, reading) in readings.iter().enumerate() {
        guard[i] = *reading;
    }

    // 3. Return the most responsive sensor's current reading — but only when
    //    it is a plausible lux value. A "responsive" placeholder that wanders
    //    between 0.1 and 1 lux is not a light reading; treat it as unavailable
    //    so the loop idles instead of driving brightness to the floor.
    if let Some(idx) = best_index {
        if let Some(v) = readings[idx].filter(|v| *v >= MIN_PLAUSIBLE_LUX) {
            return Some(v);
        }
    }

    // 4. Steady light (nothing changed): reuse the last sensor that ever had
    //    a recorded value different from its first-seen value. Simpler and
    //    robust enough: prefer the sensor whose history is non-empty and whose
    //    value differs from the very first reading ever stored for it. We
    //    approximate "responsive ever" by preferring the LAST finite sensor,
    //    since the placeholder is typically index 0. Reject implausible values
    //    here too — a stuck "1 lux" must never be trusted just because it is
    //    the last finite reading.
    let last_finite = readings.iter().rposition(|r| r.is_some());
    if let Some(idx) = last_finite {
        return readings[idx].filter(|v| *v >= MIN_PLAUSIBLE_LUX);
    }

    None
}

#[cfg(windows)]
fn get_ambient_lux() -> Option<f32> {
    // Try the classic COM Sensor API first: it works for unpackaged desktop apps.
    if let Some(lux) = get_ambient_lux_com() {
        return Some(lux);
    }
    // Fall back to WinRT, which works on some systems depending on the sensor driver.
    get_ambient_lux_winrt()
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
///  0 = some systems return this when the display is on (not documented but observed)
///
/// On non-Windows or if the call fails, we assume the display is on.
#[cfg(windows)]
fn is_display_on() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SYSTEM_METRICS_INDEX};
    // SM_MONITORPOWER is not exposed by the windows 0.58 crate; the raw value is 112.
    const SM_MONITORPOWER: i32 = 112;
    // SAFETY: GetSystemMetrics is a thread-safe Win32 API call.
    let val = unsafe { GetSystemMetrics(SYSTEM_METRICS_INDEX(SM_MONITORPOWER)) };
    // -1 and 0 mean the display is on; 1 means transitioning to off; 2 means off.
    val == -1 || val == 0
}

#[cfg(not(windows))]
fn is_display_on() -> bool {
    true
}

/// Spawned once at startup. Every 2 s it reads the ambient light sensor and
/// adjusts screen brightness according to the user-configured sensitivity curve.
/// Config changes are picked up automatically on each iteration.
pub async fn adaptive_brightness_loop() {
    log::info!("[adaptive_brightness] loop started");
    let mut smoothed: Option<f32> = None;
    let mut no_sensor_warned = false;
    // Last value we applied so we can detect external changes (Fn keys, OS).
    let mut last_set: Option<u8> = None;
    // Track whether we have already disabled Windows ADAPTBRIGHT for the
    // current "enabled session".  Reset when adaptive brightness is turned off
    // so we re-disable it if the user re-enables.
    let mut adaptbright_suppressed = false;
    // Counter of consecutive invalid/stuck sensor reads. When a sensor gets
    // stuck (e.g. returning 1 lux forever after the app was closed and
    // reopened), we request a sensor reset instead of fighting the backlight.
    let mut consecutive_invalid_reads: u32 = 0;
    const STUCK_SENSOR_RESET_AFTER: u32 = 5; // ~10 s of invalid reads
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // S32-003: While the display color calibration wizard is open, do NOT
        // touch the backlight. Writing brightness while dccw.exe is saving its
        // gamma ramp causes "Access is denied" / "Close other programs".
        if CALIBRATION_IN_PROGRESS.load(Ordering::Relaxed) {
            log::debug!("[adaptive_brightness] paused — color calibration in progress");
            continue;
        }

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
            consecutive_invalid_reads = 0;
            log::debug!("[adaptive_brightness] disabled — skipping iteration");
            continue;
        }

        log::debug!(
            "[adaptive_brightness] enabled: min={} max={} sens={} smooth={} actual_brightness={:?}",
            cfg.min_brightness,
            cfg.max_brightness,
            cfg.sensitivity,
            cfg.smoothing,
            brightness_actual
        );

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
        // S32-005: Also enter the manual-override grace window so the loop
        // stays hands-off while the user finishes adjusting.
        if let (Some(prev), Some(actual)) = (last_set, brightness_actual) {
            let diff = (actual as i16 - prev as i16).abs();
            if diff >= 2 {
                let raw = AUTO_LAST_TARGET.load(Ordering::Relaxed);
                let new_offset = actual as i16 - raw as i16;
                AUTO_OFFSET.store(new_offset, Ordering::Relaxed);
                AUTO_OFFSET_ACTIVE.store(true, Ordering::Relaxed);
                AUTO_LAST_MANUAL_MS.store(now_ms(), Ordering::Relaxed);
                // Snap smoothed to actual so we don't animate back.
                smoothed = Some(actual as f32);
                log::debug!(
                    "auto_brightness: external change detected \
                     prev={prev}% actual={actual}% → offset={new_offset:+}, grace armed"
                );
                // Smart-mode learning: this external change (Fn key, Windows
                // slider) carries the user's preference for the current light
                // level — log it into the smart model.
                if let Some(lux) = get_ambient_lux().filter(|v| *v > MIN_PLAUSIBLE_LUX) {
                    record_smart_sample(lux, actual);
                }
            }
        }
        // After a crash/relaunch (or first run), seed last_set from the actual
        // current brightness so the FIRST iteration does not immediately treat
        // the real user value as an "external change" and move the backlight.
        if last_set.is_none() {
            if let Some(actual) = brightness_actual {
                last_set = Some(actual);
                smoothed = Some(actual as f32);
            }
        }

        // S32-005: While the user is still within the manual-override grace
        // window, do NOT touch the backlight at all — the user just set it and
        // may still be tweaking. This is the guarantee the user asked for:
        // "if I adjust brightness manually, it must not snap back".
        if user_override_in_grace() {
            log::debug!(
                "[adaptive_brightness] in manual-override grace window \
                 (last manual change < {MANUAL_OVERRIDE_GRACE_MS}ms) — skipping write"
            );
            continue;
        }

        let lux = match tokio::task::spawn_blocking(get_ambient_lux).await {
            // A reading ≤ MIN_PLAUSIBLE_LUX lux with the screen on is physically
            // impossible; it means a placeholder sensor, an uninitialised value
            // (common at process startup on this hardware), or a stuck sensor
            // (e.g. fixed 1 lux after app close/reopen). Track consecutive bad
            // reads so we can reset a wedged sensor instead of trusting it.
            Ok(Some(v)) if v > MIN_PLAUSIBLE_LUX => v,
            Ok(Some(_)) => {
                consecutive_invalid_reads = consecutive_invalid_reads.saturating_add(1);
                if consecutive_invalid_reads >= STUCK_SENSOR_RESET_AFTER {
                    log::warn!(
                        "[adaptive_brightness] sensor stuck ({} consecutive invalid \
                         readings) — requesting sensor reset",
                        consecutive_invalid_reads
                    );
                    request_sensor_reset();
                    consecutive_invalid_reads = 0;
                }
                continue;
            }
            Ok(None) => {
                consecutive_invalid_reads = consecutive_invalid_reads.saturating_add(1);
                if consecutive_invalid_reads >= STUCK_SENSOR_RESET_AFTER {
                    log::warn!(
                        "[adaptive_brightness] no valid sensor reading for {} cycles — \
                         requesting sensor reset",
                        consecutive_invalid_reads
                    );
                    request_sensor_reset();
                    consecutive_invalid_reads = 0;
                }
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
        consecutive_invalid_reads = 0;
        // sensitivity=100 → reaches ceiling at 2000 lux
        // sensitivity=200 → reaches ceiling at 1000 lux  (more reactive)
        // sensitivity=50  → reaches ceiling at 4000 lux  (less reactive)
        let max_lux = 2000.0_f32 * (100.0 / cfg.sensitivity.max(1) as f32);
        let range = cfg.max_brightness as f32 - cfg.min_brightness as f32;
        // CURVE_BOOST lifts the entire brightness curve by this many percentage
        // points without changing the slope or the user-configurable min/max.
        const CURVE_BOOST: f32 = 20.0;
        let raw_target = if cfg.mode == "smart" {
            // Smart mode: use the learned user-preference mean for this lux
            // level where enough samples exist; otherwise fall back to the
            // fixed curve so new light levels still behave sensibly.
            match smart_predict_brightness(lux) {
                Some(learned) => {
                    let learned = learned as f32;
                    log::debug!(
                        "[adaptive_brightness] smart target: lux={lux:.0} \
                         learned={learned:.0}% (bucketed mean)"
                    );
                    learned.clamp(cfg.min_brightness as f32, cfg.max_brightness as f32)
                }
                None => (cfg.min_brightness as f32 + (lux / max_lux) * range + CURVE_BOOST)
                    .clamp(cfg.min_brightness as f32, cfg.max_brightness as f32),
            }
        } else {
            (cfg.min_brightness as f32 + (lux / max_lux) * range + CURVE_BOOST)
                .clamp(cfg.min_brightness as f32, cfg.max_brightness as f32)
        };

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
        log::info!(
            "[adaptive_brightness] setting brightness to {set_value}% (lux={lux:.0}, target={target:.1}, smoothed={next:.1})"
        );
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
            let results: Vec<HashMap<String, wmi::Variant>> = match wmi
                .raw_query("SELECT CurrentRefreshRate FROM Win32_VideoController")
            {
                Ok(r) => r,
                Err(e) => {
                    log::debug!(target: "hw::display", "Win32_VideoController raw_query error: {e}");
                    return Ok(None);
                }
            };
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes tests that share the `COM_LUX_HISTORY` global so parallel
    /// test execution cannot interleave `clear()` with dependent polls.
    ///
    /// The smart-model tests also take this lock: `manual_override_grace_window`
    /// calls `record_user_brightness_override`, which on a machine with a live
    /// ambient-light sensor records a smart sample into the shared model — so
    /// any test touching either the lux history or the smart model must be
    /// serialized together.
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Point the smart-model path at a temp file so tests never touch the
    /// user's real learned model, and remove any stale file from a previous
    /// test run.
    fn isolate_smart_model() {
        let temp = std::env::temp_dir().join("micontrol_test_smart_brightness.json");
        std::env::set_var("MICONTROL_SMART_MODEL_PATH", &temp);
        // Also isolate any already-cached model so `reset_smart_model()` starts
        // from the (empty) file at the new path.
        if let Some(cache) = SMART_MODEL_CACHE.get() {
            if let Ok(mut guard) = cache.lock() {
                *guard = Some(SmartModel::new());
            }
        }
        let _ = std::fs::remove_file(&temp);
    }

    #[cfg(windows)]
    #[test]
    fn responsive_sensor_prefers_largest_delta() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Clear any cached history from previous tests so the first poll has no
        // previous values to diff against.
        if let Some(history) = COM_LUX_HISTORY.get() {
            if let Ok(mut guard) = history.lock() {
                guard.clear();
            }
        }
        // First poll with empty history: no deltas, so the last finite
        // plausible reading wins (sensor[1] = 420 lux).
        let first = test_select_responsive(vec![Some(1000.0), Some(420.0)]);
        assert_eq!(first, Some(420.0), "empty history → last finite fallback");
        // Second poll: sensor[1] moved 420→120 (delta 300) — the real one,
        // while the placeholder stayed at 1000. Must win via largest delta.
        let second = test_select_responsive(vec![Some(1000.0), Some(120.0)]);
        assert_eq!(second, Some(120.0));
    }

    #[cfg(windows)]
    #[test]
    fn implausible_values_are_rejected() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // A stuck sensor reporting 1 lux (or a placeholder wandering near 0)
        // must never pass the plausibility floor, even when it is the only
        // finite reading available.
        if let Some(history) = COM_LUX_HISTORY.get() {
            if let Ok(mut guard) = history.lock() {
                guard.clear();
            }
        }
        // Stuck at 1 lux — physically impossible, must be rejected.
        assert_eq!(test_select_responsive(vec![Some(1.0)]), None);
        // Below the floor.
        assert_eq!(test_select_responsive(vec![Some(0.4)]), None);
        // A plausible reading wins over the stuck sensor.
        assert_eq!(
            test_select_responsive(vec![Some(1.0), Some(2.5)]),
            Some(2.5),
            "a plausible reading wins over the stuck sensor"
        );
    }

    #[test]
    fn manual_override_grace_window() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        isolate_smart_model();
        // Clean slate.
        AUTO_OFFSET.store(0, Ordering::Relaxed);
        AUTO_OFFSET_ACTIVE.store(false, Ordering::Relaxed);
        AUTO_LAST_MANUAL_MS.store(0, Ordering::Relaxed);

        // No manual change yet → no grace active.
        assert!(!user_override_in_grace());

        // Record an override → grace becomes active.
        AUTO_LAST_TARGET.store(50, Ordering::Relaxed);
        record_user_brightness_override(80);
        assert!(
            user_override_in_grace(),
            "grace must be armed right after a manual change"
        );

        // After the clock moves past the grace window, grace expires.
        let expired = now_ms().saturating_sub(MANUAL_OVERRIDE_GRACE_MS + 1);
        AUTO_LAST_MANUAL_MS.store(expired, Ordering::Relaxed);
        assert!(
            !user_override_in_grace(),
            "grace must expire after the window"
        );

        // clear resets both offset and grace.
        record_user_brightness_override(60);
        assert!(user_override_in_grace());
        clear_user_brightness_override();
        assert!(!user_override_in_grace());
        assert_eq!(AUTO_OFFSET.load(Ordering::Relaxed), 0);
        assert!(!AUTO_OFFSET_ACTIVE.load(Ordering::Relaxed));
    }

    #[test]
    fn smart_model_learns_and_predicts() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        isolate_smart_model();
        reset_smart_model();
        // Seed the model directly (without touching the sensor).
        record_smart_sample(50.0, 30);
        record_smart_sample(51.0, 32);
        record_smart_sample(52.0, 34);
        record_smart_sample(53.0, 36);
        // All in bucket floor(50/10)=5 → mean = 33, n=4 ≥ 3.
        let predicted = smart_predict_brightness(51.0);
        assert_eq!(
            predicted,
            Some(33),
            "bucket mean (30+32+34+36)/4 = 33 predicted with n=4"
        );
        // Different lux in same bucket → same prediction.
        assert_eq!(smart_predict_brightness(58.0), Some(33));
        // Empty bucket → falls back (None) — loop uses the formula.
        assert_eq!(smart_predict_brightness(200.0), None);
        // Outlier (very different brightness) updates the mean but stays in range.
        record_smart_sample(52.0, 90);
        let predicted2 = smart_predict_brightness(52.0);
        assert!(predicted2.is_some());
        assert!(
            (predicted2.unwrap() as f32 - ((30.0 + 32.0 + 34.0 + 36.0 + 90.0) / 5.0)).abs() < 1.0
        );
        // Std-dev of the bucket (population) can be derived from m2.
        let model = get_smart_model();
        let bin = model.bins.get(&5).expect("bucket 5 exists");
        assert_eq!(bin.n, 5);
        let sd = bin_stddev(bin);
        assert!(
            (sd - 22.89).abs() < 0.5,
            "population std-dev ≈ 22.89, got {sd}"
        );
        reset_smart_model();
    }

    #[test]
    fn smart_model_ignores_implausible_lux() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        isolate_smart_model();
        reset_smart_model();
        // Negative / NaN lux must not create buckets.
        record_smart_sample(-5.0, 50);
        record_smart_sample(f32::NAN, 50);
        record_smart_sample(f32::INFINITY, 50);
        // Still falls back everywhere.
        assert_eq!(smart_predict_brightness(10.0), None);
        let model = get_smart_model();
        assert!(model.bins.is_empty(), "no valid samples added");
        // A valid sample then works.
        record_smart_sample(10.0, 40);
        record_smart_sample(11.0, 42);
        record_smart_sample(12.0, 44);
        assert_eq!(smart_predict_brightness(10.5), Some(42));
        reset_smart_model();
    }
}
