use anyhow::{Context, Result};
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
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum HapticsIntensity {
    Low,
    Medium,
    High,
}

/// Fallback HID path used when hardware discovery has not yet found the touchpad.
/// The discovery module enumerates HID devices at first launch and replaces this.
const TOUCHPAD_HID_PATH_DEFAULT: &str =
    r"\\?\hid#bltp7853&col04#5&37166779&0&0003#{4d1e55b2-f16f-11cf-88cb-001111000030}";

/// Return the active touchpad HID path.
/// Uses the discovery profile when available; falls back to the BLTP7853 default.
#[allow(dead_code)]
fn touchpad_hid_path() -> String {
    crate::hw::discovery::global_profile()
        .and_then(|p| p.touchpad_hid_path.clone())
        .unwrap_or_else(|| TOUCHPAD_HID_PATH_DEFAULT.to_string())
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
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether the 5-finger screenshot gesture is currently enabled.
#[cfg(windows)]
static GESTURE_SCREENSHOT_ENABLED: AtomicBool = AtomicBool::new(false);

/// Whether the left/right edge slide gesture is currently enabled.
#[cfg(windows)]
static EDGE_SLIDE_ENABLED: AtomicBool = AtomicBool::new(false);

/// Prevents the Raw Input listener thread from being started more than once.
#[cfg(windows)]
static GESTURE_THREAD_STARTED: AtomicBool = AtomicBool::new(false);

// ─── Public API ───────────────────────────────────────────────────────────────

pub fn get_touchpad_info() -> Result<TouchpadInfo> {
    let info = read_touchpad_registry().unwrap_or_else(|_| TouchpadInfo {
        sensitivity: TouchpadSensitivity::Medium,
        haptics_enabled: true,
        haptics_intensity: HapticsIntensity::Medium,
        gesture_screenshot: false,
        trackpad_repress: false,
        edge_slide: false,
    });
    Ok(info)
}

pub fn set_touchpad_sensitivity(sensitivity: TouchpadSensitivity) -> Result<()> {
    let reg_val = match sensitivity {
        TouchpadSensitivity::Low => 1,
        TouchpadSensitivity::Medium => 2,
        TouchpadSensitivity::High => 3,
    };
    persist_reg_dword(TP_REG_SENSITIVITY, reg_val)?;
    // Also update the Windows standard PTP sensitivity registry so the inbox driver sees it.
    #[cfg(windows)]
    set_windows_ptp_sensitivity(&sensitivity);
    Ok(())
}

pub fn set_touchpad_haptics(enabled: bool) -> Result<()> {
    persist_reg_dword(TP_REG_HAPTICS, if enabled { 1 } else { 0 })
}

pub fn set_touchpad_haptics_intensity(intensity: HapticsIntensity) -> Result<()> {
    persist_reg_dword(TP_REG_HAPTICS_INTENSITY, match intensity {
        HapticsIntensity::Low => 1,
        HapticsIntensity::Medium => 2,
        HapticsIntensity::High => 3,
    })
}

pub fn set_touchpad_gesture_screenshot(enabled: bool) -> Result<()> {
    #[cfg(windows)]
    GESTURE_SCREENSHOT_ENABLED.store(enabled, Ordering::Relaxed);
    persist_reg_dword(TP_REG_GESTURE_SCREENSHOT, if enabled { 1 } else { 0 })?;
    // Ensure the Raw Input gesture listener thread is running.
    #[cfg(windows)]
    ensure_gesture_listener();
    Ok(())
}

pub fn set_touchpad_repress(enabled: bool) -> Result<()> {
    persist_reg_dword(TP_REG_TRACKPAD_REPRESS, if enabled { 1 } else { 0 })
}

pub fn set_touchpad_edge_slide(enabled: bool) -> Result<()> {
    #[cfg(windows)]
    EDGE_SLIDE_ENABLED.store(enabled, Ordering::Relaxed);
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
    ensure_gesture_listener();
}

// ─── Registry persistence ─────────────────────────────────────────────────────

/// Write a single DWORD value to `HKLM\SOFTWARE\MI\Touchpad`.
fn persist_reg_dword(value_name: &str, value: u32) -> Result<()> {
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY_CURRENT_USER, KEY_WRITE, REG_DWORD,
            REG_OPTION_NON_VOLATILE,
        };
        use windows::core::PCWSTR;
        unsafe {
            let key_w: Vec<u16> = OsStr::new(TP_REG_KEY).encode_wide().chain(Some(0)).collect();
            let mut hkey = std::mem::zeroed();
            RegCreateKeyExW(
                HKEY_CURRENT_USER, PCWSTR(key_w.as_ptr()), 0, None,
                REG_OPTION_NON_VOLATILE, KEY_WRITE, None, &mut hkey, None,
            ).ok().context("Create touchpad reg key")?;
            let val_w: Vec<u16> = OsStr::new(value_name).encode_wide().chain(Some(0)).collect();
            let _ = RegSetValueExW(hkey, PCWSTR(val_w.as_ptr()), 0, REG_DWORD, Some(&value.to_le_bytes())).ok();
            let _ = RegCloseKey(hkey).ok();
        }
    }
    Ok(())
}

fn read_touchpad_registry() -> Result<TouchpadInfo> {
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_CURRENT_USER, REG_VALUE_TYPE,
        };
        use windows::core::PCWSTR;
        unsafe {
            let key_w: Vec<u16> = OsStr::new(TP_REG_KEY).encode_wide().chain(Some(0)).collect();
            let mut hkey = std::mem::zeroed();
            if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(key_w.as_ptr()), 0,
                windows::Win32::System::Registry::KEY_READ, &mut hkey).is_err() {
                return Ok(TouchpadInfo {
                    sensitivity: TouchpadSensitivity::Medium,
                    haptics_enabled: true,
                    haptics_intensity: HapticsIntensity::Medium,
                    gesture_screenshot: false,
                    trackpad_repress: false,
                    edge_slide: false,
                });
            }

            let read_dword = |name: &str, default: u32| -> u32 {
                let mut ty = REG_VALUE_TYPE::default();
                let mut v: u32 = default;
                let mut size = 4u32;
                let w: Vec<u16> = OsStr::new(name).encode_wide().chain(Some(0)).collect();
                let _ = RegQueryValueExW(hkey, PCWSTR(w.as_ptr()), None, Some(&mut ty),
                    Some((&mut v as *mut u32).cast()), Some(&mut size));
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
                sensitivity: match sens_raw { 1 => TouchpadSensitivity::Low, 3 => TouchpadSensitivity::High, _ => TouchpadSensitivity::Medium },
                haptics_enabled: haptics,
                haptics_intensity: match haptics_intensity_raw { 1 => HapticsIntensity::Low, 3 => HapticsIntensity::High, _ => HapticsIntensity::Medium },
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

// ─── Windows Precision Touchpad (PTP) standard sensitivity ───────────────────

/// Write the Windows inbox PTP sensitivity registry value so the OS driver
/// picks up the change immediately.
///
/// Path:  HKCU\Software\Microsoft\Windows\CurrentVersion\PrecisionTouchPad
/// Value: Sensitivity  REG_DWORD   1=Low  3=Medium  5=High
#[cfg(windows)]
fn set_windows_ptp_sensitivity(sensitivity: &TouchpadSensitivity) {
    use winreg::{
        enums::{HKEY_CURRENT_USER, KEY_WRITE},
        RegKey,
    };
    let val: u32 = match sensitivity {
        TouchpadSensitivity::Low => 1,
        TouchpadSensitivity::Medium => 3,
        TouchpadSensitivity::High => 5,
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

#[cfg(windows)]
unsafe fn win_gesture_loop() {
    use windows::Win32::Foundation::HINSTANCE;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    use windows::Win32::UI::Input::{
        RegisterRawInputDevices, RAWINPUTDEVICE, RIDEV_INPUTSINK,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DispatchMessageW, GetMessageW, RegisterClassExW,
        TranslateMessage, HWND_MESSAGE, MSG, WINDOW_EX_STYLE, WINDOW_STYLE, WNDCLASSEXW,
    };
    use windows::core::PCWSTR;

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

    if let Err(e) =
        RegisterRawInputDevices(&[rid], std::mem::size_of::<RAWINPUTDEVICE>() as u32)
    {
        log::warn!(
            "[gesture] RegisterRawInputDevices failed: {e}. \
             Gesture detection will be unavailable."
        );
    } else {
        log::info!("[gesture] Raw Input gesture listener active");
    }

    // Message pump — WM_INPUT messages are delivered here.
    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }

    log::info!("[gesture] Gesture listener stopped");
}

/// Window procedure for the gesture message-only window.
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
        }
    }
}

#[cfg(windows)]
struct EdgeSlideState {
    side: EdgeSide,
    last_y: i32,
    /// Accumulated Y delta waiting to reach the next action threshold.
    accum: i32,
    /// Consecutive WM_INPUT frames where contact_count != 1.
    /// Edge state is preserved across brief gaps (palm rejection artefacts).
    lost_frames: u8,
}

#[cfg(windows)]
enum EdgeSide {
    Left,
    Right,
}

#[cfg(windows)]
thread_local! {
    /// Per-device preparsed HID data cache, keyed by HANDLE numeric value.
    static PREPARSED_CACHE: std::cell::RefCell<std::collections::HashMap<usize, Vec<u8>>> =
        std::cell::RefCell::new(std::collections::HashMap::new());

    static GESTURE_STATE: std::cell::RefCell<GestureState> =
        std::cell::RefCell::new(GestureState::default());
}

// ─── Raw Input processor ──────────────────────────────────────────────────────

#[cfg(windows)]
unsafe fn process_raw_input(lparam: isize) {
    use windows::Win32::Devices::HumanInterfaceDevice::{
        HidP_GetSpecificValueCaps, HidP_GetUsageValue, HidP_Input, HIDP_VALUE_CAPS,
        PHIDP_PREPARSED_DATA,
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
    let mut buf = vec![0u8; size as usize];
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

    let raw = buf.as_ptr() as *const RAWINPUT;

    // Only handle HID (touch) reports — RIM_TYPEHID = 2.
    if (*raw).header.dwType != 2 {
        return;
    }

    let device_key = (*raw).header.hDevice.0 as usize;

    // ── Get/cache preparsed data for this device ──────────────────────────────
    let pp_bytes = PREPARSED_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if !cache.contains_key(&device_key) {
            let mut pp_size: u32 = 0;
            GetRawInputDeviceInfoW(
                (*raw).header.hDevice,
                RIDI_PREPARSEDDATA,
                None,
                &mut pp_size,
            );
            if pp_size == 0 || pp_size > 65536 {
                return None;
            }
            let mut pp_buf = vec![0u8; pp_size as usize];
            let ret = GetRawInputDeviceInfoW(
                (*raw).header.hDevice,
                RIDI_PREPARSEDDATA,
                Some(pp_buf.as_mut_ptr() as *mut _),
                &mut pp_size,
            );
            if ret == u32::MAX {
                return None;
            }
            cache.insert(device_key, pp_buf);
        }
        cache.get(&device_key).cloned()
    });

    let pp_bytes = match pp_bytes {
        Some(b) => b,
        None => return,
    };

    let preparsed = PHIDP_PREPARSED_DATA(pp_bytes.as_ptr() as isize);

    // ── Extract the HID report bytes ──────────────────────────────────────────
    // bRawData is [u8;1] stub for the flexible array; use from_raw_parts.
    let hid = &(*raw).data.hid;
    if hid.dwSizeHid == 0 || hid.dwCount == 0 {
        return;
    }
    let report =
        std::slice::from_raw_parts(hid.bRawData.as_ptr(), hid.dwSizeHid as usize);

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

    // ── Read contact count (Usage Page 0x000D, Usage 0x0054, LC=0) ───────────
    let mut contact_count: u32 = 0;
    let _ = HidP_GetUsageValue(
        HidP_Input,
        0x000D,
        0,
        0x0054,
        &mut contact_count,
        preparsed,
        report,
    );

    // ── Read first contact X and Y (per-contact link collections 1..=5) ──────
    // X/Y are in Generic Desktop usage page (0x0001), not Digitizer (0x000D)
    let mut first_x: u32 = 0;
    let mut first_y: u32 = 0;
    for lc in 1u16..=5 {
        let r = HidP_GetUsageValue(
            HidP_Input,
            0x0001,
            lc,
            0x0030, // X
            &mut first_x,
            preparsed,
            report,
        );
        if r.is_ok() && first_x > 0 {
            break;
        }
    }
    for lc in 1u16..=5 {
        let r = HidP_GetUsageValue(
            HidP_Input,
            0x0001,
            lc,
            0x0031, // Y
            &mut first_y,
            preparsed,
            report,
        );
        if r.is_ok() && first_y > 0 {
            break;
        }
    }

    // ── Feed gesture state machine ────────────────────────────────────────────
    if GESTURE_SCREENSHOT_ENABLED.load(Ordering::Relaxed) {
        GESTURE_STATE.with(|state| {
            handle_five_finger_gesture(&mut state.borrow_mut(), contact_count);
        });
    }
    if EDGE_SLIDE_ENABLED.load(Ordering::Relaxed) {
        GESTURE_STATE.with(|state| {
            handle_edge_slide(
                &mut state.borrow_mut(),
                contact_count,
                first_x as i32,
                first_y as i32,
            );
        });
    }
}

// ─── Gesture handlers ─────────────────────────────────────────────────────────

/// 5+ fingers held for ≥ 300 ms → Win+Shift+S. 3-second cooldown.
#[cfg(windows)]
fn handle_five_finger_gesture(state: &mut GestureState, contact_count: u32) {
    use std::time::{Duration, Instant};

    if let Some(cd) = state.screenshot_cooldown {
        if cd.elapsed() < Duration::from_secs(3) {
            return;
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
                simulate_win_shift_s();
                state.screenshot_cooldown = Some(Instant::now());
                state.five_start = None;
            }
            Some(_) => {}
        }
    } else {
        state.five_start = None;
    }
}

/// Single-finger vertical swipe in the left (brightness) or right (volume)
/// edge zone triggers one action per step of Y movement.
#[cfg(windows)]
fn handle_edge_slide(
    state: &mut GestureState,
    contact_count: u32,
    x: i32,
    y: i32,
) {
    // Allow up to 5 consecutive frames of lost contact before resetting.
    // This handles brief palm-rejection / tracking-lost artefacts that would
    // otherwise reset the gesture mid-swipe and cause direction reversals.
    const GRACE_FRAMES: u8 = 5;

    if contact_count != 1 {
        match &mut state.edge {
            Some(edge) => {
                edge.lost_frames += 1;
                if edge.lost_frames > GRACE_FRAMES {
                    state.edge = None;
                }
            }
            None => {}
        }
        return;
    }

    let edge_thresh = state.x_max / 8;         // 12.5% of width
    let y_step = (state.y_max / 10).max(1);    // 10% of height per action (less jitter-prone)

    match &mut state.edge {
        None => {
            let side = if x < edge_thresh {
                Some(EdgeSide::Left)
            } else if x > state.x_max - edge_thresh {
                Some(EdgeSide::Right)
            } else {
                None
            };
            if let Some(side) = side {
                log::info!("[gesture] edge-slide started: side={} x={}/{} y={}", match side { EdgeSide::Left => "left", EdgeSide::Right => "right" }, x, state.x_max, y);
                state.edge = Some(EdgeSlideState { side, last_y: y, accum: 0, lost_frames: 0 });
            }
        }
        Some(edge) => {
            edge.lost_frames = 0; // contact restored
            let dy = edge.last_y - y; // positive = upward swipe
            edge.last_y = y;
            edge.accum += dy;

            while edge.accum >= y_step {
                edge.accum -= y_step;
                match edge.side {
                    EdgeSide::Left  => { log::info!("[gesture] edge-left ↑ → brightness+"); simulate_brightness_up(); }
                    EdgeSide::Right => { log::info!("[gesture] edge-right ↑ → volume+");    simulate_volume_up();     }
                }
            }
            while edge.accum <= -y_step {
                edge.accum += y_step;
                match edge.side {
                    EdgeSide::Left  => { log::info!("[gesture] edge-left ↓ → brightness-"); simulate_brightness_down(); }
                    EdgeSide::Right => { log::info!("[gesture] edge-right ↓ → volume-");    simulate_volume_down();     }
                }
            }
        }
    }
}

// ─── System action helpers ────────────────────────────────────────────────────

/// Inject Win+Shift+S (Windows Snipping Tool / screenshot region selector).
#[cfg(windows)]
fn simulate_win_shift_s() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, INPUT, VIRTUAL_KEY,
    };
    let inputs: [INPUT; 6] = unsafe {
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
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

#[cfg(windows)]
fn simulate_volume_up() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, INPUT, VIRTUAL_KEY,
    };
    let inputs: [INPUT; 2] = unsafe {
        [
            make_key_input(VIRTUAL_KEY(0xAF), KEYBD_EVENT_FLAGS(0)), // VK_VOLUME_UP down
            make_key_input(VIRTUAL_KEY(0xAF), KEYEVENTF_KEYUP),      // VK_VOLUME_UP up
        ]
    };
    unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32); }
}

#[cfg(windows)]
fn simulate_volume_down() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, INPUT, VIRTUAL_KEY,
    };
    let inputs: [INPUT; 2] = unsafe {
        [
            make_key_input(VIRTUAL_KEY(0xAE), KEYBD_EVENT_FLAGS(0)), // VK_VOLUME_DOWN down
            make_key_input(VIRTUAL_KEY(0xAE), KEYEVENTF_KEYUP),      // VK_VOLUME_DOWN up
        ]
    };
    unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32); }
}

/// Increase display brightness by 5 points (max 100).
#[cfg(windows)]
fn simulate_brightness_up() {
    if let Ok(info) = crate::hw::display::get_display_info() {
        let _ = crate::hw::display::set_brightness((info.brightness + 5).min(100));
    }
}

/// Decrease display brightness by 5 points (min 10).
#[cfg(windows)]
fn simulate_brightness_down() {
    if let Ok(info) = crate::hw::display::get_display_info() {
        let _ = crate::hw::display::set_brightness(info.brightness.saturating_sub(5).max(10));
    }
}

/// Construct a keyboard `INPUT` struct for use with `SendInput`.
#[cfg(windows)]
#[inline]
unsafe fn make_key_input(
    vk: windows::Win32::UI::Input::KeyboardAndMouse::VIRTUAL_KEY,
    flags: windows::Win32::UI::Input::KeyboardAndMouse::KEYBD_EVENT_FLAGS,
) -> windows::Win32::UI::Input::KeyboardAndMouse::INPUT {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT,
    };
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
