//! miPC Tauri application library.
//!
//! Sets up Tauri commands, application state, menu, tray, and event handling.
//! This is the main entry point for the Tauri application runtime.

mod bootstrap;
mod commands;
// Public so the integration tests (`tests/logging_e2e.rs`) and any future
// diagnostics command can resolve the log file the app actually writes.
pub mod debug_log;
pub mod elev_bridge;
pub mod elevated;
pub mod errors;
mod health_supervisor;
pub mod hw;
mod state;
pub mod util;

/// Tray popup geometry constants (logical px). Shared by the resizer and both
/// positioners so the clipping caps stay consistent.
const POPUP_W: f64 = 300.0; // matches .tray-popup CSS width
const POPUP_H_DEFAULT: f64 = 460.0; // fallback before the first dynamic resize
const POPUP_GAP: f64 = 8.0; // gap above the taskbar / below the top edge

use commands::ai::{analyze_system, get_ai_usage, reset_ai_usage, test_connection};
use commands::ai_logs::{open_ai_logs_dir, read_ai_perf_logs, write_ai_perf_log};
#[cfg(windows)]
use commands::face::{
    face_camera_preview_frame, face_camera_preview_start, face_camera_preview_stop,
    face_delete_template, face_diagnostics, face_download_models, face_enroll, face_get_settings,
    face_hello_verify, face_install_models, face_list_templates, face_list_users,
    face_models_remove_all, face_models_status, face_password_configured, face_service_ensure,
    face_service_install, face_set_password, face_set_settings, face_status,
};
use commands::hardware::{
    ensure_bridge_service, ensure_iot_service, get_audio_devices, get_audio_volume,
    get_battery_care, get_cast_devices, get_charge_hold_state, get_charge_limit_state,
    get_charging_threshold, get_ecram_map, get_function_key, get_iot_device_info,
    get_iot_region_hex, get_iot_wifi_list, get_perf_debug, get_performance_mode,
    get_primary_thermal_zone, get_scenario_rules, get_thermal_zones, hq_change_boot_option,
    hq_enable_pxe_boot, hq_load_default, hq_s5_rtc_wake_enable, hq_set_performance_mode,
    hq_set_shipping_country_code, hq_set_wifi_country_code, iot_connect_wifi, iot_delete_wifi_item,
    iot_empty_wifi_items, iot_notify_event, iot_pipe_available, iot_reset_device,
    iot_set_device_status, iot_write_wifi_item, is_elevated, read_ecram_raw, relaunch_as_admin,
    set_audio_default_endpoint, set_audio_mute, set_audio_volume, set_battery_care,
    set_charging_threshold, set_function_key, set_performance_mode, set_scenario_rules,
    start_casting, stop_casting, wifi_connect, wifi_disconnect, wifi_scan, wifi_status,
    wmi_ec_get_performance_mode, wmi_ec_read, wmi_ec_read_adapter_power,
    wmi_ec_read_battery_health, wmi_ec_read_sensor_data, wmi_ec_set_auto_illumination,
    wmi_ec_set_epof_flag, wmi_ec_set_label_mode, wmi_ec_set_lid_open_type,
    wmi_ec_set_mi_usage_type, wmi_ec_set_performance_mode, wmi_ec_set_pl1_flag,
    wmi_ec_set_removable_type, wmi_ec_set_sagv_mode, wmi_ec_set_wmid_type, wmi_ec_write,
    write_iot_hex,
};
use commands::hotkeys::{
    get_detected_key, get_hotkey_config, get_remap_apply_state, grant_script_consent,
    is_hook_active, set_hotkey_config, start_key_detect,
};
use commands::privacy::{export_user_data, open_external_url, reveal_in_explorer};
use commands::system::{
    apply_color_profile, clean_junk_files, clear_error_log, custom_security_scan, debug_ecram_dump,
    download_color_profile, find_latest_downloaded_installer, full_security_scan,
    get_ai_brightness_config, get_audio_effects, get_auto_update_config, get_autostart,
    get_available_refresh_rates, get_battery_info, get_color_status, get_crash_recovery_status,
    get_defender_status, get_display_info, get_drivers_detail, get_error_log_config,
    get_eye_protection, get_fan_info, get_hardware_profile, get_hardware_state_batch,
    get_network_perf, get_os_turbo, get_overview_snapshot, get_phone_link_status, get_process_list,
    get_smart_brightness_model, get_startup_items, get_sys_opt_status, get_system_info,
    get_task_manager, get_threat_history, get_touchpad_info, get_update_status, install_driver,
    install_update, kill_process, launch_color_calibration_wizard, launch_phone_link,
    launch_phone_link_feature, load_icc_profile, log_frontend_error, mark_clean_exit,
    open_color_management_settings, open_color_profile_search, open_phone_link_settings,
    open_windows_security, quick_security_scan, read_error_log, reset_smart_brightness_model,
    run_hardware_discovery, scan_junk_files, set_adaptive_refresh_rate, set_ai_brightness,
    set_ai_brightness_config, set_auto_update_beta_feed, set_auto_update_enabled, set_autostart,
    set_brightness, set_error_logging_enabled, set_eye_protection, set_fan_mode, set_hdr,
    set_mic_noise_canceling, set_os_turbo, set_refresh_rate, set_speaker_noise_canceling,
    set_startup_item, set_sys_opt_tweak, set_touchpad_edge_slide, set_touchpad_gesture_screenshot,
    set_touchpad_haptics, set_touchpad_haptics_intensity, set_touchpad_repress,
    set_touchpad_sensitivity, set_voice_focus, trigger_auto_update, trigger_driver_scan,
    unload_icc_profile, update_defender_signatures,
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
    // S55 FIX 13 (root cause found): window operations (show/set_focus/
    // set_always_on_top) MUST run on the process main thread on Windows.
    // The context-menu path (`open_window_sync` via on_menu_event) already
    // runs there, which is why "Open MiControl" from the right-click menu
    // worked. This command, however, is invoked from the tray popup's
    // WebView and lands on a tokio worker thread — calling win.show()/
    // set_focus() from there silently no-ops (Win32 UI calls from the wrong
    // thread), so the hidden/minimized main window never appeared.
    // Fix: marshal the whole raise sequence onto the main thread.
    //
    // S55 FIX 15: do NOT hide the popup here. Hiding the popup from the
    // backend can race the in-flight invoke (the popup's WebView gets torn
    // down while the call is still pending, cancelling it — observed as
    // "Open App does nothing"). The popup hides itself via its focus-loss
    // handler as soon as the main window takes focus.
    let app_for_main = app.clone();
    app.run_on_main_thread(move || raise_main_window(&app_for_main))
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Build a webview window with the WebView2 UDF-lock retry.
///
/// `tauri_runtime_wry` fails with `HRESULT(0x800700AA)` (`ERROR_BUSY`) while a
/// previous `msedgewebview2.exe` still holds the WebView2 user-data folder —
/// the exact failure logged twice in production (`22:19:16` / `22:19:40`,
/// 63 s and 87 s after a crash relaunch). `build()` used to be called as
/// `let _ = …build()`, so the error was dropped and the app carried on with no
/// webview at all (the pre-created tray popup then read 0×0). Retrying is safe
/// because the lock is transient; any other error is returned immediately.
fn build_webview_with_retry<T>(
    label: &str,
    build: impl FnMut() -> Result<T, tauri::Error>,
) -> Result<T, tauri::Error> {
    crate::util::webview_retry::retry_webview_create(
        label,
        &crate::util::webview_retry::WEBVIEW_RETRY_DELAYS_MS,
        build,
        std::thread::sleep,
    )
}

/// Build (or rebuild) the main window.
///
/// Every path that has to re-create it goes through here: the tray "open" item,
/// the single-instance relaunch, and the `WindowEvent::Destroyed` self-heal.
fn build_main_window(app: &tauri::AppHandle) -> Result<tauri::WebviewWindow, tauri::Error> {
    build_webview_with_retry("main", || {
        tauri::WebviewWindowBuilder::new(
            app,
            "main",
            tauri::WebviewUrl::App("index.html?window=main".into()),
        )
        .title("MiControl")
        .inner_size(950.0, 660.0)
        .resizable(true)
        .decorations(true)
        .build()
    })
}

/// Build the tray quick-access popup (hidden).
///
/// Shared by the startup pre-creation, the on-demand path and the 0×0
/// recreation path so the three cannot drift apart (they were duplicated
/// before, which is how the pre-created popup kept a size the other paths did
/// not know about).
fn build_tray_popup(app: &tauri::AppHandle) -> Result<tauri::WebviewWindow, tauri::Error> {
    build_webview_with_retry("tray", || {
        tauri::WebviewWindowBuilder::new(
            app,
            "tray",
            tauri::WebviewUrl::App("index.html?window=tray".into()),
        )
        .title("")
        .inner_size(POPUP_W, POPUP_H_DEFAULT)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build()
    })
}

/// Show, un-minimize and focus the main window. Handles the three failure
/// modes reported by users: window minimized (show() alone does not restore
/// it), window behind other windows (set_focus alone may not raise it) and
/// WebView2 focus quirks (brief always-on-top forces the z-order).
fn raise_main_window(app: &tauri::AppHandle) {
    match app.get_webview_window("main") {
        Some(win) => {
            // S55 FIX 16: call ShowWindow(SW_RESTORE) via Win32 FIRST. Tauri's
            // `show()` consults its own visibility state and can no-op when
            // that state is out of sync with reality (webview crash, external
            // hide), leaving the window invisible while the app believes it is
            // showing it. The raw Win32 call is unconditional and idempotent:
            // it restores from minimized AND unhides in one step.
            #[cfg(windows)]
            unsafe {
                use windows::Win32::Foundation::HWND;
                use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_RESTORE};
                if let Ok(hwnd) = win.hwnd() {
                    let _ = ShowWindow(HWND(hwnd.0), SW_RESTORE);
                }
            }
            let _ = win.show();
            let _ = win.set_focus();
            // Force-raise: always-on-top for an instant beats z-order races.
            let _ = win.set_always_on_top(true);
            let _ = win.set_focus();
            let app2 = app.clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(180));
                if let Some(w) = app2.get_webview_window("main") {
                    let _ = w.set_always_on_top(false);
                }
            });
        }
        None => {
            log::warn!("[window] main window missing — rebuilding (UDF-lock retry)");
            if let Err(e) = build_main_window(app) {
                log::error!("[window] failed to rebuild main window: {e}");
            }
        }
    }
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

// ── MCP integration toggle (Settings → "MCP Integration") ───────────────────

/// Get whether the MCP integration socket server is enabled (persisted).
#[tauri::command]
fn mcp_get_enabled() -> bool {
    crate::util::mcp_config::is_enabled()
}

/// Persist the MCP integration toggle. Applying the change requires the app
/// to restart (the MCP socket server is started/stopped at plugin setup).
#[tauri::command]
fn mcp_set_enabled(enabled: bool) -> Result<(), String> {
    crate::util::mcp_config::set_enabled(enabled);
    if enabled {
        log::info!("[mcp] MCP integration enabled — restart app to open the socket server (TCP localhost:4000)");
    } else {
        log::info!("[mcp] MCP integration disabled — socket server will stop on next restart");
    }
    Ok(())
}

#[tauri::command]
fn get_health_status() -> health_supervisor::HealthSnapshot {
    health_supervisor::snapshot()
}

/// S67: live kernel-handle / commit-memory leak diagnostics.
///
/// `leaking: true` means the growth rate exceeded the thresholds documented in
/// `util::leak_monitor` (3 handles/min or 2 MB commit/min over a 15 min
/// window). The same signal is logged as a throttled WARN by the sampler.
#[tauri::command]
fn get_leak_diagnostics() -> serde_json::Value {
    let report = crate::util::leak_monitor::latest();
    serde_json::json!({
        "handles": report.handles,
        "commit_bytes": report.commit_bytes,
        "handles_per_min": report.handles_per_min,
        "commit_bytes_per_min": report.commit_bytes_per_min,
        "samples": report.samples,
        "leaking": report.leaking,
        "alerts_this_session": crate::util::leak_monitor::alert_count(),
        "thresholds": {
            "handles_per_min": crate::util::leak_monitor::LEAK_HANDLES_PER_MIN,
            "commit_bytes_per_min": crate::util::leak_monitor::LEAK_COMMIT_BYTES_PER_MIN,
        },
        "sample_interval_secs": crate::util::leak_monitor::SAMPLE_INTERVAL.as_secs(),
        "window_secs": crate::util::leak_monitor::WINDOW.as_secs(),
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[allow(deprecated)]
pub fn run() {
    // ── Crash-report pipeline (agnostic; disabled by default) ─────────────
    // Must run BEFORE install_panic_hook so the hook observes the configured
    // (default: disabled) state. Reads `SOFTWARE\MiControl\CrashReporting`.
    util::crash_report::init_crash_reporting();
    util::panic::install_panic_hook();
    if let Err(e) = crate::debug_log::init_logging() {
        eprintln!("failed to initialize logging: {e:#}");
    }
    if let Some(path) = crate::debug_log::dev_log_path() {
        log::info!(
            target: "devlog",
            "persistent log file: {} (file-backed: {})",
            path.display(),
            crate::debug_log::is_file_logged()
        );
    }
    log::debug!(
        "log file location API: {}",
        crate::debug_log::log_file_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(unavailable)".into())
    );

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
                    before_send: Some(std::sync::Arc::new(|mut event| {
                        // ── PII stripping (GDPR / privacy) ───────────────────────
                        // Redact in exception stacktrace frames
                        for exception in event.exception.values.iter_mut() {
                            if let Some(ref mut stacktrace) = exception.stacktrace {
                                for frame in stacktrace.frames.iter_mut() {
                                    if let Some(ref mut filename) = frame.filename {
                                        *filename = redact_pii(filename);
                                    }
                                    if let Some(ref mut abs_path) = frame.abs_path {
                                        *abs_path = redact_pii(abs_path);
                                    }
                                }
                            }
                        }

                        // Strip server_name (computer name)
                        event.server_name = None;

                        // Strip IP addresses from extra (IPv4 and IPv6)
                        for val in event.extra.values_mut() {
                            if let Some(s) = val.as_str() {
                                // IPv4 redaction
                                let parts: Vec<&str> = s.split('.').collect();
                                let is_ipv4 = parts.len() == 4
                                    && parts
                                        .iter()
                                        .all(|p| !p.is_empty() && p.parse::<u8>().is_ok());
                                if is_ipv4 {
                                    *val = serde_json::Value::String("[REDACTED_IP]".into());
                                } else {
                                    // IPv6 redaction
                                    let redacted = redact_ipv6(s);
                                    if redacted != s {
                                        *val = serde_json::Value::String(redacted);
                                    }
                                }
                            }
                        }

                        // Strip IP addresses from contexts
                        for _ctx in event.contexts.values_mut() {
                            // Contexts don't carry IP in this Sentry version;
                            // IP is in event.request.env["REMOTE_ADDR"].
                        }

                        // Strip IP address from the request environment
                        if let Some(ref mut request) = event.request {
                            request.env.remove("REMOTE_ADDR");
                        }

                        Some(event)
                    })),
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

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // When a second instance is launched, focus the existing window
            // instead. If the main window is GONE (WebView2 crashed and the
            // window handle was destroyed — the zombie regression), rebuild it
            // on the spot so the user's launch actually shows the UI again
            // (previously nothing happened because the window no longer
            // existed and nobody recreated it).
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            } else {
                log::warn!(
                    "[single-instance] main window missing (destroyed by WebView2 crash?) — \
                     rebuilding for focus"
                );
                open_window_sync(app);
            }
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_shell::init());

    // MCP Bridge plugin — enables AI assistants to inspect and interact
    // with the Tauri app (screenshots, DOM, IPC calls, console logs).
    // S55 FIX 20: enabled in ALL builds behind the user-facing Settings →
    // "MCP Integration" toggle (persisted flag, OFF by default). It was
    // previously `#[cfg(debug_assertions)]`-only, which made the installed
    // release build impossible to debug remotely — exactly when we need it.
    // The toggle gate (`start_socket_server`/bridge init) is the security
    // boundary, not the build profile.
    //
    // VAGA F4 (audit 2026-11-07 §7.1): the bare `init()` binds 0.0.0.0, i.e.
    // the WebSocket gives *any host on the LAN* screenshots/DOM/input control
    // — and 0.0.0.0 is exactly what makes Windows raise the "Allow MiControl
    // to communicate on these networks" firewall dialog. The bridge now binds
    // 127.0.0.1 (util::mcp_config::bridge_config), which is enough for every
    // local MCP client and never triggers the prompt. The separate
    // tauri-plugin-mcp server below already listens on loopback:4000.
    #[cfg(debug_assertions)]
    let builder = builder.plugin(tauri_plugin_mcp_bridge::init_with_config(
        crate::util::mcp_config::bridge_config(),
    ));
    #[cfg(not(debug_assertions))]
    let builder = if crate::util::mcp_config::is_enabled() {
        builder.plugin(tauri_plugin_mcp_bridge::init_with_config(
            crate::util::mcp_config::bridge_config(),
        ))
    } else {
        builder
    };

    // P3GLEG tauri-plugin-mcp — MCP server (screenshots, DOM access,
    // input simulation, IPC inspection, log querying). TCP localhost:4000.
    //
    // Availability (same capabilities in dev and in the installed app):
    //   - start_socket_server = MCP integration toggle (registry flag,
    //     persisted in Settings → "MCP Integration"). The socket server is a
    //     local backdoor that grants any same-user process arbitrary JS
    //     execution — it MUST stay OFF unless the user explicitly enables it.
    //   - allow_release_builds(true) — the plugin otherwise refuses to start
    //     the socket server in release builds; the user-facing toggle needs it.
    //   - The frontend guest bindings (`setupPluginListeners`) run in all
    //     builds so query_page/click/read_text work identically in the
    //     installed app when the toggle is ON.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_mcp::init_with_config(
        tauri_plugin_mcp::PluginConfig::new("MiControl".to_string())
            .tcp_localhost(4000)
            .allow_release_builds(true)
            .start_socket_server(crate::util::mcp_config::is_enabled()),
    ));

    builder
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
            // Firmware truth for the limit (requested vs applied)
            get_charge_limit_state,
            // Live hold phase ("holding at 80", "above limit", "charging")
            get_charge_hold_state,
            // Battery Care toggle (firmware field, LONL/HBDA)
            get_battery_care,
            set_battery_care,
            get_perf_debug,
            // P2-6: scenario rules (auto perf-mode on AC/DC) — HKCU config + status
            get_scenario_rules,
            set_scenario_rules,
            get_ecram_map,
            get_iot_region_hex,
            write_iot_hex,
            read_ecram_raw,
            is_elevated,
            relaunch_as_admin,
            // IoTService IPC
            iot_pipe_available,
            ensure_iot_service,
            ensure_bridge_service,
            get_health_status,
            get_leak_diagnostics,
            get_iot_device_info,
            get_iot_wifi_list,
            iot_notify_event,
            // IoT device control — no consolidated replacement exists for these,
            // so they stay registered even though the UI does not call them yet.
            iot_connect_wifi,
            iot_write_wifi_item,
            iot_delete_wifi_item,
            iot_empty_wifi_items,
            iot_set_device_status,
            iot_reset_device,
            // Audio
            get_audio_devices,
            get_audio_volume,
            set_audio_volume,
            set_audio_mute,
            set_audio_default_endpoint,
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
            // Overview — identity + specs (NPU, memory modules, storage, panel)
            get_overview_snapshot,
            // Battery
            get_battery_info,
            // Display
            get_display_info,
            set_brightness,
            set_hdr,
            set_ai_brightness,
            get_ai_brightness_config,
            set_ai_brightness_config,
            // Smart brightness (learned user-preference curve)
            get_smart_brightness_model,
            reset_smart_brightness_model,
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
            // Windows startup manager (runs, folders, logon tasks)
            get_startup_items,
            set_startup_item,
            // Update Nucleus (Phase 9)
            get_update_status,
            trigger_driver_scan,
            // Hardware Discovery (Phase 10)
            get_hardware_profile,
            run_hardware_discovery,
            install_driver,
            // S45-001: Silent self-update via the MiControlBridge service
            install_update,
            find_latest_downloaded_installer,
            // S45-002: Auto-update config (off by default) + beta feed (dev)
            get_auto_update_config,
            set_auto_update_enabled,
            set_auto_update_beta_feed,
            trigger_auto_update,
            // Hotkeys (keyboard remapping)
            get_hotkey_config,
            set_hotkey_config,
            start_key_detect,
            get_detected_key,
            is_hook_active,
            get_remap_apply_state,
            // S29-001: Script hotkey consent grant command
            grant_script_consent,
            // Display refresh rate
            get_available_refresh_rates,
            set_refresh_rate,
            set_adaptive_refresh_rate,
            // Process list
            get_process_list,
            // Global task manager (per-process CPU/GPU/NPU/RAM + kill)
            get_task_manager,
            kill_process,
            get_network_perf,
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
            // MCP integration toggle (Settings → "MCP Integration")
            mcp_get_enabled,
            mcp_set_enabled,
            // Data export — GDPR Art.20 (S19-16)
            export_user_data,
            reveal_in_explorer,
            open_external_url,
            // Cross-device (MIOT-04+) — BLE presence
            commands::crossdevice::get_presence_status,
            commands::crossdevice::get_presence_config,
            commands::crossdevice::set_presence_config,
            commands::crossdevice::scan_presence_now,
            commands::crossdevice::ble_discover,
            // Cross-device — BLE advertising (PC discoverability)
            commands::crossdevice::ble_advertise_config,
            commands::crossdevice::set_ble_advertise_config,
            commands::crossdevice::ble_advertise_status,
            // Cross-device (MIOT-06) — LocalSend LAN file transfer
            commands::crossdevice::localsend_discover,
            commands::crossdevice::localsend_send_files,
            commands::crossdevice::localsend_receiver_start,
            commands::crossdevice::localsend_receiver_stop,
            commands::crossdevice::localsend_receiver_status,
            // Cross-device (MIOT-06) — LocalSend consent + receiver settings
            commands::crossdevice::localsend_respond_consent,
            commands::crossdevice::localsend_config,
            commands::crossdevice::localsend_set_auto_accept,
            commands::crossdevice::localsend_set_max_upload_mb,
            // Cross-device (MIOT-07) — scrcpy phone-camera webcam
            commands::crossdevice::scrcpy_status,
            commands::crossdevice::scrcpy_start,
            commands::crossdevice::scrcpy_stop,
            commands::crossdevice::scrcpy_install,
            // Cross-device (MIOT-08) — on-device transcription via sherpa-onnx
            commands::crossdevice::transcription_status,
            commands::crossdevice::transcription_download_model,
            commands::crossdevice::transcription_install_binary,
            commands::crossdevice::transcribe_audio,
            // Cross-device (MIOT-10) — KDE Connect discovery + ping MVP
            commands::crossdevice::kde_discover,
            commands::crossdevice::kde_ping,
            // Cross-device (MIOT-11) — NFC pairing guidance + custom tag payload
            commands::crossdevice::nfc_guidance,
            commands::crossdevice::nfc_build_custom,
            commands::crossdevice::nfc_open_link,
            // Cross-device (MIOT-12) — Syncthing REST status + folder toggle
            commands::crossdevice::syncthing_status,
            commands::crossdevice::syncthing_set_folder,
            // WMAA / WMI MiInterface (elevated bridge)
            wmi_ec_read,
            wmi_ec_write,
            wmi_ec_get_performance_mode,
            wmi_ec_set_performance_mode,
            wmi_ec_read_battery_health,
            wmi_ec_read_adapter_power,
            wmi_ec_read_sensor_data,
            wmi_ec_set_sagv_mode,
            wmi_ec_set_pl1_flag,
            wmi_ec_set_epof_flag,
            wmi_ec_set_mi_usage_type,
            wmi_ec_set_wmid_type,
            wmi_ec_set_lid_open_type,
            wmi_ec_set_removable_type,
            wmi_ec_set_auto_illumination,
            wmi_ec_set_label_mode,
            // HQWmiCommonInterface (BIOS control)
            hq_set_performance_mode,
            hq_change_boot_option,
            hq_load_default,
            hq_s5_rtc_wake_enable,
            hq_enable_pxe_boot,
            hq_set_wifi_country_code,
            hq_set_shipping_country_code,
            // Thermal zone (ACPI temperature)
            get_thermal_zones,
            get_primary_thermal_zone,
            // Fn-Key Customization (EC 0x4A)
            get_function_key,
            set_function_key,
            // Eye Protection (blue light filter)
            get_eye_protection,
            set_eye_protection,
            // OS Turbo (system optimization)
            get_os_turbo,
            set_os_turbo,
            get_sys_opt_status,
            set_sys_opt_tweak,
            // Crash Recovery
            get_crash_recovery_status,
            mark_clean_exit,
            // Driver Details
            get_drivers_detail,
            // AI Noise Cancellation
            get_audio_effects,
            set_mic_noise_canceling,
            set_speaker_noise_canceling,
            set_voice_focus,
            // System Cleanup
            scan_junk_files,
            clean_junk_files,
            // Security Scan
            quick_security_scan,
            full_security_scan,
            custom_security_scan,
            update_defender_signatures,
            get_defender_status,
            get_threat_history,
            open_windows_security,
            // Phone Link
            get_phone_link_status,
            launch_phone_link,
            launch_phone_link_feature,
            open_phone_link_settings,
            // Color Calibration
            get_color_status,
            download_color_profile,
            apply_color_profile,
            open_color_profile_search,
            get_error_log_config,
            set_error_logging_enabled,
            read_error_log,
            clear_error_log,
            log_frontend_error,
            load_icc_profile,
            unload_icc_profile,
            open_color_management_settings,
            launch_color_calibration_wizard,
            // Face Unlock (Windows Hello-style, RGB webcam)
            face_status,
            face_service_install,
            face_service_ensure,
            face_list_templates,
            face_delete_template,
            face_get_settings,
            face_set_settings,
            face_set_password,
            face_password_configured,
            face_list_users,
            face_hello_verify,
            face_diagnostics,
            face_enroll,
            face_download_models,
            face_install_models,
            face_models_status,
            face_models_remove_all,
            face_camera_preview_start,
            face_camera_preview_stop,
            face_camera_preview_frame,
        ])
        .setup(|app| {
            // S26-006: `--minimized` start (autostart / watchdog relaunch):
            // begin tray-only — hide the main window right away.
            if std::env::args().any(|a| a == "--minimized") {
                if let Some(main_win) = app.get_webview_window("main") {
                    let _ = main_win.hide();
                    log::info!("Started with --minimized — started to tray");
                }
            }

            // Hardware discovery — load cached profile or scan on first run
            let data_dir = app
                .path()
                .app_data_dir()
                .ok();
            crate::hw::discovery::init(data_dir);

            // S24-016: Load persisted AI usage stats on startup.
            crate::util::ai_usage::load_on_startup();

            // S26-005: Initialize crash recovery (Restart Manager + WER LocalDumps).
            if let Err(e) = crate::hw::crash_recovery::init_crash_recovery() {
                log::warn!("Crash recovery init failed (non-fatal): {e}");
            }

            // S67: kernel-handle leak monitor. The audited leak grew ~11
            // handles/min unnoticed for 218 h until the process held 188,715
            // handles / 2.5 GB commit. This sampler turns any future
            // regression into a WARN within minutes.
            crate::util::leak_monitor::start();

            // S26-005b: Re-arm the interactive watchdog after an intentional
            // quit. The app is starting now (any reason), so clear the
            // user-quit sentinel: from this moment the bridge watchdog is
            // armed again for this new session. We only observe whether the
            // sentinel was present for diagnostics.
            if crate::hw::crash_recovery::user_quit_pending() {
                log::info!("App restarted after a user quit — watchdog re-armed for this session");
            }
            crate::hw::crash_recovery::clear_user_quit_marker();

            // Mirror the autostart preference to the SYSTEM bridge watchdog so
            // it only auto-relaunches the app when the user wants it on boot.
            let autostart_enabled = crate::hw::startup::get_autostart()
                .unwrap_or(false);
            crate::hw::crash_recovery::set_watchdog_enabled(autostart_enabled);

            // Initialize error logging system (7-day retention, on by default)
            crate::util::error_log::init();

            // S26-004: Auto-rotate HMAC key if needed (replaces misleading --rotate-key message).
            if crate::util::auth::key_needs_rotation() {
                log::info!("HMAC key is older than 30 days — auto-rotating...");
                if let Err(e) = crate::util::auth::rotate_key() {
                    log::warn!("HMAC key auto-rotation failed: {e}");
                }
            }

            // Sync the discovered profile into Tauri managed state
            if let Some(profile) = crate::hw::discovery::global_profile() {
                app.state::<AppState>().set_profile(profile);
            }

            // Start keyboard hook (intercepts Xiaomi AI / PCManager / Copilot keys)
            // S24-004: Handle error gracefully instead of panicking.
            if let Err(e) = crate::hw::hotkeys::start_hook() {
                log::warn!("Hotkey hook failed to start, continuing without hotkeys: {e}");
            }

            // S32-002: Ensure the autonomous MiControlBridge service is
            // installed and running (installed at install time; self-heal here
            // for dev builds / upgrades). This runs BEFORE any elevated
            // command so the app never falls back to repeated UAC prompts.
            //
            // Post-reboot self-heal for the MiControlFace auth service runs
            // AFTER the bridge: it crashes ~60 min after boot (0xc0000005 in
            // FrameServerClient.dll_unloaded — MSMF camera in a Session-0
            // SYSTEM service) and — lacking SCM failure actions — stays
            // STOPPED-1067 forever, breaking Face Unlock after every reboot.
            // Serializing the two ensures the autonomous bridge (preferred
            // channel, no UAC) is up to carry the face command instead of the
            // UI process racing ahead via UAC. Both remain best-effort: the
            // health supervisor re-probes and heals them in the background.
            tauri::async_runtime::spawn(async {
                match crate::elev_bridge::ensure_bridge_service().await {
                    Ok(status) => {
                        log::info!("[bridge] ensure_bridge_service: {status}");
                    }
                    Err(e) => {
                        log::warn!(
                            "[bridge] ensure_bridge_service failed (will use scheduled task): {e}"
                        );
                    }
                }
                match crate::elev_bridge::ensure_face_service().await {
                    Ok(status) => {
                        log::info!("[face] ensure_face_service: {status}");
                    }
                    Err(e) => {
                        log::warn!(
                            "[face] ensure_face_service failed (will try again from UI): {e}"
                        );
                    }
                }
            });

            // Keep recoverable services and hardware probes healthy after boot.
            // The supervisor uses bounded recovery cooldowns and never force-kills
            // processes from the unprivileged UI process.
            crate::health_supervisor::start();

            // P8 (audit 2026-11-07): one-time summary of every elevated command
            // that failed during the first two minutes — command, elapsed and
            // reason. Background commands never prompt for UAC (MIOT-38), so
            // without this summary their failures were silent.
            crate::elev_bridge::spawn_boot_summary();

            // Apply Copilot key interception fixes (disables Windows Shell
            // interception + writes Scancode Map for permanent remap).
            // This is async because it dispatches through the elevated bridge.
            tauri::async_runtime::spawn(async {
                crate::hw::hotkeys::apply_copilot_fix().await;
            });

            // Register focus callback: Xiaomi key / AI key / Copilot key fires this.
            // We toggle the tray quick-access popup, exactly like XiaomiPCManager did.
            //
            // The whole flow is dispatched to the main thread (the callback runs on
            // the WMI reader thread): `show_tray_popup` may have to *create* the
            // window and then read its size, and both are only synchronous on the
            // main thread. Storing TRAY_SHOWN_AT_MS inside the closure keeps it
            // ordered with the focus-loss handler that also runs there.
            {
                let app_handle = app.handle().clone();
                crate::hw::hotkeys::set_focus_callback(Box::new(move || {
                    let app = app_handle.clone();
                    if let Err(e) = app_handle.run_on_main_thread(move || {
                        match app.get_webview_window("tray") {
                            Some(popup) if popup.is_visible().unwrap_or(false) => {
                                log::info!("[tray] focus_callback: hiding popup");
                                let _ = popup.hide();
                            }
                            _ => {
                                log::info!("[tray] focus_callback: showing popup");
                                show_tray_popup(&app, None);
                            }
                        }
                    }) {
                        log::error!("[tray] focus_callback: dispatch to main thread failed: {e}");
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

            // System Optimization tweaks are NOT re-applied at boot.
            // The automatic sweep was disabled in S55 FIX 17: it triggered a
            // SYSTEM-service crash-loop (boot → sweep → bridge crash 0xc0000005
            // → SCM restart → health check redispatch → crash again). The
            // unreachable elevated branch behind it (`restore_sys_opt_all` +
            // `sys_opt::restore_all`) was removed in the 2026-11 audit cleanup
            // — no command or frontend path ever dispatched it. Every tweak is
            // persisted in the registry and the user can re-apply any of them
            // individually from the Debloater UI. Reinstate a sweep only
            // alongside a crash-proof, observable implementation.

            // Give the gesture thread access to the app handle so it can show the OSD.
            crate::hw::touchpad::set_app_handle(app.handle().clone());

            // Start the native Win32 brightness OSD (GDI layered window, no WebView2).
            #[cfg(windows)]
            crate::hw::osd::init();

            // Start power event listener for sleep/resume detection.
            // Resets sensors (ambient light, WMI cache) after the system
            // wakes from sleep to prevent "Sensor unavailable" errors.
            #[cfg(windows)]
            crate::hw::power_listener::start_power_listener();

            // P1-2 (recheck 2026-11-11): the EC forgets the charging limit
            // across S3 and across power-source changes. The supervisor
            // re-asserts the user's requested limit at startup, on resume, on
            // AC plug and every 5 min thereafter (gated to at most one write
            // per window, backoff on failure), and logs the real hold phase
            // every 45 s. The 20:17 limit survived until the 02:23 → 10:55
            // resume and then silently reverted to 100 %.
            #[cfg(windows)]
            crate::hw::charging::start_charge_hold_supervisor();

            // MIOT-01: IoT event listener — synthesizes power/charging/low-
            // battery events into native OSD notifications. Reads config from
            // HKCU\SOFTWARE\MiControl\IotEvents; fully isolated from other HW.
            // MIOT-02: the hook now applies user scenario rules (auto perf-mode
            // on AC/battery). P2-6 (audit 2026-11-07): the rules ride the
            // elevated dispatcher — the previous in-process call died on the
            // HKLM ACL — and the HKCU config key is created up front so the
            // feature has a durable configuration instead of "no rules".
            #[cfg(windows)]
            {
                crate::hw::iot_events::set_event_hook(crate::hw::scenario_rules::apply_event);
                if let Err(e) = crate::hw::iot_events::IotEventsConfig::ensure_key() {
                    log::warn!("[iot_events] could not create config key: {e}");
                }
                crate::hw::iot_events::start_iot_event_listener();
            }

            // MIOT-03: IoT pipe watchdog — if the ecram/MCPI pipe disappears
            // (crash, update, service stop) the recovery path reinstalls and
            // restarts the ecram service. Runs on a 30 s cycle with cooldown.
            #[cfg(windows)]
            crate::hw::iot_watchdog::start_iot_watchdog();

            // MIOT-04: BLE phone presence — starts only if the user enabled it
            // in the registry; the monitor is a no-op otherwise.
            #[cfg(windows)]
            if crate::hw::ble_presence::get_presence_status().enabled {
                crate::hw::ble_presence::start_presence_monitor();
            }

            // MIOT-38: BLE advertising. The publisher lives on its own COM
            // thread and does not survive a restart, so a persisted
            // `BleAdvertise\Enabled=1` was silently ignored until the user
            // toggled it again — while the UI claimed "advertising active".
            // Re-arm it here and let the UI read `ble_advertise_status` for
            // the real state.
            #[cfg(windows)]
            if crate::hw::ble_advertise::get_config().enabled {
                crate::hw::ble_advertise::start_advertising();
            }

            // Start adaptive brightness background task
            tauri::async_runtime::spawn(crate::hw::display::adaptive_brightness_loop());

            // F5: reconcile the persisted flag with the machine before the
            // worker starts. A stale snapshot (every recorded process exited)
            // is recaptured and re-applied through the bridge; a flag that is
            // off while recovery state is still on disk finishes its disable.
            // The old `restore_os_turbo()` was an empty body with no caller,
            // which is how the feature stayed dead for 12 days.
            crate::hw::os_turbo::restore_os_turbo();

            // S39-003: bounded periodic OS Turbo measurement worker. Idempotent;
            // the worker self-gates on the persisted enabled flag, so starting it
            // while OS Turbo is disabled is a no-op. It only reads back process
            // state and, on drift, dispatches a bridge-only reapply — never EC/
            // fan/power-limit writes, never UAC or scheduled-task fallback.
            crate::hw::os_turbo::start_worker();

            // S32-005: Heartbeat — the SYSTEM bridge watchdog uses this to
            // distinguish a healthy UI from a frozen zombie. Write the first
            // one NOW (setup has finished, tray+main window exist) and keep
            // refreshing it on a ticker. The app handle lets the heartbeat
            // probe real UI liveness (main window alive) — not just the
            // process — so a WebView2 crash without a handler cannot keep a
            // fresh heartbeat while the UI is dead.
            crate::hw::crash_recovery::set_app_handle(app.handle().clone());
            crate::hw::crash_recovery::write_heartbeat();
            crate::hw::crash_recovery::start_heartbeat_ticker();

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
                        // S26-005: Mark clean exit AND the user-quit sentinel
                        // before quitting from tray, so the bridge watchdog
                        // knows this exit was intentional and does NOT relaunch.
                        crate::hw::crash_recovery::mark_clean_exit();
                        crate::hw::crash_recovery::mark_user_quit();
                        // S39-003: cancel the bounded OS Turbo worker (the
                        // loop exits at its next tick; a running sweep finishes
                        // its bounded work first).
                        crate::hw::os_turbo::stop_worker();
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
            // `build_tray_popup` retries the transient WebView2 UDF lock
            // (0x800700AA) — otherwise the popup exists but never initialises and
            // reports 0×0 on the first click.
            match build_tray_popup(app.handle()) {
                Ok(_) => log::info!("[tray] pre-created tray popup OK"),
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
                            // S26-005: Mark clean exit + user-quit before shutting down.
                            crate::hw::crash_recovery::mark_clean_exit();
                            crate::hw::crash_recovery::mark_user_quit();
                            // S39-003: cancel the bounded OS Turbo worker.
                            crate::hw::os_turbo::stop_worker();
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
                    // P6: log a real duration, not the epoch. `0` in the slot
                    // means "never shown this session" → `age_since_shown=unset`
                    // (the old code printed `now - 0` = 56.8 years).
                    let now = now_ms();
                    let shown_at = TRAY_SHOWN_AT_MS.load(Ordering::Relaxed);
                    log::info!(
                        "[tray] Focused(false): {}",
                        crate::util::log_fields::duration_field("age_since_shown", now, shown_at)
                    );
                    // A popup that was never shown has no show-age to protect.
                    if crate::util::log_fields::elapsed_since(now, shown_at).unwrap_or(u64::MAX) < 500
                    {
                        return; // too soon after show — ignore this focus-loss
                    }
                    TRAY_HIDDEN_AT_MS.store(now_ms(), Ordering::Relaxed);
                    window.hide().ok();
                }
                tauri::WindowEvent::Destroyed if window.label() == "main" => {
                    // S38: The main window (and its WebView2) was destroyed
                    // without the process exiting — e.g. a webview/tray crash
                    // that killed the browser process but left the Tauri
                    // process alive. Previously this produced a zombie: the
                    // app kept running (tray + heartbeat) but clicking "Open
                    // MiControl" did nothing because the window was gone.
                    //
                    // Self-heal by rebuilding the main window. If the rebuild
                    // itself fails (e.g. system-wide WebView2 outage), the
                    // watchdog will still detect `ui_ok=0` in the heartbeat
                    // and force-restart the process.
                    log::error!(
                        "[window] main window destroyed — rebuilding (self-heal after webview crash)"
                    );
                    let app = window.app_handle().clone();
                    let app2 = app.clone();
                    let _ = app.run_on_main_thread(move || {
                        open_window_sync(&app2);
                    });
                }
                _ => {}
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building MiControl")
        .run(|_app_handle, event| {
            // P4: every clean shutdown funnels through `RunEvent::Exit` —
            // tray Quit, `app.exit(0)` from any command, and the dev-mode
            // window close. Log the lifecycle marker and persist it so the
            // next start can tell "clean" from "crash/kill".
            if let tauri::RunEvent::Exit = event {
                crate::util::session_marker::mark_clean_shutdown();
            }
        });
}

/// Resize the tray popup window, keeping the bottom edge fixed (grows upward).
/// `height` is in logical (CSS) pixels, as reported by ResizeObserver on the frontend.
#[tauri::command]
async fn resize_tray_popup(app: tauri::AppHandle, height: f64) -> Result<(), String> {
    const MIN_H: f64 = 200.0;
    const MAX_H: f64 = 780.0;
    let height = height.clamp(MIN_H, MAX_H);
    if let Some(window) = app.get_webview_window("tray") {
        // S64: resolve the scale from the window's monitor, not from
        // `window.scale_factor()` — the latter is stale for a hidden window.
        let scale = popup_scale(&window, None);
        let pos = window.outer_position().map_err(|e| e.to_string())?;
        let cur = window.inner_size().map_err(|e| e.to_string())?;

        // S28-0xx (tray clipping): cap the popup height to the work area of the
        // monitor that currently contains the window, minus the taskbar gap.
        // A fixed MAX_H of 780 px overflows shorter work areas (e.g. 720p
        // screens with a taskbar), clipping the bottom of the popup.
        let max_h_logical = {
            #[cfg(windows)]
            {
                use windows::Win32::Foundation::POINT;
                use windows::Win32::Graphics::Gdi::{
                    GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
                };
                unsafe {
                    // SAFETY: read-only Win32 monitor query on a stack-local,
                    // zeroed MONITORINFO with cbSize set (POD).
                    let pt = POINT {
                        x: pos.x + cur.width as i32 / 2,
                        y: pos.y + cur.height as i32 / 2,
                    };
                    let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
                    let mut info = MONITORINFO {
                        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
                        ..std::mem::zeroed()
                    };
                    if GetMonitorInfoW(hmon, &mut info).as_bool() {
                        let work_h = info.rcWork.bottom - info.rcWork.top;
                        ((work_h as f64 / scale) - POPUP_GAP).max(MIN_H).floor()
                    } else {
                        MAX_H
                    }
                }
            }
            #[cfg(not(windows))]
            {
                MAX_H
            }
        };
        let height = height.min(max_h_logical);

        // Anchor: physical y of the bottom edge
        let bottom_phys = pos.y + cur.height as i32;
        let new_h_phys = (height * scale).round() as u32;
        let new_y = (bottom_phys - new_h_phys as i32).max(0);
        // S63: never trust the current width. A hidden/pre-created popup window can
        // report (or get clamped to) a wrong inner_size — observed live as a width
        // collapse from 300 to 171 px, which this resizer then preserved forever,
        // clipping the right side of the popup. The CSS content width is fixed at
        // POPUP_W, so enforce it on every resize. This also self-heals a corrupted
        // window on the first ResizeObserver tick after show.
        let new_w_phys = (POPUP_W * scale).round() as u32;
        // Apply — size first, then position so there's no flicker
        window
            .set_size(tauri::PhysicalSize::new(new_w_phys, new_h_phys))
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

/// S64: authoritative DPI scale for the tray popup.
///
/// `window.scale_factor()` is unreliable for a HIDDEN or just-shown popup: it
/// caches the scale from the monitor the window was last associated with, so a
/// display-scaling change (or a move between monitors with different DPI) makes
/// it report a stale value — observed live as `scale=1.75` on one call and
/// `scale=1.00` on the very next call within the same show, with a physical
/// size (525x1365) left over from the 1.75 era. The resulting mismatch put the
/// popup at the top of the screen (`y≈19`) with a stale physical size.
///
/// Resolve the scale from the monitor that actually contains the tray click
/// (or the window, for the hotkey path) via `GetDpiForMonitor`, and fall back
/// to the window value only if the Win32 query fails.
#[cfg(windows)]
fn popup_scale(window: &tauri::WebviewWindow, probe: Option<tauri::PhysicalPosition<f64>>) -> f64 {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };
    use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};

    let pt = match probe {
        Some(p) => POINT {
            x: p.x as i32,
            y: p.y as i32,
        },
        None => {
            // Hotkey path: derive a point from the window's own rect.
            let (pos, size) = match (window.outer_position(), window.inner_size()) {
                (Ok(pos), Ok(size)) => (pos, size),
                _ => return window.scale_factor().unwrap_or(1.0),
            };
            POINT {
                x: pos.x + size.width as i32 / 2,
                y: pos.y + size.height as i32 / 2,
            }
        }
    };

    unsafe {
        // SAFETY: read-only Win32 monitor/DPI queries on POD stack values.
        let hmon = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
        // Validate the monitor handle by asking for its work area; a failed
        // lookup means we should trust the window instead.
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..std::mem::zeroed()
        };
        if !GetMonitorInfoW(hmon, &mut info).as_bool() {
            return window.scale_factor().unwrap_or(1.0);
        }
        let mut dpi_x: u32 = 96;
        let mut dpi_y: u32 = 96;
        if GetDpiForMonitor(hmon, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y).is_ok() && dpi_x > 0 {
            return dpi_x as f64 / 96.0;
        }
    }
    window.scale_factor().unwrap_or(1.0)
}

#[cfg(not(windows))]
fn popup_scale(window: &tauri::WebviewWindow, _probe: Option<tauri::PhysicalPosition<f64>>) -> f64 {
    window.scale_factor().unwrap_or(1.0)
}

fn open_window_sync(app: &tauri::AppHandle) {
    // S55 FIX 10: same robustness as raise_main_window (unminimize + forced
    // z-order raise). Reuses the helper, rebuilding the window when absent.
    if app.get_webview_window("main").is_some() {
        raise_main_window(app);
    } else if let Err(e) = build_main_window(app) {
        // UDF lock (0x800700AA) survives all retries, or WebView2 is genuinely
        // unavailable. Never panic: the watchdog's ui_ok=0 path picks this up.
        log::error!("[window] failed to create main window: {e}");
    }
}

/// True when the popup window handle exists but has never been given a client
/// area — the signature of a WebView2 that failed to initialise (observed in
/// production: `[tray] popup physical size looks stale (0x0 px …)` right after
/// `failed to create webview: HRESULT(0x800700AA)`). `set_size` cannot revive
/// such a window: only a fresh WebView2 host window can.
fn popup_is_uninitialised(window: &tauri::WebviewWindow) -> bool {
    match window.inner_size() {
        Ok(size) => size.width == 0 && size.height == 0,
        Err(_) => true,
    }
}

/// Return a usable tray popup, destroying and rebuilding a dead one.
///
/// The old heal path only forced 300×460 with `set_size` and carried on, so a
/// 0×0 popup stayed 0×0: it "opened" invisible while Tauri reported
/// `is_visible=true`, and the next tray click took the toggle-off branch. A
/// WebView2 that never initialised cannot be resized back to life — recreate it.
fn ensure_tray_popup(app: &tauri::AppHandle) -> Option<tauri::WebviewWindow> {
    if let Some(popup) = app.get_webview_window("tray") {
        if !popup_is_uninitialised(&popup) {
            return Some(popup);
        }
        let (w, h) = popup
            .inner_size()
            .map(|s| (s.width, s.height))
            .unwrap_or((0, 0));
        log::warn!(
            "[tray] popup webview never initialised ({w}x{h} px) — destroying and recreating"
        );
        if let Err(e) = popup.destroy() {
            log::warn!("[tray] popup.destroy() failed: {e} — reusing the existing window");
            return Some(popup);
        }
        // On the main thread the destroy is synchronous; from another thread it
        // is queued, so the label may still resolve. Never build a second
        // window with the same label.
        if let Some(stale) = app.get_webview_window("tray") {
            log::warn!("[tray] destroyed popup still registered (destroy queued) — reusing it");
            return Some(stale);
        }
    } else {
        log::warn!(
            "[tray] popup window not found — creating on-demand (pre-creation must have failed)"
        );
    }

    match build_tray_popup(app) {
        Ok(popup) => {
            if popup_is_uninitialised(&popup) {
                log::error!(
                    "[tray] recreated popup is still 0x0 px — WebView2 runtime unavailable? \
                     The tray click cannot show a popup this cycle"
                );
            } else {
                log::info!("[tray] tray popup (re)created OK");
            }
            Some(popup)
        }
        Err(e) => {
            log::error!("[tray] failed to create tray popup: {e}");
            None
        }
    }
}

/// Position, show and focus the tray popup, recreating it when necessary.
///
/// Called from the tray click (main thread) and, via `run_on_main_thread`,
/// from the Xiaomi/AI key callback.
fn show_tray_popup(app: &tauri::AppHandle, click_pos: Option<&tauri::PhysicalPosition<f64>>) {
    let Some(popup) = ensure_tray_popup(app) else {
        return;
    };
    let place = |w: &tauri::WebviewWindow| match click_pos {
        Some(pos) => position_popup(w, pos),
        None => position_popup_at_tray(w),
    };
    place(&popup);
    TRAY_SHOWN_AT_MS.store(now_ms(), Ordering::Relaxed);
    match popup.show() {
        Ok(_) => {
            // Re-position after show: a hidden window may have reported a wrong
            // scale_factor() / inner_size() before it was associated with a monitor.
            place(&popup);
            // Verify the healed size actually took effect (the 0×0 regression
            // was silent: show() succeeded on a window with no client area).
            let (w, h) = popup
                .inner_size()
                .map(|s| (s.width, s.height))
                .unwrap_or((0, 0));
            if w == 0 {
                log::error!(
                    "[tray] popup shown but still degenerate ({w}x{h} px) — the WebView2 \
                     host window has no client area"
                );
            } else if h == 0 {
                log::warn!(
                    "[tray] popup shown with no height yet ({w}x{h} px) — the frontend \
                     ResizeObserver sets it on first render"
                );
            }
            if let Ok(pos) = popup.outer_position() {
                log::info!(
                    "[tray] show() OK — outer_pos=({},{}) size={}x{} is_visible={}",
                    pos.x,
                    pos.y,
                    w,
                    h,
                    popup.is_visible().unwrap_or(false)
                );
            }
            let _ = popup.set_focus();
        }
        Err(e) => log::error!("[tray] show() FAILED: {e}"),
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
        // P6: real duration (or `unset` when the popup was never hidden).
        let now = now_ms();
        let hidden_at = TRAY_HIDDEN_AT_MS.load(Ordering::Relaxed);
        log::info!(
            "[tray] {}",
            crate::util::log_fields::duration_field("elapsed_since_hidden", now, hidden_at)
        );
        if crate::util::log_fields::elapsed_since(now, hidden_at).unwrap_or(u64::MAX) < 300 {
            log::info!("[tray] debounce active, aborting show");
            return;
        }
    }
    // Exists but hidden long enough ago (or missing) — position and show.
    // `show_tray_popup` recreates the window when the webview never initialised.
    show_tray_popup(app, Some(click_pos));
}

/// Heal a stale popup physical size and return the height to use for layout.
///
/// A previously shown popup can carry a width collapsed to 171 px, or a size
/// left over from a previous display scale (observed 525×1365 from a 1.75 era
/// while the current scale was 1.00). A stale physical size makes the Y math
/// place the popup at the top of the screen (`y≈19`), so the design width is
/// enforced on every show and the stored height is only trusted when it is
/// non-zero.
///
/// Note this only *heals* an existing window. A window whose WebView2 never
/// initialised reports 0×0 and ignores `set_size`; `ensure_tray_popup` destroys
/// and recreates that one before we get here.
fn heal_popup_size(window: &tauri::WebviewWindow, scale: f64) -> f64 {
    let (cur_w, cur_h) = window
        .inner_size()
        .map(|s| (s.width, s.height))
        .unwrap_or((0, 0));
    let want_w = (POPUP_W * scale).round() as u32;
    if cur_w != want_w {
        log::warn!(
            "[tray] popup physical size looks stale ({cur_w}x{cur_h} px, expected width \
             {want_w}) — healing"
        );
        let healed_h = if cur_h > 0 {
            cur_h
        } else {
            (POPUP_H_DEFAULT * scale).round() as u32
        };
        let _ = window.set_size(tauri::PhysicalSize::new(want_w, healed_h));
    }
    // A hidden window may report height=0 before its first render; a stale or
    // zero height must never drive the Y math.
    if cur_w != want_w || cur_h == 0 {
        POPUP_H_DEFAULT * scale
    } else {
        cur_h as f64
    }
}

/// Position the popup window flush above the taskbar, centred on the tray icon.
/// Uses GetMonitorInfo to find the work-area bottom so the result is always
/// just above the taskbar regardless of taskbar height, size, or DPI.
/// Uses the window's CURRENT height so that a previous dynamic resize is honoured.
fn position_popup(window: &tauri::WebviewWindow, click_pos: &tauri::PhysicalPosition<f64>) {
    // S64: resolve the scale from the monitor under the click, not from the
    // window (which caches a stale value for a hidden/just-shown popup).
    let scale = popup_scale(window, Some(*click_pos));
    let pw = POPUP_W * scale;
    // S63/S64: heal a stale physical size on every show, then use the healed
    // height for the Y math (see `heal_popup_size`).
    let ph = heal_popup_size(window, scale);
    let gap = POPUP_GAP * scale;

    // Get the work area (screen minus taskbar) in physical pixels for the
    // monitor that contains the tray icon click.
    #[cfg(windows)]
    let (work_right, work_bottom) = {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        };
        unsafe {
            // SAFETY: MonitorFromPoint and GetMonitorInfoW are read-only Win32 display queries.
            // POINT is a POD struct initialized from valid click coordinates. MONITORINFO is
            // POD with cbSize explicitly set; zeroed() is valid for remaining fields.
            // GetMonitorInfoW writes to the stack-local MONITORINFO before we read rcWork.
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
    // S64: resolve the scale from the window's monitor (the hotkey path has no
    // click point) instead of the stale `window.scale_factor()`.
    let scale = popup_scale(window, None);
    let pw = POPUP_W * scale;
    // S63/S64: same stale-size heal as position_popup — see `heal_popup_size`.
    let ph = heal_popup_size(window, scale);
    let gap = POPUP_GAP * scale;

    #[cfg(windows)]
    let (work_right, work_bottom) = {
        use windows::Win32::Foundation::POINT;
        use windows::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
        };
        unsafe {
            // SAFETY: MonitorFromPoint and GetMonitorInfoW are read-only Win32 display queries.
            // POINT { 0, 0 } targets the primary monitor. MONITORINFO is POD with cbSize
            // explicitly set; zeroed() is valid for remaining fields.
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

// ── PII redaction helpers (S25-002) ──────────────────────────────────────────

/// Redact usernames in file paths for all drive letters (A: through Z:).
///
/// `C:\Users\{username}\` → `C:\Users\<redacted>\`
/// `D:\Users\{username}\` → `D:\Users\<redacted>\`
fn redact_path_username(s: &str) -> String {
    let mut result = s.to_string();
    for drive in (b'A'..=b'Z').map(|c| c as char) {
        let prefix = format!("{drive}:\\Users\\");
        // S27-001: Redact ALL occurrences, not just the first.
        // Use a search offset to avoid re-matching the same prefix after replacement.
        let mut search_from = 0;
        while let Some(rel_start) = result[search_from..].find(&prefix) {
            let start = search_from + rel_start;
            let user_start = start + prefix.len();
            if let Some(end) = result[user_start..].find('\\') {
                let username = result[user_start..user_start + end].to_string();
                let full_match = format!("{prefix}{username}");
                let replacement = format!("{prefix}<redacted>");
                result = result.replacen(&full_match, &replacement, 1);
                // Advance past the replacement (prefix + "<redacted>")
                search_from = start + prefix.len() + "<redacted>".len();
            } else {
                // Username at end of string with no trailing backslash
                let username = result[user_start..].to_string();
                let full_match = format!("{prefix}{username}");
                let replacement = format!("{prefix}<redacted>");
                result = result.replacen(&full_match, &replacement, 1);
                // No more matches possible after end of string
                break;
            }
        }
    }
    result
}

/// Redact UNC paths: `\\server\share\` → `\\[REDACTED_PATH]\`
fn redact_unc_path(s: &str) -> String {
    if !s.contains("\\\\") {
        return s.to_string();
    }
    let mut result = s.to_string();
    // S27-001: Redact ALL UNC path occurrences, not just the first.
    // Use a search offset to avoid re-matching the replacement text.
    let mut search_from = 0;
    while let Some(rel_start) = result[search_from..].find("\\\\") {
        let start = search_from + rel_start;
        let after = &result[start + 2..];
        if let Some(first_bs) = after.find('\\') {
            let after_server = &after[first_bs + 1..];
            if let Some(second_bs) = after_server.find('\\') {
                let end = start + 2 + first_bs + 1 + second_bs;
                let unc_prefix = result[start..end].to_string();
                result = result.replacen(&unc_prefix, "\\\\[REDACTED_PATH]", 1);
            } else {
                // \\server\share without trailing backslash
                let end = start + 2 + first_bs;
                let unc_prefix = result[start..end].to_string();
                result = result.replacen(&unc_prefix, "\\\\[REDACTED_PATH]", 1);
            }
        } else {
            // \\server with no share — just redact what we have
            let unc_prefix = result[start..].to_string();
            result = result.replacen(&unc_prefix, "\\\\[REDACTED_PATH]", 1);
        }
        // Advance past the replacement text
        search_from = start + "\\\\[REDACTED_PATH]".len();
    }
    result
}

/// Redact IPv6 addresses (e.g., `2001:db8::1` → `[REDACTED_IP]`).
///
/// Detects sequences of hex digits and colons with at least 2 colons.
fn redact_ipv6(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut result = String::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_hexdigit() {
            let mut j = i;
            let mut colon_count = 0;
            while j < chars.len() && (chars[j].is_ascii_hexdigit() || chars[j] == ':') {
                if chars[j] == ':' {
                    colon_count += 1;
                }
                j += 1;
            }
            // IPv6 has at least 2 colons and is at least 5 chars (e.g., ::1)
            if colon_count >= 2 && j > i + 4 {
                result.push_str("[REDACTED_IP]");
                i = j;
                continue;
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

/// Combined PII redaction for a single string.
fn redact_pii(s: &str) -> String {
    let s = redact_path_username(s);
    let s = redact_unc_path(&s);
    redact_ipv6(&s)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redact_path_username_c_drive() {
        let input = r"C:\Users\johnsmith\AppData\Local\file.txt";
        let result = redact_path_username(input);
        assert!(result.contains("<redacted>"));
        assert!(!result.contains("johnsmith"));
    }

    #[test]
    fn test_redact_path_username_d_drive() {
        let input = r"D:\Users\alice\Documents\file.txt";
        let result = redact_path_username(input);
        assert!(result.contains("<redacted>"));
        assert!(!result.contains("alice"));
    }

    #[test]
    fn test_redact_path_username_z_drive() {
        let input = r"Z:\Users\bob\data.txt";
        let result = redact_path_username(input);
        assert!(result.contains("<redacted>"));
        assert!(!result.contains("bob"));
    }

    #[test]
    fn test_redact_unc_path() {
        let input = r"\\server\share\file.txt";
        let result = redact_unc_path(input);
        assert!(result.contains("[REDACTED_PATH]"));
        assert!(!result.contains("server"));
        assert!(!result.contains("share"));
    }

    #[test]
    fn test_redact_ipv6_full() {
        let input = "2001:db8::1";
        let result = redact_ipv6(input);
        assert_eq!(result, "[REDACTED_IP]");
    }

    #[test]
    fn test_redact_ipv6_in_text() {
        let input = "Connecting to fe80::1%eth0 from host";
        let result = redact_ipv6(input);
        assert!(result.contains("[REDACTED_IP]"));
        assert!(!result.contains("fe80::1"));
    }

    #[test]
    fn test_redact_ipv6_not_triggered_for_non_ipv6() {
        let input = "version 1.2.3";
        let result = redact_ipv6(input);
        assert_eq!(result, input);
    }

    #[test]
    fn test_redact_pii_combined() {
        let input = r"\\fileserver\share\C:\Users\charlie\2001:db8::1";
        let result = redact_pii(input);
        assert!(result.contains("[REDACTED_PATH]"));
        assert!(result.contains("<redacted>"));
        assert!(result.contains("[REDACTED_IP]"));
        assert!(!result.contains("charlie"));
        assert!(!result.contains("fileserver"));
        assert!(!result.contains("2001:db8"));
    }
    #[test]
    fn test_redact_multiple_unc_paths() {
        let input = r"Error at \\server1\share1\file1 and also at \\server2\share2\file2";
        let result = redact_unc_path(input);
        assert!(result.contains("\\\\[REDACTED_PATH]"));
        assert!(!result.contains("server1"));
        assert!(!result.contains("server2"));
    }

    #[test]
    fn test_redact_multiple_user_paths() {
        let input = r"C:\Users\alice\file1 and D:\Users\bob\file2";
        let result = redact_path_username(input);
        assert!(result.contains("<redacted>"));
        assert!(!result.contains("alice"));
        assert!(!result.contains("bob"));
    }
}
