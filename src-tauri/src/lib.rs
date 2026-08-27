//! miPC Tauri application library.
//!
//! Sets up Tauri commands, application state, menu, tray, and event handling.
//! This is the main entry point for the Tauri application runtime.

mod commands;
mod debug_log;
pub mod elev_bridge;
pub mod elevated;
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
#[allow(deprecated)]
use commands::hardware::{
    ensure_bridge_service, ensure_iot_service, get_audio_devices, get_audio_volume,
    get_battery_care, get_cast_devices, get_charging_threshold, get_ecram_map, get_function_key,
    get_iot_bind_status, get_iot_device_id, get_iot_device_info, get_iot_device_status,
    get_iot_fw_version, get_iot_model, get_iot_region_hex, get_iot_wifi_by_index,
    get_iot_wifi_count, get_iot_wifi_list, get_iot_wifi_status, get_perf_debug,
    get_performance_mode, get_primary_thermal_zone, get_thermal_zones, hq_change_boot_option,
    hq_enable_pxe_boot, hq_load_default, hq_s5_rtc_wake_enable, hq_set_performance_mode,
    hq_set_shipping_country_code, hq_set_wifi_country_code, iot_connect_wifi, iot_delete_wifi_item,
    iot_empty_wifi_items, iot_notify_ec_event, iot_notify_event, iot_notify_power_event,
    iot_pipe_available, iot_report_shutting_down, iot_report_suspending, iot_report_windows_ready,
    iot_reset_device, iot_set_device_status, iot_write_wifi_item, is_elevated, read_ecram_raw,
    relaunch_as_admin, send_iot_laptop_status, set_audio_default_endpoint, set_audio_mute,
    set_audio_volume, set_battery_care, set_charging_threshold, set_function_key,
    set_performance_mode, start_casting, stop_casting, wifi_connect, wifi_disconnect, wifi_scan,
    wifi_status, wmi_ec_get_performance_mode, wmi_ec_read, wmi_ec_read_adapter_power,
    wmi_ec_read_battery_health, wmi_ec_read_sensor_data, wmi_ec_set_auto_illumination,
    wmi_ec_set_brightness_data, wmi_ec_set_epof_flag, wmi_ec_set_label_mode,
    wmi_ec_set_lid_open_type, wmi_ec_set_mi_usage_type, wmi_ec_set_performance_mode,
    wmi_ec_set_pl1_flag, wmi_ec_set_removable_type, wmi_ec_set_sagv_mode, wmi_ec_set_wmid_type,
    wmi_ec_write, write_iot_hex,
};
use commands::hotkeys::{
    get_detected_key, get_hotkey_config, grant_script_consent, is_hook_active, set_hotkey_config,
    start_key_detect,
};
use commands::privacy::{export_user_data, reveal_in_explorer};
use commands::system::{
    check_official_driver_updates, clean_junk_files, clear_error_log, custom_security_scan,
    debug_ecram_dump, download_driver_package, fetch_official_drivers, full_security_scan,
    get_ai_brightness_config, get_audio_effects, get_autostart, get_available_refresh_rates,
    get_battery_info, get_color_status, get_crash_recovery_status, get_defender_status,
    get_display_info, get_drivers_detail, get_error_log_config, get_eye_protection, get_fan_info,
    get_hardware_profile, get_hardware_state_batch, get_model_code, get_os_turbo,
    get_phone_link_status, get_process_list, get_system_info, get_threat_history,
    get_touchpad_info, get_update_status, install_driver, launch_color_calibration_wizard,
    launch_phone_link, launch_phone_link_feature, load_icc_profile, log_frontend_error,
    mark_clean_exit, open_color_management_settings, open_phone_link_settings,
    open_windows_security, quick_security_scan, read_error_log, run_hardware_discovery,
    scan_junk_files, set_adaptive_refresh_rate, set_ai_brightness, set_ai_brightness_config,
    set_autostart, set_brightness, set_error_logging_enabled, set_eye_protection, set_fan_mode,
    set_hdr, set_mic_noise_canceling, set_os_turbo, set_refresh_rate, set_speaker_noise_canceling,
    set_touchpad_edge_slide, set_touchpad_gesture_screenshot, set_touchpad_haptics,
    set_touchpad_haptics_intensity, set_touchpad_repress, set_touchpad_sensitivity,
    set_voice_focus, trigger_driver_scan, unload_icc_profile, update_defender_signatures,
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
#[allow(deprecated)]
pub fn run() {
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
            // When a second instance is launched, focus the existing window instead.
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_shell::init());

    // MCP Bridge plugin — enables AI assistants to inspect and interact
    // with the Tauri app (screenshots, DOM, IPC calls, console logs).
    // Debug builds only (interactive debugging aid; not shipped to users
    // unless they opt in via the Settings → "MCP Integration" toggle below).
    #[cfg(debug_assertions)]
    let builder = builder.plugin(tauri_plugin_mcp_bridge::init());

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
            // Battery Care toggle (EC 0xA4)
            get_battery_care,
            set_battery_care,
            get_perf_debug,
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
            get_iot_device_info,
            get_iot_wifi_list,
            iot_notify_event,
            // Deprecated wrappers (kept for backward compatibility)
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
            // S29-001: Script hotkey consent grant command
            grant_script_consent,
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
            // MCP integration toggle (Settings → "MCP Integration")
            mcp_get_enabled,
            mcp_set_enabled,
            // Data export — GDPR Art.20 (S19-16)
            export_user_data,
            reveal_in_explorer,
            // WMAA / WMI MiInterface (elevated bridge)
            wmi_ec_read,
            wmi_ec_write,
            wmi_ec_get_performance_mode,
            wmi_ec_set_performance_mode,
            wmi_ec_read_battery_health,
            wmi_ec_read_adapter_power,
            wmi_ec_read_sensor_data,
            wmi_ec_set_brightness_data,
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
            // Crash Recovery
            get_crash_recovery_status,
            mark_clean_exit,
            // Driver Details
            get_drivers_detail,
            // Official Driver Update Check
            check_official_driver_updates,
            fetch_official_drivers,
            get_model_code,
            download_driver_package,
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

            // Apply Copilot key interception fixes (disables Windows Shell
            // interception + writes Scancode Map for permanent remap).
            // This is async because it dispatches through the elevated bridge.
            tauri::async_runtime::spawn(async {
                crate::hw::hotkeys::apply_copilot_fix().await;
            });

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

            // Start power event listener for sleep/resume detection.
            // Resets sensors (ambient light, WMI cache) after the system
            // wakes from sleep to prevent "Sensor unavailable" errors.
            #[cfg(windows)]
            crate::hw::power_listener::start_power_listener();

            // Start adaptive brightness background task
            tauri::async_runtime::spawn(crate::hw::display::adaptive_brightness_loop());

            // S32-005: Heartbeat — the SYSTEM bridge watchdog uses this to
            // distinguish a healthy UI from a frozen zombie. Write the first
            // one NOW (setup has finished, tray+main window exist) and keep
            // refreshing it on a ticker.
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
                            // S26-005: Mark clean exit + user-quit before shutting down.
                            crate::hw::crash_recovery::mark_clean_exit();
                            crate::hw::crash_recovery::mark_user_quit();
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
        let scale = window.scale_factor().map_err(|e| e.to_string())?;
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
