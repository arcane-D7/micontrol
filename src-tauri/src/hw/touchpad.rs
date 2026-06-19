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
/// The discovery module enumerates HID devices at first launch and replaces this.
const TOUCHPAD_HID_PATH_DEFAULT: &str =
    r"\\?\hid#bltp7853&col04#5&37166779&0&0003#{4d1e55b2-f16f-11cf-88cb-001111000030}";

/// Return the active touchpad HID path.
/// Uses the discovery profile when available; falls back to the BLTP7853 default.
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
/// Emergency stability switch: keep cursor movement untouched even during
/// edge-slide handling. This avoids pointer conflicts on some touchpad/driver
/// stacks where WM_MOUSEMOVE suppression causes jumpy behavior.
#[cfg(windows)]
const EDGE_POINTER_SUPPRESSION: bool = false;
/// True while an edge-slide gesture is actively tracking a contact.
/// Used by the WH_MOUSE_LL hook proc to suppress cursor movement without
/// accessing the thread-local RefCell (which may be mutably borrowed).
#[cfg(windows)]
static EDGE_GESTURE_ACTIVE: AtomicBool = AtomicBool::new(false);
/// Prevents the Raw Input listener thread from being started more than once.
#[cfg(windows)]
static GESTURE_THREAD_STARTED: AtomicBool = AtomicBool::new(false);

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

// ─── Public API ───────────────────────────────────────────────────────────────

pub fn get_touchpad_info() -> Result<TouchpadInfo> {
    let info = read_touchpad_registry().unwrap_or(TouchpadInfo {
        sensitivity: TouchpadSensitivity::Medium,
        haptics_enabled: true,
        haptics_intensity: HapticsIntensity::Medium,
        gesture_screenshot: false,
        trackpad_repress: false,
        edge_slide: false,
    });
    log::trace!(
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

pub fn set_touchpad_sensitivity(sensitivity: TouchpadSensitivity) -> Result<()> {
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
    Ok(())
}

pub fn set_touchpad_haptics(enabled: bool) -> Result<()> {
    persist_reg_dword(TP_REG_HAPTICS, if enabled { 1 } else { 0 })?;
    #[cfg(windows)]
    {
        // Read current intensity from registry so we can send the full combined report.
        let intensity = read_touchpad_registry()
            .map(|i| i.haptics_intensity)
            .unwrap_or(HapticsIntensity::Medium);
        send_haptics_hid_report(enabled, &intensity)
            .unwrap_or_else(|e| log::debug!("[touchpad] haptics HID: {e}"));
    }
    Ok(())
}

pub fn set_touchpad_haptics_intensity(intensity: HapticsIntensity) -> Result<()> {
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
        // Read current enabled state from registry.
        let enabled = read_touchpad_registry()
            .map(|i| i.haptics_enabled)
            .unwrap_or(true);
        send_haptics_hid_report(enabled, &intensity)
            .unwrap_or_else(|e| log::debug!("[touchpad] haptics HID: {e}"));
    }
    Ok(())
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
    {
        EDGE_SLIDE_ENABLED.store(enabled, Ordering::Relaxed);
        // Defensive reset: if edge-slide is disabled while a gesture is active,
        // immediately release WM_MOUSEMOVE suppression.
        if !enabled {
            EDGE_GESTURE_ACTIVE.store(false, Ordering::Relaxed);
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
    ensure_gesture_listener();
}

// ─── Registry persistence ─────────────────────────────────────────────────────

/// Write a single DWORD value to `HKLM\SOFTWARE\MI\Touchpad`.
fn persist_reg_dword(value_name: &str, value: u32) -> Result<()> {
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
            let key_w: Vec<u16> = OsStr::new(TP_REG_KEY)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let mut hkey = std::mem::zeroed();
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(key_w.as_ptr()),
                0,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                &mut hkey,
                None,
            )
            .ok()
            .context("Create touchpad reg key")?;
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

fn read_touchpad_registry() -> Result<TouchpadInfo> {
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_CURRENT_USER, REG_VALUE_TYPE,
        };
        unsafe {
            let key_w: Vec<u16> = OsStr::new(TP_REG_KEY)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let mut hkey = std::mem::zeroed();
            if RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(key_w.as_ptr()),
                0,
                windows::Win32::System::Registry::KEY_READ,
                &mut hkey,
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

// ─── BLTP7853 COL04 HID haptics write ────────────────────────────────────────
//
// The BLTP7853 vendor HID collection (COL04) accepts haptics settings via a
// Feature Report.  The report layout below was derived from analysis of the
// Bosch BLTP7853 firmware and Xiaomi PC Manager HID traffic:
//
//   Byte 0 : Report ID  = 0x07
//   Byte 1 : 0x00 = haptics off  /  0x01 = haptics on
//   Byte 2 : Intensity  0x00 = Low  /  0x01 = Medium  /  0x02 = High
//   Bytes 3…N : zero-padding to FeatureReportByteLength
//
// If the haptics do not respond, capture HID traffic from XiaomiPCManager with
// USBPcap/Wireshark and compare the Feature Report payload to update these bytes.

#[cfg(windows)]
fn send_haptics_hid_report(enabled: bool, intensity: &HapticsIntensity) -> Result<()> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Devices::HumanInterfaceDevice::{
        HidD_FreePreparsedData, HidD_GetPreparsedData, HidD_SetFeature, HidP_GetCaps, HIDP_CAPS,
        PHIDP_PREPARSED_DATA,
    };
    use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };

    let path = touchpad_hid_path();
    let path_w: Vec<u16> = OsStr::new(&path).encode_wide().chain(Some(0)).collect();

    let handle = unsafe {
        CreateFileW(
            PCWSTR(path_w.as_ptr()),
            (GENERIC_READ | GENERIC_WRITE).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
        .context("Open BLTP7853 COL04 for haptics")?
    };

    // Query feature report byte length from the device's HID descriptor.
    let feature_len = unsafe {
        let mut preparsed = PHIDP_PREPARSED_DATA(0);
        let mut caps = HIDP_CAPS::default();
        if HidD_GetPreparsedData(handle, &mut preparsed).as_bool() && preparsed.0 != 0 {
            let _ = HidP_GetCaps(preparsed, &mut caps);
            HidD_FreePreparsedData(preparsed);
        }
        caps.FeatureReportByteLength as usize
    };

    // Build the Feature Report; always at least 8 bytes to cover the report ID
    // plus the two data bytes.
    let report_len = feature_len.clamp(8, 64);
    let mut report = vec![0u8; report_len];
    report[0] = 0x07; // BLTP7853 haptics Feature Report ID
    report[1] = if enabled { 0x01 } else { 0x00 }; // haptics on/off
    report[2] = match intensity {
        HapticsIntensity::Low => 0x00,
        HapticsIntensity::Medium => 0x01,
        HapticsIntensity::High => 0x02,
    };

    let ok = unsafe {
        HidD_SetFeature(handle, report.as_mut_ptr() as *mut _, report.len() as u32).as_bool()
    };
    unsafe {
        let _ = CloseHandle(handle);
    }

    if ok {
        log::info!(
            "[touchpad] BLTP7853 haptics HID: enabled={enabled} intensity={:?}",
            intensity
        );
        Ok(())
    } else {
        let err = unsafe { windows::Win32::Foundation::GetLastError() };
        anyhow::bail!("HidD_SetFeature BLTP7853: {err:?}")
    }
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

    // Install mouse suppression hook only when explicitly enabled.
    if EDGE_POINTER_SUPPRESSION {
        use windows::Win32::UI::WindowsAndMessaging::{SetWindowsHookExW, WINDOWS_HOOK_ID};
        match SetWindowsHookExW(
            WINDOWS_HOOK_ID(14), // WH_MOUSE_LL
            Some(mouse_hook_proc),
            HINSTANCE::default(),
            0,
        ) {
            Ok(_hook) => {
                log::info!(
                    "[gesture] Mouse hook installed — cursor suppressed during edge gestures"
                );
                let _ = _hook;
            }
            Err(e) => log::warn!("[gesture] Failed to install mouse hook: {e}"),
        }
    } else {
        log::info!("[gesture] Pointer suppression disabled for edge-slide stability");
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

/// Low-level mouse hook proc — blocks WM_MOUSEMOVE while an edge gesture is
/// active so the cursor does not drift while the user swipes the edge zone.
///
/// This runs on the gesture thread (it was installed there), so reading the
/// thread-local GESTURE_STATE is safe.
#[cfg(windows)]
unsafe extern "system" fn mouse_hook_proc(
    code: i32,
    wparam: windows::Win32::Foundation::WPARAM,
    lparam: windows::Win32::Foundation::LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    use windows::Win32::Foundation::LRESULT;
    use windows::Win32::UI::WindowsAndMessaging::{CallNextHookEx, WM_MOUSEMOVE};

    if !EDGE_POINTER_SUPPRESSION {
        return CallNextHookEx(None, code, wparam, lparam);
    }

    if code >= 0 && wparam.0 as u32 == WM_MOUSEMOVE {
        // Use the dedicated atomic — avoids touching the RefCell which may
        // already be mutably borrowed on this thread (gesture loop).
        let suppress = EDGE_GESTURE_ACTIVE.load(Ordering::Relaxed);
        if suppress {
            return LRESULT(1); // swallow the event
        }
    }
    CallNextHookEx(None, code, wparam, lparam)
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
    static PREPARSED_CACHE: std::cell::RefCell<std::collections::HashMap<usize, Vec<u8>>> =
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

#[cfg(windows)]
unsafe fn process_raw_input(lparam: isize) {
    use windows::Win32::Devices::HumanInterfaceDevice::{
        HidP_GetSpecificValueCaps, HidP_GetUsageValue, HidP_GetUsages, HidP_Input, HIDP_VALUE_CAPS,
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
            log::trace!(target: "hw::touchpad", "raw input device name query failed for device_key={device_key}");
            return false;
        };
        let matched = is_touchpad_device_path(name, &expected_touchpad);
        cache.insert(device_key, matched);
        log::trace!(
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

    // If edge-slide has been disabled, force-release suppression and clear
    // any stale in-progress session state.
    if !EDGE_SLIDE_ENABLED.load(Ordering::Relaxed) {
        EDGE_GESTURE_ACTIVE.store(false, Ordering::Relaxed);
        GESTURE_STATE.with(|state| {
            let mut s = state.borrow_mut();
            s.edge = None;
            s.edge_contact_active = false;
            s.edge_contact_start_side = None;
            s.edge_contact_lost_frames = 0;
        });
    }

    // ── Get/cache preparsed data for this device ──────────────────────────────
    let pp_bytes = PREPARSED_CACHE.with(|cache| {
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
                return Vec::new();
            }
            let mut pp_buf = vec![0u8; pp_size as usize];
            let ret = GetRawInputDeviceInfoW(
                (*raw).header.hDevice,
                RIDI_PREPARSEDDATA,
                Some(pp_buf.as_mut_ptr() as *mut _),
                &mut pp_size,
            );
            if ret == u32::MAX {
                return Vec::new();
            }
            pp_buf
        });
        if pp_buf.is_empty() {
            None
        } else {
            Some(pp_buf.clone())
        }
    });

    let pp_bytes = match pp_bytes {
        Some(b) => b,
        None => return,
    };

    let preparsed = PHIDP_PREPARSED_DATA(pp_bytes.as_ptr() as isize);

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
    log::trace!(
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
    // X/Y are in Generic Desktop usage page (0x0001), not Digitizer (0x000D).
    // Both coordinates must come from the *same* link collection to avoid
    // mismatched X from LC-1 and Y from LC-2 when contacts shuffle.
    let mut first_lc: u16 = 0;
    let mut first_x: u32 = 0;
    let mut first_y: u32 = 0;
    for lc in 1u16..=5 {
        let mut x_val: u32 = 0;
        let mut y_val: u32 = 0;
        let rx = HidP_GetUsageValue(
            HidP_Input, 0x0001, lc, 0x0030, &mut x_val, preparsed, report,
        );
        let ry = HidP_GetUsageValue(
            HidP_Input, 0x0001, lc, 0x0031, &mut y_val, preparsed, report,
        );
        if rx.is_ok() && ry.is_ok() && x_val > 0 && y_val > 0 {
            first_lc = lc;
            first_x = x_val;
            first_y = y_val;
            break;
        }
    }

    // ── Read TipSwitch (0x42) for the active contact's link collection ────────
    // HidP_GetUsages returns the list of active button usages on the Digitizer
    // page for the given LC. TipSwitch (0x42) being in the list means the
    // finger is physically touching the pad; its absence means the finger has
    // lifted (even when contact_count still shows 1 due to BLTP7853 quirks).
    let mut tip_switch = false;
    let mut tip_switch_known = false;
    if first_lc > 0 {
        let mut usage_buf = [0u16; 8]; // 8 slots >> max PTP buttons per contact
        let mut usage_len: u32 = usage_buf.len() as u32;
        // HidP_GetUsages needs &mut [u8]; clone report bytes (small, ~10-40 B)
        let mut report_mut = report.to_vec();
        let r = HidP_GetUsages(
            HidP_Input,
            0x000D, // Digitizer page — where TipSwitch and Confidence live
            first_lc,
            usage_buf.as_mut_ptr(),
            &mut usage_len,
            preparsed,
            &mut report_mut,
        );
        if r.is_ok() {
            tip_switch_known = true;
            let n = usage_len.min(8) as usize;
            tip_switch = usage_buf[..n].contains(&0x0042); // TipSwitch
        }
    }

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
            // Harden TipSwitch fallback: when the tip switch state is unknown
            // (HidP_GetUsages failed), do NOT assume contact — reject the frame
            // instead of treating unknown as "touching". This prevents corrupted
            // HID reports from triggering edge-slide gestures.
            let contact_active = tip_switch_known && tip_switch;
            handle_edge_slide(
                &mut state.borrow_mut(),
                contact_count,
                first_x as i32,
                first_y as i32,
                contact_active,
            )
        })
    } else {
        (0, 0)
    };

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
    fn edge_slide_captures_only_after_clear_vertical_intent() {
        let mut state = super::GestureState::default();
        // Arm on edge-start, but do not capture immediately.
        let _ = handle_edge_slide(&mut state, 1, 200, 4000, true);
        assert!(!state.edge.as_ref().expect("edge state").captured);

        // Small drift should still not capture.
        let _ = handle_edge_slide(&mut state, 1, 240, 3992, true);
        assert!(!state.edge.as_ref().expect("edge state").captured);

        // Strong vertical intent with limited lateral drift captures.
        let _ = handle_edge_slide(&mut state, 1, 240, 3600, true);
        assert!(state.edge.as_ref().expect("edge state").captured);
    }

    #[test]
    fn edge_slide_generates_steps_after_capture() {
        let mut state = super::GestureState::default();
        // Start in left edge.
        let _ = handle_edge_slide(&mut state, 1, 200, 4000, true);
        // Trigger capture with clear vertical movement.
        let _ = handle_edge_slide(&mut state, 1, 220, 3860, true);
        assert!(state.edge.as_ref().expect("edge state").captured);
        // Continue moving up enough to generate at least one brightness step.
        let (b, v) = handle_edge_slide(&mut state, 1, 220, 3500, true);
        assert!(b > 0, "expected brightness step after capture");
        assert_eq!(v, 0);
    }

    #[test]
    fn edge_slide_still_captures_with_large_logical_range() {
        let mut state = super::GestureState::default();
        state.y_max = 65_535; // common PTP logical max
        state.x_max = 65_535;

        // Start in left-edge zone for this range.
        let _ = handle_edge_slide(&mut state, 1, 2_000, 32_000, true);
        assert!(state.edge.is_some());
        assert!(!state.edge.as_ref().expect("edge state").captured);

        // Moderate vertical swipe should now capture due to threshold clamp.
        let _ = handle_edge_slide(&mut state, 1, 2_100, 31_700, true);
        assert!(state.edge.as_ref().expect("edge state").captured);

        // Additional motion should generate at least one step.
        let (b, v) = handle_edge_slide(&mut state, 1, 2_100, 31_200, true);
        assert!(b > 0, "expected brightness step with high y_max");
        assert_eq!(v, 0);
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
/// edge zone triggers one action per step of Y movement.
/// Returns `(brightness_steps, volume_steps)` — positive = up, negative = down.
/// The caller must invoke simulate_* AFTER this function returns so that the
/// GESTURE_STATE borrow is fully released before any COM/WMI call.
#[cfg(windows)]
fn handle_edge_slide(
    state: &mut GestureState,
    contact_count: u32,
    x: i32,
    y: i32,
    tip_switch: bool,
) -> (i32, i32) {
    // Exponential decay: drift the accumulator toward zero during idle/noise
    // frames so stale Y-deltas don't accumulate into phantom gestures.
    // This is defense-in-depth against EMI-corrupted Y values that could
    // otherwise build up in the accumulator.
    if let Some(edge) = &mut state.edge {
        if edge.accum != 0 {
            // Decay by ~25% per frame (shift right by 2), rounding toward zero.
            let decay = edge.accum.abs() / 4;
            if decay > 0 {
                edge.accum -= if edge.accum > 0 { decay } else { -decay };
            } else if edge.accum.abs() == 1 {
                edge.accum = 0; // snap small residual to zero
            }
        }
    }

    // Allow up to 5 consecutive frames of lost contact before resetting.
    const GRACE_FRAMES: u8 = 5;

    // Treat both a missing contact and a lifted finger (TipSwitch=0) as
    // "no valid contact".  The BLTP7853 sometimes reports contact_count=1
    // even after the finger has lifted; TipSwitch catches that case.
    let no_contact = contact_count != 1 || !tip_switch;

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
                state.edge = None;
                EDGE_GESTURE_ACTIVE.store(false, Ordering::Relaxed);
            }
        }
        return (0, 0);
    }

    let edge_thresh = state.x_max / 8; // 12.5% of width
    let y_step = (state.y_max / 24).clamp(28, 320); // ~4.2% of height per action
                                                    // A Y jump larger than this in one frame means a new finger was placed
                                                    // (the touchpad sent no intermediate contact_count=0 report).
    let jump_thresh = state.y_max * 15 / 100; // 15% of height
                                              // Edge slide is only captured after clear vertical intent so the edge
                                              // region continues to behave like a normal touchpad zone by default.
    let activation_dy = (state.y_max / 80).clamp(12, 120); // ~1.25% of height

    // Contact-session gating: edge slide can only start if the finger touch
    // itself started in the edge initiation zone.
    if !state.edge_contact_active {
        state.edge_contact_active = true;
        state.edge_contact_lost_frames = 0;
        state.edge_contact_start_side = if x < edge_thresh {
            Some(EdgeSide::Left)
        } else if x > state.x_max - edge_thresh {
            Some(EdgeSide::Right)
        } else {
            None
        };
    } else if state.edge_contact_lost_frames > 0 {
        state.edge_contact_lost_frames = 0;
    }

    match &mut state.edge {
        None => {
            let side = state.edge_contact_start_side;
            if let Some(side) = side {
                log::debug!(
                    "[gesture] edge-slide armed: side={} x={}/{} y={}",
                    match side {
                        EdgeSide::Left => "left",
                        EdgeSide::Right => "right",
                    },
                    x,
                    state.x_max,
                    y
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
                });
            }
            (0, 0)
        }
        Some(edge) => {
            // ── X-zone validation (FIRST check) ────────────────────────────────────────
            // Gesture continuation zone is 33% from the edge — wider than the
            // 12.5% initiation zone so normal finger drift doesn't kill the
            // gesture, but a new finger placed in the center terminates it.
            // This is the primary fix for the "stuck gesture" bug: when the
            // BLTP7853 skips the contact_count=0 frame between two touches, a
            // new contact in the center was silently continuing the edge swipe.
            let in_zone = match edge.side {
                EdgeSide::Left => x < state.x_max / 3,
                EdgeSide::Right => x > state.x_max * 2 / 3,
            };
            if !in_zone {
                log::info!(
                    "[gesture] edge-slide ended: x={}/{} left edge zone ({} side)",
                    x,
                    state.x_max,
                    match edge.side {
                        EdgeSide::Left => "left",
                        EdgeSide::Right => "right",
                    },
                );
                state.edge = None;
                EDGE_GESTURE_ACTIVE.store(false, Ordering::Relaxed);
                return (0, 0);
            }

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
                // Capture only when vertical movement is clear enough and
                // stronger than lateral drift.
                if total_dy.abs() >= activation_dy
                    && (total_dy.abs() >= total_dx.abs()
                        || total_dy.abs() >= activation_dy.saturating_mul(2))
                {
                    edge.captured = true;
                    edge.accum = 0;
                    edge.last_x = x;
                    edge.last_y = y;
                    EDGE_GESTURE_ACTIVE.store(true, Ordering::Relaxed);
                    log::info!(
                        "[gesture] edge-slide captured: side={} total_dy={} total_dx={}",
                        match edge.side {
                            EdgeSide::Left => "left",
                            EdgeSide::Right => "right",
                        },
                        total_dy,
                        total_dx,
                    );
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
            // Cap at half the y_step so the deadband never rejects a movement
            // large enough to legitimately generate an action step (important
            // when y_max is large but y_step is clamped to a small value).
            let deadband = charger_aware_y_deadband(state.y_max).min(y_step / 2);
            if dy.abs() < deadband {
                // Delta too small — likely EMI noise, skip it.
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
            }
            if volume != 0 {
                log::info!("[gesture] edge-right steps={volume}");
            }
            (brightness, volume)
        }
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
        SendInput, INPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    };
    let inputs: [INPUT; 2] = unsafe {
        [
            make_key_input(VIRTUAL_KEY(0xAF), KEYBD_EVENT_FLAGS(0)), // VK_VOLUME_UP down
            make_key_input(VIRTUAL_KEY(0xAF), KEYEVENTF_KEYUP),      // VK_VOLUME_UP up
        ]
    };
    unsafe {
        SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

#[cfg(windows)]
fn simulate_volume_down() {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY,
    };
    let inputs: [INPUT; 2] = unsafe {
        [
            make_key_input(VIRTUAL_KEY(0xAE), KEYBD_EVENT_FLAGS(0)), // VK_VOLUME_DOWN down
            make_key_input(VIRTUAL_KEY(0xAE), KEYEVENTF_KEYUP),      // VK_VOLUME_DOWN up
        ]
    };
    unsafe {
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
