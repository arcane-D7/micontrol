mod commands;
mod hw;
mod state;
pub mod elevated;
mod elev_bridge;

use commands::hardware::{get_performance_mode, set_performance_mode, get_charging_threshold, set_charging_threshold};
use commands::hotkeys::{get_hotkey_config, set_hotkey_config};
use commands::system::{
    get_battery_info, get_display_info, set_brightness, set_hdr,
    set_ai_brightness, get_ai_brightness_config, set_ai_brightness_config,
    get_fan_info, set_fan_mode, get_touchpad_info,
    set_touchpad_sensitivity, set_touchpad_haptics, get_system_info,
    get_autostart, set_autostart, get_update_status, trigger_driver_scan,
    get_hardware_profile, run_hardware_discovery, install_driver,
};
use state::AppState;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager,
};

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
    env_logger::init();

    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_shell::init())
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            // Window
            open_main_window,
            // Hardware - performance + charging
            get_performance_mode,
            set_performance_mode,
            get_charging_threshold,
            set_charging_threshold,
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
                        let app = tray.app_handle();
                        toggle_tray_popup(app, &position);
                    }
                })
                .build(app)?;

            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    // Hide to tray instead of closing
                    window.hide().ok();
                    api.prevent_close();
                }
                tauri::WindowEvent::Focused(false) => {
                    // Auto-hide tray popup when it loses focus
                    if window.label() == "tray" {
                        window.hide().ok();
                    }
                }
                _ => {}
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running MiControl");
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
    // If popup exists and is visible, hide it (toggle off)
    if let Some(popup) = app.get_webview_window("tray") {
        if popup.is_visible().unwrap_or(false) {
            let _ = popup.hide();
            return;
        }
        // Exists but hidden — reposition and show
        position_popup(&popup, click_pos);
        let _ = popup.show();
        let _ = popup.set_focus();
        return;
    }

    // First use: create the popup window
    let popup = match tauri::WebviewWindowBuilder::new(
        app,
        "tray",
        tauri::WebviewUrl::App("index.html?window=tray".into()),
    )
    .title("")
    .inner_size(320.0, 460.0)
    .decorations(false)
    .resizable(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible(false)
    .build()
    {
        Ok(w) => w,
        Err(e) => {
            log::error!("Failed to build tray popup: {e}");
            return;
        }
    };

    position_popup(&popup, click_pos);
    let _ = popup.show();
    let _ = popup.set_focus();
}

/// Position the popup window above and centred on the tray icon click position.
fn position_popup(window: &tauri::WebviewWindow, click_pos: &tauri::PhysicalPosition<f64>) {
    const POPUP_W: f64 = 320.0;
    const POPUP_H: f64 = 460.0;
    const GAP: f64 = 12.0;
    let x = (click_pos.x - POPUP_W / 2.0).max(0.0).round() as i32;
    let y = (click_pos.y - POPUP_H - GAP).max(0.0).round() as i32;
    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
}
