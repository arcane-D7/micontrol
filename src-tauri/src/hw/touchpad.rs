//! Touchpad configuration (sensitivity, haptics, gestures).
//!
//! Controls Precision Touchpad settings via Windows Touchpad API
//! or IoT service for Xiaomi-specific features.

use crate::hw::errors::{HardwareError, HardwareResult};
use crate::util::panic::lock_or_recover;
use anyhow::Context;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct TouchpadInfo {
    pub sensitivity: TouchpadSensitivity,
    pub haptics_enabled: bool,
    pub haptics_intensity: HapticsIntensity,
    pub gesture_screenshot: bool,
    pub trackpad_repress: bool,
    pub edge_slide: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TouchpadSensitivity {
    Low,
    Medium,
    High,
    VeryHigh,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum HapticsIntensity {
    Low,
    Medium,
    High,
}

/// Fallback HID path used when hardware discovery has not yet found the touchpad.
///
/// **IMPORTANT**: Xiaomi's SvrCModule.dll communicates with the BLTP7853 touchpad
/// via **COL05** (not COL04). This was confirmed by:
/// 1. The string `"col05"` in SvrCModule.dll's `TouchSettingManager::Init()`
/// 2. The HID probe showing COL05 has UsagePage=0xFF01, Output:33
/// 3. SvrCModule log showing `SetOutputReport` calls (not `SetFeature`)
///
/// COL04 (UsagePage=0xFF00) is a different vendor collection that does not
/// accept haptics output reports.
const TOUCHPAD_HID_PATH_DEFAULT: &str =
    r"\\?\hid#bltp7853&col05#5&37166779&0&0004#{4d1e55b2-f16f-11cf-88cb-001111000030}";

/// Return the active touchpad HID path.
/// Uses the discovery profile when available; falls back to the BLTP7853 COL05 default.
fn touchpad_hid_path() -> String {
    // Try discovery profile first (the discovery module enumerates HID devices
    // at first launch and populates touchpad_hid_path via SetupAPI).
    if let Some(p) = crate::hw::discovery::global_profile() {
        if let Some(path) = &p.touchpad_hid_path {
            return path.clone();
        }
    }

    // Fall back to hardcoded default
    log::warn!("[touchpad] Using fallback HID path — device may not respond");
    TOUCHPAD_HID_PATH_DEFAULT.to_string()
}

const TP_REG_KEY: &str = r"SOFTWARE\MI\Touchpad";
const TP_REG_SENSITIVITY: &str = "Sensitivity";
const TP_REG_HAPTICS: &str = "HapticsEnabled";
const TP_REG_HAPTICS_INTENSITY: &str = "HapticsIntensity";
const TP_REG_GESTURE_SCREENSHOT: &str = "GestureScreenshot";
const TP_REG_TRACKPAD_REPRESS: &str = "TrackpadRepress";
const TP_REG_EDGE_SLIDE: &str = "EdgeSlide";

// ─── Gesture feature flags (Windows-only) ───────────────────────────────────

#[cfg(windows)]
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU64, Ordering};

/// Whether the 5-finger screenshot gesture is currently enabled.
#[cfg(windows)]
static GESTURE_SCREENSHOT_ENABLED: AtomicBool = AtomicBool::new(false);

/// Whether the left/right edge slide gesture is currently enabled.
#[cfg(windows)]
static EDGE_SLIDE_ENABLED: AtomicBool = AtomicBool::new(false);

/// Edge **initiation** band width, in percent of the pad's logical X range,
/// measured from the physical left/right border.
///
/// Xiaomi's own gesture recogniser (`PluginGesture.dll` 5.8.0.57, disassembled
/// 2026-11-08) uses the outer **4 %** only: the anchor and every following
/// sample must satisfy `x < x_max/25` (left) or `x > x_max*24/25` (right).
/// On the BLTP7853 (x_max 3282 ≈ 31.25 units/mm) that is a ~4.2 mm strip, so a
/// finger landing 6 mm from the border never arms. Our default is deliberately
/// ~4× wider, and the "don't hijack normal use" job is done by the activation
/// gate (`EDGE_ACTIVATION_PERMILLE`) exactly as the original does with its
/// 250-unit (`cmp |dy|, 0xFA`) pre-roll.
///
/// Overridable at runtime through `HKCU\Software\MiControl\Touchpad`
/// (`ZoneStartPct`, `ZoneContinuePct`) — see `load_edge_zone_config`.
#[cfg(windows)]
static EDGE_ZONE_START_PCT: AtomicI32 = AtomicI32::new(16);
/// Continuation band, wider than the initiation band so ordinary finger drift
/// does not kill a running gesture (exit hysteresis). Xiaomi has no hysteresis
/// (the same 4 % is required for the whole gesture, which is why their gesture
/// dies if you drift inward); we keep a wider band on purpose.
#[cfg(windows)]
static EDGE_ZONE_CONTINUE_PCT: AtomicI32 = AtomicI32::new(28);
/// Vertical travel that must be covered **inside the band** before a swipe is
/// captured, in permille of `y_max`.
///
/// 118 ‰ of 2124 = 250 logical units = 8.0 mm, which is the literal constant in
/// `PluginGesture.dll` (`cmp |dy|, 0xFA` before it sets its "captured" flag).
/// This single gate is what keeps a normal click/scroll near the border from
/// turning into a brightness/volume change; our previous 0.85 mm gate
/// (`y_max/80`) fired on the first flick of an ordinary touch.
#[cfg(windows)]
const EDGE_ACTIVATION_PERMILLE: i32 = 118;

/// True while an edge-slide gesture is actively tracking a contact.
/// Used as the single source of truth for the cursor pin watchdog.
#[cfg(windows)]
static EDGE_GESTURE_ACTIVE: AtomicBool = AtomicBool::new(false);
/// Prevents the Raw Input listener thread from being started more than once.
#[cfg(windows)]
static GESTURE_THREAD_STARTED: AtomicBool = AtomicBool::new(false);

/// Cursor position captured when the edge gesture pinned the pointer.
#[cfg(windows)]
struct CursorPin {
    x: i32,
    y: i32,
    at: std::time::Instant,
}

/// `Some(..)` while we hold a `ClipCursor` confinement. Stored outside the
/// gesture thread so `set_touchpad_edge_slide(false)` (Tauri command thread) and
/// the watchdog can both release it.
#[cfg(windows)]
static CURSOR_PIN: std::sync::Mutex<Option<CursorPin>> = std::sync::Mutex::new(None);

/// Hard ceiling on how long the pointer may stay confined. If the state machine
/// ever fails to deliver a terminal frame (missed HID report, device reset) the
/// clip is dropped after this instead of leaving the user without a cursor.
#[cfg(windows)]
const CURSOR_PIN_MAX: std::time::Duration = std::time::Duration::from_secs(4);

/// Monotonic milliseconds since process start (used to rate-limit the haptic
/// tick without pulling in another Win32 feature).
#[cfg(windows)]
fn monotonic_ms() -> u64 {
    static EPOCH: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    EPOCH
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}

#[cfg(windows)]
static LAST_EDGE_TICK_MS: AtomicU64 = AtomicU64::new(0);

/// Cached `OutputReportByteLength` for the haptics handle (33 on COL05).
/// Querying `HidP_GetCaps` on every write is wasteful on the per-step tick path.
#[cfg(windows)]
static OUTPUT_REPORT_LEN: AtomicU32 = AtomicU32::new(0);

/// App handle stored at startup so the gesture thread can emit Tauri events
/// (e.g. show the brightness OSD) without needing a channel or mutex.
static APP_HANDLE: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

/// Returns true when the AC adapter is currently connected (charger plugged in).
/// Used to increase filtering aggressiveness when EMI coupling is most likely.
#[cfg(windows)]
fn is_charger_connected() -> bool {
    use windows::Win32::System::Power::GetSystemPowerStatus;
    let mut status = windows::Win32::System::Power::SYSTEM_POWER_STATUS::default();
    unsafe {
        // SAFETY: GetSystemPowerStatus writes to the stack-local SYSTEM_POWER_STATUS struct
        // which is POD; default-initialization is valid. The call returns an HRESULT indicating
        // success before we read ACLineStatus.
        if GetSystemPowerStatus(&mut status).is_ok() {
            // ACLineStatus: 0 = offline, 1 = online, 255 = unknown
            return status.ACLineStatus == 1;
        }
    }
    false
}

/// Returns a charger-aware Y-axis deadband threshold (as a fraction of y_max).
/// When the charger is connected, EMI coupling is more likely, so we apply a
/// larger deadband to reject small corrupted Y deltas.
#[cfg(windows)]
fn charger_aware_y_deadband(y_max: i32) -> i32 {
    // 2% of height normally; 4% when charger connected (EMI defense-in-depth).
    let pct = if is_charger_connected() { 4 } else { 2 };
    (y_max as u32 * pct / 100) as i32
}

/// Called once during app setup to give the gesture thread access to Tauri.
pub fn set_app_handle(h: tauri::AppHandle) {
    let _ = APP_HANDLE.set(h);
}

// ─── Edge-slide geometry configuration ───────────────────────────────────────

/// Read a single DWORD from `HKCU\Software\MiControl\Touchpad`, if present.
#[cfg(windows)]
fn read_tp_reg_dword(value_name: &str) -> Option<u32> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_CURRENT_USER, KEY_READ, REG_VALUE_TYPE,
    };
    unsafe {
        // SAFETY: both strings are null-terminated wide buffers that outlive the
        // calls; `hkey` is only used after RegOpenKeyExW reports success, and
        // RegQueryValueExW writes at most 4 bytes into a stack-local u32.
        let key_w: Vec<u16> = OsStr::new(TP_REG_KEY)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let mut hkey = std::mem::MaybeUninit::uninit();
        if RegOpenKeyExW(
            HKEY_CURRENT_USER,
            PCWSTR(key_w.as_ptr()),
            0,
            KEY_READ,
            hkey.as_mut_ptr(),
        )
        .is_err()
        {
            return None;
        }
        let hkey = hkey.assume_init();
        let mut ty = REG_VALUE_TYPE::default();
        let mut value: u32 = 0;
        let mut size = 4u32;
        let name_w: Vec<u16> = OsStr::new(value_name)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let res = RegQueryValueExW(
            hkey,
            PCWSTR(name_w.as_ptr()),
            None,
            Some(&mut ty),
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        );
        let _ = RegCloseKey(hkey).ok();
        // WIN32_ERROR is a newtype around u32; 0 == ERROR_SUCCESS.
        if res.0 == 0 {
            Some(value)
        } else {
            None
        }
    }
}

/// Refresh [`EDGE_ZONE_START_PCT`] / [`EDGE_ZONE_CONTINUE_PCT`] from the
/// registry. Called at startup and on every EdgeSlide toggle, so the band can be
/// tuned without a rebuild:
///
/// ```text
/// reg add HKCU\Software\MiControl\Touchpad /v ZoneStartPct    /t REG_DWORD /d 10
/// reg add HKCU\Software\MiControl\Touchpad /v ZoneContinuePct /t REG_DWORD /d 22
/// ```
#[cfg(windows)]
pub fn load_edge_zone_config() {
    let start = read_tp_reg_dword("ZoneStartPct")
        .map(|v| v as i32)
        .unwrap_or(EDGE_ZONE_START_PCT.load(Ordering::Relaxed));
    // 4 % is Xiaomi's own band; 30 % is already 3 cm on this pad — beyond that the
    // gesture would own a third of the touchpad.
    let start = start.clamp(4, 30);
    let continue_pct = read_tp_reg_dword("ZoneContinuePct")
        .map(|v| v as i32)
        .unwrap_or(EDGE_ZONE_CONTINUE_PCT.load(Ordering::Relaxed));
    // Hysteresis must exist: the continuation band has to be strictly wider.
    let continue_pct = continue_pct.clamp(start + 4, 40);

    EDGE_ZONE_START_PCT.store(start, Ordering::Relaxed);
    EDGE_ZONE_CONTINUE_PCT.store(continue_pct, Ordering::Relaxed);
    log::info!(
        "[gesture] edge-slide zone: start<{}% continue<{}% of x_max",
        start,
        continue_pct
    );
}

/// Current `(start_pct, continue_pct)` band, clamped so a corrupted atomic can
/// never produce a zero-width or inverted zone.
#[cfg(windows)]
fn edge_zone_pcts() -> (i32, i32) {
    let start = EDGE_ZONE_START_PCT.load(Ordering::Relaxed).clamp(4, 40);
    let cont = EDGE_ZONE_CONTINUE_PCT
        .load(Ordering::Relaxed)
        .clamp(start + 2, 50);
    (start, cont)
}

// ─── Cursor confinement ("o rato não anda") ─────────────────────────────────

/// Pin the pointer for the duration of an edge gesture with `ClipCursor`.
///
/// Why not a `WH_MOUSE_LL` hook (the previous, always-disabled approach): the
/// hook runs on *this* thread, and this thread also performs the Tauri/OSD and
/// brightness work for every step — a slow step stalls mouse input system-wide,
/// and the low-level movement the hook swallows is re-applied as a jump when it
/// is released. `ClipCursor` is a one-call kernel-side clamp: no hook, no
/// per-event latency, and — because the clamp discards the excess instead of
/// queueing it — no jump when the gesture ends.
///
/// Xiaomi's original does **not** do this (PluginGesture.dll imports neither
/// `ClipCursor` nor `SetCursorPos`); it relies on its 4 % band keeping the
/// pointer against the screen border. We keep the pointer genuinely still
/// because that is what the field report asked for.
#[cfg(windows)]
fn pin_cursor() {
    use windows::Win32::Foundation::{POINT, RECT};
    use windows::Win32::UI::WindowsAndMessaging::{ClipCursor, GetCursorPos};

    if lock_or_recover(&CURSOR_PIN).is_some() {
        return; // already pinned — keep the original anchor and deadline
    }

    unsafe {
        // SAFETY: POINT/RECT are plain C structs; ClipCursor copies the rect.
        let mut p = POINT::default();
        if GetCursorPos(&mut p).is_err() {
            return;
        }
        // RECT is *inclusive* on right/bottom for ClipCursor: a rect of
        // (x, y, x+1, y+1) still allows one pixel of travel (measured:
        // 638,805 → 639,806). right==left/bottom==top pins the single pixel.
        let rect = RECT {
            left: p.x,
            top: p.y,
            right: p.x,
            bottom: p.y,
        };
        if ClipCursor(Some(&rect)).is_err() {
            log::debug!("[gesture] ClipCursor failed — pointer will follow the gesture");
            return;
        }
        *lock_or_recover(&CURSOR_PIN) = Some(CursorPin {
            x: p.x,
            y: p.y,
            at: std::time::Instant::now(),
        });
        log::info!(
            "[gesture] pointer pinned at ({}, {}) for the edge gesture",
            p.x,
            p.y
        );
    }
}

/// Drop the `ClipCursor` confinement. Safe to call when nothing is pinned.
#[cfg(windows)]
fn release_cursor_pin(reason: &str) {
    use windows::Win32::UI::WindowsAndMessaging::ClipCursor;

    let pin = {
        let mut guard = lock_or_recover(&CURSOR_PIN);
        guard.take()
    };
    if let Some(p) = pin {
        unsafe {
            // SAFETY: passing NULL releases the confinement for this desktop.
            let _ = ClipCursor(None);
        }
        log::info!(
            "[gesture] pointer released at ({}, {}) after {} ms ({reason})",
            p.x,
            p.y,
            p.at.elapsed().as_millis()
        );
    }
}

/// True when the pin has outlived [`CURSOR_PIN_MAX`] and must be dropped.
#[cfg(windows)]
fn cursor_pin_expired() -> bool {
    lock_or_recover(&CURSOR_PIN)
        .as_ref()
        .is_some_and(|p| p.at.elapsed() > CURSOR_PIN_MAX)
}

// ─── Per-step haptic tick ───────────────────────────────────────────────────

/// Send the one-frame haptic tick the original emits after every edge-slide step.
///
/// Byte-for-byte from `PluginGesture.dll` (function at `0x180018750` in the
/// 5.8.0.57 build, which builds the frame inline and calls `HidD_SetOutputReport`
/// with a 33-byte buffer):
///
/// ```text
/// 0D 09 FD 01 00 03 00 00 02 32 CE
/// │  │  │  │  │  │  │  │  │  └── payload[1] = 0xCE
/// │  │  │  │  │  │  │  │  └───── payload[0] = 0x32 (50)
/// │  │  │  │  │  │  │  └─────── cmd = 0x02 (single click)
/// │  │  │  │  │  │  └────────── reserved
/// │  │  │  │  │  └───────────── payload_len + 1 = 3
/// │  │  │  │  └──────────────── phase 0x01 (this frame IS the commit)
/// │  │  │  └─────────────────── checksum = XOR(bytes[3..31]) + 1
/// │  │  └────────────────────── len = payload_len + 7 = 9
/// │  └───────────────────────── magic
/// └──────────────────────────── HID report id (0x0D) of the vendor collection
/// ```
///
/// Note there is **no** second commit frame and no 120 ms gap here: that pair
/// (`cmd 0x01`, `payload [cmd, 0x100-cmd]`) is only for *configuration* writes
/// such as 0x5D (motor gears), 0x59 (heavy press) and 0x5B (force threshold).
#[cfg(windows)]
fn send_edge_slide_tick(reason: &str) {
    /// The tick is a nicety, not the function: never queue more than ~25/s, and
    /// never let a slow HID write back up the raw-input thread.
    const TICK_MIN_INTERVAL_MS: u64 = 40;

    let now = monotonic_ms();
    let last = LAST_EDGE_TICK_MS.load(Ordering::Relaxed);
    if last != 0 && now.saturating_sub(last) < TICK_MIN_INTERVAL_MS {
        return;
    }
    LAST_EDGE_TICK_MS.store(now, Ordering::Relaxed);

    let frame = build_touchpad_report(0x02, &[0x32, 0xCE], true);
    match send_touchpad_output_report(&frame) {
        Ok(()) => log::trace!("[gesture] edge haptic tick sent ({reason})"),
        Err(e) => log::debug!("[gesture] edge haptic tick failed ({reason}): {e}"),
    }
}

// ─── Public API ───────────────────────────────────────────────────────────────

pub fn get_touchpad_info() -> HardwareResult<TouchpadInfo> {
    let info = read_touchpad_registry().unwrap_or(TouchpadInfo {
        sensitivity: TouchpadSensitivity::Medium,
        haptics_enabled: true,
        haptics_intensity: HapticsIntensity::Medium,
        gesture_screenshot: false,
        trackpad_repress: false,
        edge_slide: false,
    });
    log::debug!(
        target: "hw::touchpad",
        "get_touchpad_info: sensitivity={:?} haptics={} gesture_screenshot={} repress={} edge_slide={}",
        info.sensitivity,
        info.haptics_enabled,
        info.gesture_screenshot,
        info.trackpad_repress,
        info.edge_slide
    );
    Ok(info)
}

pub fn set_touchpad_sensitivity(sensitivity: TouchpadSensitivity) -> HardwareResult<()> {
    let reg_val = match sensitivity {
        TouchpadSensitivity::Low => 1,
        TouchpadSensitivity::Medium => 2,
        TouchpadSensitivity::High => 3,
        TouchpadSensitivity::VeryHigh => 4,
    };
    persist_reg_dword(TP_REG_SENSITIVITY, reg_val)?;
    // Also update the Windows standard PTP sensitivity registry so the inbox driver sees it.
    #[cfg(windows)]
    set_windows_ptp_sensitivity(&sensitivity);
    // Send the force threshold HID output report to the touchpad hardware.
    // Xiaomi's SvrCModule.dll calls WriteForceGears() which sends an output
    // report with force threshold bytes. Without this, the registry change
    // has no effect on the actual touchpad behavior.
    #[cfg(windows)]
    {
        if let Err(e) = send_force_threshold_hid_report(&sensitivity) {
            log::warn!(
                "[touchpad] Force threshold HID report failed: {e} — registry updated but hardware may not reflect change"
            );
            return Err(e);
        }
    }
    Ok(())
}

pub fn set_touchpad_haptics(enabled: bool) -> HardwareResult<()> {
    persist_reg_dword(TP_REG_HAPTICS, if enabled { 1 } else { 0 })?;
    #[cfg(windows)]
    {
        // S50 fix: report 0x59 is `WriteEnableHeavyPress` (the repress /
        // second-tap feature), NOT haptics on/off — sending it from the
        // haptics toggle silently corrupted the repress state. Xiaomi's own
        // app has NO haptics on/off setter (only a getter); haptics on this
        // pad is firmware-always-on with configurable intensity. So the
        // toggle only re-sends the motor gears for the current intensity,
        // which is the only device-side haptics control that exists.
        let intensity = read_touchpad_registry()
            .map(|i| i.haptics_intensity)
            .unwrap_or(HapticsIntensity::Medium);
        if let Err(e) = send_motor_gears_hid_report(&intensity) {
            log::warn!("[touchpad] haptics gears HID report failed: {e} — registry updated but hardware may not reflect change");
            return Err(e);
        }
    }
    Ok(())
}

pub fn set_touchpad_haptics_intensity(intensity: HapticsIntensity) -> HardwareResult<()> {
    persist_reg_dword(
        TP_REG_HAPTICS_INTENSITY,
        match intensity {
            HapticsIntensity::Low => 1,
            HapticsIntensity::Medium => 2,
            HapticsIntensity::High => 3,
        },
    )?;
    #[cfg(windows)]
    {
        if let Err(e) = send_motor_gears_hid_report(&intensity) {
            log::warn!("[touchpad] intensity HID report failed: {e} — registry updated but hardware may not reflect change");
            return Err(e);
        }
    }
    Ok(())
}

pub fn set_touchpad_gesture_screenshot(enabled: bool) -> HardwareResult<()> {
    #[cfg(windows)]
    GESTURE_SCREENSHOT_ENABLED.store(enabled, Ordering::Relaxed);
    persist_reg_dword(TP_REG_GESTURE_SCREENSHOT, if enabled { 1 } else { 0 })?;
    // Ensure the Raw Input gesture listener thread is running.
    #[cfg(windows)]
    ensure_gesture_listener();
    Ok(())
}

/// S50 fix: repress (second-tap / heavy-press) now sends the REAL hardware
/// control — HID report 0x59 `WriteEnableHeavyPress`, which the old haptics
/// toggle was wrongly spending. Previously this was a registry-only stub and
/// the touchpad behavior never changed.
pub fn set_touchpad_repress(enabled: bool) -> HardwareResult<()> {
    persist_reg_dword(TP_REG_TRACKPAD_REPRESS, if enabled { 1 } else { 0 })?;
    #[cfg(windows)]
    send_heavy_press_hid_report(enabled)?;
    Ok(())
}

pub fn set_touchpad_edge_slide(enabled: bool) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        EDGE_SLIDE_ENABLED.store(enabled, Ordering::Relaxed);
        // Re-read the band so a registry tweak applies on the next toggle.
        load_edge_zone_config();
        // Defensive reset: if edge-slide is disabled while a gesture is active,
        // drop the pointer confinement immediately — never leave the user
        // without a cursor because a toggle landed mid-gesture.
        if !enabled {
            EDGE_GESTURE_ACTIVE.store(false, Ordering::Relaxed);
            release_cursor_pin("edge-slide disabled");
        }
    }
    persist_reg_dword(TP_REG_EDGE_SLIDE, if enabled { 1 } else { 0 })?;
    // Ensure the Raw Input gesture listener thread is running.
    #[cfg(windows)]
    ensure_gesture_listener();
    Ok(())
}

/// Call once at app startup: restores persisted flags to atomics, then
/// starts the background Raw Input gesture listener if any feature is enabled.
pub fn start_gesture_listener() {
    if let Ok(info) = get_touchpad_info() {
        #[cfg(windows)]
        {
            GESTURE_SCREENSHOT_ENABLED.store(info.gesture_screenshot, Ordering::Relaxed);
            EDGE_SLIDE_ENABLED.store(info.edge_slide, Ordering::Relaxed);
        }
    }
    #[cfg(windows)]
    {
        load_edge_zone_config();
        ensure_gesture_listener();
    }
}

// ─── Registry persistence ─────────────────────────────────────────────────────

/// Write a single DWORD value to `HKLM\SOFTWARE\MI\Touchpad`.
fn persist_reg_dword(value_name: &str, value: u32) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY_CURRENT_USER, KEY_WRITE, REG_DWORD,
            REG_OPTION_NON_VOLATILE,
        };
        unsafe {
            // SAFETY: Null-terminated wide strings; RegCreateKeyExW writes to MaybeUninit<HKEY>
            // before assume_init. Stack-local DWORD values have valid alignment.
            let key_w: Vec<u16> = OsStr::new(TP_REG_KEY)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let mut hkey = std::mem::MaybeUninit::uninit();
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(key_w.as_ptr()),
                0,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                hkey.as_mut_ptr(),
                None,
            )
            .ok()
            .context("Create touchpad reg key")?;
            let hkey = hkey.assume_init();
            let val_w: Vec<u16> = OsStr::new(value_name)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let _ = RegSetValueExW(
                hkey,
                PCWSTR(val_w.as_ptr()),
                0,
                REG_DWORD,
                Some(&value.to_le_bytes()),
            )
            .ok();
            let _ = RegCloseKey(hkey).ok();
        }
    }
    Ok(())
}

fn read_touchpad_registry() -> HardwareResult<TouchpadInfo> {
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_CURRENT_USER, REG_VALUE_TYPE,
        };
        unsafe {
            // SAFETY: Null-terminated wide strings; hkey is assume_init only after
            // RegOpenKeyExW succeeds. The read_dword closure uses safe pointer casts for
            // RegQueryValueExW with stack-local u32 values.
            let key_w: Vec<u16> = OsStr::new(TP_REG_KEY)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let mut hkey = std::mem::MaybeUninit::uninit();
            if RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(key_w.as_ptr()),
                0,
                windows::Win32::System::Registry::KEY_READ,
                hkey.as_mut_ptr(),
            )
            .is_err()
            {
                return Ok(TouchpadInfo {
                    sensitivity: TouchpadSensitivity::Medium,
                    haptics_enabled: true,
                    haptics_intensity: HapticsIntensity::Medium,
                    gesture_screenshot: false,
                    trackpad_repress: false,
                    edge_slide: false,
                });
            }
            let hkey = hkey.assume_init();

            let read_dword = |name: &str, default: u32| -> u32 {
                let mut ty = REG_VALUE_TYPE::default();
                let mut v: u32 = default;
                let mut size = 4u32;
                let w: Vec<u16> = OsStr::new(name).encode_wide().chain(Some(0)).collect();
                let _ = RegQueryValueExW(
                    hkey,
                    PCWSTR(w.as_ptr()),
                    None,
                    Some(&mut ty),
                    Some((&mut v as *mut u32).cast()),
                    Some(&mut size),
                );
                v
            };

            let sens_raw = read_dword(TP_REG_SENSITIVITY, 2);
            let haptics = read_dword(TP_REG_HAPTICS, 1) != 0;
            let haptics_intensity_raw = read_dword(TP_REG_HAPTICS_INTENSITY, 2);
            let gesture_screenshot = read_dword(TP_REG_GESTURE_SCREENSHOT, 0) != 0;
            let trackpad_repress = read_dword(TP_REG_TRACKPAD_REPRESS, 0) != 0;
            let edge_slide = read_dword(TP_REG_EDGE_SLIDE, 0) != 0;

            let _ = RegCloseKey(hkey).ok();

            Ok(TouchpadInfo {
                sensitivity: match sens_raw {
                    1 => TouchpadSensitivity::Low,
                    3 => TouchpadSensitivity::High,
                    4 => TouchpadSensitivity::VeryHigh,
                    _ => TouchpadSensitivity::Medium,
                },
                haptics_enabled: haptics,
                haptics_intensity: match haptics_intensity_raw {
                    1 => HapticsIntensity::Low,
                    3 => HapticsIntensity::High,
                    _ => HapticsIntensity::Medium,
                },
                gesture_screenshot,
                trackpad_repress,
                edge_slide,
            })
        }
    }
    #[cfg(not(windows))]
    {
        Ok(TouchpadInfo {
            sensitivity: TouchpadSensitivity::Medium,
            haptics_enabled: true,
            haptics_intensity: HapticsIntensity::Medium,
            gesture_screenshot: false,
            trackpad_repress: false,
            edge_slide: false,
        })
    }
}

// ─── BLTP7853 COL05 HID output report (Xiaomi protocol) ─────────────────────
//
// The BLTP7853 vendor HID collection **COL05** (UsagePage=0xFF01, Output:33)
// accepts touchpad configuration via **Output Reports** sent with
// `HidD_SetOutputReport`.
//
// This protocol was reverse-engineered from Xiaomi's SvrCModule.dll strings
// and verified against the SvrCModule log file at:
//   C:\ProgramData\MI\SvrCModule\log\svrc_log.txt
//
// ## SvrCModule.dll function chain:
//
//   TouchSettingManager::Init()          → finds "col05" device
//   TouchSettingManager::SetOutputReport() → HidD_SetOutputReport(handle, buf, len)
//
// ## Protocol summary (from log analysis):
//
// ### Vibration Intensity (WriteMotorGears)
//   API: set_touchpad_vibration_intensity {"mode": N}
//   Log: "write Motor touch:N" → "write touch Motor:VALUE"
//
//   mode 1 (Low)    → Motor value: 56  (0x38)
//   mode 2 (Medium) → Motor value: 80  (0x50)
//   mode 3 (High)   → Motor value: 104 (0x68)
//
// ### Pressing Sensitivity (WriteForceGears / OpenWriteForceThreshold)
//   API: set_touchpad_pressing_sensitivity {"mode": N}
//   Log: "write touch Force:N" → "write fore touch:HEXDUMP"
//
//   mode 1 (Light) → Force:3, bytes: [0x14, 0x04, 0x65, 0x00, 0x04, 0x00]
//   mode 2 (Med)   → Force:2, bytes: [0x12, 0x03, 0x95, 0x00, 0x04, 0x00]
//   mode 3 (Firm)  → Force:1, bytes: [0x10, 0x03, 0x35, 0x00, 0x04, 0x00]
//
//   Note: mode→Force mapping is INVERTED (mode 1 = Force:3, mode 3 = Force:1)
//
// ### Haptic Feedback On/Off (WriteEnableHeavyPress)
//   API: set_haptic_feedback
//   Log: "write EnableHeavyPress:VALUE" / "write touch IsEnableHeary:VALUE"
//
//   enabled=true  → byte value: 0x01
//   enabled=false → byte value: 0x00
//
// ### Edge Slide / Gesture Screenshot
//   These are registry-only — no HID output report is sent.
//   SvrCModule log shows only "get RegistryValue" / "Send get/set request"
//   with no corresponding SetOutputReport call.

/// Wrapper around `HANDLE` that implements `Send` and `Sync`.
/// `HANDLE` is `*mut c_void` which is neither `Send` nor `Sync` but Windows
/// handles are safe to share across threads.
#[cfg(windows)]
#[derive(Clone, Copy)]
struct SendHandle(windows::Win32::Foundation::HANDLE);

#[cfg(windows)]
unsafe impl Send for SendHandle {}
#[cfg(windows)]
unsafe impl Sync for SendHandle {}

/// Cached HID device handle for output report writes.
/// Opened once and reused for the lifetime of the process.
#[cfg(windows)]
static HAPTICS_HANDLE: std::sync::OnceLock<std::sync::Mutex<Option<SendHandle>>> =
    std::sync::OnceLock::new();

/// Get or open the haptics HID device handle.
#[cfg(windows)]
fn get_haptics_handle() -> windows::core::Result<windows::Win32::Foundation::HANDLE> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    let lock = HAPTICS_HANDLE.get_or_init(|| std::sync::Mutex::new(None));
    let mut guard = lock_or_recover(lock);

    if let Some(send_handle) = *guard {
        let handle = send_handle.0;
        if handle != INVALID_HANDLE_VALUE {
            return Ok(handle);
        }
    }

    // Open a new handle
    let device_path = touchpad_hid_path();
    log::info!("[touchpad] Opening HID device: {device_path}");
    let path_w: Vec<u16> = OsStr::new(&device_path)
        .encode_wide()
        .chain(Some(0))
        .collect();

    // SAFETY: CreateFileW with valid path and standard access flags.
    // We need GENERIC_READ | GENERIC_WRITE because HidD_SetOutputReport
    // requires write access, and GetInputReport (for reading current state)
    // requires read access.
    let handle = unsafe {
        CreateFileW(
            PCWSTR(path_w.as_ptr()),
            FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    };

    let handle = match handle {
        Ok(h) => h,
        Err(e) => {
            log::error!("[touchpad] Failed to open HID device at path: {device_path} — {e}");
            return Err(e);
        }
    };

    *guard = Some(SendHandle(handle));
    Ok(handle)
}

/// Close the cached haptics handle. Called when the gesture thread exits.
#[cfg(windows)]
pub fn close_haptics_handle() {
    if let Some(lock) = HAPTICS_HANDLE.get() {
        let mut guard = lock_or_recover(lock);
        if let Some(send_handle) = guard.take() {
            // SAFETY: handle was created by CreateFileW.
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(send_handle.0);
            }
        }
    }
}

/// Log an output report with its exact bytes (needed to undo any write).
///
/// The per-step click tick (cmd 0x02, ~25×/s during a swipe) is logged at debug
/// so it does not drown the log; every configuration write (0x5D motor gears,
/// 0x59 heavy press, 0x5B force threshold) stays at INFO with its full byte
/// sequence, which is what makes a field change reversible.
#[cfg(windows)]
fn log_output_report(prefix: &str, data: &[u8]) {
    let is_click_tick = data.get(8).copied() == Some(0x02);
    if is_click_tick {
        log::debug!("{prefix}: {} bytes, data={:02X?}", data.len(), data);
    } else {
        log::info!("{prefix}: {} bytes, data={:02X?}", data.len(), data);
    }
}

/// Send an output report to the BLTP7853 touchpad via `HidD_SetOutputReport`.
///
/// This is the exact same API call that Xiaomi's SvrCModule.dll makes:
/// `TouchSettingManager::SetOutputReport(vector<uchar>)` → `HidD_SetOutputReport`.
///
/// The output report byte length is queried from the HID descriptor
/// (OutputReportByteLength = 33 for COL05).
///
/// **Important**: SvrCModule.dll opens, writes, and closes the HID device for
/// each operation (see `touch UnInit HandleDeviceDisconnected` in svrc_log.txt).
/// We replicate this pattern — the handle is closed after each write to avoid
/// stale handle issues when the device disconnects.
#[cfg(windows)]
fn send_touchpad_output_report(report_data: &[u8]) -> HardwareResult<()> {
    use windows::Win32::Devices::HumanInterfaceDevice::{
        HidD_FreePreparsedData, HidD_GetPreparsedData, HidD_SetOutputReport, HidP_GetCaps,
        HIDP_CAPS, PHIDP_PREPARSED_DATA,
    };

    let handle = get_haptics_handle().context("Get touchpad HID handle")?;

    // Query output report byte length from the device's HID descriptor once and
    // cache it: the per-step haptic tick calls this path up to ~25×/s and the
    // preparsed-data round trip is pure overhead after the first write.
    let cached_len = OUTPUT_REPORT_LEN.load(Ordering::Relaxed) as usize;
    let output_len = if cached_len > 0 {
        cached_len
    } else {
        unsafe {
            // SAFETY: HidD_GetPreparsedData allocates a preparsed data buffer associated with the
            // HID handle. If it succeeds, we query the caps and immediately free the buffer.
            let mut preparsed = PHIDP_PREPARSED_DATA(0);
            let mut caps = HIDP_CAPS::default();
            if HidD_GetPreparsedData(handle, &mut preparsed).as_bool() && preparsed.0 != 0 {
                let _ = HidP_GetCaps(preparsed, &mut caps);
                log::debug!(
                    "[touchpad] HID caps: OutputReportByteLength={}, InputReportByteLength={}, FeatureReportByteLength={}",
                    caps.OutputReportByteLength,
                    caps.InputReportByteLength,
                    caps.FeatureReportByteLength,
                );
                HidD_FreePreparsedData(preparsed);
            }
            caps.OutputReportByteLength as usize
        }
    };
    if output_len > 0 {
        OUTPUT_REPORT_LEN.store(output_len as u32, Ordering::Relaxed);
    }

    // The output report length includes the Report ID byte at position 0.
    // BLTP7853 COL05 has OutputReportByteLength=33 (1 byte ID + 32 bytes data).
    let report_len = if output_len > 0 {
        output_len
    } else {
        // Fallback: use the standard 33-byte output report size.
        33
    };

    // Build the full output report buffer.
    let mut report = [0u8; 64]; // Stack allocation — no heap allocation
    let copy_len = report_data.len().min(report.len());
    report[..copy_len].copy_from_slice(&report_data[..copy_len]);

    let ok = unsafe {
        // SAFETY: report is a valid array owned by this function; HidD_SetOutputReport copies
        // the report data into the HID stack and does not retain the pointer.
        HidD_SetOutputReport(handle, report.as_mut_ptr() as *mut _, report_len as u32).as_bool()
    };

    // Always close the handle after each write — SvrCModule.dll disconnects
    // the device after every operation (see svrc_log.txt "HandleDeviceDisconnected").
    // Keeping a stale handle causes subsequent writes to fail silently.
    close_haptics_handle();

    if ok {
        log_output_report(
            "[touchpad] BLTP7853 output report sent",
            &report_data[..copy_len],
        );
        Ok(())
    } else {
        // Retry once with a fresh handle
        let handle = get_haptics_handle().context("Reopen touchpad HID handle after failure")?;

        let ok = unsafe {
            HidD_SetOutputReport(handle, report.as_mut_ptr() as *mut _, report_len as u32).as_bool()
        };

        close_haptics_handle();

        if ok {
            log_output_report(
                "[touchpad] BLTP7853 output report sent (retry)",
                &report_data[..copy_len],
            );
            Ok(())
        } else {
            let err = unsafe {
                // SAFETY: GetLastError is a lightweight call that returns the thread's last-error
                // code; it always succeeds and requires no special state.
                windows::Win32::Foundation::GetLastError()
            };
            Err(HardwareError::Touchpad(format!(
                "HidD_SetOutputReport BLTP7853 COL05 (retry): {err:?}"
            )))
        }
    }
}

/// Motor gear values for vibration intensity, verified against Xiaomi's
/// SvrCModule.dll disassembly (WriteMotorGears) and confirmed on hardware by
/// Oksion (issue #106):
///   mode 1 (Low)    → gears (56, 80)
///   mode 2 (Medium) → gears (80, 104)
///   mode 3 (High)   → gears (104, 128)  [was (104, 104) — hardware-verified
///                      wrong; the firmware ignores the second short when it
///                      equals the first]
/// The two shorts are sent as a 4-byte payload with report ID 0x5D.
#[cfg(windows)]
fn motor_gears(intensity: &HapticsIntensity) -> (u16, u16) {
    match intensity {
        HapticsIntensity::Low => (56, 80),
        HapticsIntensity::Medium => (80, 104),
        HapticsIntensity::High => (104, 128),
    }
}

/// Force threshold shorts for pressing sensitivity, verified against Xiaomi's
/// SvrCModule.dll disassembly + svrc log hexdump:
///
///   log hexdump for mode 2 (Medium): `"12039500400"` →
///   `12 03 95 00 40 0?` = payload shorts (120, 39, 500, 400) — i.e.
///   `OpenWriteForceThreshold(threshold, scaled, max1=500, max2=400)`.
///
///   mode 1 (Light)  → threshold 100, scaled = (int)(100*0.33) = 33
///   mode 2 (Medium) → threshold 120, scaled = (int)(120*0.33) = 39
///   mode 3 (Firm)   → threshold 140, scaled = (int)(140*0.33) = 46
///
/// S50 fix: the previous payload `(scaled, 400, 0, 0)` dropped the raw
/// threshold AND zeroed the 500 param — the device rejected/clamped the
/// report, making the sensitivity slider a silent no-op. Now we send the
/// full 4-short payload `(threshold, scaled, 500, 400)` per the log.
#[cfg(windows)]
fn force_threshold_shorts(sensitivity: &TouchpadSensitivity) -> [u16; 4] {
    let threshold: u16 = match sensitivity {
        TouchpadSensitivity::Low => 100,
        TouchpadSensitivity::Medium => 120,
        // High and VeryHigh both map to the firm threshold (no separate value).
        TouchpadSensitivity::High | TouchpadSensitivity::VeryHigh => 140,
    };
    let scaled = (threshold as f64 * 0.33) as u16; // 33 / 39 / 46
    [threshold, scaled, 500, 400]
}

/// Build a 33-byte BLTP7853 output report frame.
///
/// Protocol verified on TM2424 hardware by Oksion (XiControl maintainer,
/// GitHub issue #106, 2026-10-25) by reading values back after each write:
/// - byte 0: fixed 0x0D magic
/// - byte 1: payload_len + 7
/// - byte 2: checksum = XOR of bytes [3..31] + 1 — covers the phase flag,
///   length, command and payload (NOT just the payload as previously
///   believed; the old formula produced frames the firmware ignored)
/// - byte 3: phase flag — FIRST frame 0x00, COMMIT frame 0x01 (inverted
///   relative to the old assumption)
/// - byte 5: payload_len + 1
/// - byte 8: report ID
/// - bytes 9..: payload
///
/// A write is TWO frames: the command frame, then a COMMIT frame
/// (cmd 0x01, payload `[cmd, 0x100 - cmd]`) — without the commit frame the
/// firmware does not apply the write. ~120 ms between frames (at ~10 ms the
/// readback is all zeros).
///
/// Example (vibration Medium, 0x5D (80,104)):
///   write : 0D 0B 61 00 00 05 00 00 5D 50 00 68 00
///   commit: 0D 09 FE 01 00 03 00 00 01 5D A3
#[cfg(windows)]
fn build_touchpad_report(report_id: u8, payload: &[u8], commit: bool) -> [u8; 33] {
    let mut r = [0u8; 33];
    r[0] = 0x0D;
    r[1] = payload.len() as u8 + 7;
    r[8] = report_id;
    let copy_len = payload.len().min(24); // max payload region
    r[9..9 + copy_len].copy_from_slice(&payload[..copy_len]);
    // Phase flag BEFORE the checksum: the checksum covers bytes [3..31].
    r[3] = if commit { 0x01 } else { 0x00 };
    r[5] = payload.len() as u8 + 1;
    // Checksum: XOR of bytes [3..31] + 1 (phase flag, len, cmd, payload).
    let mut cks = 0u8;
    for &b in &r[3..31] {
        cks ^= b;
    }
    r[2] = cks.wrapping_add(1);
    r
}

/// Build the COMMIT frame that makes the firmware apply the preceding write.
///
/// Verified on hardware (issue #106): cmd 0x01, payload `[cmd, 0x100 - cmd]`
/// where `cmd` is the report ID of the write being committed. Example for
/// 0x5D: `0D 09 FE 01 00 03 00 00 01 5D A3`.
#[cfg(windows)]
fn build_commit_frame(cmd: u8) -> [u8; 33] {
    let payload = [cmd, (0x100u16.wrapping_sub(cmd as u16)) as u8];
    build_touchpad_report(0x01, &payload, true)
}

/// Send a command frame followed by its commit frame, with the inter-frame
/// delay verified on hardware (~120 ms; at ~10 ms the readback is all zeros).
#[cfg(windows)]
fn send_touchpad_command(report_id: u8, payload: &[u8]) -> HardwareResult<()> {
    let write = build_touchpad_report(report_id, payload, false);
    send_touchpad_output_report(&write)?;
    std::thread::sleep(std::time::Duration::from_millis(120));
    let commit = build_commit_frame(report_id);
    send_touchpad_output_report(&commit)?;
    Ok(())
}

/// Send heavy-press (repress / second-tap) enable/disable via HID output
/// report 0x59 `WriteEnableHeavyPress` — replicated from Xiaomi's
/// SvrCModule.dll. Issue #106 (hardware-verified): the payload is ONE
/// LITTLE-ENDIAN SHORT (01 00 = on, 02 00 = off), not a single byte, and the
/// write must be followed by the commit frame.
#[cfg(windows)]
fn send_heavy_press_hid_report(enabled: bool) -> HardwareResult<()> {
    let value: u16 = if enabled { 0x01 } else { 0x02 };
    let heavy_payload = value.to_le_bytes(); // 01 00 / 02 00
    send_touchpad_command(0x59, &heavy_payload)?;
    log::info!("[touchpad] WriteEnableHeavyPress: enabled={}", enabled);
    Ok(())
}

/// Send haptic vibration intensity via HID output report 0x5D
/// `WriteMotorGears` (2 LE shorts gear pair + commit frame) — replicated
/// from Xiaomi's SvrCModule.dll. Issue #106 (hardware-verified): High is
/// (104, 128), not (104, 104).
#[cfg(windows)]
fn send_motor_gears_hid_report(intensity: &HapticsIntensity) -> HardwareResult<()> {
    let (g1, g2) = motor_gears(intensity);
    let mut motor_payload = [0u8; 4];
    motor_payload[..2].copy_from_slice(&g1.to_le_bytes());
    motor_payload[2..].copy_from_slice(&g2.to_le_bytes());
    send_touchpad_command(0x5D, &motor_payload)?;

    log::info!(
        "[touchpad] WriteMotorGears: intensity={:?} gears=({},{})",
        intensity,
        g1,
        g2
    );

    Ok(())
}

/// Send pressing sensitivity (force threshold) via HID output report.
///
/// This replicates Xiaomi's SvrCModule.dll call chain (verified by reverse
/// engineering):
///   `OpenWriteForceThreshold(a, b, c, d)` + `WriteForceGears` → report ID
///   0x5B, payload = 4 × little-endian shorts (scaled threshold + params).
#[cfg(windows)]
fn send_force_threshold_hid_report(sensitivity: &TouchpadSensitivity) -> HardwareResult<()> {
    let shorts = force_threshold_shorts(sensitivity);
    let mut payload = [0u8; 8];
    for (i, s) in shorts.iter().enumerate() {
        payload[i * 2..i * 2 + 2].copy_from_slice(&s.to_le_bytes());
    }
    send_touchpad_command(0x5B, &payload)?;

    log::info!(
        "[touchpad] WriteForceGears: sensitivity={:?} shorts={:?}",
        sensitivity,
        shorts
    );

    Ok(())
}

// ─── Windows Precision Touchpad (PTP) standard sensitivity ───────────────────

/// Write the Windows inbox PTP sensitivity registry value so the OS driver
/// picks up the change immediately.
///
/// Path:  HKCU\Software\Microsoft\Windows\CurrentVersion\PrecisionTouchPad
/// Value: Sensitivity  REG_DWORD   1=Low  2=MedLow  3=Medium  4=MedHigh  5=High
#[cfg(windows)]
fn set_windows_ptp_sensitivity(sensitivity: &TouchpadSensitivity) {
    use winreg::{
        enums::{HKEY_CURRENT_USER, KEY_WRITE},
        RegKey,
    };
    let val: u32 = match sensitivity {
        TouchpadSensitivity::Low => 1,
        TouchpadSensitivity::Medium => 3,
        TouchpadSensitivity::High => 4,
        TouchpadSensitivity::VeryHigh => 5,
    };
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    if let Ok((key, _)) = hkcu.create_subkey_with_flags(
        r"Software\Microsoft\Windows\CurrentVersion\PrecisionTouchPad",
        KEY_WRITE,
    ) {
        let _ = key.set_value("Sensitivity", &val);
    }
}

// ─── Gesture listener lifecycle ───────────────────────────────────────────────

/// Start the background gesture listener thread (once).
/// Uses compare_exchange so concurrent callers are safe.
#[cfg(windows)]
fn ensure_gesture_listener() {
    if GESTURE_THREAD_STARTED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return; // Already running.
    }
    if let Err(e) = std::thread::Builder::new()
        .name("mi-gesture".into())
        .spawn(|| unsafe { win_gesture_loop() })
    {
        log::error!("[gesture] Failed to spawn gesture thread: {e}");
        GESTURE_THREAD_STARTED.store(false, Ordering::SeqCst);
    }
}

// ─── Raw Input gesture loop ───────────────────────────────────────────────────

/// Raw input gesture listener thread loop.
///
/// # Safety
///
/// Must be called on a dedicated thread. Creates a message-only window and
/// registers for Precision Touchpad raw input (Usage Page 0x000D, Usage 0x0005).
/// The window procedure receives WM_INPUT messages and dispatches them to
/// `process_raw_input`. This function runs an infinite message pump and never
/// returns until the window is destroyed.
#[cfg(windows)]
unsafe fn win_gesture_loop() {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HINSTANCE;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    use windows::Win32::UI::Input::{RegisterRawInputDevices, RAWINPUTDEVICE, RIDEV_INPUTSINK};
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DispatchMessageW, GetMessageW, RegisterClassExW, TranslateMessage,
        HWND_MESSAGE, MSG, WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSEXW,
    };

    // Initialise COM so brightness calls (WMI/IGCL) work from this thread.
    let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

    // Register window class.
    let class_name: Vec<u16> = "MiControlGesture\0".encode_utf16().collect();
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        lpfnWndProc: Some(gesture_wnd_proc),
        hInstance: HINSTANCE::default(),
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..Default::default()
    };
    RegisterClassExW(&wc);

    // Create a message-only window (HWND_MESSAGE parent → invisible, no taskbar entry).
    let hwnd = match CreateWindowExW(
        WINDOW_EX_STYLE::default(),
        PCWSTR(class_name.as_ptr()),
        PCWSTR::null(),
        WINDOW_STYLE::default(),
        0,
        0,
        0,
        0,
        HWND_MESSAGE,
        None,
        None,
        None,
    ) {
        Ok(h) => h,
        Err(e) => {
            log::error!("[gesture] Failed to create message-only window: {e}");
            GESTURE_THREAD_STARTED.store(false, Ordering::SeqCst);
            return;
        }
    };

    // Register for Precision Touchpad raw input (Usage Page 0x000D, Usage 0x0005).
    // RIDEV_INPUTSINK: receive input even when our window is not in the foreground.
    let rid = RAWINPUTDEVICE {
        usUsagePage: 0x000D,
        usUsage: 0x0005,
        dwFlags: RIDEV_INPUTSINK,
        hwndTarget: hwnd,
    };

    if let Err(e) = RegisterRawInputDevices(&[rid], std::mem::size_of::<RAWINPUTDEVICE>() as u32) {
        log::warn!(
            "[gesture] RegisterRawInputDevices failed: {e}. \
             Gesture detection will be unavailable."
        );
    } else {
        log::info!("[gesture] Raw Input gesture listener active");
    }

    // Pointer confinement is per-gesture (ClipCursor in `pin_cursor`), not a
    // permanent hook: see the rationale on that function. Nothing to install
    // here, but the listener owns the pin lifecycle, so drop any stale pin left
    // behind by a previous session before the message pump starts.
    release_cursor_pin("listener start");

    // Message pump — WM_INPUT messages are delivered here.
    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }

    // The thread is going away: a live ClipCursor would leave the desktop
    // cursor stuck, so release it before unwinding.
    release_cursor_pin("listener stop");
    log::info!("[gesture] Gesture listener stopped");
}

/// Window procedure for the gesture message-only window.
///
/// # Safety
///
/// Must be registered via RegisterClassExW and called by the Windows message dispatcher.
/// Standard window procedure signature; receives WM_INPUT and forwards to process_raw_input.
#[cfg(windows)]
unsafe extern "system" fn gesture_wnd_proc(
    hwnd: windows::Win32::Foundation::HWND,
    msg: u32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::UI::WindowsAndMessaging::{DefWindowProcW, WM_INPUT};
    if msg == WM_INPUT {
        process_raw_input(lparam.0);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

// ─── Gesture state (gesture thread — thread-local) ────────────────────────────

#[cfg(windows)]
struct GestureState {
    /// Logical X maximum read from HID caps (for edge-zone threshold).
    x_max: i32,
    /// Logical Y maximum read from HID caps (for step calculation).
    y_max: i32,
    caps_read: bool,

    // 5-finger screenshot
    five_start: Option<std::time::Instant>,
    screenshot_cooldown: Option<std::time::Instant>,

    // Edge slide
    edge: Option<EdgeSlideState>,
    /// Whether a single-finger contact session is currently active.
    edge_contact_active: bool,
    /// Side where the current contact session started (None = center start).
    edge_contact_start_side: Option<EdgeSide>,
    /// Consecutive frames without a valid single-finger contact.
    /// Used to preserve edge state across brief contact losses (palm rejection).
    edge_contact_lost_frames: u8,
    /// The handler asks for the pointer to be confined on the frame that
    /// captures the gesture. The caller performs `ClipCursor` **after** the
    /// RefCell borrow is released, so the state machine stays pure (and unit
    /// tests can run it without touching the real cursor).
    edge_want_pin: bool,
    /// The handler asks for the confinement to be dropped (gesture ended, finger
    /// lifted, zone left, feature disabled).
    edge_want_release: bool,
    /// Haptic ticks the caller must emit for this frame (one per logical step,
    /// capped at 4). The original sends exactly one per step.
    edge_ticks: u8,
}

#[cfg(windows)]
impl Default for GestureState {
    fn default() -> Self {
        Self {
            x_max: 10000,
            y_max: 7000,
            caps_read: false,
            five_start: None,
            screenshot_cooldown: None,
            edge: None,
            edge_contact_active: false,
            edge_contact_start_side: None,
            edge_contact_lost_frames: 0,
            edge_want_pin: false,
            edge_want_release: false,
            edge_ticks: 0,
        }
    }
}

#[cfg(windows)]
struct EdgeSlideState {
    side: EdgeSide,
    /// X at gesture/session start, used to infer vertical-intent dominance.
    start_x: i32,
    /// Y at gesture/session start, used to infer vertical-intent dominance.
    start_y: i32,
    /// Last X sample for drift tracking.
    last_x: i32,
    last_y: i32,
    /// Accumulated Y delta waiting to reach the next action threshold.
    accum: i32,
    /// False while we are only "armed"; true after intentional edge-slide capture.
    captured: bool,
    /// Consecutive WM_INPUT frames where contact_count != 1.
    /// Edge state is preserved across brief gaps (palm rejection artefacts).
    lost_frames: u8,
    /// Consecutive frames with X outside the continuation zone. A single
    /// out-of-zone sample is usually a slot shuffle in the HID report, not the
    /// finger leaving the edge, so the gesture only dies after a short run.
    out_of_zone_frames: u8,
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EdgeSide {
    Left,
    Right,
}

#[cfg(windows)]
thread_local! {
    /// Per-device preparsed HID data cache, keyed by HANDLE numeric value.
    static PREPARSED_CACHE:
        std::cell::RefCell<std::collections::HashMap<usize, std::rc::Rc<Vec<u8>>>> =
            std::cell::RefCell::new(std::collections::HashMap::new());
    /// Per-device "is this the touchpad?" decision cache.
    static TOUCHPAD_DEVICE_CACHE: std::cell::RefCell<std::collections::HashMap<usize, bool>> =
        std::cell::RefCell::new(std::collections::HashMap::new());

    static GESTURE_STATE: std::cell::RefCell<GestureState> =
        std::cell::RefCell::new(GestureState::default());
}

// ─── Raw Input processor ──────────────────────────────────────────────────────

#[cfg(windows)]
fn normalize_hid_path(path: &str) -> String {
    let mut p = path
        .trim_matches('\0')
        .replace('/', "\\")
        .to_ascii_lowercase();
    if let Some(stripped) = p.strip_prefix(r"\??\") {
        p = format!(r"\\?\{stripped}");
    }
    p
}

/// Returns the hardware identifier segment from a HID path.
/// Example: "\\\\?\\hid#bltp7853&col04#...#..." -> "bltp7853".
#[cfg(windows)]
fn hid_hardware_key(path: &str) -> Option<String> {
    let normalized = normalize_hid_path(path);
    let mut parts = normalized.split('#');
    let _prefix = parts.next()?;
    let hardware = parts.next()?;
    Some(
        hardware
            .split("&col")
            .next()
            .unwrap_or(hardware)
            .to_string(),
    )
}

#[cfg(windows)]
fn hid_vid_pid_key(path: &str) -> Option<String> {
    let normalized = normalize_hid_path(path);
    let mut parts = normalized.split('#');
    let _prefix = parts.next()?;
    let hardware = parts.next()?.to_ascii_lowercase();
    let vid_pos = hardware.find("vid_")?;
    let pid_pos = hardware.find("pid_")?;
    let vid = hardware.get(vid_pos..vid_pos.saturating_add(8))?;
    let pid = hardware.get(pid_pos..pid_pos.saturating_add(8))?;
    Some(format!("{vid}&{pid}"))
}

#[cfg(windows)]
fn hid_instance_root(path: &str) -> Option<String> {
    let normalized = normalize_hid_path(path);
    let mut parts = normalized.split('#');
    let _prefix = parts.next()?;
    let _hardware = parts.next()?;
    let instance = parts.next()?.to_ascii_lowercase();
    if instance.contains("&0&") {
        if let Some((root, _tail)) = instance.rsplit_once('&') {
            return Some(root.to_string());
        }
    }
    Some(instance)
}

#[cfg(windows)]
fn is_touchpad_device_path(raw_input_path: &str, touchpad_path: &str) -> bool {
    let raw_norm = normalize_hid_path(raw_input_path);
    let tp_norm = normalize_hid_path(touchpad_path);
    if raw_norm == tp_norm {
        return true;
    }
    if let (Some(raw_vidpid), Some(tp_vidpid)) =
        (hid_vid_pid_key(&raw_norm), hid_vid_pid_key(&tp_norm))
    {
        let raw_root = hid_instance_root(&raw_norm);
        let tp_root = hid_instance_root(&tp_norm);
        if raw_vidpid == tp_vidpid && raw_root.is_some() && raw_root == tp_root {
            return true;
        }
    }
    match (hid_hardware_key(&raw_norm), hid_hardware_key(&tp_norm)) {
        (Some(raw_key), Some(tp_key)) => raw_key == tp_key,
        _ => false,
    }
}

/// Query the device name for a raw input device handle.
///
/// # Safety
///
/// `hdevice` must be a valid HANDLE returned by the WM_INPUT/RAWINPUTHEADER.
/// The buffer length is validated (max 1024 wide chars) to prevent overreads.
#[cfg(windows)]
unsafe fn query_raw_input_device_name(
    hdevice: windows::Win32::Foundation::HANDLE,
) -> Option<String> {
    use windows::Win32::UI::Input::{GetRawInputDeviceInfoW, RIDI_DEVICENAME};

    let mut name_len: u32 = 0;
    let _ = GetRawInputDeviceInfoW(hdevice, RIDI_DEVICENAME, None, &mut name_len);
    if name_len == 0 || name_len > 1024 {
        return None;
    }
    let mut name_buf = vec![0u16; name_len as usize];
    let ret = GetRawInputDeviceInfoW(
        hdevice,
        RIDI_DEVICENAME,
        Some(name_buf.as_mut_ptr() as *mut _),
        &mut name_len,
    );
    if ret == u32::MAX || ret == 0 {
        return None;
    }
    let len = name_buf
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(ret as usize);
    Some(String::from_utf16_lossy(&name_buf[..len]))
}

/// One contact slot selected from a HID report.
#[cfg(windows)]
#[derive(Debug, Clone, Copy)]
struct ContactPick {
    /// Value of the Contact Count usage (0x000D/0x0054).
    count: u32,
    x: i32,
    y: i32,
    /// TipSwitch (0x0042) of the *selected* slot.
    tip: bool,
    lc: u16,
    /// How many slots carried a plausible (in-range) coordinate.
    candidates: u8,
}

/// Pick the contact that should drive the gesture state machine.
///
/// The BLTP7853 keeps up to five contact slots in one report and leaves stale
/// coordinates in the unused ones. The old code took the first slot with
/// `x>0 && y>0`, which produced the audit's impossible frames: a left-edge
/// gesture whose next sample read `x=3240` (right edge) and ended, then
/// re-armed in the middle of the pad, 3 167 times.
///
/// Selection order:
///   1. only slots whose coordinates are inside the logical range;
///   2. prefer slots whose TipSwitch is set (finger actually down);
///   3. among those, the slot closest to `prev` (temporal continuity).
///
/// `prev` is `None` when no gesture is in flight, in which case the first
/// touching slot wins.
#[cfg(windows)]
fn select_contact(
    preparsed: windows::Win32::Devices::HumanInterfaceDevice::PHIDP_PREPARSED_DATA,
    report: &[u8],
    x_max: i32,
    y_max: i32,
    prev: Option<(i32, i32)>,
) -> ContactPick {
    use windows::Win32::Devices::HumanInterfaceDevice::{
        HidP_GetUsageValue, HidP_GetUsages, HidP_Input,
    };

    let mut count: u32 = 0;
    let _ =
        unsafe { HidP_GetUsageValue(HidP_Input, 0x000D, 0, 0x0054, &mut count, preparsed, report) };

    let mut best: Option<ContactPick> = None;
    let mut best_score = i64::MAX;
    let mut candidates: u8 = 0;

    for lc in 1u16..=5 {
        let mut x_val: u32 = 0;
        let mut y_val: u32 = 0;
        // X/Y live in the Generic Desktop page (0x0001), not Digitizer.
        let rx = unsafe {
            HidP_GetUsageValue(
                HidP_Input, 0x0001, lc, 0x0030, &mut x_val, preparsed, report,
            )
        };
        let ry = unsafe {
            HidP_GetUsageValue(
                HidP_Input, 0x0001, lc, 0x0031, &mut y_val, preparsed, report,
            )
        };
        if rx.is_err() || ry.is_err() {
            continue;
        }
        let (x, y) = (x_val as i32, y_val as i32);
        // 0/0 is an empty slot; anything outside the logical range is an
        // EMI-corrupted or foreign-usage value and must not be a position.
        if x <= 0 || y <= 0 || x > x_max || y > y_max {
            continue;
        }
        candidates = candidates.saturating_add(1);

        let mut usage_buf = [0u16; 8];
        let mut usage_len: u32 = usage_buf.len() as u32;
        let mut report_mut = report.to_vec();
        let tip = unsafe {
            HidP_GetUsages(
                HidP_Input,
                0x000D, // Digitizer page — TipSwitch and Confidence live here
                lc,
                usage_buf.as_mut_ptr(),
                &mut usage_len,
                preparsed,
                &mut report_mut,
            )
        }
        .is_ok()
            && usage_buf[..(usage_len.min(8) as usize)].contains(&0x0042);

        let continuity = match prev {
            Some((px, py)) => i64::from((x - px).abs()) + i64::from((y - py).abs()),
            None => 0,
        };
        // A lifted slot loses to any touching slot, however far away.
        let score = continuity + if tip { 0 } else { 1_000_000 };

        if score < best_score {
            best_score = score;
            best = Some(ContactPick {
                count,
                x,
                y,
                tip,
                lc,
                candidates,
            });
        }
    }

    match best {
        Some(mut pick) => {
            pick.candidates = candidates;
            pick
        }
        None => ContactPick {
            count,
            x: 0,
            y: 0,
            tip: false,
            lc: 0,
            candidates: 0,
        },
    }
}

/// Process a single WM_INPUT message from the Precision Touchpad.
///
/// # Safety
///
/// `lparam` must be a valid HRAWINPUT handle from a WM_INPUT message.
/// The function reads from preparsed HID data cached per device and from
/// the raw input buffer. Buffer size is validated against RAWINPUTHEADER.
#[cfg(windows)]
unsafe fn process_raw_input(lparam: isize) {
    use windows::Win32::Devices::HumanInterfaceDevice::{
        HidP_GetSpecificValueCaps, HidP_Input, HIDP_VALUE_CAPS, PHIDP_PREPARSED_DATA,
    };
    use windows::Win32::UI::Input::{
        GetRawInputData, GetRawInputDeviceInfoW, HRAWINPUT, RAWINPUT, RAWINPUTHEADER,
        RIDI_PREPARSEDDATA, RID_INPUT,
    };

    // ── Get required buffer size ──────────────────────────────────────────────
    let mut size: u32 = 0;
    let r = GetRawInputData(
        HRAWINPUT(lparam as *mut _),
        RID_INPUT,
        None,
        &mut size,
        std::mem::size_of::<RAWINPUTHEADER>() as u32,
    );
    if r == u32::MAX || size == 0 || size > 4096 {
        return;
    }

    // ── Fetch the raw input struct ────────────────────────────────────────────
    let mut buf = [0u8; 4096]; // Stack allocation — no heap allocation per frame
    let written = GetRawInputData(
        HRAWINPUT(lparam as *mut _),
        RID_INPUT,
        Some(buf.as_mut_ptr() as *mut _),
        &mut size,
        std::mem::size_of::<RAWINPUTHEADER>() as u32,
    );
    if written == u32::MAX || written == 0 {
        return;
    }

    // Validate buffer is large enough to contain a RAWINPUT struct
    if (written as usize) < std::mem::size_of::<RAWINPUT>() {
        return;
    }

    let raw = buf.as_ptr() as *const RAWINPUT;

    // Only handle HID (touch) reports — RIM_TYPEHID = 2.
    if (*raw).header.dwType != 2 {
        return;
    }

    let device_key = (*raw).header.hDevice.0 as usize;

    // Hard filter: process gesture data only from the physical touchpad.
    // This prevents touchscreen/stylus/raw digitizer packets from entering
    // the edge-slide state machine and disturbing pointer behavior.
    let is_touchpad = TOUCHPAD_DEVICE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(cached) = cache.get(&device_key) {
            return *cached;
        }
        let raw_name = query_raw_input_device_name((*raw).header.hDevice);
        let expected_touchpad = touchpad_hid_path();
        let Some(name) = raw_name.as_deref() else {
            // Device name query may transiently fail. Be conservative: ignore
            // this frame to avoid accepting packets from the wrong HID source.
            // Do not cache this result so a later successful query can recover.
            log::warn!(target: "hw::touchpad", "raw input device name query failed for device_key={device_key}");
            return false;
        };
        let matched = is_touchpad_device_path(name, &expected_touchpad);
        cache.insert(device_key, matched);
        log::debug!(
            target: "hw::touchpad",
            "raw input device filter: key={} matched={} raw={} expected={}",
            device_key,
            matched,
            name,
            expected_touchpad
        );
        if !matched {
            log::debug!("[gesture] Ignoring non-touchpad raw device: {name}");
        }
        matched
    });
    if !is_touchpad {
        return;
    }

    // If edge-slide has been disabled, force-release the pointer confinement and
    // clear any stale in-progress session state.
    if !EDGE_SLIDE_ENABLED.load(Ordering::Relaxed) {
        EDGE_GESTURE_ACTIVE.store(false, Ordering::Relaxed);
        GESTURE_STATE.with(|state| {
            let mut s = state.borrow_mut();
            s.edge = None;
            s.edge_contact_active = false;
            s.edge_contact_start_side = None;
            s.edge_contact_lost_frames = 0;
            s.edge_want_pin = false;
            s.edge_want_release = false;
            s.edge_ticks = 0;
        });
        release_cursor_pin("edge-slide disabled");
    }

    // ── Get/cache preparsed data for this device ──────────────────────────────
    let pp_data = PREPARSED_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        let pp_buf = cache.entry(device_key).or_insert_with(|| {
            let mut pp_size: u32 = 0;
            GetRawInputDeviceInfoW(
                (*raw).header.hDevice,
                RIDI_PREPARSEDDATA,
                None,
                &mut pp_size,
            );
            if pp_size == 0 || pp_size > 65536 {
                return std::rc::Rc::new(Vec::new());
            }
            let mut pp_buf = vec![0u8; pp_size as usize];
            let ret = GetRawInputDeviceInfoW(
                (*raw).header.hDevice,
                RIDI_PREPARSEDDATA,
                Some(pp_buf.as_mut_ptr() as *mut _),
                &mut pp_size,
            );
            if ret == u32::MAX {
                return std::rc::Rc::new(Vec::new());
            }
            std::rc::Rc::new(pp_buf)
        });
        if pp_buf.is_empty() {
            None
        } else {
            Some(std::rc::Rc::clone(pp_buf))
        }
    });

    let pp_rc = match pp_data {
        Some(rc) => rc,
        None => return,
    };

    let preparsed = PHIDP_PREPARSED_DATA(pp_rc.as_ptr() as isize);

    // ── Extract the HID report bytes ──────────────────────────────────────────
    // Harden dwSizeHid: validate against the actual raw input buffer size
    // to prevent out-of-bounds reads if the driver reports a bogus length.
    let hid = &(*raw).data.hid;
    if hid.dwSizeHid == 0 || hid.dwCount == 0 {
        return;
    }
    // bRawData is the flexible array at the end of RAWHID; compute the safe
    // available length from the overall buffer size minus the header offset.
    let raw_data_offset = (hid.bRawData.as_ptr() as usize) - (buf.as_ptr() as usize);
    let safe_len = buf
        .len()
        .saturating_sub(raw_data_offset)
        .min(hid.dwSizeHid as usize);
    if safe_len == 0 {
        log::debug!(
            target: "hw::touchpad",
            "rejecting raw touchpad frame: dwSizeHid={} but safe_len=0 (buf={}, offset={})",
            hid.dwSizeHid, buf.len(), raw_data_offset
        );
        return;
    }
    log::debug!(
        target: "hw::touchpad",
        "accepted raw touchpad frame: device_key={} size_hid={} count={} safe_len={}",
        device_key,
        hid.dwSizeHid,
        hid.dwCount,
        safe_len
    );
    let report = std::slice::from_raw_parts(hid.bRawData.as_ptr(), safe_len);

    // ── Read logical maxima once per device (for edge/step sizing) ────────────
    GESTURE_STATE.with(|state| {
        let mut s = state.borrow_mut();
        if !s.caps_read {
            let mut caps: HIDP_VALUE_CAPS = std::mem::zeroed();
            let mut caps_len: u16 = 1;
            // Y logical max — X/Y are in Generic Desktop (0x0001), not Digitizer (0x000D).
            // HidP_GetSpecificValueCaps returns BUFFER_TOO_SMALL when there are multiple
            // matching caps (one per contact slot). Ignore the error; the first cap is still
            // written to `caps` and `caps_len` is set to the total count (> 0).
            let _ = HidP_GetSpecificValueCaps(
                HidP_Input,
                0x0001,
                0,
                0x0031,
                &mut caps,
                &mut caps_len,
                preparsed,
            );
            if caps_len > 0 && caps.LogicalMax > 100 {
                s.y_max = caps.LogicalMax;
            }
            // X logical max
            caps_len = 1;
            let _ = HidP_GetSpecificValueCaps(
                HidP_Input,
                0x0001,
                0,
                0x0030,
                &mut caps,
                &mut caps_len,
                preparsed,
            );
            if caps_len > 0 && caps.LogicalMax > 100 {
                s.x_max = caps.LogicalMax;
            }
            s.caps_read = true;
            log::info!(
                "[gesture] Touchpad logical range: x_max={}, y_max={}",
                s.x_max,
                s.y_max
            );
        }
    });

    // ── Read the active contact (count + the right slot) ──────────────────────
    // The BLTP7853 reports up to five contact slots per report and keeps
    // stale coordinates in the unused ones. Reading "the first slot with
    // x>0 && y>0" (the old behaviour) meant a finger at the left edge could be
    // reported at the right edge on the next frame, because slot 1 and slot 2
    // had swapped — the audit's `ended` lines show x=3240 on a *left* gesture.
    // The picker below prefers a slot whose TipSwitch is set and, among those,
    // the one closest to the last processed position (continuity).
    let prev_point = GESTURE_STATE.with(|state| {
        state
            .borrow()
            .edge
            .as_ref()
            .map(|edge| (edge.last_x, edge.last_y))
    });
    let (x_max, y_max) = GESTURE_STATE.with(|state| {
        let s = state.borrow();
        (s.x_max, s.y_max)
    });
    let pick = select_contact(preparsed, report, x_max, y_max, prev_point);
    let contact_count = pick.count;
    let contact_active = pick.tip;
    log::trace!(
        target: "hw::touchpad",
        "contact pick: lc={} x={} y={} tip={} count={} candidates={}",
        pick.lc, pick.x, pick.y, pick.tip, pick.count, pick.candidates
    );

    // ── Feed gesture state machine ────────────────────────────────────────────
    // Each handler returns a value describing the action to perform so that
    // simulate_* calls happen OUTSIDE the GESTURE_STATE borrow.  This prevents
    // a double-borrow panic when simulate_brightness_*() dispatches COM messages
    // on this thread (WMI internally pumps the message queue), which would
    // otherwise re-enter the gesture loop while borrow_mut() is still held.
    let fire_screenshot = GESTURE_SCREENSHOT_ENABLED.load(Ordering::Relaxed)
        && GESTURE_STATE
            .with(|state| handle_five_finger_gesture(&mut state.borrow_mut(), contact_count));

    // (brightness_steps, volume_steps) — positive = up, negative = down
    let (brightness_steps, volume_steps) = if EDGE_SLIDE_ENABLED.load(Ordering::Relaxed) {
        GESTURE_STATE.with(|state| {
            handle_edge_slide(
                &mut state.borrow_mut(),
                contact_count,
                pick.x,
                pick.y,
                contact_active,
            )
        })
    } else {
        (0, 0)
    };

    // ── Drain per-frame side effects ──────────────────────────────────────────
    // Pin/release/tick are the only operations here that touch the desktop or
    // the HID device. They run with the GESTURE_STATE borrow released, for the
    // same reason the simulate_* calls do: a nested call can pump the message
    // loop and re-enter this function.
    let (want_pin, want_release, ticks) = GESTURE_STATE.with(|state| {
        let mut s = state.borrow_mut();
        let v = (s.edge_want_pin, s.edge_want_release, s.edge_ticks);
        s.edge_want_pin = false;
        s.edge_want_release = false;
        s.edge_ticks = 0;
        v
    });

    if want_release {
        release_cursor_pin("gesture ended");
    }
    if want_pin {
        pin_cursor();
    }
    for _ in 0..ticks {
        send_edge_slide_tick("edge step");
    }

    // Watchdog: if the terminal frame of a gesture never arrives (missed report,
    // device reset, Tauri shutdown mid-gesture) never leave the user without a
    // cursor — drop the confinement and reset the state machine so the next
    // touch can arm normally.
    if EDGE_GESTURE_ACTIVE.load(Ordering::Relaxed) && cursor_pin_expired() {
        release_cursor_pin("watchdog");
        EDGE_GESTURE_ACTIVE.store(false, Ordering::Relaxed);
        GESTURE_STATE.with(|state| {
            let mut s = state.borrow_mut();
            s.edge = None;
            s.edge_contact_start_side = None;
            s.edge_contact_active = false;
        });
    }

    // ── Execute actions — borrow fully released here ───────────────────────────
    if fire_screenshot {
        simulate_win_shift_s();
    }
    for _ in 0..brightness_steps {
        simulate_brightness_up();
    }
    for _ in 0..(-brightness_steps) {
        simulate_brightness_down();
    }
    for _ in 0..volume_steps {
        simulate_volume_up();
    }
    for _ in 0..(-volume_steps) {
        simulate_volume_down();
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::{
        handle_edge_slide, hid_hardware_key, hid_instance_root, hid_vid_pid_key,
        is_touchpad_device_path, normalize_hid_path,
    };

    // ── Issue #106: BLTP7853 frame protocol (hardware-verified by Oksion) ──

    #[cfg(windows)]
    #[test]
    fn issue106_write_frame_matches_verified_example() {
        // Vibration Medium, 0x5D (80, 104) — the exact frame Oksion verified
        // on TM2424 hardware: 0D 0B 61 00 00 05 00 00 5D 50 00 68 00
        let payload = [
            80u16.to_le_bytes()[0],
            80u16.to_le_bytes()[1],
            104u16.to_le_bytes()[0],
            104u16.to_le_bytes()[1],
        ];
        let f = super::build_touchpad_report(0x5D, &payload, false);
        assert_eq!(f[0], 0x0D, "magic");
        assert_eq!(f[1], 0x0B, "len = payload_len(4) + 7");
        assert_eq!(f[2], 0x61, "checksum XOR(bytes[3..31])+1");
        assert_eq!(f[3], 0x00, "phase flag: FIRST frame is 0x00");
        assert_eq!(f[5], 0x05, "payload_len + 1");
        assert_eq!(f[8], 0x5D, "report id");
        assert_eq!(&f[9..13], &[0x50, 0x00, 0x68, 0x00], "payload LE shorts");
    }

    #[cfg(windows)]
    #[test]
    fn issue106_commit_frame_matches_verified_example() {
        // Commit frame for 0x5D: 0D 09 FE 01 00 03 00 00 01 5D A3
        let f = super::build_commit_frame(0x5D);
        assert_eq!(f[0], 0x0D, "magic");
        assert_eq!(f[1], 0x09, "len = payload_len(2) + 7");
        assert_eq!(f[2], 0xFE, "checksum");
        assert_eq!(f[3], 0x01, "phase flag: COMMIT frame is 0x01");
        assert_eq!(f[5], 0x03, "payload_len + 1");
        assert_eq!(f[8], 0x01, "commit command id");
        assert_eq!(&f[9..11], &[0x5D, 0xA3], "payload [cmd, 0x100-cmd]");
    }

    #[cfg(windows)]
    #[test]
    fn issue106_checksum_covers_header_not_just_payload() {
        // The old (wrong) formula XORed only the payload bytes; the correct
        // one XORs bytes [3..31]. With payload [0x00, 0x00] the payload XOR
        // is 0 (old checksum would be 0x01); the correct one folds in
        // byte 5 (len+1 = 0x03) and byte 8 (cmd 0x59) → 0x03^0x59 = 0x5A,
        // +1 = 0x5B.
        let f = super::build_touchpad_report(0x59, &[0x00, 0x00], false);
        assert_eq!(
            f[2], 0x5B,
            "checksum must cover bytes [3..31], not just payload"
        );
    }

    #[cfg(windows)]
    #[test]
    fn issue106_heavy_press_payload_is_le_short() {
        // 0x59 payload is ONE LE short (01 00 on / 02 00 off), not 1 byte.
        // Build the frame and inspect the payload region.
        let on: u16 = 0x01;
        let f = super::build_touchpad_report(0x59, &on.to_le_bytes(), false);
        assert_eq!(&f[9..11], &[0x01, 0x00], "on = LE short 01 00");
        let off: u16 = 0x02;
        let f2 = super::build_touchpad_report(0x59, &off.to_le_bytes(), false);
        assert_eq!(&f2[9..11], &[0x02, 0x00], "off = LE short 02 00");
    }

    #[cfg(windows)]
    #[test]
    fn issue106_motor_gears_high_is_104_128() {
        // High was (104, 104) — hardware-verified wrong; it is (104, 128).
        assert_eq!(
            super::motor_gears(&super::HapticsIntensity::High),
            (104, 128)
        );
        assert_eq!(super::motor_gears(&super::HapticsIntensity::Low), (56, 80));
        assert_eq!(
            super::motor_gears(&super::HapticsIntensity::Medium),
            (80, 104)
        );
    }

    #[cfg(windows)]
    #[test]
    fn issue106_single_haptic_click_frame_checksum() {
        // Sanity check from PluginGesture.dll (edge-slider tick):
        // 0D 09 FD 01 00 03 00 00 02 32 CE — cmd 0x02, payload [0x32, 0xCE]
        // (strength + trailing byte; payload_len = 2 per bytes 1 and 5).
        let f = super::build_touchpad_report(0x02, &[0x32, 0xCE], true);
        assert_eq!(f[1], 0x09, "len = payload_len(2) + 7");
        assert_eq!(f[2], 0xFD, "checksum for the known click frame");
        assert_eq!(f[3], 0x01, "commit phase");
        assert_eq!(f[5], 0x03, "payload_len + 1");
        assert_eq!(f[8], 0x02, "cmd");
        assert_eq!(&f[9..11], &[0x32, 0xCE], "payload");
    }

    #[test]
    fn normalize_hid_path_handles_case_and_prefix() {
        let raw = r"\??\HID#BLTP7853&COL04#5&ABC#{4D1E55B2-F16F-11CF-88CB-001111000030}";
        let norm = normalize_hid_path(raw);
        assert!(norm.starts_with(r"\\?\hid#bltp7853"));
    }

    #[test]
    fn hid_hardware_key_strips_collection_suffix() {
        let path = r"\\?\hid#bltp7853&col04#5&abc#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        let key = hid_hardware_key(path).expect("hardware key");
        assert_eq!(key, "bltp7853");
    }

    #[test]
    fn hid_vid_pid_key_handles_mi_and_collection_variants() {
        let p1 =
            r"\\?\hid#vid_3151&pid_8888&mi_01&col05#7&abc#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        let p2 =
            r"\\?\hid#vid_3151&pid_8888&mi_00&col01#7&def#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        assert_eq!(hid_vid_pid_key(p1).as_deref(), Some("vid_3151&pid_8888"));
        assert_eq!(hid_vid_pid_key(p2).as_deref(), Some("vid_3151&pid_8888"));
    }

    #[test]
    fn hid_instance_root_ignores_collection_suffix() {
        let p = r"\\?\hid#vid_3151&pid_8888&mi_01&col05#7&5a6d3c2&0&0004#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        assert_eq!(hid_instance_root(p).as_deref(), Some("7&5a6d3c2&0"));
    }

    #[test]
    fn touchpad_path_match_accepts_same_device_different_collection() {
        let raw = r"\\?\hid#bltp7853&col01#5&abc#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        let touchpad = r"\\?\hid#bltp7853&col04#5&abc#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        assert!(is_touchpad_device_path(raw, touchpad));
    }

    #[test]
    fn touchpad_path_match_rejects_same_vid_pid_different_mi() {
        let raw =
            r"\\?\hid#vid_3151&pid_8888&mi_00&col01#7&abc#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        let touchpad =
            r"\\?\hid#vid_3151&pid_8888&mi_01&col05#7&def#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        assert!(!is_touchpad_device_path(raw, touchpad));
    }

    #[test]
    fn touchpad_path_match_accepts_same_vid_pid_same_instance_root() {
        let raw = r"\\?\hid#vid_3151&pid_8888&mi_01&col01#7&5a6d3c2&0&0000#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        let touchpad = r"\\?\hid#vid_3151&pid_8888&mi_01&col05#7&5a6d3c2&0&0004#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        assert!(is_touchpad_device_path(raw, touchpad));
    }

    #[test]
    fn touchpad_path_match_rejects_other_digitizer_hardware() {
        let raw = r"\\?\hid#elan2514&col01#7&def#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        let touchpad = r"\\?\hid#bltp7853&col04#5&abc#{4d1e55b2-f16f-11cf-88cb-001111000030}";
        assert!(!is_touchpad_device_path(raw, touchpad));
    }

    #[test]
    fn edge_slide_does_not_activate_if_touch_started_in_center() {
        let mut state = super::GestureState::default();
        // Touch starts in center.
        let r1 = super::handle_edge_slide(&mut state, 1, 5000, 4000, true);
        assert_eq!(r1, (0, 0));
        assert!(state.edge.is_none());
        // Same finger drifts into edge; still must remain normal pointer behavior.
        let r2 = super::handle_edge_slide(&mut state, 1, 200, 3800, true);
        assert_eq!(r2, (0, 0));
        assert!(state.edge.is_none());
    }

    #[test]
    fn edge_slide_activates_if_touch_started_in_edge() {
        let mut state = super::GestureState::default();
        let r = handle_edge_slide(&mut state, 1, 200, 4000, true);
        assert_eq!(r, (0, 0));
        assert!(state.edge.is_some());
        assert!(!state.edge.as_ref().expect("edge state").captured);
    }

    #[test]
    fn edge_slide_requires_the_eight_millimetre_pre_roll() {
        // PluginGesture.dll gates the gesture on |dy| > 0xFA = 250 units, which
        // on this pad (2124 units ≈ 68 mm) is ~8 mm. Nothing may commit before
        // that, no matter how long the finger rests in the band.
        let mut state = real_pad();
        let _ = handle_edge_slide(&mut state, 1, 200, 1_800, true);
        assert!(state.edge.is_some(), "must arm from the edge");

        // 240 units of travel: still armed, still not captured, pointer untouched.
        let (b, v) = handle_edge_slide(&mut state, 1, 200, 1_560, true);
        assert_eq!((b, v), (0, 0));
        assert!(!state.edge.as_ref().expect("edge").captured);
        assert!(
            !state.edge_want_pin,
            "the pointer must not be pinned before the gesture commits"
        );
        assert_eq!(state.edge_ticks, 0);

        // 260 units: commits, and does so with exactly one action + one tick.
        let (b, v) = handle_edge_slide(&mut state, 1, 200, 1_540, true);
        assert!(state.edge.as_ref().expect("edge").captured);
        assert_eq!((b, v), (1, 0), "capture emits exactly one brightness step");
        assert_eq!(state.edge_ticks, 1, "one haptic tick per step");
        assert!(state.edge_want_pin, "capture must ask for the pointer pin");
    }

    #[test]
    fn edge_slide_ignores_diagonal_travel_that_is_mostly_lateral() {
        // 300 units down but 400 units inward is a pan, not an edge swipe.
        let mut state = real_pad();
        let _ = handle_edge_slide(&mut state, 1, 200, 1_800, true);
        let (b, v) = handle_edge_slide(&mut state, 1, 700, 1_500, true);
        assert_eq!((b, v), (0, 0));
        assert!(!state.edge.as_ref().expect("edge").captured);
    }

    #[test]
    fn edge_slide_releases_the_pin_when_the_finger_leaves_the_band() {
        let mut state = real_pad();
        let _ = handle_edge_slide(&mut state, 1, 200, 1_800, true);
        let _ = handle_edge_slide(&mut state, 1, 200, 1_500, true); // capture
        assert!(state.edge_want_pin);
        state.edge_want_pin = false; // what the caller does after pinning

        // Continuation band is 28 % of 3282 = 919; walk out of it.
        let mut released = false;
        for x in [1_000, 1_200, 1_400, 1_600] {
            let _ = handle_edge_slide(&mut state, 1, x, 1_500, true);
            released |= state.edge_want_release;
        }
        assert!(released, "leaving the band must release the pointer");
        assert!(state.edge.is_none());
    }

    #[test]
    fn edge_slide_survives_drift_inside_the_continuation_band() {
        // Hysteresis: armed at 6 %, drifted to 20 % — outside the start band,
        // inside the continuation band. The gesture must live.
        let mut state = real_pad();
        let _ = handle_edge_slide(&mut state, 1, 200, 1_800, true);
        let _ = handle_edge_slide(&mut state, 1, 200, 1_500, true); // capture
        let _ = handle_edge_slide(&mut state, 1, 656, 1_490, true);
        assert!(
            state.edge.is_some(),
            "drift inside the continuation band must not end the gesture"
        );
    }

    #[test]
    fn edge_slide_slow_swipe_still_steps() {
        // 10 units/frame at 250 Hz ≈ 7 cm/s: deliberate but unhurried. The old
        // unconditional accumulator decay reached equilibrium at 8×10 = 80 <
        // y_step = 88, so the gesture would capture and then never emit a step.
        let mut state = real_pad();
        let _ = handle_edge_slide(&mut state, 1, 200, 1_800, true);

        let mut brightness_total = 0;
        let mut y = 1_800;
        for _ in 0..90 {
            y -= 10;
            let (b, _v) = handle_edge_slide(&mut state, 1, 200, y, true);
            brightness_total += b;
        }
        assert!(
            brightness_total >= 3,
            "a slow swipe must still produce steps (got {brightness_total})"
        );
    }

    #[test]
    fn edge_slide_still_captures_with_large_logical_range() {
        let mut state = super::GestureState {
            y_max: 65_535, // common PTP logical max
            x_max: 65_535,
            ..Default::default()
        };

        // Start in left-edge zone for this range.
        let _ = handle_edge_slide(&mut state, 1, 2_000, 32_000, true);
        assert!(state.edge.is_some());
        assert!(!state.edge.as_ref().expect("edge state").captured);

        // The pre-roll is capped at 900 units for absurd logical ranges, so this
        // 1 000-unit swipe must commit.
        let _ = handle_edge_slide(&mut state, 1, 2_000, 31_000, true);
        assert!(state.edge.as_ref().expect("edge state").captured);

        // Additional motion should generate at least one step.
        let (b, v) = handle_edge_slide(&mut state, 1, 2_000, 30_000, true);
        assert!(b > 0, "expected brightness step with high y_max");
        assert_eq!(v, 0);
    }

    // ── Field-geometry regressions (BLTP7853 on this laptop) ─────────────────
    //
    // x_max=3282 / y_max=2124 are the values the deployed build logged at
    // 13:26:38 (`Touchpad logical range: x_max=3282, y_max=2124`). The old
    // deadband was min(2% of height, y_step/2) = 42 units, while a normal
    // swipe at 250 Hz moves ~18 units per report: every real sample was
    // dropped as "noise", the accumulator never reached y_step=88, and the log
    // therefore contains 54 `captured` lines and ZERO `edge-left steps=` lines.

    fn real_pad() -> super::GestureState {
        super::GestureState {
            x_max: 3_282,
            y_max: 2_124,
            ..Default::default()
        }
    }

    #[test]
    fn edge_slide_real_geometry_emits_steps_at_250hz() {
        let mut state = real_pad();
        // Contact starts in the left edge zone (x < 16% of 3282 = 525).
        let _ = handle_edge_slide(&mut state, 1, 200, 1_800, true);
        assert!(state.edge.is_some(), "must arm from the edge");

        // 18 logical units per frame ≈ a 4 cm/s swipe at 250 Hz. The 8 mm
        // pre-roll (250 units) is crossed around frame 14; steps follow every
        // y_step = 88 units (~2.8 mm).
        let mut brightness_total = 0;
        let mut y = 1_800;
        for i in 0..80 {
            y -= 18;
            let (b, _v) = handle_edge_slide(&mut state, 1, 200 + (i % 3), y, true);
            brightness_total += b;
        }
        assert!(
            brightness_total > 0,
            "a normal swipe must produce brightness steps (got {brightness_total})"
        );
        assert!(
            state.edge.as_ref().expect("edge").captured,
            "the swipe must capture before it can step"
        );
    }

    #[test]
    fn edge_slide_survives_a_contact_slot_shuffle() {
        let mut state = real_pad();
        let _ = handle_edge_slide(&mut state, 1, 200, 1_800, true);
        // Capture with vertical intent (300 units > the 250-unit pre-roll).
        let _ = handle_edge_slide(&mut state, 1, 205, 1_500, true);
        assert!(state.edge.as_ref().expect("edge").captured);

        // One frame reads the *other* contact slot: x jumps to the right edge,
        // exactly what the deployed log recorded (`ended: x=3240/3282` on a
        // left-side gesture). This must re-anchor, not kill the gesture.
        let _ = handle_edge_slide(&mut state, 1, 3_240, 1_500, true);
        assert!(
            state.edge.is_some(),
            "a slot shuffle must not end the gesture"
        );

        let mut brightness_total = 0;
        let mut y = 1_500;
        for _ in 0..30 {
            y -= 18;
            let (b, _v) = handle_edge_slide(&mut state, 1, 205, y, true);
            brightness_total += b;
        }
        assert!(
            brightness_total > 0,
            "gesture must keep producing steps after a slot shuffle"
        );
    }

    #[test]
    fn edge_slide_does_not_rearm_after_leaving_the_zone() {
        let mut state = real_pad();
        let _ = handle_edge_slide(&mut state, 1, 200, 1_800, true);
        assert!(state.edge.is_some());
        // Finger travels to the middle of the pad (beyond 3282/3 = 1094).
        for x in [500, 800, 1_200, 1_500, 1_800, 2_100, 2_400] {
            let _ = handle_edge_slide(&mut state, 1, x, 1_790, true);
        }
        assert!(state.edge.is_none(), "gesture must end away from the edge");
        // The log storm the audit found: every further frame re-armed from the
        // stale session side and ended again (3 167 `ended` lines). It must not.
        for x in [2_000, 2_100, 2_200, 2_300, 2_400, 2_500] {
            let _ = handle_edge_slide(&mut state, 1, x, 1_790, true);
            assert!(
                state.edge.is_none(),
                "must not re-arm from a stale edge session at x={x}"
            );
        }
    }

    #[test]
    fn edge_slide_ignores_out_of_range_coordinates() {
        let mut state = real_pad();
        // x beyond the logical max is an EMI-corrupted report, not a position.
        let _ = handle_edge_slide(&mut state, 1, 4_000, 1_800, true);
        assert!(
            state.edge.is_none(),
            "out-of-range X must not arm a gesture"
        );
        // Arm legitimately, then feed a corrupt frame: it must not capture.
        let _ = handle_edge_slide(&mut state, 1, 200, 1_800, true);
        assert!(state.edge.is_some());
        let _ = handle_edge_slide(&mut state, 1, 200, 9_000, true);
        assert!(
            !state.edge.as_ref().expect("edge").captured,
            "an out-of-range Y must not count as vertical intent"
        );
    }

    #[test]
    fn edge_slide_right_edge_drives_volume() {
        let mut state = real_pad();
        // Right initiation zone: x > 3282 - 410 = 2872.
        let _ = handle_edge_slide(&mut state, 1, 3_100, 1_800, true);
        let mut volume_total = 0;
        let mut y = 1_800;
        for _ in 0..40 {
            y -= 18;
            let (_b, v) = handle_edge_slide(&mut state, 1, 3_100, y, true);
            volume_total += v;
        }
        assert!(volume_total > 0, "right edge must drive volume");
    }
}

// ─── Gesture handlers ─────────────────────────────────────────────────────────

/// 5+ fingers held for ≥ 300 ms → Win+Shift+S. 3-second cooldown.
/// Returns true when the screenshot action should fire.
#[cfg(windows)]
fn handle_five_finger_gesture(state: &mut GestureState, contact_count: u32) -> bool {
    use std::time::{Duration, Instant};

    if let Some(cd) = state.screenshot_cooldown {
        if cd.elapsed() < Duration::from_secs(3) {
            return false;
        }
        state.screenshot_cooldown = None;
    }

    if contact_count >= 5 {
        match state.five_start {
            None => {
                state.five_start = Some(Instant::now());
            }
            Some(start) if start.elapsed() >= Duration::from_millis(300) => {
                log::info!("[gesture] 5-finger gesture → Win+Shift+S");
                state.screenshot_cooldown = Some(Instant::now());
                state.five_start = None;
                return true;
            }
            Some(_) => {}
        }
    } else {
        state.five_start = None;
    }
    false
}

/// Single-finger vertical swipe in the left (brightness) or right (volume)
/// edge band triggers one action per step of Y movement.
///
/// Returns `(brightness_steps, volume_steps)` — positive = up, negative = down —
/// and leaves two side-effect requests on `state` for the caller to drain after
/// the RefCell borrow is released:
///   * `edge_want_pin` / `edge_want_release` → `ClipCursor` confinement of the
///     pointer ("o rato não anda"),
///   * `edge_ticks` → number of haptic tick frames to write.
///
/// The caller must invoke simulate_*/ClipCursor/HidD_* AFTER this function
/// returns so that the GESTURE_STATE borrow is fully released before any
/// COM/WMI/HID call.
#[cfg(windows)]
fn handle_edge_slide(
    state: &mut GestureState,
    contact_count: u32,
    x: i32,
    y: i32,
    tip_switch: bool,
) -> (i32, i32) {
    // Per-frame side effects are drained by the caller; start from zero so a
    // frame that produces nothing never replays the previous frame's requests.
    state.edge_ticks = 0;

    // Reject frames whose coordinates are outside the pad's logical range.
    // The BLTP7853 emits out-of-range X/Y on EMI-corrupted reports (the audit
    // saw x=3240 on a pad whose logical max is 3282, and start_x values that
    // produced total_dx=-3200). Those frames used to arm, capture and reset
    // gestures; they carry no usable position, so they must not move the state
    // machine at all. `contact_count`/`tip_switch` are still honoured below so
    // a lifted finger is not ignored just because the coordinates are junk.
    let coords_valid = x >= 0 && y >= 0 && x <= state.x_max && y <= state.y_max;

    // Allow up to 5 consecutive frames of lost contact before resetting.
    const GRACE_FRAMES: u8 = 5;
    /// Out-of-zone frames tolerated before the gesture is considered over.
    const OUT_OF_ZONE_GRACE: u8 = 3;

    // Treat both a missing contact, a lifted finger (TipSwitch=0) and an
    // out-of-range coordinate as "no valid contact" for the *position* part.
    let no_contact = contact_count != 1 || !tip_switch || !coords_valid;

    if no_contact {
        if state.edge_contact_active {
            state.edge_contact_lost_frames = state.edge_contact_lost_frames.saturating_add(1);
            if state.edge_contact_lost_frames > GRACE_FRAMES {
                state.edge_contact_active = false;
                state.edge_contact_start_side = None;
                state.edge_contact_lost_frames = 0;
            }
        }
        if let Some(edge) = &mut state.edge {
            edge.lost_frames += 1;
            if edge.lost_frames > GRACE_FRAMES {
                log::debug!(
                    "[gesture] edge-slide dropped: contact lost for {} frames",
                    edge.lost_frames
                );
                state.edge = None;
                EDGE_GESTURE_ACTIVE.store(false, Ordering::Relaxed);
                state.edge_want_release = true;
            }
        }
        return (0, 0);
    }

    // Band geometry (percent of x_max) and the activation gate both come from
    // the RE of PluginGesture.dll — see EDGE_ZONE_START_PCT / EDGE_ACTIVATION_PERMILLE.
    let (start_pct, continue_pct) = edge_zone_pcts();
    let edge_thresh = (state.x_max * start_pct / 100).max(1);
    let continue_thresh = (state.x_max * continue_pct / 100).max(edge_thresh + 1);
    let y_step = (state.y_max / 24).clamp(28, 320); // ~4.2% of height per action
                                                    // A Y jump larger than this in one frame means a new finger was placed
                                                    // (the touchpad sent no intermediate contact_count=0 report).
    let jump_thresh = state.y_max * 15 / 100; // 15% of height
                                              // Pre-roll: a swipe is only captured after this much vertical travel
                                              // *inside the band*, so an ordinary click/scroll near the border never
                                              // turns into a brightness/volume change. 118 ‰ of 2124 = 250 units
                                              // = 8 mm, the literal `cmp |dy|, 0xFA` of the original.
    let activation_dy =
        ((state.y_max as i64 * EDGE_ACTIVATION_PERMILLE as i64 / 1000) as i32).clamp(180, 900);

    // Contact-session gating: edge slide can only start if the finger touch
    // itself started in the edge initiation zone.
    if !state.edge_contact_active {
        state.edge_contact_active = true;
        state.edge_contact_lost_frames = 0;
        state.edge_contact_start_side = edge_side_of(x, state.x_max, edge_thresh);
    } else if state.edge_contact_lost_frames > 0 {
        state.edge_contact_lost_frames = 0;
    }

    match &mut state.edge {
        None => {
            // Re-arm only if BOTH hold:
            //   1. the contact session started at an edge, and
            //   2. the finger is *still* inside the initiation zone right now.
            //
            // (2) is what stops the log storm the audit found: the old code
            // re-armed from a stale session side at whatever X the finger was
            // at, so after the first exit every subsequent frame armed in the
            // middle of the pad and "ended" on the next one — 3 167 `ended`
            // lines with x=3240 on a left-side gesture.
            let session_side = state.edge_contact_start_side;
            let current_side = edge_side_of(x, state.x_max, edge_thresh);
            if let (Some(side), Some(cur)) = (session_side, current_side) {
                if side == cur {
                    log::debug!(
                        "[gesture] edge-slide armed: side={} x={}/{} y={} y_step={} activation_dy={}",
                        match side {
                            EdgeSide::Left => "left",
                            EdgeSide::Right => "right",
                        },
                        x,
                        state.x_max,
                        y,
                        y_step,
                        activation_dy
                    );
                    state.edge = Some(EdgeSlideState {
                        side,
                        start_x: x,
                        start_y: y,
                        last_x: x,
                        last_y: y,
                        accum: 0,
                        captured: false,
                        lost_frames: 0,
                        out_of_zone_frames: 0,
                    });
                }
            }
            (0, 0)
        }
        Some(edge) => {
            // ── X-zone validation ─────────────────────────────────────────────
            // Gesture continuation zone is 33% from the edge — wider than the
            // 12.5% initiation zone so normal finger drift doesn't kill the
            // gesture, but a new finger placed in the center terminates it.
            //
            // A large X jump (more than half the pad) is not travel: it is the
            // HID report switching which contact slot we are reading. Re-anchor
            // instead of ending, otherwise a slot shuffle kills the gesture.
            let in_zone = match edge.side {
                EdgeSide::Left => x < continue_thresh,
                EdgeSide::Right => x > state.x_max - continue_thresh,
            };
            if !in_zone {
                let slot_switch = (x - edge.last_x).abs() > state.x_max / 2;
                if slot_switch {
                    log::debug!(
                        "[gesture] edge-slide contact switch (dx={}) — re-anchoring",
                        x - edge.last_x
                    );
                    edge.last_x = x;
                    edge.last_y = y;
                    edge.out_of_zone_frames = 0;
                    return (0, 0);
                }
                edge.out_of_zone_frames = edge.out_of_zone_frames.saturating_add(1);
                if edge.out_of_zone_frames <= OUT_OF_ZONE_GRACE {
                    return (0, 0);
                }
                // Only a captured gesture is worth an INFO line: the audit
                // counted 348 of these in 10 minutes from ordinary edge use.
                if edge.captured {
                    log::info!(
                        "[gesture] edge-slide ended: side={} x={}/{} (was captured)",
                        match edge.side {
                            EdgeSide::Left => "left",
                            EdgeSide::Right => "right",
                        },
                        x,
                        state.x_max,
                    );
                } else {
                    log::debug!(
                        "[gesture] edge-slide ended (armed, never captured): x={}/{} side={}",
                        x,
                        state.x_max,
                        match edge.side {
                            EdgeSide::Left => "left",
                            EdgeSide::Right => "right",
                        },
                    );
                }
                state.edge = None;
                // A contact that already left the zone must not re-arm from
                // its old session side.
                state.edge_contact_start_side = None;
                EDGE_GESTURE_ACTIVE.store(false, Ordering::Relaxed);
                state.edge_want_release = true;
                return (0, 0);
            }
            edge.out_of_zone_frames = 0;

            // Contact just restored after a brief tracking loss — re-anchor.
            if edge.lost_frames > 0 {
                edge.last_x = x;
                edge.last_y = y;
                edge.lost_frames = 0;
                return (0, 0);
            }

            let dy = edge.last_y - y; // positive = upward swipe
            let _dx = x - edge.last_x;

            if !edge.captured {
                let total_dy = edge.start_y - y;
                let total_dx = x - edge.start_x;
                // Pre-roll gate (8 mm on this pad) + vertical dominance.
                if total_dy.abs() >= activation_dy
                    && (total_dy.abs() >= total_dx.abs()
                        || total_dy.abs() >= activation_dy.saturating_mul(2))
                {
                    edge.captured = true;
                    edge.accum = 0;
                    edge.last_x = x;
                    edge.last_y = y;
                    EDGE_GESTURE_ACTIVE.store(true, Ordering::Relaxed);
                    let side = match edge.side {
                        EdgeSide::Left => "left",
                        EdgeSide::Right => "right",
                    };
                    log::info!(
                        "[gesture] edge-slide captured: side={} total_dy={} total_dx={} (y_step={} activation={} band={}%/{})",
                        side,
                        total_dy,
                        total_dx,
                        y_step,
                        activation_dy,
                        start_pct,
                        continue_pct,
                    );
                    // Confine the pointer for the rest of the gesture and give one
                    // haptic tick. The original also emits exactly ONE action on the
                    // activation frame (its `pct > 2` test is already satisfied by
                    // the 250-unit pre-roll), then re-anchors — so the user gets
                    // feedback the moment the gesture commits, not 3 mm later.
                    state.edge_want_pin = true;
                    state.edge_ticks = 1;
                    let first = if total_dy > 0 { 1 } else { -1 };
                    return match edge.side {
                        EdgeSide::Left => (first, 0),
                        EdgeSide::Right => (0, first),
                    };
                } else {
                    // Stay armed but do not hijack pointer movement.
                    edge.last_x = x;
                    edge.last_y = y;
                }
                return (0, 0);
            }

            // Large position jump → new finger placed without a contact_count=0 gap.
            // Re-anchor instead of calculating a bogus huge delta.
            if dy.abs() > jump_thresh {
                log::debug!("[gesture] edge-slide position jump ({dy}), re-anchoring at y={y}");
                edge.last_x = x;
                edge.last_y = y;
                edge.accum = 0;
                return (0, 0);
            }

            edge.last_x = x;
            edge.last_y = y;
            // Apply charger-aware Y deadband to reject EMI-corrupted small deltas.
            //
            // The deadband must stay well below `y_step`: it used to be capped at
            // `y_step/2`, and at 250 Hz a normal swipe moves ~15-30 logical units
            // per frame while a 2124-unit pad gave a 42-unit deadband. Every real
            // sample was therefore "noise" and the accumulator never reached a
            // step — which is exactly why the log has captures but zero
            // `edge-left steps=` lines. `y_step/10` (8 units ≈ 0.26 mm) keeps
            // rejecting 1-3 unit jitter while letting a 7 cm/s swipe through; the
            // 250-unit pre-roll now does the "is this intentional?" work that the
            // oversized deadband was wrongly doing before.
            let deadband = charger_aware_y_deadband(state.y_max).min(y_step / 10);
            if dy.abs() < deadband {
                // Idle/noise frame: bleed the accumulator instead of adding to it,
                // so a finger resting on the border does not leave a charged
                // accumulator behind for the next touch. Decaying *only* here —
                // rather than on every frame — is what lets a slow swipe
                // (≈10 units/frame, 8 cm/s) still reach a step; decaying
                // unconditionally gives an equilibrium of 8×dy = 80 < y_step and
                // the gesture would capture and then never step.
                let decay = edge.accum.abs() / 8;
                if decay > 0 {
                    edge.accum -= if edge.accum > 0 { decay } else { -decay };
                } else {
                    edge.accum = 0; // snap small residual to zero
                }
                return (0, 0);
            }
            edge.accum += dy;

            let mut brightness = 0i32;
            let mut volume = 0i32;

            while edge.accum >= y_step {
                edge.accum -= y_step;
                match edge.side {
                    EdgeSide::Left => brightness += 1,
                    EdgeSide::Right => volume += 1,
                }
            }
            while edge.accum <= -y_step {
                edge.accum += y_step;
                match edge.side {
                    EdgeSide::Left => brightness -= 1,
                    EdgeSide::Right => volume -= 1,
                }
            }

            if brightness != 0 {
                log::info!("[gesture] edge-left steps={brightness}");
                state.edge_ticks = brightness.unsigned_abs().min(4) as u8;
            }
            if volume != 0 {
                log::info!("[gesture] edge-right steps={volume}");
                state.edge_ticks = volume.unsigned_abs().min(4) as u8;
            }
            (brightness, volume)
        }
    }
}

/// Which edge initiation zone `x` falls in, if any.
#[cfg(windows)]
fn edge_side_of(x: i32, x_max: i32, edge_thresh: i32) -> Option<EdgeSide> {
    if x < edge_thresh {
        Some(EdgeSide::Left)
    } else if x > x_max - edge_thresh {
        Some(EdgeSide::Right)
    } else {
        None
    }
}

// ─── System action helpers ────────────────────────────────────────────────────

/// Inject Win+Shift+S (Windows Snipping Tool / screenshot region selector).
#[cfg(windows)]
fn simulate_win_shift_s() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    };
    let inputs: [INPUT; 6] = unsafe {
        // SAFETY: make_key_input initializes each INPUT struct with valid VK codes and
        // flags. The array is fully initialized before being passed to SendInput.
        [
            make_key_input(VIRTUAL_KEY(0x5B), KEYBD_EVENT_FLAGS(0)), // VK_LWIN down
            make_key_input(VIRTUAL_KEY(0x10), KEYBD_EVENT_FLAGS(0)), // VK_SHIFT down
            make_key_input(VIRTUAL_KEY(0x53), KEYBD_EVENT_FLAGS(0)), // 'S' down
            make_key_input(VIRTUAL_KEY(0x53), KEYEVENTF_KEYUP),      // 'S' up
            make_key_input(VIRTUAL_KEY(0x10), KEYEVENTF_KEYUP),      // VK_SHIFT up
            make_key_input(VIRTUAL_KEY(0x5B), KEYEVENTF_KEYUP),      // VK_LWIN up
        ]
    };
    unsafe {
        // SAFETY: inputs is a fully initialized array of 6 INPUT structs with correct
        // size parameter. SendInput copies the input data into the system queue and
        // does not retain the pointer.
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

#[cfg(windows)]
fn simulate_volume_up() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    };
    let inputs: [INPUT; 2] = unsafe {
        // SAFETY: Same pattern as simulate_win_shift_s — make_key_input initializes
        // each INPUT struct fully with a VK code and flags.
        [
            make_key_input(VIRTUAL_KEY(0xAF), KEYBD_EVENT_FLAGS(0)), // VK_VOLUME_UP down
            make_key_input(VIRTUAL_KEY(0xAF), KEYEVENTF_KEYUP),      // VK_VOLUME_UP up
        ]
    };
    unsafe {
        // SAFETY: Fully initialized array; SendInput does not retain the pointer.
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

#[cfg(windows)]
fn simulate_volume_down() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    };
    let inputs: [INPUT; 2] = unsafe {
        // SAFETY: Same pattern — make_key_input initializes each INPUT struct fully
        // with a VK code and flags.
        [
            make_key_input(VIRTUAL_KEY(0xAE), KEYBD_EVENT_FLAGS(0)), // VK_VOLUME_DOWN down
            make_key_input(VIRTUAL_KEY(0xAE), KEYEVENTF_KEYUP),      // VK_VOLUME_DOWN up
        ]
    };
    unsafe {
        // SAFETY: Fully initialized array; SendInput does not retain the pointer.
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Increase display brightness by 5 points and show the OSD overlay.
#[cfg(windows)]
fn simulate_brightness_up() {
    let cur = crate::hw::display::current_brightness();
    let new_level = (cur + 5).min(100);
    let _ = crate::hw::display::set_brightness(new_level);
    show_brightness_osd(new_level);
}

/// Decrease display brightness by 5 points and show the OSD overlay.
#[cfg(windows)]
fn simulate_brightness_down() {
    let cur = crate::hw::display::current_brightness();
    let new_level = cur.saturating_sub(5).max(10);
    let _ = crate::hw::display::set_brightness(new_level);
    show_brightness_osd(new_level);
}

/// Show the always-on-top brightness OSD window (without stealing keyboard focus)
/// and emit the new level so the frontend can render the indicator.
#[cfg(windows)]
fn show_brightness_osd(level: u8) {
    // Delegate to the native Win32 GDI OSD (no WebView2 / IPC dependency).
    crate::hw::osd::show_brightness_osd(level);
}

/// Construct a keyboard `INPUT` struct for use with `SendInput`.
///
/// # Safety
///
/// The function uses unsafe `std::mem::zeroed()` internally for the union fields.
/// The caller must ensure the returned INPUT is used only with SendInput and
/// that the associated KEYBDINPUT ki fields are valid for the intended key event.
#[cfg(windows)]
#[inline]
unsafe fn make_key_input(
    vk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY,
    flags: windows::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS,
) -> windows::Win32::UI::Input::KeyboardAndMouse::INPUT {
    use windows::Win32::UI::Input::KeyboardAndMouse::{INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT};
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}
