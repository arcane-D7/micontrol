//! Power event listener for sleep/resume detection.
//!
//! Creates a hidden message-only window that receives `WM_POWERBROADCAST`
//! messages. When a resume event is detected, it triggers sensor resets
//! across all hardware modules that are affected by sleep/wake cycles.
//!
//! This solves the "Sensor unavailable after sleep" problem where the
//! ambient light sensor, thermal sensors, and other hardware stop
//! responding after the system wakes from sleep.

#![cfg(windows)]

use std::sync::atomic::{AtomicBool, Ordering};
use windows::core::w;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassExW, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_DESTROY, WM_POWERBROADCAST, WNDCLASSEXW,
};

/// Message-only window class name.
const CLASS_NAME: PCWSTR = w!("MiControlPowerListener");

/// Power broadcast event types.
const PBT_APMRESUMESUSPEND: u32 = 7;
const PBT_APMRESUMEAUTOMATIC: u32 = 18;
const PBT_APMSUSPEND: u32 = 4;

static LISTENER_RUNNING: AtomicBool = AtomicBool::new(false);

/// Start the power event listener in a background thread.
///
/// This creates a hidden message-only window that receives power broadcast
/// messages. When a resume event is detected, it calls `on_resume()` to
/// trigger sensor resets.
pub fn start_power_listener() {
    if LISTENER_RUNNING.swap(true, Ordering::SeqCst) {
        return; // Already running
    }

    std::thread::spawn(|| {
        unsafe {
            // Register window class
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                lpfnWndProc: Some(window_proc),
                lpszClassName: CLASS_NAME,
                hInstance: windows::Win32::System::LibraryLoader::GetModuleHandleW(None)
                    .unwrap_or_default()
                    .into(),
                ..Default::default()
            };

            if RegisterClassExW(&wc) == 0 {
                log::warn!("[power_listener] Failed to register window class");
                LISTENER_RUNNING.store(false, Ordering::SeqCst);
                return;
            }

            // Create message-only window (HWND_MESSAGE = -3)
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                CLASS_NAME,
                w!("MiControlPowerListener"),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                HWND(std::ptr::null_mut()),
                None,
                wc.hInstance,
                None,
            );

            if hwnd.is_err() {
                log::warn!("[power_listener] Failed to create window: {:?}", hwnd.err());
                LISTENER_RUNNING.store(false, Ordering::SeqCst);
                return;
            }

            log::info!("[power_listener] Listening for power events");

            // Message loop
            let mut msg = windows::Win32::UI::WindowsAndMessaging::MSG::default();
            while windows::Win32::UI::WindowsAndMessaging::GetMessageW(&mut msg, None, 0, 0).into()
            {
                let _ = windows::Win32::UI::WindowsAndMessaging::TranslateMessage(&msg);
                windows::Win32::UI::WindowsAndMessaging::DispatchMessageW(&msg);
            }

            let _ = DestroyWindow(hwnd.unwrap());
            LISTENER_RUNNING.store(false, Ordering::SeqCst);
            log::info!("[power_listener] Stopped");
        }
    });
}

/// Window procedure — handles WM_POWERBROADCAST messages.
extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_POWERBROADCAST {
        let event_type = wparam.0 as u32;
        match event_type {
            PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => {
                log::info!("[power_listener] Resume from sleep detected — resetting sensors");
                on_resume();
            }
            PBT_APMSUSPEND => {
                log::info!("[power_listener] Sleep/suspend detected");
            }
            _ => {
                log::debug!("[power_listener] Power event: {event_type}");
            }
        }
        return LRESULT(1);
    }

    if msg == WM_DESTROY {
        unsafe { windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0) };
    }

    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Called when a resume from sleep is detected.
///
/// Resets all hardware sensors that may have become unresponsive during sleep.
fn on_resume() {
    // Reset the ambient light sensor
    crate::hw::display::request_sensor_reset();

    // Clear WMI cache so fresh queries are made after resume
    crate::hw::wmi_cache::invalidate();

    log::info!("[power_listener] Sensor reset complete");
}
