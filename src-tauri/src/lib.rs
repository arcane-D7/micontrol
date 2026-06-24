//! miPC Tauri application library.
//!
//! Sets up Tauri commands, application state, menu, tray, and event handling.
//! This is the main entry point for the Tauri application runtime.

mod commands;
mod debug_log;
mod elev_bridge;
pub mod elevated;
mod hw;
mod state;
pub mod util;

use commands::ai::{analyze_system, get_ai_usage, reset_ai_usage, test_connection};
use commands::ai_logs::{open_ai_logs_dir, read_ai_perf_logs, write_ai_perf_log};
use commands::hardware::{
    get_audio_devices, get_audio_volume, get_cast_devices, get_charging_threshold, get_ecram_map,
    get_iot_bind_status, get_iot_device_id, get_iot_device_info, get_iot_device_status,
    get_iot_fw_version, get_iot_model, get_iot_region_hex, get_iot_wifi_by_index,
    get_iot_wifi_count, get_iot_wifi_status, get_perf_debug, get_performance_mode,
    iot_connect_wifi, iot_delete_wifi_item, iot_empty_wifi_items, iot_notify_ec_event,
    iot_notify_power_event, iot_pipe_available, iot_report_shutting_down, iot_report_suspending,
    iot_report_windows_ready, iot_reset_device, iot_set_device_status, iot_write_wifi_item,
    is_elevated, read_ecram_raw, relaunch_as_admin, send_iot_laptop_status, set_audio_mute,
    set_audio_volume, set_charging_threshold, set_performance_mode, start_casting, stop_casting,
    wifi_connect, wifi_disconnect, wifi_scan, wifi_status, write_iot_hex,
};
use commands::hotkeys::{
    get_detected_key, get_hotkey_config, is_hook_active, set_hotkey_config, start_key_detect,
};
use commands::system::{
    debug_ecram_dump, get_ai_brightness_config, get_autostart, get_available_refresh_rates,
    get_battery_info, get_display_info, get_fan_info, get_hardware_profile,
    get_hardware_state_batch, get_process_list, get_system_info, get_touchpad_info,
    get_update_status, install_driver, run_hardware_discovery, set_adaptive_refresh_rate,
    set_ai_brightness, set_ai_brightness_config, set_autostart, set_brightness, set_fan_mode,
    set_hdr, set_refresh_rate, set_touchpad_edge_slide, set_touchpad_gesture_screenshot,
    set_touchpad_haptics, set_touchpad_haptics_intensity, set_touchpad_repress,
    set_touchpad_sensitivity, trigger_driver_scan,
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

// ── Data deletion (GDPR Art.17, S10-012) ─────────────────────────────────────

#[tauri::command]
fn delete_all_user_data(
    app: tauri::AppHandle,
) -> Result<util::data_deletion::DeleteDataReport, String> {
    util::data_deletion::delete_all_user_data(&app)
}

#[tauri::command]
fn rotate_logs(app: tauri::AppHandle) -> Result<u32, String> {
    util::data_deletion::rotate_logs(&app)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    util::panic::install_panic_hook();
    if let Err(e) = crate::debug_log::init_logging() {
        eprintln!("failed to initialize logging: {e:#}");
    }
    if let Some(path) = crate::debug_log::dev_log_path() {
        log::info!(target: "devlog", "current dev log file: {}", path.display());
    }

    // ── Sentry crash reporting ──────────────────────────────────────────────
    // Initialize before the Tauri builder so that panics during setup are caught.
    // The guard MUST leak by std::mem::forget to live for the entire process lifetime.
    // Only initialize Sentry if the user has granted telemetry consent.
    let sentry_consent = util::consent_audit::check_sentry_consent();
    if let Ok(dsn) = std::env::var("SENTRY_DSN") {
        if !dsn.is_empty() && sentry_consent {
            let guard = sentry::init((
                dsn,
                sentry::ClientOptions {
                    release: Some(format!("micontrol@{}", env!("CARGO_PKG_VERSION")).into()),
                    environment: Some(
                        (if cfg!(debug_assertions) {
                            "development"
                        } else {
                            "production"
                        })
                        .into(),
                    ),
                    ..Default::default()
                },
            ));
            log::info!("Sentry crash reporting initialized");
            // Leak the guard so it lives for the entire process lifetime.
            // If dropped, the Sentry client shuts down and stops capturing panics.
            std::mem::forget(guard);
        }
    }

    // ── Initialization order ─────────────────────────────────────────────────
    // 1. Create Tauri builder with managed state
    // 2. Initialize logging
    // 3. Detect hardware profile (discovery::detect_hardware)
    // 4. Initialize global profile (discovery::init)
    // 5. Set profile in AppState
    // 6. Verify task elevation (elevated::verify_task_elevation)
    // 7. Start hardware polling
    // 8. Run Tauri application

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
            get_iot_region_hex,
            write_iot_hex,
            read_ecram_raw,
            is_elevated,
            relaunch_as_admin,
            // IoTService IPC
            iot_pipe_available,
            get_iot_device_info,
            get_iot_model,
            get_iot_fw_version,
            get_iot_bind_status,
            get_iot_device_id,
            get_iot_device_status,
            send_iot_laptop_status,
            iot_report_windows_ready,
            get_iot_wifi_status,
            get_iot_wifi_count,
            get_iot_wifi_by_index,
            iot_connect_wifi,
            iot_write_wifi_item,
            iot_delete_wifi_item,
            iot_empty_wifi_items,
            iot_set_device_status,
            iot_reset_device,
            iot_notify_power_event,
            iot_notify_ec_event,
            iot_report_suspending,
            iot_report_shutting_down,
            // Audio
            get_audio_devices,
            get_audio_volume,
            set_audio_volume,
            set_audio_mute,
            // Screen Cast
            get_cast_devices,
            start_casting,
            stop_casting,
            // WiFi
            wifi_scan,
            wifi_status,
            wifi_connect,
            wifi_disconnect,
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
            // AI analysis
            analyze_system,
            test_connection,
            get_ai_usage,
            reset_ai_usage,
            // AI performance logs
            write_ai_perf_log,
            read_ai_perf_logs,
            open_ai_logs_dir,
            // ECRAM debug
            debug_ecram_dump,
            // Batched hardware state (S4-002)
            get_hardware_state_batch,
            // Credential store (S6-002)
            commands::credentials::set_secret,
            commands::credentials::get_secret,
            commands::credentials::delete_secret,
            // Data deletion (S10-012)
            delete_all_user_data,
            rotate_logs,
        ])
        .setup(|app| {
            // Hardware discovery — load cached profile or scan on first run
            let data_dir = app
                .path()
                .app_data_dir()
                .ok();
            crate::hw::discovery::init(data_dir);

            // Sync the discovered profile into Tauri managed state
            if let Some(profile) = crate::hw::discovery::global_profile() {
                app.state::<AppState>().set_profile(profile);
            }

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
                .icon(app.default_window_icon().cloned().unwrap_or_else(|| {
                    log::warn!("No default window icon configured, using built-in fallback");
                    tauri::image::Image::from_bytes(include_bytes!("../icons/32x32.png"))
                        .expect("built-in fallback icon to be valid")
                }))
                .tooltip("MiControl")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "quit" => {
                        app.exit(0);
                    }
                    "open" => {
                        open_window_sync(app);
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
                    if cfg!(debug_assertions) {
                        // In dev we keep a hidden tray window pre-created, so simply
                        // allowing close can still leave the process alive. Force full
                        // app shutdown when the main window is closed.
                        if window.label() == "main" {
                            window.app_handle().exit(0);
                        }
                    } else {
                        // Production: hide to tray instead of closing.
                        window.hide().ok();
                        api.prevent_close();
                    }
                }
                tauri::WindowEvent::Focused(false) if window.label() == "tray" => {
                    // Auto-hide tray popup when it loses focus.
                    // Guard 1: ignore focus-loss for 500 ms after the popup was shown
                    //          (Windows gives focus back to the taskbar right after our
                    //          set_focus() call on the first tray-icon click).
                    // Guard 2: record the hide timestamp so toggle_tray_popup can tell
                    //          whether the focus-loss was caused by a tray-icon click
                    //          (mouse-down steals focus before mouse-up fires Click).
                    let age = now_ms().saturating_sub(TRAY_SHOWN_AT_MS.load(Ordering::Relaxed));
                    log::info!("[tray] Focused(false): age_since_shown={age}ms");
                    if age < 500 {
                        return; // too soon after show — ignore this focus-loss
                    }
                    TRAY_HIDDEN_AT_MS.store(now_ms(), Ordering::Relaxed);
                    window.hide().ok();
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
        let pos = window.outer_position().map_err(|e| e.to_string())?;
        let cur = window.inner_size().map_err(|e| e.to_string())?;
        // Anchor: physical y of the bottom edge
        let bottom_phys = pos.y + cur.height as i32;
        let new_h_phys = (height * scale).round() as u32;
        let new_y = (bottom_phys - new_h_phys as i32).max(0);
        // Apply — size first, then position so there's no flicker
        window
            .set_size(tauri::PhysicalSize::new(cur.width, new_h_phys))
            .map_err(|e| e.to_string())?;
        window
            .set_position(tauri::PhysicalPosition::new(pos.x, new_y))
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
    log::info!(
        "[tray] toggle_tray_popup click=({:.0},{:.0})",
        click_pos.x,
        click_pos.y
    );
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
                    log::info!(
                        "[tray] show() OK — outer_pos=({},{}) is_visible={}",
                        pos.x,
                        pos.y,
                        popup.is_visible().unwrap_or(false)
                    );
                }
                let _ = popup.set_focus();
            }
            Err(e) => log::error!("[tray] show() FAILED: {e}"),
        }
        return;
    }

    log::warn!(
        "[tray] popup window not found — creating on-demand (pre-creation must have failed)"
    );
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
            log::info!(
                "[tray] on-demand show() OK — is_visible={}",
                popup.is_visible().unwrap_or(false)
            );
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
    const POPUP_W: f64 = 300.0; // logical px — matches .tray-popup CSS width
    const POPUP_H_DEFAULT: f64 = 460.0; // fallback before first dynamic resize
    const GAP: f64 = 8.0; // logical px gap above taskbar
    let scale = window.scale_factor().unwrap_or(1.0);
    let pw = POPUP_W * scale;
    // Guard: a hidden window may report height=0 before first render; fall back to default.
    let ph = window
        .inner_size()
        .map(|s| {
            if s.height > 0 {
                s.height as f64
            } else {
                POPUP_H_DEFAULT * scale
            }
        })
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
            let pt = POINT {
                x: click_pos.x as i32,
                y: click_pos.y as i32,
            };
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
    let x = (click_pos.x - pw / 2.0)
        .max(0.0)
        .min(work_right - pw)
        .round() as i32;
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
    let ph = window
        .inner_size()
        .map(|s| {
            if s.height > 0 {
                s.height as f64
            } else {
                460.0 * scale
            }
        })
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
            let mut info = MONITORINFO {
                cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                ..std::mem::zeroed()
            };
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
