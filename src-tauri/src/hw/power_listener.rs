//! Power event listener for sleep/resume detection **and lid state**.
//!
//! Creates a hidden message-only window that receives `WM_POWERBROADCAST`
//! messages. When a resume event is detected, it triggers sensor resets
//! across all hardware modules that are affected by sleep/wake cycles.
//!
//! This solves the "Sensor unavailable after sleep" problem where the
//! ambient light sensor, thermal sensors, and other hardware stop
//! responding after the system wakes from sleep.
//!
//! ## Lid state (clamshell detection)
//!
//! The same window also registers for `GUID_LIDSWITCH_STATE_CHANGE`
//! (`RegisterPowerSettingNotification`, `DEVICE_NOTIFY_WINDOW_HANDLE`), the only
//! OS-level lid source on this platform:
//!
//! > **Why user32 and not powrprof (P1-1, recheck 2026-11-11).** The app used
//! > to call `powrprof!PowerSettingRegisterNotification`, which returns
//! > `ERROR_INVALID_PARAMETER` (87) on this host no matter the window kind —
//! > proven by an isolated probe from a clean process with a valid, message-only
//! > *and* top-level window. `user32!RegisterPowerSettingNotification` returns a
//! > valid `HPOWERNOTIFY` for the same window and the same GUID. The powrprof
//! > variant stays as a fallback (it is correct on other builds), but the
//! > primary is user32. A registration failure is now an **ERROR**, never a
//! > silent WARN: the previous WARN hid a dead feature for 19 h 30 m while the
//! > machine actually entered S3.
//!
//! - `Win32_Fan` / ESIF expose **no** lid property,
//! - WMAA has LOTS (`0x0C00/0x02`), a *configuration* value (lid open type),
//!   not a live switch state,
//! - `Kernel-Power` logs event 107 only when the lid action *acts* (sleep or
//!   shutdown). With "lid close = do nothing" — the owner's setting — the
//!   System log contains **no** lid records at all, which is why a
//!   "fan ramps with the lid closed" complaint could not be correlated after
//!   the fact.
//!
//! Every transition is logged at INFO with a thermal snapshot (temps, package
//! power, EC mode) taken on a separate thread — the message loop must never
//! block on WMI. The state is exposed through [`lid_state`] so other modules
//! (e.g. `hw::fan`) can report it instead of guessing.

#![cfg(windows)]

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use windows::core::{w, GUID, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Power::{
    PowerSettingRegisterNotification, RegisterPowerSettingNotification, POWERBROADCAST_SETTING,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, RegisterClassExW, DEVICE_NOTIFY_WINDOW_HANDLE,
    PBT_POWERSETTINGCHANGE, WINDOW_EX_STYLE, WINDOW_STYLE, WM_DESTROY, WM_POWERBROADCAST,
    WNDCLASSEXW,
};

/// Message-only window class name.
const CLASS_NAME: PCWSTR = w!("MiControlPowerListener");

/// Power broadcast event types.
const PBT_APMRESUMESUSPEND: u32 = 7;
const PBT_APMRESUMEAUTOMATIC: u32 = 18;
const PBT_APMSUSPEND: u32 = 4;

/// `GUID_LIDSWITCH_STATE_CHANGE` — `{BA3E0F4D-B817-4094-A2D1-D56379E6A0F3}`.
///
/// Not exported by the `windows` crate; spelled out here so the value is
/// reviewable. Data byte: non-zero = lid open, zero = lid closed.
const GUID_LIDSWITCH_STATE_CHANGE: GUID =
    GUID::from_u128(0xBA3E_0F4D_B817_4094_A2D1_D563_79E6_A0F3);

/// Lid state as reported by the OS. `Unknown` until the first notification.
const LID_UNKNOWN: u8 = 0;
const LID_OPEN: u8 = 1;
const LID_CLOSED: u8 = 2;

static LISTENER_RUNNING: AtomicBool = AtomicBool::new(false);
static LID_STATE: AtomicU8 = AtomicU8::new(LID_UNKNOWN);

/// Latest OS-reported lid state.
///
/// `Some(true)` = lid open, `Some(false)` = lid closed, `None` = the platform
/// has not reported the lid switch (notification not registered, or no switch).
pub fn lid_state() -> Option<bool> {
    match LID_STATE.load(Ordering::Relaxed) {
        LID_OPEN => Some(true),
        LID_CLOSED => Some(false),
        _ => None,
    }
}

/// `true` when the OS reports the lid closed. Unknown state is **not** closed:
/// callers must not suppress behaviour on a missing notification.
pub fn is_lid_closed() -> bool {
    LID_STATE.load(Ordering::Relaxed) == LID_CLOSED
}

/// Map a `PBT_POWERSETTINGCHANGE` payload to a lid state.
///
/// Returns `None` for any other power setting (the window receives several:
/// AC/DC source, display state, battery level, …). Pure so it can be pinned by
/// a unit test without a window.
fn lid_state_from_setting(setting: &GUID, data: &[u8]) -> Option<bool> {
    if *setting != GUID_LIDSWITCH_STATE_CHANGE {
        return None;
    }
    let first = *data.first()?;
    Some(first != 0)
}

/// Persist a transition and return `(previous, current)` as option-bools.
fn store_lid_state(open: bool) -> (Option<bool>, Option<bool>) {
    let previous = lid_state();
    LID_STATE.store(if open { LID_OPEN } else { LID_CLOSED }, Ordering::Relaxed);
    (previous, lid_state())
}

// ── Lid-switch subscription (P1-1) ──────────────────────────────────────────
//
// Which API actually accepted the subscription. Recorded so the startup
// self-test can prove the feature is live instead of assuming it.

/// How the lid-switch subscription ended up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LidRegistration {
    /// `user32!RegisterPowerSettingNotification` accepted the window (primary).
    User32,
    /// user32 was refused; `powrprof!PowerSettingRegisterNotification` accepted.
    PowrProf,
    /// Both APIs refused: no lid notification will ever arrive this session.
    Unavailable,
}

impl LidRegistration {
    /// `true` when at least one API accepted the subscription.
    pub fn is_registered(self) -> bool {
        !matches!(self, Self::Unavailable)
    }

    /// Short label for the log line.
    pub fn describe(self) -> &'static str {
        match self {
            Self::User32 => "user32!RegisterPowerSettingNotification",
            Self::PowrProf => "powrprof!PowerSettingRegisterNotification (fallback)",
            Self::Unavailable => "none",
        }
    }
}

/// Log level an outcome must be reported at.
///
/// This is the startup self-test: a dead subscription is a **missing feature**
/// and must be loud. The 2026-11-11 recheck found `Win32 error 87` at WARN once
/// at boot and then 19 h 30 m of silence — including a real S3 cycle — while
/// `lid_open` stayed `null`. An operator reading the log could not tell that
/// from "the lid never moved".
pub fn registration_log_level(outcome: LidRegistration) -> log::Level {
    match outcome {
        LidRegistration::User32 | LidRegistration::PowrProf => log::Level::Info,
        LidRegistration::Unavailable => log::Level::Error,
    }
}

/// Result of the two-step subscription attempt.
///
/// Carries the error codes so the caller can put them in the log; the decision
/// itself ([`LidRegistration`]) stays a plain enum so it is cheap to assert on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LidRegistrationOutcome {
    pub registration: LidRegistration,
    /// Registration handle (`HPOWERNOTIFY` / powrprof registration), if any.
    pub handle: Option<isize>,
    /// `HRESULT`/Win32 code from the user32 attempt.
    pub user32_error: Option<u32>,
    /// `WIN32_ERROR` code from the powrprof fallback.
    pub powrprof_error: Option<u32>,
}

/// One registration attempt, reduced to a handle or an error code.
type RegisterAttempt = Result<isize, u32>;

/// Try user32 first, powrprof second, and report what happened.
///
/// The two attempts are injected so the failure path — the one that was
/// silently swallowed in production — is unit-testable without a window, a
/// message loop, or a real failure of either DLL.
fn register_lid_notifications_with<U, P>(user32: U, powrprof: P) -> LidRegistrationOutcome
where
    U: FnOnce() -> RegisterAttempt,
    P: FnOnce() -> RegisterAttempt,
{
    match user32() {
        Ok(handle) => LidRegistrationOutcome {
            registration: LidRegistration::User32,
            handle: Some(handle),
            user32_error: None,
            powrprof_error: None,
        },
        Err(user32_error) => match powrprof() {
            Ok(handle) => LidRegistrationOutcome {
                registration: LidRegistration::PowrProf,
                handle: Some(handle),
                user32_error: Some(user32_error),
                powrprof_error: None,
            },
            Err(powrprof_error) => LidRegistrationOutcome {
                registration: LidRegistration::Unavailable,
                handle: None,
                user32_error: Some(user32_error),
                powrprof_error: Some(powrprof_error),
            },
        },
    }
}

/// Emit the subscription result at the level [`registration_log_level`] picks.
fn log_lid_registration(outcome: &LidRegistrationOutcome) {
    match outcome.registration {
        LidRegistration::User32 => log::info!(
            target: "hw::power_listener",
            "[power_listener] lid-switch notifications registered via {}",
            outcome.registration.describe()
        ),
        LidRegistration::PowrProf => log::warn!(
            target: "hw::power_listener",
            "[power_listener] lid-switch registered via {} — user32 refused it (0x{:08X}); \
             lid events will arrive through the slower powrprof path",
            outcome.registration.describe(),
            outcome.user32_error.unwrap_or(0)
        ),
        LidRegistration::Unavailable => log::error!(
            target: "hw::power_listener",
            "[power_listener] lid notifications unavailable: user32 RegisterPowerSettingNotification \
             failed (0x{:08X}) and the powrprof fallback failed (Win32 error {}) — lid_open will \
             stay unknown for this session and no resume-triggered re-apply will fire from the lid \
             subscription (resume broadcasts still arrive)",
            outcome.user32_error.unwrap_or(0),
            outcome.powrprof_error.unwrap_or(0)
        ),
    }
}

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

            // Lid switch notifications. Registration also delivers the current
            // state immediately, so `lid_state()` is populated from boot.
            //
            // P1-1: user32 first (powrprof returns 87 on this host), with the
            // powrprof call kept as a fallback. Both failures are an ERROR —
            // the outcome is logged, not swallowed.
            let hwnd = hwnd.unwrap();
            let outcome = register_lid_notifications_with(
                || {
                    // SAFETY: hwnd is a live window on this thread; the GUID is
                    // a 'static constant. The returned HPOWERNOTIFY stays valid
                    // for as long as the window does.
                    RegisterPowerSettingNotification(
                        hwnd,
                        &GUID_LIDSWITCH_STATE_CHANGE,
                        DEVICE_NOTIFY_WINDOW_HANDLE,
                    )
                    .map(|handle| handle.0)
                    .map_err(|e| e.code().0 as u32)
                },
                || {
                    let mut registration: *mut core::ffi::c_void = std::ptr::null_mut();
                    // SAFETY: same live hwnd and 'static GUID; the out-pointer
                    // is a valid stack slot.
                    let rc = PowerSettingRegisterNotification(
                        &GUID_LIDSWITCH_STATE_CHANGE,
                        DEVICE_NOTIFY_WINDOW_HANDLE,
                        hwnd,
                        &mut registration,
                    );
                    if rc.0 == 0 {
                        Ok(registration as isize)
                    } else {
                        Err(rc.0)
                    }
                },
            );
            log_lid_registration(&outcome);

            log::info!("[power_listener] Listening for power events");

            // Message loop
            let mut msg = windows::Win32::UI::WindowsAndMessaging::MSG::default();
            while windows::Win32::UI::WindowsAndMessaging::GetMessageW(&mut msg, None, 0, 0).into()
            {
                let _ = windows::Win32::UI::WindowsAndMessaging::TranslateMessage(&msg);
                windows::Win32::UI::WindowsAndMessaging::DispatchMessageW(&msg);
            }

            let _ = DestroyWindow(hwnd);
            LISTENER_RUNNING.store(false, Ordering::SeqCst);
            log::info!("[power_listener] Stopped");
        }
    });
}

/// Window procedure — handles WM_POWERBROADCAST messages.
extern "system" fn window_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if msg == WM_POWERBROADCAST {
        let event_type = wparam.0 as u32;

        if event_type == PBT_POWERSETTINGCHANGE {
            // SAFETY: `lParam` is the `POWERBROADCAST_SETTING` the OS built for
            // this message; `lid_from_lparam` validates the pointer and length.
            if let Some(open) = unsafe { lid_from_lparam(lparam) } {
                let (previous, current) = store_lid_state(open);
                if previous != current {
                    log::info!(
                        target: "hw::power_listener",
                        "[lid] {} (previous {})",
                        if open { "OPEN" } else { "CLOSED" },
                        match previous {
                            Some(true) => "open",
                            Some(false) => "closed",
                            None => "unknown",
                        }
                    );
                    log_lid_snapshot(open);
                }
            }
            return LRESULT(1);
        }

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

/// Read the lid flag out of a `PBT_POWERSETTINGCHANGE` `lParam`.
///
/// `lParam` points at a `POWERBROADCAST_SETTING` whose `Data` is a flexible
/// array of `DataLength` bytes; only the first byte is read.
unsafe fn lid_from_lparam(lparam: LPARAM) -> Option<bool> {
    if lparam.0 == 0 {
        return None;
    }
    let setting = &*(lparam.0 as *const POWERBROADCAST_SETTING);
    if setting.DataLength == 0 {
        return None;
    }
    lid_state_from_setting(&setting.PowerSetting, &setting.Data)
}

/// Log one thermal snapshot next to a lid transition.
///
/// The snapshot is the evidence the historical investigation lacked: it records
/// what the machine was doing at the exact moment the lid moved, so a later
/// "the fan ramped" report can be checked against temps/power/EC mode instead
/// of being reconstructed from unrelated timestamps. Runs on its own thread —
/// the WMI/pipe reads must not block the window message loop.
fn log_lid_snapshot(open: bool) {
    std::thread::spawn(move || match crate::hw::fan::get_fan_info() {
        Ok(info) => log::info!(
            target: "hw::power_listener",
            "[lid] snapshot lid_open={open} cpu={:?} gpu={:?} ambient={:?} tdp={:?} \
             mode={:?} requested={} rpm={:?} source={:?}",
            info.cpu_temp_celsius,
            info.gpu_temp_celsius,
            info.ambient_temp_celsius,
            info.tdp_watts,
            info.mode,
            info.requested_speed_percent,
            info.speed_rpm,
            info.temp_source
        ),
        Err(e) => log::warn!(target: "hw::power_listener", "[lid] snapshot failed: {e}"),
    });
}

/// Called when a resume from sleep is detected.
///
/// Resets all hardware sensors that may have become unresponsive during sleep.
fn on_resume() {
    // Reset the ambient light sensor
    crate::hw::display::request_sensor_reset();

    // S37-006: Reset the elevated thermal circuit-breaker + TTL cache.
    // After sleep/resume the MiControlBridge service may be momentarily
    // unavailable; without this reset the breaker stays open (60 s backoff)
    // and temperatures remain suppressed long after wake.
    crate::hw::fan::reset_elevated_thermal_circuit();

    // Clear WMI cache so fresh queries are made after resume
    crate::hw::wmi_cache::invalidate();

    // P1-2 (recheck 2026-11-11): the EC forgets the charging limit across an
    // S3 cycle — the 02:23 → 10:55 resume left the hardware at 100 % while
    // the user had asked for 80 %. Re-assert the requested value here, on the
    // resume broadcast, through the gated reconciler (read-back + log; never
    // an unconditional write loop). Fire-and-forget: this runs on the window
    // message thread and must not block on an elevated round-trip.
    tauri::async_runtime::spawn(async {
        crate::hw::charging::reconcile_charge_limit("resume").await;
    });

    log::info!("[power_listener] Sensor reset complete");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A different power setting (AC/DC source) must not be read as a lid state.
    #[test]
    fn other_power_settings_are_ignored() {
        let acdc = GUID::from_u128(0x5D3E_9A6C_5D5D_4B7A_9E1C_9C2D_2A1F_0B6B);
        assert_eq!(lid_state_from_setting(&acdc, &[1]), None);
    }

    #[test]
    fn lid_payload_zero_is_closed_and_nonzero_is_open() {
        assert_eq!(
            lid_state_from_setting(&GUID_LIDSWITCH_STATE_CHANGE, &[0]),
            Some(false)
        );
        assert_eq!(
            lid_state_from_setting(&GUID_LIDSWITCH_STATE_CHANGE, &[1]),
            Some(true)
        );
        // Windows has shipped values other than 0/1 for "open"; anything
        // non-zero is open.
        assert_eq!(
            lid_state_from_setting(&GUID_LIDSWITCH_STATE_CHANGE, &[2]),
            Some(true)
        );
    }

    #[test]
    fn empty_payload_is_not_a_lid_reading() {
        assert_eq!(
            lid_state_from_setting(&GUID_LIDSWITCH_STATE_CHANGE, &[]),
            None
        );
    }

    /// Unknown must never be treated as "closed": suppression on a missing
    /// notification would disable work on platforms without a lid switch.
    #[test]
    fn unknown_state_is_not_closed() {
        LID_STATE.store(LID_UNKNOWN, Ordering::Relaxed);
        assert_eq!(lid_state(), None);
        assert!(!is_lid_closed());
    }

    #[test]
    fn store_reports_the_previous_and_current_state() {
        LID_STATE.store(LID_UNKNOWN, Ordering::Relaxed);
        let (prev, cur) = store_lid_state(false);
        assert_eq!((prev, cur), (None, Some(false)));
        assert!(is_lid_closed());

        let (prev, cur) = store_lid_state(true);
        assert_eq!((prev, cur), (Some(false), Some(true)));
        assert!(!is_lid_closed());

        // Repeated identical notifications must not look like transitions.
        let (prev, cur) = store_lid_state(true);
        assert_eq!(prev, cur);
    }

    // ── P1-1: the subscription self-test ────────────────────────────────────

    /// The production failure: both APIs refuse. That must be an ERROR, not
    /// the WARN that hid a dead listener for 19 h 30 m.
    #[test]
    fn both_apis_failing_is_an_error_not_a_warning() {
        let outcome = register_lid_notifications_with(|| Err(0x8007_0057), || Err(87));
        assert_eq!(outcome.registration, LidRegistration::Unavailable);
        assert!(!outcome.registration.is_registered());
        assert_eq!(outcome.handle, None);
        assert_eq!(outcome.user32_error, Some(0x8007_0057));
        assert_eq!(outcome.powrprof_error, Some(87));
        assert_eq!(
            registration_log_level(outcome.registration),
            log::Level::Error,
            "a dead lid subscription must be logged at ERROR"
        );
    }

    /// user32 is the primary: when it works, powrprof must not even be tried.
    #[test]
    fn user32_success_does_not_fall_back() {
        let outcome = register_lid_notifications_with(
            || Ok(0x1234),
            || panic!("powrprof must not be called when user32 succeeds"),
        );
        assert_eq!(outcome.registration, LidRegistration::User32);
        assert_eq!(outcome.handle, Some(0x1234));
        assert_eq!(outcome.user32_error, None);
        assert_eq!(
            registration_log_level(outcome.registration),
            log::Level::Info
        );
    }

    /// The powrprof path is kept as a fallback and is reported as such.
    #[test]
    fn powrprof_is_the_fallback_when_user32_fails() {
        let outcome = register_lid_notifications_with(|| Err(0x8007_0057), || Ok(0x5678));
        assert_eq!(outcome.registration, LidRegistration::PowrProf);
        assert_eq!(outcome.handle, Some(0x5678));
        assert_eq!(outcome.user32_error, Some(0x8007_0057));
        assert_eq!(outcome.powrprof_error, None);
        assert!(outcome.registration.is_registered());
        assert_eq!(
            registration_log_level(outcome.registration),
            log::Level::Info
        );
    }
}
