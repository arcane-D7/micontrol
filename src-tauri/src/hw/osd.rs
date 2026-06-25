//! Native Win32 brightness OSD overlay.
//!
//! Renders a Windows 11-style pill-card OSD for brightness, mic mute,
//! and keyboard backlight changes. No WebView2 dependency.

// hw/osd.rs
//
// Native Win32 brightness OSD — Windows 11 style.
// Pill card, spinning sun icon on entry, smooth fade-in / fade-out.
// Also handles notification OSD for mic mute / keyboard backlight.
// No WebView2 or Tauri IPC.

#![allow(non_snake_case)]

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering};

use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, EndPaint, FillRect, GetMonitorInfoW,
    GetStockObject, InvalidateRect, MonitorFromPoint, RoundRect, SelectObject, SetBkMode,
    SetGraphicsMode, SetTextAlign, SetTextColor, SetWorldTransform, TextOutW, CLIP_DEFAULT_PRECIS,
    DEFAULT_CHARSET, GM_ADVANCED, GM_COMPATIBLE, HBRUSH, HDC, HGDIOBJ, MONITORINFO,
    MONITOR_DEFAULTTOPRIMARY, NULL_BRUSH, NULL_PEN, OUT_DEFAULT_PRECIS, PAINTSTRUCT, TA_CENTER,
    TA_LEFT, TRANSPARENT, XFORM,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, KillTimer, LoadCursorW,
    PostMessageW, RegisterClassExW, SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow,
    TranslateMessage, CS_HREDRAW, CS_VREDRAW, IDC_ARROW, LWA_ALPHA, LWA_COLORKEY, MSG,
    SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_SHOWNOACTIVATE, WM_ERASEBKGND, WM_PAINT, WM_TIMER,
    WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};

// ── Shared state between gesture thread and OSD message loop ─────────────────

static OSD_HWND: AtomicUsize = AtomicUsize::new(0);
static OSD_LEVEL: AtomicU8 = AtomicU8::new(50);
static OSD_HIDE_VER: AtomicU64 = AtomicU64::new(0);
static OSD_ALPHA: AtomicU8 = AtomicU8::new(0);
static OSD_SPIN_FRAME: AtomicU8 = AtomicU8::new(0);
/// 0=hidden  1=entering(fade-in+spin)  2=showing  3=leaving(fade-out)
static OSD_ANIM_PHASE: AtomicU8 = AtomicU8::new(0);
/// Tracks mic mute state locally.
static MIC_MUTED: AtomicBool = AtomicBool::new(false);

/// 0 = brightness  1 = mic-muted  2 = mic-active  3 = keyboard-light
static OSD_MODE: AtomicU8 = AtomicU8::new(0);
/// Unicode codepoint (u16 stored in u32) of the notification icon to draw.
/// Used when OSD_MODE != 0.
static OSD_NOTIF_ICON: AtomicU32 = AtomicU32::new(0);
/// Current keyboard backlight level (0–10). 0xFF = unknown (not reported by this event path).
static OSD_KBL_LEVEL: AtomicU8 = AtomicU8::new(0xFF);

// ── Layout constants (96-dpi logical pixels) ─────────────────────────────────

const OSD_W: i32 = 368;
const OSD_H: i32 = 92; // double height × 1.15
const CORNER_R: i32 = 46; // = OSD_H/2 → perfect pill
const NOTIF_W: i32 = 420; // notification pill width  (2.5× fonts)
const NOTIF_H: i32 = 150; // notification pill height (2.5× fonts)
const NOTIF_R: i32 = NOTIF_H / 2; // = 75 → perfect pill corners
const BAR_X: i32 = 64;
const BAR_END: i32 = 354; // OSD_W - 14
const BAR_H: i32 = 8; // original 6 × 1.15 × 1.20
const BAR_Y: i32 = 42; // (OSD_H - BAR_H) / 2
const ICON_X: i32 = 18;
const ICON_Y: i32 = 34; // (OSD_H - ICON_SZ) / 2
const ICON_SZ: i32 = 23;

// ── Colours (COLORREF = 0x00BBGGRR) ──────────────────────────────────────────

const fn rgb(r: u8, g: u8, b: u8) -> COLORREF {
    COLORREF(r as u32 | ((g as u32) << 8) | ((b as u32) << 16))
}

const COLORKEY: COLORREF = rgb(255, 0, 254); // transparent corners (hot magenta)
const BG: COLORREF = rgb(31, 31, 31); // near-black (Windows dark OSD)
const BAR_TRACK: COLORREF = rgb(68, 68, 68); // unfilled track (medium gray)
const BAR_FILL: COLORREF = rgb(0, 120, 212); // filled — Windows accent #0078D4
const ICON_CLR: COLORREF = rgb(255, 255, 255); // icon colour (white)

const TIMER_HIDE: usize = 1;
const TIMER_ANIM: usize = 2;
const HIDE_MS: u32 = 1500;
const ANIM_MS: u32 = 16; // ~60 fps
const SPIN_TOTAL_FRAMES: u8 = 60; // 960 ms full rotation
const FADE_IN_ALPHA_STEP: u8 = 28; // 0 → 255 in ~9 frames ≈ 150 ms
const FADE_OUT_ALPHA_STEP: u8 = 26; // 255 → 0 in ~10 frames ≈ 160 ms
/// Custom message posted from gesture thread to the OSD message loop.
const WM_OSD_SHOW: u32 = 0x0401; // WM_USER + 1  — brightness / default
const WM_OSD_NOTIF: u32 = 0x0402; // WM_USER + 2  — notification OSD (mic/keyboard)

// Notification-OSD colours
const TEXT_CLR: COLORREF = rgb(235, 235, 235); // near-white label text
const TEXT2_CLR: COLORREF = rgb(160, 160, 160); // secondary lighter text

// ── Public API ────────────────────────────────────────────────────────────────

/// Spawn the OSD message-loop thread.  Must be called once before any gesture.
pub fn init() {
    // S27-006: Graceful degradation — OSD is non-critical (brightness overlay).
    if let Err(e) = std::thread::Builder::new()
        .name("osd-msg-loop".into())
        .spawn(|| {
            // SAFETY: run_message_loop creates and manages a Win32 window on a
            // dedicated thread. All Win32 calls (RegisterClassExW, CreateWindowExW,
            // GetMessageW, DispatchMessageW) are FFI. The thread has no borrow
            // conflicts with the main thread because shared state goes through
            // atomics (OSD_HWND, OSD_ALPHA, etc.).
            unsafe { run_message_loop() }
        })
    {
        log::warn!("OSD thread spawn failed, continuing without OSD: {e}");
    }
}

/// Show (or refresh) the brightness OSD.  Safe to call from any thread.
pub fn show_brightness_osd(level: u8) {
    OSD_MODE.store(0, Ordering::Relaxed);
    OSD_LEVEL.store(level, Ordering::Relaxed);
    OSD_HIDE_VER.fetch_add(1, Ordering::Relaxed);
    let raw = OSD_HWND.load(Ordering::Relaxed);
    if raw == 0 {
        return;
    }
    let hwnd = HWND(raw as *mut core::ffi::c_void);
    // SAFETY: hwnd was created by CreateWindowExW in run_message_loop and
    // stored atomically in OSD_HWND; it remains valid for the window's lifetime.
    // PostMessageW is thread-safe and does not require the calling thread to own
    // the window (it merely queues a message).
    unsafe {
        let _ = PostMessageW(hwnd, WM_OSD_SHOW, WPARAM(0), LPARAM(0));
    }
}

/// Show mic mute OSD with known mute state (e.g. from WMI event).
/// Stores the state and shows the appropriate icon and label.
/// Safe to call from any thread.
pub fn show_mic_mute_osd(muted: bool) {
    MIC_MUTED.store(muted, Ordering::Relaxed);
    log::info!("[osd] Mic mute OSD: muted={}", muted);
    // Icon: U+E8D4 = MicOff (Segoe MDL2), U+E720 = Microphone (active)
    let (mode, icon) = if muted {
        (1u8, 0xE8D4u16)
    } else {
        (2u8, 0xE720u16)
    };
    show_notification_osd(mode, icon);
}

/// Toggle mic mute state and show OSD.
/// Use from Win32 HID consumer path where the resulting state is not known.
/// Safe to call from any thread.
pub fn show_mic_mute_osd_toggle() {
    let muted = !MIC_MUTED.fetch_xor(true, Ordering::Relaxed);
    log::info!("[osd] Mic mute OSD (toggle): muted={}", muted);
    let (mode, icon) = if muted {
        (1u8, 0xE8D4u16)
    } else {
        (2u8, 0xE720u16)
    };
    show_notification_osd(mode, icon);
}

/// Show keyboard backlight OSD.
/// `level`: 0–10 = current backlight level; 0xFF = level unknown.
/// Safe to call from any thread.
pub fn show_keyboard_osd(level: u8) {
    OSD_KBL_LEVEL.store(level, Ordering::Relaxed);
    log::info!("[osd] Keyboard backlight OSD: level={}", level);
    // Icon: U+E765 = Keyboard (Segoe MDL2)
    show_notification_osd(3, 0xE765);
}

/// Internal helper — set mode+icon, post WM_OSD_NOTIF.
fn show_notification_osd(mode: u8, icon_cp: u16) {
    OSD_MODE.store(mode, Ordering::Relaxed);
    OSD_NOTIF_ICON.store(icon_cp as u32, Ordering::Relaxed);
    OSD_HIDE_VER.fetch_add(1, Ordering::Relaxed);
    let raw = OSD_HWND.load(Ordering::Relaxed);
    if raw == 0 {
        return;
    }
    let hwnd = HWND(raw as *mut core::ffi::c_void);
    // SAFETY: hwnd was created by CreateWindowExW and stored atomically in
    // OSD_HWND; it remains valid for the OSD window's lifetime. PostMessageW
    // is safe to call from any thread (merely queues a message to the queue).
    unsafe {
        let _ = PostMessageW(hwnd, WM_OSD_NOTIF, WPARAM(0), LPARAM(0));
    }
}

// ── WASAPI mic mute query — replaced by local toggle state (see MIC_MUTED) ─────
// The mic key is a TOGGLE: pressing it toggles the mute state.
// We track that toggle locally. This avoids a WASAPI COM dependency and is
// equally correct once the key has been pressed at least once.

// ── Message loop (dedicated thread) ──────────────────────────────────────────

/// # Safety
///
/// Must only be called once from a dedicated thread. Creates and owns a Win32
/// window via RegisterClassExW/CreateWindowExW; the resulting HWND is stored in
/// OSD_HWND for cross-thread message posting. The caller must ensure no concurrent
/// window-procedure registration for the same class name.
unsafe fn run_message_loop() {
    let hinstance = GetModuleHandleW(None).unwrap_or_default().into();

    // Register window class
    let class_name = w!("MiCtrl_BrightnessOSD_v1");
    let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();
    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wnd_proc),
        hInstance: hinstance,
        hCursor: cursor,
        hbrBackground: HBRUSH(GetStockObject(NULL_BRUSH).0),
        lpszClassName: class_name,
        ..Default::default()
    };
    RegisterClassExW(&wc); // ignore duplicate-class error on re-launch

    // Position bottom-centre of primary monitor's work area
    let hmon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let _ = GetMonitorInfoW(hmon, &mut mi);
    let work = mi.rcWork;
    let x = work.left + (work.right - work.left - OSD_W) / 2;
    let y = work.bottom - OSD_H - 48;

    // Create window (hidden initially)
    let hwnd = match CreateWindowExW(
        WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW,
        class_name,
        w!(""),
        WS_POPUP,
        x,
        y,
        OSD_W,
        OSD_H,
        None,
        None,
        hinstance,
        None,
    ) {
        Ok(h) => h,
        Err(_) => return,
    };

    // Colorkey for rounded corners + alpha channel for fade-in/out.
    let _ = SetLayeredWindowAttributes(hwnd, COLORKEY, 0, LWA_COLORKEY | LWA_ALPHA);

    OSD_HWND.store(hwnd.0 as usize, Ordering::Relaxed);

    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

// ── Window procedure ──────────────────────────────────────────────────────────

/// # Safety
///
/// This is a Win32 window procedure (WNDPROC) called by the OS on each message
/// dispatch. hwnd must be a valid window handle created by CreateWindowExW.
/// msg, wp, lp are provided by the OS message pump. The function calls Win32
/// GDI painting functions (BeginPaint, EndPaint, etc.) which require a valid
/// device context obtained within the same WM_PAINT handler.
unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_PAINT => {
            paint(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_TIMER if wp.0 == TIMER_HIDE => {
            let _ = KillTimer(hwnd, TIMER_HIDE);
            // Start fade-out instead of immediate hide.
            OSD_ANIM_PHASE.store(3, Ordering::Relaxed);
            let _ = SetTimer(hwnd, TIMER_ANIM, ANIM_MS, None);
            LRESULT(0)
        }
        WM_TIMER if wp.0 == TIMER_ANIM => {
            handle_anim_frame(hwnd);
            LRESULT(0)
        }
        _ if msg == WM_OSD_SHOW => {
            OSD_MODE.store(0, Ordering::Relaxed); // ensure brightness mode
            reposition_osd(hwnd, OSD_W, OSD_H);
            start_show_animation(hwnd);
            LRESULT(0)
        }
        _ if msg == WM_OSD_NOTIF => {
            // Notification mode (mic / keyboard): resize to bigger pill, then fade-in.
            reposition_osd(hwnd, NOTIF_W, NOTIF_H);
            start_show_animation(hwnd);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

// ── Animation helpers ────────────────────────────────────────────────────────────────

/// Resize + reposition the OSD window to bottom-centre of the primary monitor.
/// Safe to call even when the window is already at the requested size.
///
/// # Safety
///
/// # Safety
///
/// hwnd must be a valid window handle created by CreateWindowExW and not yet
/// destroyed. Calls GetMonitorInfoW, MonitorFromPoint, and SetWindowPos which
/// are FFI functions that require valid pointers and handles.
unsafe fn reposition_osd(hwnd: HWND, w: i32, h: i32) {
    let hmon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let _ = GetMonitorInfoW(hmon, &mut mi);
    let work = mi.rcWork;
    let x = work.left + (work.right - work.left - w) / 2;
    let y = work.bottom - h - 48;
    let _ = SetWindowPos(hwnd, None, x, y, w, h, SWP_NOACTIVATE | SWP_NOZORDER);
}

/// # Safety
///
/// hwnd must be a valid window handle created by CreateWindowExW. Calls Win32
/// timer and show-window functions (SetTimer, KillTimer, ShowWindow,
/// SetLayeredWindowAttributes, InvalidateRect) which require valid handles.
unsafe fn start_show_animation(hwnd: HWND) {
    match OSD_ANIM_PHASE.load(Ordering::Relaxed) {
        1 => {
            // Already fading in — just repaint with the new level; let the
            // ongoing animation finish naturally.
            let _ = InvalidateRect(hwnd, None, true);
        }
        2 => {
            // Fully visible — extend the hide countdown and repaint.
            let _ = KillTimer(hwnd, TIMER_HIDE);
            let _ = SetTimer(hwnd, TIMER_HIDE, HIDE_MS, None);
            let _ = InvalidateRect(hwnd, None, true);
        }
        _ => {
            // Hidden (0) or fading out (3) — play the full enter animation.
            let _ = KillTimer(hwnd, TIMER_HIDE);
            let _ = KillTimer(hwnd, TIMER_ANIM);
            OSD_SPIN_FRAME.store(0, Ordering::Relaxed);
            OSD_ALPHA.store(0, Ordering::Relaxed);
            let _ = SetLayeredWindowAttributes(hwnd, COLORKEY, 0, LWA_COLORKEY | LWA_ALPHA);
            let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
            OSD_ANIM_PHASE.store(1, Ordering::Relaxed);
            let _ = InvalidateRect(hwnd, None, true);
            let _ = SetTimer(hwnd, TIMER_ANIM, ANIM_MS, None);
        }
    }
}

/// # Safety
///
/// hwnd must be a valid window handle. Calls Win32 timer functions (KillTimer,
/// SetTimer), SetLayeredWindowAttributes, InvalidateRect, and ShowWindow which
/// are FFI calls that require valid handles.
unsafe fn handle_anim_frame(hwnd: HWND) {
    match OSD_ANIM_PHASE.load(Ordering::Relaxed) {
        1 => {
            // Fade in + spin simultaneously.
            let new_alpha = OSD_ALPHA
                .load(Ordering::Relaxed)
                .saturating_add(FADE_IN_ALPHA_STEP);
            OSD_ALPHA.store(new_alpha, Ordering::Relaxed);
            let new_sf = (OSD_SPIN_FRAME.load(Ordering::Relaxed) + 1).min(SPIN_TOTAL_FRAMES);
            OSD_SPIN_FRAME.store(new_sf, Ordering::Relaxed);
            let _ = SetLayeredWindowAttributes(hwnd, COLORKEY, new_alpha, LWA_COLORKEY | LWA_ALPHA);
            let _ = InvalidateRect(hwnd, None, true);
            if new_alpha == 255 && new_sf >= SPIN_TOTAL_FRAMES {
                OSD_ANIM_PHASE.store(2, Ordering::Relaxed);
                let _ = KillTimer(hwnd, TIMER_ANIM);
                let _ = SetTimer(hwnd, TIMER_HIDE, HIDE_MS, None);
            }
        }
        3 => {
            // Fade out.
            let alpha = OSD_ALPHA.load(Ordering::Relaxed);
            if alpha <= FADE_OUT_ALPHA_STEP {
                OSD_ALPHA.store(0, Ordering::Relaxed);
                OSD_ANIM_PHASE.store(0, Ordering::Relaxed);
                let _ = KillTimer(hwnd, TIMER_ANIM);
                // Reset to opaque before hiding so next show starts clean.
                let _ = SetLayeredWindowAttributes(hwnd, COLORKEY, 255, LWA_COLORKEY | LWA_ALPHA);
                let _ = ShowWindow(hwnd, SW_HIDE);
            } else {
                let new_alpha = alpha - FADE_OUT_ALPHA_STEP;
                OSD_ALPHA.store(new_alpha, Ordering::Relaxed);
                let _ =
                    SetLayeredWindowAttributes(hwnd, COLORKEY, new_alpha, LWA_COLORKEY | LWA_ALPHA);
                let _ = InvalidateRect(hwnd, None, true);
            }
        }
        _ => {
            let _ = KillTimer(hwnd, TIMER_ANIM);
        }
    }
}

// ── GDI painting ─────────────────────────────────────────────────────────────

/// # Safety
///
/// hwnd must be a valid window handle. Calls BeginPaint/EndPaint which require
/// a valid HWND whose window class was registered with the calling thread.
unsafe fn paint(hwnd: HWND) {
    if OSD_MODE.load(Ordering::Relaxed) == 0 {
        paint_brightness(hwnd);
    } else {
        paint_notification(hwnd);
    }
}

// ── GDI font helpers ──────────────────────────────────────────────────────────

/// Create a Segoe MDL2 Assets icon font.  Positive `height` = cell height.
///
/// # Safety
///
/// Calls CreateFontW (FFI) which allocates a GDI font object. The returned
/// HGDIOBJ must be deleted with DeleteObject when no longer needed.
unsafe fn new_mdl2_font(height: i32) -> HGDIOBJ {
    HGDIOBJ(
        CreateFontW(
            height,
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            DEFAULT_CHARSET.0 as u32,
            OUT_DEFAULT_PRECIS.0 as u32,
            CLIP_DEFAULT_PRECIS.0 as u32,
            4,
            0,
            w!("Segoe MDL2 Assets"),
        )
        .0,
    )
}

/// Create a Segoe UI text font.  Negative `height` = cap height; `weight` 400/600/700.
unsafe fn new_segoe_font(height: i32, weight: i32) -> HGDIOBJ {
    HGDIOBJ(
        CreateFontW(
            height,
            0,
            0,
            0,
            weight,
            0,
            0,
            0,
            DEFAULT_CHARSET.0 as u32,
            OUT_DEFAULT_PRECIS.0 as u32,
            CLIP_DEFAULT_PRECIS.0 as u32,
            5,
            0,
            w!("Segoe UI"),
        )
        .0,
    )
}

/// Map the raw firmware byte to `(bar_fill_pct, display_label)`.
/// Firmware cycles: 0x00 (Off) → 0x05 (33%) → 0x0A (66%) → 0x80 (100%).
fn keyboard_level_info(raw: u8) -> (i32, &'static str) {
    match raw {
        0 => (0, "Off"),
        1..=7 => (33, "33%"),  // 0x05
        8..=63 => (66, "66%"), // 0x0A
        _ => (100, "100%"),    // 0x80
    }
}

// ── Notification OSD painting ─────────────────────────────────────────────────

/// Draw mic-mute (mode 1/2) or keyboard-backlight (mode 3) notification OSD.
///
/// Mic layout  (368 × 92 pill):
///   [mic icon 40 px, coloured]   Microphone  (small, muted-gray)
///                                Muted / Active  (large, bold, coloured)
///
/// Keyboard layout:
///   [kbd icon 28 px, white]   Keyboard Light       (small, muted-gray)
///                             [████████░░░░░░]  50% (bar + percentage)
///                             Medium               (small, muted-gray)
///
/// # Safety
///
/// hwnd must be a valid window handle created by CreateWindowExW. Calls Win32
/// GDI functions (BeginPaint, CreateSolidBrush, SelectObject, RoundRect, etc.)
/// which require a valid HDC obtained from BeginPaint. All GDI resources are
/// cleaned up before the function returns.
///
/// Mic layout  (368 × 92 pill):
///   [mic icon 40 px, coloured]   Microphone  (small, muted-gray)
///                                Muted / Active  (large, bold, coloured)
///
/// Keyboard layout:
///   [kbd icon 28 px, white]   Keyboard Light       (small, muted-gray)
///                             [████████░░░░░░]  50% (bar + percentage)
///                             Medium               (small, muted-gray)
unsafe fn paint_notification(hwnd: HWND) {
    let mode = OSD_MODE.load(Ordering::Relaxed);
    let icon_cp = OSD_NOTIF_ICON.load(Ordering::Relaxed) as u16;

    let mut ps = PAINTSTRUCT::default();
    let hdc: HDC = BeginPaint(hwnd, &mut ps);

    let pen_null = HGDIOBJ(GetStockObject(NULL_PEN).0);
    let orig_pen = SelectObject(hdc, pen_null);

    // 1. Colorkey fill for transparent corners.
    let br_key = CreateSolidBrush(COLORKEY);
    let _ = FillRect(
        hdc,
        &RECT {
            left: 0,
            top: 0,
            right: NOTIF_W,
            bottom: NOTIF_H,
        },
        br_key,
    );
    let _ = DeleteObject(HGDIOBJ(br_key.0));

    // 2. Dark pill background (NOTIF_R = NOTIF_H/2 → perfect half-circle ends).
    let br_bg = CreateSolidBrush(BG);
    let old_br = SelectObject(hdc, HGDIOBJ(br_bg.0));
    let _ = RoundRect(hdc, 0, 0, NOTIF_W, NOTIF_H, NOTIF_R, NOTIF_R);
    SelectObject(hdc, old_br);
    let _ = DeleteObject(HGDIOBJ(br_bg.0));

    SetBkMode(hdc, TRANSPARENT);
    let _ = SetTextAlign(hdc, TA_CENTER); // centre all text horizontally at CX
    const CX: i32 = NOTIF_W / 2; // = 210

    if mode == 1 || mode == 2 {
        // ── Mic mute / active ─────────────────────────────────────────────────
        // Side-by-side, space-evenly: 420 / 3 = 140 → icon at X=140, text at X=280.
        // Both vertically centred — icon centre ≈ Y 74, text block centre ≈ Y 75.
        const ICN_SZ: i32 = 67;
        const CX_R: i32 = 280; // centre of right text zone
        let accent = if mode == 1 {
            rgb(210, 80, 80)
        } else {
            rgb(60, 200, 110)
        };

        // Icon — coloured, left third, vertically centred.
        SetTextColor(hdc, accent);
        let hf = new_mdl2_font(ICN_SZ);
        let old_f = SelectObject(hdc, hf);
        let _ = TextOutW(hdc, 140, (NOTIF_H - ICN_SZ) / 2, &[icon_cp]);
        SelectObject(hdc, old_f);
        let _ = DeleteObject(hf);

        // "Microphone" — font -20, muted-gray, right column.
        SetTextColor(hdc, TEXT2_CLR);
        let hf2 = new_segoe_font(-20, 400);
        let old_f2 = SelectObject(hdc, hf2);
        let lbl: Vec<u16> = "Microphone".encode_utf16().collect();
        let _ = TextOutW(hdc, CX_R, 42, &lbl);
        SelectObject(hdc, old_f2);
        let _ = DeleteObject(hf2);

        // "Muted" / "Active" — font -37, bold, accent colour, right column.
        SetTextColor(hdc, accent);
        let hf3 = new_segoe_font(-37, 700);
        let old_f3 = SelectObject(hdc, hf3);
        let state: Vec<u16> = (if mode == 1 { "Muted" } else { "Active" })
            .encode_utf16()
            .collect();
        let _ = TextOutW(hdc, CX_R, 70, &state);
        SelectObject(hdc, old_f3);
        let _ = DeleteObject(hf3);
    } else {
        // ── Keyboard backlight ────────────────────────────────────────────────
        // Icon: 70 px, white keyboard icon, vertically centred.
        const ICN_SZ: i32 = 47;
        // Icon — white, near top, centred at CX.
        SetTextColor(hdc, ICON_CLR);
        let hf = new_mdl2_font(ICN_SZ);
        let old_f = SelectObject(hdc, hf);
        let _ = TextOutW(hdc, CX, 16, &[icon_cp]);
        SelectObject(hdc, old_f);
        let _ = DeleteObject(hf);

        // Map raw byte to (bar_fill_pct, label).
        let (pct, level_lbl) = keyboard_level_info(OSD_KBL_LEVEL.load(Ordering::Relaxed));

        // "Keyboard Light" label — font -19, muted-gray, centred.
        SetTextColor(hdc, TEXT2_CLR);
        let hf2 = new_segoe_font(-19, 400);
        let old_f2 = SelectObject(hdc, hf2);
        let lbl: Vec<u16> = "Keyboard Light".encode_utf16().collect();
        let _ = TextOutW(hdc, CX, 69, &lbl);
        SelectObject(hdc, old_f2);
        let _ = DeleteObject(hf2);

        // Bar (centred, 256 px wide, 10 px tall).
        const BAR_W: i32 = 256;
        const BAR_L: i32 = CX - BAR_W / 2; // = 82
        const BAR_R: i32 = CX + BAR_W / 2; // = 338
        const BAR_T: i32 = 94;
        const BAR_HT: i32 = 10;

        // Bar track (unfilled, always drawn).
        let br_track = CreateSolidBrush(BAR_TRACK);
        let old_br2 = SelectObject(hdc, HGDIOBJ(br_track.0));
        let _ = RoundRect(hdc, BAR_L, BAR_T, BAR_R, BAR_T + BAR_HT, BAR_HT, BAR_HT);
        SelectObject(hdc, old_br2);
        let _ = DeleteObject(HGDIOBJ(br_track.0));

        // Fill — proportional to pct (0, 33, 66, 100).
        let fill_w = pct * BAR_W / 100;
        if fill_w >= BAR_HT {
            let fill_clr = match pct {
                1..=33 => rgb(80, 140, 220), // 33%  — dim blue
                34..=66 => BAR_FILL,         // 66%  — accent blue
                _ => rgb(255, 200, 80),      // 100% — amber/gold
            };
            let br_fill = CreateSolidBrush(fill_clr);
            let old_br3 = SelectObject(hdc, HGDIOBJ(br_fill.0));
            let _ = RoundRect(
                hdc,
                BAR_L,
                BAR_T,
                BAR_L + fill_w,
                BAR_T + BAR_HT,
                BAR_HT,
                BAR_HT,
            );
            SelectObject(hdc, old_br3);
            let _ = DeleteObject(HGDIOBJ(br_fill.0));
        }

        // Level label — font -23, semi-bold, white, centred below bar.
        SetTextColor(hdc, TEXT_CLR);
        let hf3 = new_segoe_font(-23, 600);
        let old_f3 = SelectObject(hdc, hf3);
        let lvl_u16: Vec<u16> = level_lbl.encode_utf16().collect();
        let _ = TextOutW(hdc, CX, 110, &lvl_u16);
        SelectObject(hdc, old_f3);
        let _ = DeleteObject(hf3);
    }

    let _ = SetTextAlign(hdc, TA_LEFT); // restore default alignment
    SelectObject(hdc, orig_pen);
    let _ = EndPaint(hwnd, &ps);
}

/// # Safety
///
/// hwnd must be a valid window handle created by CreateWindowExW. Calls Win32
/// GDI functions (BeginPaint, CreateSolidBrush, SelectObject, RoundRect,
/// SetWorldTransform, TextOutW, etc.) which require a valid HDC and proper
/// GDI object management. All created GDI objects (brushes, fonts) are deleted
/// and the world transform is reset to identity before returning.
unsafe fn paint_brightness(hwnd: HWND) {
    let level = OSD_LEVEL.load(Ordering::Relaxed) as i32;
    let spin_frame = OSD_SPIN_FRAME.load(Ordering::Relaxed);
    // Map frame 0-30 to degrees 0-360; frame 0 = no transform (icon at rest).
    let spin_deg = (spin_frame as i32 * 360) / SPIN_TOTAL_FRAMES as i32;

    let mut ps = PAINTSTRUCT::default();
    let hdc: HDC = BeginPaint(hwnd, &mut ps);

    let pen_null = HGDIOBJ(GetStockObject(NULL_PEN).0);
    let orig_pen = SelectObject(hdc, pen_null);

    // 1. Colorkey fill — rounded pill corners appear transparent.
    let br_key = CreateSolidBrush(COLORKEY);
    let _ = FillRect(
        hdc,
        &RECT {
            left: 0,
            top: 0,
            right: OSD_W,
            bottom: OSD_H,
        },
        br_key,
    );
    let _ = DeleteObject(HGDIOBJ(br_key.0));

    // 2. Dark pill background.
    let br_bg = CreateSolidBrush(BG);
    let old_br = SelectObject(hdc, HGDIOBJ(br_bg.0));
    let _ = RoundRect(hdc, 0, 0, OSD_W, OSD_H, CORNER_R, CORNER_R);
    SelectObject(hdc, old_br);
    let _ = DeleteObject(HGDIOBJ(br_bg.0));

    // 3. Brightness icon (Segoe MDL2 Assets U+E706) with spin transform on entry.
    SetBkMode(hdc, TRANSPARENT);
    SetTextColor(hdc, ICON_CLR);
    let hfont_icon = CreateFontW(
        ICON_SZ,
        0,
        0,
        0,
        400,
        0,
        0,
        0,
        DEFAULT_CHARSET.0 as u32,
        OUT_DEFAULT_PRECIS.0 as u32,
        CLIP_DEFAULT_PRECIS.0 as u32,
        4, // ANTIALIASED_QUALITY — works better under world transforms
        0,
        w!("Segoe MDL2 Assets"),
    );
    let old_font = SelectObject(hdc, HGDIOBJ(hfont_icon.0));

    // Apply spin (rotation + zoom-in) around the icon centre during entry animation.
    let spinning = spin_frame > 0 && spin_frame < SPIN_TOTAL_FRAMES;
    // Always enter GM_ADVANCED so we can explicitly reset after the icon draw.
    let _ = SetGraphicsMode(hdc, GM_ADVANCED);
    if spinning {
        let t = spin_frame as f32 / SPIN_TOTAL_FRAMES as f32; // 0 → 1
        let scale = 0.3_f32 + 0.7_f32 * t; // zoom 0.3x → 1.0x
        let rad = (spin_deg as f32).to_radians();
        let cos_a = rad.cos();
        let sin_a = rad.sin();
        // Centre of icon in logical coords.
        let cx = (ICON_X + ICON_SZ / 2) as f32;
        let cy = (ICON_Y + ICON_SZ / 2) as f32;
        // Combined scale+rotation XFORM around (cx, cy).
        // x' = x*eM11 + y*eM21 + eDx
        let xform = XFORM {
            eM11: scale * cos_a,
            eM12: scale * sin_a,
            eM21: -scale * sin_a,
            eM22: scale * cos_a,
            eDx: cx * (1.0 - scale * cos_a) + cy * scale * sin_a,
            eDy: cy * (1.0 - scale * cos_a) - cx * scale * sin_a,
        };
        let _ = SetWorldTransform(hdc, &xform);
    }

    let icon: Vec<u16> = vec![0xE706u16];
    let _ = TextOutW(hdc, ICON_X, ICON_Y, &icon);

    // Explicitly reset world transform to identity before drawing the bar.
    let identity = XFORM {
        eM11: 1.0,
        eM12: 0.0,
        eM21: 0.0,
        eM22: 1.0,
        eDx: 0.0,
        eDy: 0.0,
    };
    let _ = SetWorldTransform(hdc, &identity);
    let _ = SetGraphicsMode(hdc, GM_COMPATIBLE);

    SelectObject(hdc, old_font);
    let _ = DeleteObject(HGDIOBJ(hfont_icon.0));

    // 4. Progress bar track (pill).
    let br_track = CreateSolidBrush(BAR_TRACK);
    let ob = SelectObject(hdc, HGDIOBJ(br_track.0));
    let _ = RoundRect(hdc, BAR_X, BAR_Y, BAR_END, BAR_Y + BAR_H, BAR_H, BAR_H);
    SelectObject(hdc, ob);
    let _ = DeleteObject(HGDIOBJ(br_track.0));

    // 5. Progress bar fill (pill, on top of track).
    let fill_x2 = BAR_X + ((BAR_END - BAR_X) * level.clamp(0, 100)) / 100;
    if fill_x2 > BAR_X + BAR_H {
        let br_fill = CreateSolidBrush(BAR_FILL);
        let ob2 = SelectObject(hdc, HGDIOBJ(br_fill.0));
        let _ = RoundRect(hdc, BAR_X, BAR_Y, fill_x2, BAR_Y + BAR_H, BAR_H, BAR_H);
        SelectObject(hdc, ob2);
        let _ = DeleteObject(HGDIOBJ(br_fill.0));
    }

    SelectObject(hdc, orig_pen);
    let _ = EndPaint(hwnd, &ps);
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Layout constants ──────────────────────────────────────────────────────

    #[test]
    fn test_constants_osd_dimensions() {
        assert_eq!(OSD_W, 368);
        assert_eq!(OSD_H, 92);
        assert_eq!(CORNER_R, 46);
    }

    #[test]
    fn test_constants_notification_dimensions() {
        assert_eq!(NOTIF_W, 420);
        assert_eq!(NOTIF_H, 150);
        assert_eq!(NOTIF_R, 75);
    }

    #[test]
    fn test_constants_bar_layout() {
        assert_eq!(BAR_X, 64);
        assert_eq!(BAR_END, 354);
        assert_eq!(BAR_H, 8);
        assert_eq!(BAR_Y, 42);
    }

    #[test]
    fn test_constants_icon_layout() {
        assert_eq!(ICON_X, 18);
        assert_eq!(ICON_Y, 34);
        assert_eq!(ICON_SZ, 23);
    }

    #[test]
    fn test_constants_colours() {
        // COLORREF = 0x00BBGGRR
        assert_eq!(COLORKEY, rgb(255, 0, 254));
        assert_eq!(BG, rgb(31, 31, 31));
        assert_eq!(BAR_TRACK, rgb(68, 68, 68));
        assert_eq!(BAR_FILL, rgb(0, 120, 212));
        assert_eq!(ICON_CLR, rgb(255, 255, 255));
    }

    #[test]
    fn test_colourref_format() {
        // Verify COLORREF byte layout: 0x00BBGGRR
        assert_eq!(rgb(255, 0, 0).0, 0x0000_00FF); // red
        assert_eq!(rgb(0, 255, 0).0, 0x0000_FF00); // green
        assert_eq!(rgb(0, 0, 255).0, 0x00FF_0000); // blue
        assert_eq!(rgb(128, 64, 32).0, 0x0020_4080); // mixed
    }

    #[test]
    fn test_constants_timing() {
        assert_eq!(HIDE_MS, 1500);
        assert_eq!(ANIM_MS, 16);
        assert_eq!(SPIN_TOTAL_FRAMES, 60);
        assert_eq!(FADE_IN_ALPHA_STEP, 28);
        assert_eq!(FADE_OUT_ALPHA_STEP, 26);
    }

    #[test]
    fn test_constants_window_messages() {
        assert_eq!(WM_OSD_SHOW, 0x0401); // WM_USER + 1
        assert_eq!(WM_OSD_NOTIF, 0x0402); // WM_USER + 2
    }

    // ── keyboard_level_info ───────────────────────────────────────────────────

    #[test]
    fn test_keyboard_level_off() {
        let (pct, label) = keyboard_level_info(0);
        assert_eq!(pct, 0);
        assert_eq!(label, "Off");
    }

    #[test]
    fn test_keyboard_level_33_percent() {
        for raw in 1..=7 {
            let (pct, label) = keyboard_level_info(raw);
            assert_eq!(pct, 33);
            assert_eq!(label, "33%");
        }
    }

    #[test]
    fn test_keyboard_level_66_percent() {
        for raw in 8..=63 {
            let (pct, label) = keyboard_level_info(raw);
            assert_eq!(pct, 66);
            assert_eq!(label, "66%");
        }
    }

    #[test]
    fn test_keyboard_level_100_percent() {
        for raw in 64..=255u8 {
            let (pct, label) = keyboard_level_info(raw);
            assert_eq!(pct, 100);
            assert_eq!(label, "100%");
        }
    }

    // ── Atomic initial states ─────────────────────────────────────────────────

    #[test]
    fn test_atomic_initial_hwnd() {
        assert_eq!(OSD_HWND.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_atomic_initial_alpha() {
        assert_eq!(OSD_ALPHA.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_atomic_initial_anim_phase() {
        // 0 = hidden
        assert_eq!(OSD_ANIM_PHASE.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_atomic_initial_spin_frame() {
        assert_eq!(OSD_SPIN_FRAME.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_atomic_initial_osd_mode() {
        // 0 = brightness mode
        assert_eq!(OSD_MODE.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_atomic_initial_mic_muted() {
        assert!(!MIC_MUTED.load(Ordering::Relaxed));
    }

    #[test]
    fn test_default_level_and_kbl() {
        assert_eq!(OSD_LEVEL.load(Ordering::Relaxed), 50);
        assert_eq!(OSD_KBL_LEVEL.load(Ordering::Relaxed), 0xFF);
    }
}
