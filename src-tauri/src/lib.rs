mod commands;
mod hw;
mod state;
pub mod elevated;
mod elev_bridge;

use commands::ai_logs::{write_ai_perf_log, read_ai_perf_logs, open_ai_logs_dir};
use commands::hardware::{get_performance_mode, set_performance_mode, get_charging_threshold, set_charging_threshold, get_perf_debug, get_ecram_map};
use commands::hotkeys::{get_hotkey_config, set_hotkey_config, start_key_detect, get_detected_key, is_hook_active};
use commands::system::{
    get_battery_info, get_display_info, set_brightness, set_hdr,
    set_ai_brightness, get_ai_brightness_config, set_ai_brightness_config,
    get_fan_info, set_fan_mode, get_touchpad_info,
    set_touchpad_sensitivity, set_touchpad_haptics,
    set_touchpad_haptics_intensity, set_touchpad_gesture_screenshot,
    set_touchpad_repress, set_touchpad_edge_slide, get_system_info,
    get_autostart, set_autostart, get_update_status, trigger_driver_scan,
    get_hardware_profile, run_hardware_discovery, install_driver,
    get_available_refresh_rates, set_refresh_rate, set_adaptive_refresh_rate, get_process_list,
    debug_ecram_dump,
};
use state::AppState;
use std::sync::atomic::{AtomicU64, Ordering};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};

/// Millisecond timestamp of when the tray popup was last hidden by the focus-loss handler.
/// Used to debounce the race: click-on-tray-icon → focus-loss → hide fires BEFORE the
/// tray click event, which would otherwise immediately re-show the popup.
static TRAY_HIDDEN_AT_MS: AtomicU64 = AtomicU64::new(0);

/// Millisecond timestamp of when the tray popup was last shown.
/// Guards against Windows giving focus back to the taskbar immediately after we call
/// set_focus() — which fires Focused(false) and would auto-close the popup.
static TRAY_SHOWN_AT_MS: AtomicU64 = AtomicU64::new(0);

/// Open (or show) the main application window.
#[tauri::command]
async fn open_main_window(app: tauri::AppHandle) -> Result<(), String> {
    match app.get_webview_window("main") {
        Some(win) => {
            win.show().map_err(|e| e.to_string())?;
            win.set_focus().map_err(|e| e.to_string())?;
        }
        None => {
            tauri::WebviewWindowBuilder::new(
                &app,
                "main",
                tauri::WebviewUrl::App("index.html?window=main".into()),
            )
            .title("MiControl")
            .inner_size(950.0, 660.0)
            .resizable(true)
            .decorations(true)
            .build()
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    env_logger::Builder::from_default_env()
        .filter_level(log::LevelFilter::Info)
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_shell::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            // Window
            open_main_window,
            resize_tray_popup,
            // Hardware - performance + charging
            get_performance_mode,
            set_performance_mode,
            get_charging_threshold,
            set_charging_threshold,
            get_perf_debug,
            get_ecram_map,
            // System info
            get_system_info,
            // Battery
            get_battery_info,
            // Display
            get_display_info,
            set_brightness,
            set_hdr,
            set_ai_brightness,
            get_ai_brightness_config,
            set_ai_brightness_config,
            // Fan
            get_fan_info,
            set_fan_mode,
            // Touchpad
            get_touchpad_info,
            set_touchpad_sensitivity,
            set_touchpad_haptics,
            set_touchpad_haptics_intensity,
            set_touchpad_gesture_screenshot,
            set_touchpad_repress,
            set_touchpad_edge_slide,
            // Startup
            get_autostart,
            set_autostart,
            // Update Nucleus (Phase 9)
            get_update_status,
            trigger_driver_scan,
            // Hardware Discovery (Phase 10)
            get_hardware_profile,
            run_hardware_discovery,
            install_driver,
            // Hotkeys (keyboard remapping)
            get_hotkey_config,
            set_hotkey_config,
            start_key_detect,
            get_detected_key,
            is_hook_active,
            // Display refresh rate
            get_available_refresh_rates,
            set_refresh_rate,
            set_adaptive_refresh_rate,
            // Process list
            get_process_list,
            // AI performance logs
            write_ai_perf_log,
            read_ai_perf_logs,
            open_ai_logs_dir,
            // ECRAM debug
            debug_ecram_dump,
        ])
        .setup(|app| {
            // Hardware discovery — load cached profile or scan on first run
            let data_dir = app
                .path()
                .app_data_dir()
                .ok();
            crate::hw::discovery::init(data_dir);

            // Start keyboard hook (intercepts Xiaomi AI / PCManager / Copilot keys)
            crate::hw::hotkeys::start_hook();

            // Register focus callback: Xiaomi key / AI key / Copilot key fires this.
            // We toggle the tray quick-access popup, exactly like XiaomiPCManager did.
            // WebviewWindow show/hide/set_focus are thread-safe in Tauri v2 (dispatched
            // through the winit event loop internally), so we call them directly here.
            // Do NOT wrap in run_on_main_thread — the WMI thread is NOT the main thread,
            // but run_on_main_thread would queue the task and return before it executes,
            // meaning the TRAY_SHOWN_AT_MS store and focus-loss guard race with each other.
            {
                let app_handle = app.handle().clone();
                crate::hw::hotkeys::set_focus_callback(Box::new(move || {
                    match app_handle.get_webview_window("tray") {
                        None => log::warn!("[tray] focus_callback: popup window not found (tray pre-creation failed?)"),
                        Some(popup) => {
                            if popup.is_visible().unwrap_or(false) {
                                log::info!("[tray] focus_callback: hiding popup");
                                let _ = popup.hide();
                            } else {
                                log::info!("[tray] focus_callback: showing popup");
                                position_popup_at_tray(&popup);
                                TRAY_SHOWN_AT_MS.store(now_ms(), Ordering::Relaxed);
                                if let Err(e) = popup.show() {
                                    log::error!("[tray] popup.show() error: {e}");
                                } else {
                                    // Re-position after show: a hidden window may report a
                                    // wrong scale_factor() / inner_size() before it's been
                                    // associated with a monitor.  The second call uses the
                                    // real values now that the window is visible.
                                    position_popup_at_tray(&popup);
                                    if let Ok(pos) = popup.outer_position() {
                                        log::info!("[tray] focus_callback shown at outer_pos=({},{}) is_visible={}",
                                            pos.x, pos.y, popup.is_visible().unwrap_or(false));
                                    }
                                    let _ = popup.set_focus();
                                }
                            }
                        }
                    }
                }));
            }

            // Register open-main-window callback for the `OpenMainWindow` hotkey action.
            {
                let app_handle = app.handle().clone();
                crate::hw::hotkeys::set_open_main_callback(Box::new(move || {
                    let app = app_handle.clone();
                    let _ = app_handle.run_on_main_thread(move || {
                        open_window_sync(&app);
                    });
                }));
            }

            // Start touchpad gesture listener (5-finger screenshot, edge slide volume/brightness)
            crate::hw::touchpad::start_gesture_listener();

            // Give the gesture thread access to the app handle so it can show the OSD.
            crate::hw::touchpad::set_app_handle(app.handle().clone());

            // Start the native Win32 brightness OSD (GDI layered window, no WebView2).
            #[cfg(windows)]
            crate::hw::osd::init();

            // Start adaptive brightness background task
            tauri::async_runtime::spawn(crate::hw::display::adaptive_brightness_loop());

            // Build system tray menu
            let quit = MenuItem::with_id(app, "quit", "Quit MiControl", true, None::<&str>)?;
            let open = MenuItem::with_id(app, "open", "Open MiControl", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &quit])?;

            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("MiControl")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => {
                        app.exit(0);
                    }
                    "open" => {
                        let _ = open_window_sync(app);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, position, .. } = event {
                        // NOTE: on_tray_icon_event fires on the main thread in Tauri v2 (Windows
                        // message pump).  Call toggle_tray_popup directly — do NOT wrap in
                        // run_on_main_thread, that would deadlock the message pump.
                        let app = tray.app_handle();
                        toggle_tray_popup(app, &position);
                    }
                })
                .build(app)?;

            // Pre-create the tray popup window (hidden) so the first click is instant.
            // WebView2 initialisation takes 2-5 s; doing it eagerly at startup avoids
            // that cold-start delay when the user first clicks the tray icon.
            match tauri::WebviewWindowBuilder::new(
                app,
                "tray",
                tauri::WebviewUrl::App("index.html?window=tray".into()),
            )
            .title("")
            .inner_size(300.0, 460.0)
            .decorations(false)
            .transparent(true)
            .shadow(false)
            .resizable(false)
            .always_on_top(true)
            .skip_taskbar(true)
            .visible(false)
            .build() {
                Ok(_)  => log::info!("[tray] pre-created tray popup OK"),
                Err(e) => log::error!("[tray] FAILED to pre-create tray popup: {e}"),
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    // In dev mode: allow the window to close so the process exits when
                    // the Vite dev server stops (Ctrl+C). Without this the Tauri binary
                    // stays alive as a zombie and the next `tauri dev` spawns a duplicate.
                    if cfg!(not(debug_assertions)) {
                        // Production: hide to tray instead of closing.
                        window.hide().ok();
                        api.prevent_close();
                    }
                }
                tauri::WindowEvent::Focused(false) => {
                    // Auto-hide tray popup when it loses focus.
                    // Guard 1: ignore focus-loss for 500 ms after the popup was shown
                    //          (Windows gives focus back to the taskbar right after our
                    //          set_focus() call on the first tray-icon click).
                    // Guard 2: record the hide timestamp so toggle_tray_popup can tell
                    //          whether the focus-loss was caused by a tray-icon click
                    //          (mouse-down steals focus before mouse-up fires Click).
                    if window.label() == "tray" {
                        let age = now_ms().saturating_sub(TRAY_SHOWN_AT_MS.load(Ordering::Relaxed));
                        log::info!("[tray] Focused(false): age_since_shown={age}ms");
                        if age < 500 {
                            return; // too soon after show — ignore this focus-loss
                        }
                        TRAY_HIDDEN_AT_MS.store(now_ms(), Ordering::Relaxed);
                        window.hide().ok();
                    }
                }
                _ => {}
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running MiControl");
}

/// Resize the tray popup window, keeping the bottom edge fixed (grows upward).
/// `height` is in logical (CSS) pixels, as reported by ResizeObserver on the frontend.
#[tauri::command]
async fn resize_tray_popup(app: tauri::AppHandle, height: f64) -> Result<(), String> {
    const MIN_H: f64 = 200.0;
    const MAX_H: f64 = 780.0;
    let height = height.clamp(MIN_H, MAX_H);
    if let Some(window) = app.get_webview_window("tray") {
        // Only resize when visible — avoid corrupting the hidden position
        if !window.is_visible().unwrap_or(false) {
            return Ok(());
        }
        let scale = window.scale_factor().map_err(|e| e.to_string())?;
        let pos   = window.outer_position().map_err(|e| e.to_string())?;
        let cur   = window.inner_size().map_err(|e| e.to_string())?;
        // Anchor: physical y of the bottom edge
        let bottom_phys = pos.y + cur.height as i32;
        let new_h_phys  = (height * scale).round() as u32;
        let new_y       = (bottom_phys - new_h_phys as i32).max(0);
        // Apply — size first, then position so there's no flicker
        window.set_size(tauri::PhysicalSize::new(cur.width, new_h_phys))
              .map_err(|e| e.to_string())?;
        window.set_position(tauri::PhysicalPosition::new(pos.x, new_y))
              .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Current time in milliseconds (monotonic-ish, using SystemTime).
/// Used for the tray popup focus-loss debounce.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn open_window_sync(app: &tauri::AppHandle) {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.set_focus();
    } else {
        let _ = tauri::WebviewWindowBuilder::new(
            app,
            "main",
            tauri::WebviewUrl::App("index.html?window=main".into()),
        )
        .title("MiControl")
        .inner_size(950.0, 660.0)
        .resizable(true)
        .decorations(true)
        .build();
    }
}

/// Toggle the tray quick-access popup near the taskbar.
/// Left-click on the tray icon calls this; subsequent clicks toggle visibility.
fn toggle_tray_popup(app: &tauri::AppHandle, click_pos: &tauri::PhysicalPosition<f64>) {
    log::info!("[tray] toggle_tray_popup click=({:.0},{:.0})", click_pos.x, click_pos.y);
    // If popup exists and is visible, hide it (toggle off)
    if let Some(popup) = app.get_webview_window("tray") {
        let visible = popup.is_visible().unwrap_or(false);
        log::info!("[tray] popup found, is_visible={visible}");
        if visible {
            let _ = popup.hide();
            return;
        }
        // Popup is hidden.  Check whether it was just hidden by the focus-loss
        // handler that fired when the user clicked the tray icon (mouse-down on
        // tray area steals focus before mouse-up fires the TrayIconEvent::Click).
        // If hidden less than 300 ms ago, treat this click as a toggle-off and
        // do NOT re-show — the popup should stay closed.
        let elapsed = now_ms().saturating_sub(TRAY_HIDDEN_AT_MS.load(Ordering::Relaxed));
        log::info!("[tray] elapsed_since_hidden={elapsed}ms");
        if elapsed < 300 {
            log::info!("[tray] debounce active, aborting show");
            return;
        }
        // Exists but hidden long enough ago — reposition and show
        position_popup(&popup, click_pos);
        TRAY_SHOWN_AT_MS.store(now_ms(), Ordering::Relaxed);
        match popup.show() {
            Ok(_) => {
                // Re-position after show: a hidden window may have reported a wrong
                // scale_factor() / inner_size() before it was associated with a monitor.
                position_popup(&popup, click_pos);
                if let Ok(pos) = popup.outer_position() {
                    log::info!("[tray] show() OK — outer_pos=({},{}) is_visible={}",
                        pos.x, pos.y, popup.is_visible().unwrap_or(false));
                }
                let _ = popup.set_focus();
            }
            Err(e) => log::error!("[tray] show() FAILED: {e}"),
        }
        return;
    }

    log::warn!("[tray] popup window not found — creating on-demand (pre-creation must have failed)");
    // Fallback: pre-creation at startup failed — create the window now.
    let popup = match tauri::WebviewWindowBuilder::new(
        app,
        "tray",
        tauri::WebviewUrl::App("index.html?window=tray".into()),
    )
    .title("")
    .inner_size(300.0, 460.0)
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .resizable(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible(false)
    .build()
    {
        Ok(w) => w,
        Err(e) => {
            log::error!("[tray] Failed to build tray popup on-demand: {e}");
            return;
        }
    };

    position_popup(&popup, click_pos);
    TRAY_SHOWN_AT_MS.store(now_ms(), Ordering::Relaxed);
    match popup.show() {
        Ok(_) => {
            log::info!("[tray] on-demand show() OK — is_visible={}", popup.is_visible().unwrap_or(false));
            let _ = popup.set_focus();
        }
        Err(e) => log::error!("[tray] on-demand show() FAILED: {e}"),
    }
}

/// Position the popup window flush above the taskbar, centred on the tray icon.
/// Uses GetMonitorInfo to find the work-area bottom so the result is always
/// just above the taskbar regardless of taskbar height, size, or DPI.
/// Uses the window's CURRENT height so that a previous dynamic resize is honoured.
fn position_popup(window: &tauri::WebviewWindow, click_pos: &tauri::PhysicalPosition<f64>) {
    const POPUP_W: f64 = 300.0;   // logical px — matches .tray-popup CSS width
    const POPUP_H_DEFAULT: f64 = 460.0; // fallback before first dynamic resize
    const GAP: f64 = 8.0;         // logical px gap above taskbar
    let scale = window.scale_factor().unwrap_or(1.0);
    let pw = POPUP_W * scale;
    // Guard: a hidden window may report height=0 before first render; fall back to default.
    let ph = window.inner_size()
        .map(|s| if s.height > 0 { s.height as f64 } else { POPUP_H_DEFAULT * scale })
        .unwrap_or(POPUP_H_DEFAULT * scale);
    let gap = GAP * scale;

    // Get the work area (screen minus taskbar) in physical pixels for the
    // monitor that contains the tray icon click.
    #[cfg(windows)]
    let (work_right, work_bottom) = {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        };
        unsafe {
            let pt = POINT { x: click_pos.x as i32, y: click_pos.y as i32 };
            let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..std::mem::zeroed()
            };
            if GetMonitorInfoW(hmon, &mut info).as_bool() {
                (info.rcWork.right as f64, info.rcWork.bottom as f64)
            } else {
                (click_pos.x + pw / 2.0 + 1.0, click_pos.y)
            }
        }
    };
    #[cfg(not(windows))]
    let (work_right, work_bottom) = (click_pos.x + pw / 2.0 + 1.0, click_pos.y);

    // X: centred on the click, clamped so it doesn't overflow the right edge.
    let x = (click_pos.x - pw / 2.0).max(0.0).min(work_right - pw).round() as i32;
    // Y: popup bottom sits at work-area bottom (top of taskbar) minus a small gap.
    let y = (work_bottom - ph - gap).max(0.0).round() as i32;
    log::info!("[tray] position_popup: scale={scale:.2} pw={pw:.0} ph={ph:.0} work=({work_right:.0},{work_bottom:.0}) → pos=({x},{y})");
    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
}

/// Position the tray popup at the bottom-right of the work area (near system tray).
/// Used when toggling via hotkey where there is no tray-icon click position.
fn position_popup_at_tray(window: &tauri::WebviewWindow) {
    const POPUP_W: f64 = 300.0;
    const GAP: f64 = 8.0;
    let scale = window.scale_factor().unwrap_or(1.0);
    let pw = POPUP_W * scale;
    // Guard: a hidden window may report height=0 before first render; fall back to default.
    let ph = window.inner_size()
        .map(|s| if s.height > 0 { s.height as f64 } else { 460.0 * scale })
        .unwrap_or(460.0 * scale);
    let gap = GAP * scale;

    #[cfg(windows)]
    let (work_right, work_bottom) = {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
        };
        unsafe {
            let hmon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
            let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..std::mem::zeroed() };
            if GetMonitorInfoW(hmon, &mut info).as_bool() {
                (info.rcWork.right as f64, info.rcWork.bottom as f64)
            } else {
                (1920.0, 1040.0)
            }
        }
    };
    #[cfg(not(windows))]
    let (work_right, work_bottom) = (1920.0_f64, 1040.0_f64);

    // Align popup bottom-right of the work area (system tray is bottom-right)
    let x = (work_right - pw - gap).max(0.0).round() as i32;
    let y = (work_bottom - ph - gap).max(0.0).round() as i32;
    log::info!("[tray] position_popup_at_tray: scale={scale:.2} pw={pw:.0} ph={ph:.0} work=({work_right:.0},{work_bottom:.0}) → pos=({x},{y})");
    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
}
