//! Display brightness, HDR, and refresh rate control.
//!
//! Uses IGCL (Intel Graphics Command Library) for brightness and WMI
//! for display info queries on Windows.

use crate::hw::errors::{HardwareError, HardwareResult};
use crate::util::process_spawn;
use crate::util::state_store;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicI16, AtomicU64, AtomicU8, Ordering};

/// `false` while IGCL writes are known to fail, so a broken ControlLib is not
/// retried on every brightness change. It is re-armed by the slow retry
/// (`IGCL_RETRY_SECS`) and by a successful device re-enumeration (P10).
static IGCL_SET_AVAILABLE: AtomicBool = AtomicBool::new(true);

// ── Brightness source state (P10) ────────────────────────────────────────────
//
// IGCL can be present (ControlLib.dll loads) yet enumerate ZERO devices, which
// silently pins brightness to the WMI path for the whole session. The old code
// logged two WARNs ("no IGCL devices enumerated" + "falling back to WMI
// permanently") with no state, no reason and no retry. This block keeps the
// live source, exposes it through `DisplayInfo`, logs one clear transition and
// persists it so a post-mortem (or the next session's UI) still shows why the
// machine is on the fallback.
const SOURCE_UNKNOWN: u8 = 0;
const SOURCE_IGCL: u8 = 1;
const SOURCE_WMI_FALLBACK: u8 = 2;
static BRIGHTNESS_SOURCE: AtomicU8 = AtomicU8::new(SOURCE_UNKNOWN);

/// Retry IGCL slowly after a failure: a driver update / GPU re-enumeration can
/// restore it without restarting the app. (P10 requirement: ~10 min.)
const IGCL_RETRY_SECS: u64 = 600;

fn igcl_retry_at() -> &'static std::sync::Mutex<Option<std::time::Instant>> {
    static RETRY_AT: std::sync::OnceLock<std::sync::Mutex<Option<std::time::Instant>>> =
        std::sync::OnceLock::new();
    RETRY_AT.get_or_init(|| std::sync::Mutex::new(None))
}

fn schedule_igcl_retry() {
    let mut guard = crate::util::panic::lock_or_recover(igcl_retry_at());
    *guard = Some(std::time::Instant::now() + std::time::Duration::from_secs(IGCL_RETRY_SECS));
}

/// Pure deadline check so the slow-retry window is unit-testable with an
/// injected `now` (no sleeps).
fn retry_due(deadline: Option<std::time::Instant>, now: std::time::Instant) -> bool {
    deadline.is_some_and(|at| now >= at)
}

/// True once the slow IGCL retry window has elapsed (and consumes it).
fn igcl_retry_due() -> bool {
    let mut guard = crate::util::panic::lock_or_recover(igcl_retry_at());
    let due = retry_due(*guard, std::time::Instant::now());
    if due {
        *guard = None;
    }
    due
}

fn brightness_source_name(source: u8) -> &'static str {
    match source {
        SOURCE_IGCL => "igcl",
        SOURCE_WMI_FALLBACK => "wmi-fallback",
        _ => "unknown",
    }
}

/// Current brightness backend, for logs / `DisplayInfo` / the UI.
pub fn brightness_source() -> &'static str {
    let current = BRIGHTNESS_SOURCE.load(Ordering::Relaxed);
    if current != SOURCE_UNKNOWN {
        return brightness_source_name(current);
    }
    persisted_brightness_source()
}

#[derive(Debug, Serialize, Deserialize)]
struct BrightnessSourceState {
    source: String,
    reason: String,
    updated_at_ms: u64,
}

fn brightness_source_path() -> std::path::PathBuf {
    if let Ok(override_path) = std::env::var("MICONTROL_BRIGHTNESS_SOURCE_PATH") {
        return std::path::PathBuf::from(override_path);
    }
    let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into());
    std::path::PathBuf::from(base)
        .join("MiControl")
        .join("brightness_source.json")
}

fn persist_brightness_source(source: &str, reason: &str) {
    let state = BrightnessSourceState {
        source: source.to_string(),
        reason: reason.to_string(),
        updated_at_ms: now_ms(),
    };
    let path = brightness_source_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match serde_json::to_string_pretty(&state) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                log::debug!(target: "hw::display", "failed to persist brightness source: {e}");
            }
        }
        Err(e) => log::debug!(target: "hw::display", "failed to serialize brightness source: {e}"),
    }
}

/// Last persisted source (previous session), used only until the first live
/// brightness operation records the current one.
fn persisted_brightness_source() -> &'static str {
    static CACHED: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();
    CACHED.get_or_init(|| {
        let Ok(data) = std::fs::read_to_string(brightness_source_path()) else {
            return "unknown";
        };
        let Ok(state) = serde_json::from_str::<BrightnessSourceState>(&data) else {
            return "unknown";
        };
        match state.source.as_str() {
            "igcl" => "igcl",
            "wmi-fallback" => "wmi-fallback",
            _ => "unknown",
        }
    })
}

/// Record a source transition. Logs exactly ONE line per change (the previous
/// code emitted a WARN on every failed brightness write) and persists the state.
fn set_brightness_source(source: u8, reason: &str) {
    let previous = BRIGHTNESS_SOURCE.swap(source, Ordering::Relaxed);
    if previous == source {
        return;
    }
    let name = brightness_source_name(source);
    if source == SOURCE_WMI_FALLBACK {
        log::info!(
            target: "hw::display",
            "brightness source: {name} ({reason}); writes use WMI, IGCL retried every {IGCL_RETRY_SECS}s"
        );
    } else {
        log::info!(target: "hw::display", "brightness source: {name} ({reason})");
    }
    persist_brightness_source(name, reason);
}

/// Called by the IGCL session when ControlLib.dll loads but no device is
/// usable (P10). Keeps the diagnostic to a single line via
/// `set_brightness_source` (which only logs on transitions).
#[cfg(windows)]
fn report_igcl_unavailable(reason: &str) {
    set_brightness_source(SOURCE_WMI_FALLBACK, reason);
    schedule_igcl_retry();
}

/// Called by the IGCL session when a device shows up again.
#[cfg(windows)]
fn report_igcl_available(reason: &str) {
    IGCL_SET_AVAILABLE.store(true, Ordering::Relaxed);
    *crate::util::panic::lock_or_recover(igcl_retry_at()) = None;
    set_brightness_source(SOURCE_IGCL, reason);
}

/// Probe IGCL once per process so `DisplayInfo.brightness_source` reports the
/// live backend instead of the value persisted by the previous session.
///
/// Cheap: it opens the (already cached, load-once) IGCL session and reads the
/// current brightness; subsequent calls are a no-op because the source is then
/// known.
#[cfg(windows)]
fn probe_brightness_source_once() {
    if BRIGHTNESS_SOURCE.load(Ordering::Relaxed) != SOURCE_UNKNOWN {
        return;
    }
    match get_brightness_igcl() {
        Ok(_) => report_igcl_available("IGCL brightness read succeeded"),
        Err(e) => set_brightness_source(SOURCE_WMI_FALLBACK, &format!("IGCL unavailable: {e}")),
    }
}

#[cfg(not(windows))]
fn probe_brightness_source_once() {}

/// Persistent IGCL session (load-once, never-close).
///
/// Intel's ControlLib.dll (IGCL) is NOT designed for repeated
/// load→ctlInit→…→ctlClose→free cycles: it holds driver-side global state and
/// spawns internal threads that keep running code from the DLL. Each
/// load/init/close/free cycle therefore risks an access violation in
/// `ControlLib.dll_unloaded` (BEX64, fault offset 0x10180) once the DLL image
/// is unmapped while an internal IGCL thread is still executing inside it —
/// this is the exact crash that kept killing the app (and the watchdog kept
/// relaunching it) whenever the adaptive-brightness loop touched the backlight.
///
/// Instead of re-loading per call, we initialize **once** and keep the
/// `Library` + API/device handles alive for the whole process. `ctlClose` is
/// deliberately never called (control reaches here at most a handful of times
/// per session anyway) and the `Library` is never dropped — so ControlLib.dll
/// stays mapped and its internal threads keep their runtime. This converts a
/// per-operation crash-prone teardown into a stable shared session.
#[cfg(windows)]
mod igcl_session {
    use super::igcl::*;
    use libloading::Library;
    use std::sync::{LazyLock, Mutex};

    pub struct IgclSession {
        /// Keep the loaded Library alive for the whole process so we NEVER
        /// unload ControlLib.dll (its internal threads keep executing it).
        #[allow(dead_code)]
        lib: Library,
        /// API handle returned by `ctlInit`. Kept so re-enumeration (P10) does
        /// not need a second `ctlInit`.
        api: CtlApiHandle,
        device: *mut std::ffi::c_void,
        /// Last `ctlEnumerateDevices` attempt — throttles recovery retries.
        last_enum: std::time::Instant,
    }

    // SAFETY: the session is opened once at first use; the handles are opaque
    // IGCL pointers managed by the DLL and only ever used under the same lock.
    unsafe impl Send for IgclSession {}
    // SAFETY: all access is serialized by IGCL_LOCK in with_igcl_device_pub.
    unsafe impl Sync for IgclSession {}

    pub static IGG_SESSION: LazyLock<Mutex<Option<IgclSession>>> =
        LazyLock::new(|| Mutex::new(None));

    /// Minimum interval between `ctlEnumerateDevices` recovery attempts when
    /// the session has no usable device (P10). Brightness writes additionally
    /// gate on a 10-minute retry, so the driver is never hammered.
    const REENUM_MIN_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

    /// Enumerate IGCL display devices.
    ///
    /// * `Ok(Some(device))` — usable device handle.
    /// * `Ok(None)` — the driver reports zero devices (driver/device not
    ///   supported by IGCL, e.g. no Intel GPU exposed to ControlLib).
    /// * `Err(msg)` — symbol missing or a hard HRESULT.
    fn enumerate_devices(
        lib: &Library,
        api: CtlApiHandle,
    ) -> Result<Option<*mut std::ffi::c_void>, String> {
        let ctl_enumerate: libloading::Symbol<FnCtlEnumerateDevices> =
            unsafe { lib.get(b"ctlEnumerateDevices\0") }
                .map_err(|e| format!("ctlEnumerateDevices symbol missing: {e}"))?;
        let mut count: u32 = 0;
        let rc = unsafe { ctl_enumerate(api, &mut count, std::ptr::null_mut()) };
        if rc != 0 && (rc & 0x8000_0000) != 0 {
            return Err(format!("ctlEnumerateDevices count failed rc=0x{rc:08X}"));
        }
        if count == 0 {
            return Ok(None);
        }
        let mut devices = vec![std::ptr::null_mut::<std::ffi::c_void>(); count as usize];
        let rc2 = unsafe { ctl_enumerate(api, &mut count, devices.as_mut_ptr()) };
        if rc2 != 0 && (rc2 & 0x8000_0000) != 0 {
            return Err(format!("ctlEnumerateDevices fill failed rc=0x{rc2:08X}"));
        }
        Ok(devices.first().copied().filter(|d| !d.is_null()))
    }

    /// Open (or reuse) a persistent IGCL session. Returns the stored handles
    /// without ever calling `ctlClose` or unloading the library.
    ///
    /// # Crash safety (MIOT-39)
    /// Never drop the `Library` once loaded: ControlLib.dll spawns internal
    /// threads that keep executing inside it even after `ctlInit` returns, so
    /// unmapping the DLL (dropping the last `libloading::Library`) crashes the
    /// process (`ControlLib.dll_unloaded`, `0xc0000005` in combase/COM).
    ///
    /// Intel returns *warning* HRESULTs with the severity bit CLEAR
    /// (`rc != 0` but `rc & 0x80000000 == 0`) — observed `0x40000021` and
    /// `0x40000009` from `ctlInit` on this platform. Those are informational
    /// (e.g. "already initialized by the display driver"), NOT fatal. We log
    /// them and continue; only hard failures (`rc & 0x80000000 != 0`) are
    /// treated as errors — and even then the session is stored holding the
    /// Library so the DLL stays mapped.
    pub fn get_session() -> Result<(Library, *mut std::ffi::c_void), String> {
        let mut guard = IGG_SESSION.lock().map_err(|e| e.to_string())?;
        if let Some(s) = guard.as_mut() {
            // P10: a null device used to stick for the whole session ("falling
            // back to WMI permanently"). Re-enumerate slowly — never reload or
            // unload the DLL — so a driver re-init or GPU re-enumeration can
            // restore hardware brightness without restarting the app.
            if s.device.is_null() && s.last_enum.elapsed() >= REENUM_MIN_INTERVAL {
                s.last_enum = std::time::Instant::now();
                match enumerate_devices(&s.lib, s.api) {
                    Ok(Some(device)) => {
                        s.device = device;
                        super::report_igcl_available("IGCL device appeared on re-enumeration");
                    }
                    Ok(None) => {
                        log::debug!("[igcl_session] re-enumeration still reports 0 IGCL devices");
                    }
                    Err(e) => log::debug!("[igcl_session] re-enumeration failed: {e}"),
                }
            }
            // Session already stored — its `lib` handle keeps ControlLib.dll
            // mapped for the whole process. A failed reload here only means
            // the caller falls back to WMI; the DLL stays mapped regardless.
            let lib_for_caller = loader_reload(&s.path()).ok_or_else(|| {
                format!(
                    "ControlLib.dll reload failed (session: device={:?})",
                    s.device
                )
            })?;
            return Ok((lib_for_caller, s.device));
        }
        // First use: load once and initialize the SDK.
        let path = s_path();
        let lib = unsafe { Library::new(&path) }
            .map_err(|e| format!("LoadLibrary ControlLib.dll: {e}"))?;

        // ── ctlInit: warning HRESULTs are NOT fatal ─────────────────────────
        // Intel IGCL init can return 0x40000021 / 0x40000009 (INFO/WARNING:
        // severity bit clear) even on success paths. Treating those as fatal
        // made us drop `lib` → ControlLib.dll unloaded → crash. Hard failures
        // (severity bit 0x80000000 SET) are logged loudly but we STILL store
        // the session so the DLL never unloads; runtime ops then fall back to
        // WMI without dismantling the mapped library.
        let ctl_init: libloading::Symbol<FnCtlInit> = unsafe {
            match lib.get(b"ctlInit\0") {
                Ok(s) => s,
                Err(e) => {
                    log::warn!(
                        "[igcl_session] ctlInit symbol missing: {e} — \
                         storing session anyway so ControlLib.dll stays mapped"
                    );
                    let s = IgclSession {
                        lib,
                        api: std::ptr::null_mut(),
                        device: std::ptr::null_mut(),
                        last_enum: std::time::Instant::now(),
                    };
                    *guard = Some(s);
                    super::report_igcl_unavailable(
                        "ControlLib.dll has no ctlInit symbol (incompatible IGCL build)",
                    );
                    let lib_for_caller = loader_reload(&path).ok_or_else(|| {
                        "ControlLib.dll reload failed after ctlInit symbol error".to_string()
                    })?;
                    return Ok((lib_for_caller, std::ptr::null_mut()));
                }
            }
        };
        let mut init_args = CtlInitArgs {
            size: std::mem::size_of::<CtlInitArgs>() as u32,
            app_version: 1,
            flags: 0,
        };
        let mut api_handle: CtlApiHandle = std::ptr::null_mut();
        let rc = unsafe { ctl_init(&mut init_args, &mut api_handle) };
        if rc != 0 {
            let severity = rc & 0x8000_0000;
            if severity != 0 {
                log::warn!(
                    "[igcl_session] ctlInit hard failure rc=0x{rc:08X} — \
                     storing session anyway so ControlLib.dll stays mapped \
                     (brightness will fall back to WMI)"
                );
            } else {
                log::warn!(
                    "[igcl_session] ctlInit warning rc=0x{rc:08X} (severity bit clear) — \
                     treating as non-fatal, continuing"
                );
            }
        }

        // ── ctlEnumerateDevices: best-effort, one clear diagnostic ──────────
        // Warnings are non-fatal; a failed enumerate leaves a null device and
        // runtime ops fall back to WMI — but the DLL stays mapped either way.
        // P10: ONE diagnostic line (via the brightness-source transition) and a
        // slow retry, instead of a WARN here plus a "falls back permanently"
        // WARN from the caller.
        let device = match enumerate_devices(&lib, api_handle) {
            Ok(Some(device)) => {
                super::report_igcl_available("IGCL device enumerated at session start");
                device
            }
            Ok(None) => {
                super::report_igcl_unavailable(&format!(
                    "ControlLib.dll loaded ({path}) but ctlEnumerateDevices returned 0 devices — \
                     this Intel display driver does not expose IGCL devices"
                ));
                std::ptr::null_mut()
            }
            Err(e) => {
                super::report_igcl_unavailable(&format!(
                    "ControlLib.dll loaded ({path}) but IGCL enumeration failed: {e}"
                ));
                std::ptr::null_mut()
            }
        };

        // ALWAYS store the session: the Library must never be dropped.
        let s = IgclSession {
            lib,
            api: api_handle,
            device,
            last_enum: std::time::Instant::now(),
        };
        *guard = Some(s);
        // Re-load for the caller so the stored session keeps its own refcount:
        // LoadLibrary increments the module refcount and does NOT unload it.
        // Store FIRST so a (virtually impossible) reload failure can never
        // unmap the DLL — the session already holds its own handle.
        let lib_for_caller = loader_reload(&path).ok_or_else(|| {
            format!(
                "ControlLib.dll reload failed (stored session keeps DLL mapped; device={device:?})"
            )
        })?;
        log::debug!("[igcl_session] persistent IGCL session stored (device={device:?})");
        Ok((lib_for_caller, device))
    }

    impl IgclSession {
        fn path(&self) -> String {
            // The stored Library keeps the DLL mapped; the caller re-binds
            // symbols from a fresh handle to the same path.
            s_path()
        }
    }

    /// Load a fresh `libloading::Library` handle for the caller. The stored
    /// session keeps its own handle mapped, so this just bumps the module
    /// refcount — the DLL is never unmapped while we hold either handle.
    ///
    /// Returns `None` only if LoadLibrary fails while the DLL is already
    /// mapped by the session (effectively impossible); the caller treats that
    /// as a soft failure — the session itself keeps the DLL alive.
    fn loader_reload(path: &str) -> Option<Library> {
        unsafe { Library::new(path) }
            .map_err(|e| log::warn!("[igcl_session] ControlLib.dll reload failed: {e}"))
            .ok()
    }

    fn s_path() -> String {
        crate::hw::discovery::global_profile()
            .and_then(|p| p.igcl_dll_path)
            .unwrap_or_else(|| super::IGCL_DLL.to_string())
    }
}

#[cfg(windows)]
pub fn with_igcl_device_pub<F, T>(f: F) -> HardwareResult<T>
where
    F: FnOnce(*mut std::ffi::c_void, &libloading::Library) -> HardwareResult<T>,
{
    // Serialize ALL IGCL calls: IGCL/ControlLib is NOT thread-safe. The
    // brightness loop reads+sets on tokio blocking threads while the frontend
    // may query brightness simultaneously. Hold the lock for the whole
    // session-open + op sequence so each IGCL operation is atomic.
    static IGCL_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();

    // S56 FIX (2026-09-13 UI freeze): a hung ControlLib call used to hold this
    // mutex forever. Every later brightness/display/fan call then blocked its
    // thread indefinitely, starving the poll loops and every UI command that
    // touched them — the whole app froze while hotkeys (a plain OS thread)
    // kept working. Two guards now bound the damage:
    //   1. Fail fast while a recent hang is suspected (poison window).
    //   2. Bound lock acquisition; a caller that cannot get the lock returns
    //      an error instead of blocking forever behind the stuck holder.
    if igcl_poisoned() {
        return Err(HardwareError::Display(
            "IGCL unavailable: a previous ControlLib call hung; failing fast".into(),
        ));
    }

    let mutex = IGCL_LOCK.get_or_init(|| std::sync::Mutex::new(()));
    let Some(_igcl_guard) = acquire_igcl_lock(mutex) else {
        arm_igcl_poison();
        log::error!(
            target: "hw::display",
            "IGCL lock not acquired within {IGCL_LOCK_WAIT:?} — another \
             ControlLib call appears stuck; arming poison window"
        );
        return Err(HardwareError::Display(
            "IGCL busy: another ControlLib call is stuck; failing fast".into(),
        ));
    };

    let started = std::time::Instant::now();
    let result = (|| {
        let (lib, device) = igcl_session::get_session()
            .map_err(|e| HardwareError::Display(format!("IGCL session: {e}")))?;
        f(device, &lib)
    })();

    // A healthy IGCL op takes single-digit milliseconds. If one took longer
    // than the watchdog threshold, the driver stalled inside the call — arm
    // the poison window so subsequent callers fail fast instead of piling up
    // behind the (possibly still hung) mutex holder.
    let elapsed = started.elapsed();
    if elapsed >= IGCL_OP_WATCHDOG {
        arm_igcl_poison();
        log::error!(
            target: "hw::display",
            "IGCL call took {elapsed:?} (threshold {IGCL_OP_WATCHDOG:?}) — \
             arming poison window"
        );
    }

    result
}

/// How long a caller waits for the IGCL mutex before giving up (error).
const IGCL_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(3);
/// A healthy IGCL op is single-digit ms; anything at or above this is a stall.
const IGCL_OP_WATCHDOG: std::time::Duration = std::time::Duration::from_secs(10);
/// After a detected stall, callers fail fast (no lock wait) for this long,
/// then probe the driver again in case it recovered.
const IGCL_POISON_WINDOW: std::time::Duration = std::time::Duration::from_secs(60);

/// Unix ms until which IGCL callers should fail fast (0 = healthy).
static IGCL_POISONED_UNTIL: AtomicU64 = AtomicU64::new(0);

#[cfg(windows)]
fn igcl_poisoned() -> bool {
    IGCL_POISONED_UNTIL.load(Ordering::Relaxed) > now_ms()
}

#[cfg(windows)]
fn arm_igcl_poison() {
    IGCL_POISONED_UNTIL.store(
        now_ms() + IGCL_POISON_WINDOW.as_millis() as u64,
        Ordering::Relaxed,
    );
}

/// Try to acquire the IGCL mutex with a bounded wait. Never panics on mutex
/// poisoning (a panicked holder must not take IGCL down permanently).
#[cfg(windows)]
fn acquire_igcl_lock(mutex: &std::sync::Mutex<()>) -> Option<std::sync::MutexGuard<'_, ()>> {
    use std::sync::TryLockError;

    let deadline = std::time::Instant::now() + IGCL_LOCK_WAIT;
    loop {
        match mutex.try_lock() {
            Ok(guard) => return Some(guard),
            Err(TryLockError::Poisoned(poisoned)) => {
                return Some(poisoned.into_inner());
            }
            Err(TryLockError::WouldBlock) => {
                if std::time::Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }
}

/// S32-003: Set while the Windows display color calibration wizard (dccw.exe)
/// is open. While set, the adaptive-brightness loop stops touching the
/// backlight/LUT so the wizard can save its calibration without
/// "Access is denied" (the LUT would otherwise be locked by our gamma ramp).
pub static CALIBRATION_IN_PROGRESS: AtomicBool = AtomicBool::new(false);

/// Returns `true` while the display color calibration wizard is running.
pub fn is_calibration_in_progress() -> bool {
    CALIBRATION_IN_PROGRESS.load(Ordering::Relaxed)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DisplayInfo {
    pub brightness: u8,
    pub hdr_enabled: bool,
    /// True when at least one connected display reports HDR (advanced color)
    /// support. The UI disables the toggle when this is false, instead of
    /// offering a switch that would write to paths that cannot honour it.
    #[serde(default)]
    pub hdr_supported: bool,
    /// Advanced-color state per display, so "HDR enabled" can say *which*
    /// monitor it means instead of an aggregate true/false.
    #[serde(default)]
    pub hdr_displays: Vec<HdrDisplayState>,
    pub refresh_rate_hz: u32,
    /// All Hz values supported by the primary display at its current resolution.
    pub available_refresh_rates: Vec<u32>,
    /// True when the user has selected the max available refresh rate.
    pub dynamic_refresh_rate_capable: bool,
    /// Intel PSR2 DRRS (Panel Self Refresh 2 Display Refresh Rate Switching).
    /// Controlled via the Intel Arc driver registry key Psr2DrrsEnable.
    pub adaptive_refresh_rate: bool,
    /// True when the PSR2 DRRS value was written during the current boot, i.e.
    /// the registry holds the new intent but the driver has not reloaded it.
    /// There is no API to observe DRRS, so this is the only honest state.
    #[serde(default)]
    pub adaptive_refresh_rate_pending_reboot: bool,
    pub ai_brightness: bool,
    pub ai_brightness_config: AiBrightnessConfig,
    /// Current ambient illuminance from the light sensor (lux). None when unavailable.
    pub ambient_lux: Option<f32>,
    /// Sensor diagnosis (available / no_sensor / frozen_report / unknown) so
    /// the UI can say "no sensor on this device" or "present but not
    /// reporting (last reading D Mon)" instead of a generic unavailable.
    #[serde(default)]
    pub ambient_sensor: AmbientSensorInfo,
    /// Which backend actually writes brightness: "igcl", "wmi-fallback" or
    /// "unknown" (P10). When it is "wmi-fallback" the UI can explain that
    /// hardware brightness control is unavailable on this driver.
    #[serde(default)]
    pub brightness_source: String,
}

const IGCL_DLL: &str = r"C:\Windows\System32\ControlLib.dll";
const AI_BRIGHTNESS_REG_KEY: &str = r"SOFTWARE\MI\DisplaySettings";
const AI_BRIGHTNESS_REG_VALUE: &str = "AiAdaptiveBrightness";
const AI_BRIGHTNESS_MIN_VALUE: &str = "AiBrightnessMin";
const AI_BRIGHTNESS_MAX_VALUE: &str = "AiBrightnessMax";
const AI_BRIGHTNESS_SENS_VALUE: &str = "AiBrightnessSensitivity";
const AI_BRIGHTNESS_SMTH_VALUE: &str = "AiBrightnessSmoothing";
/// 0 = "curve" (fixed min/max formula), 1 = "smart" (learned user preference curve).
const AI_BRIGHTNESS_MODE_VALUE: &str = "AiBrightnessMode";

/// Smart-mode persistence: `%LOCALAPPDATA%\MiControl\state\smart_model.json`
/// through the `smart_model` state store (schema 1, user scope). The path is
/// resolved by the store registry; tests redirect the whole state root via
/// `MICONTROL_STATE_ROOT`, and the legacy `smart_brightness.json` (still read
/// once for migration) can be pointed anywhere via `MICONTROL_SMART_MODEL_PATH`
/// so unit tests never clobber the user's real learned model.
fn smart_model_store() -> Result<state_store::Store<SmartModel>, state_store::StoreError> {
    Ok(state_store::Store::new(state_store::smart_model_spec()?))
}

/// Sensitivity / range configuration for our own adaptive brightness loop.
///
/// Formula per iteration (every 2 s):
///   max_lux  = 2000 / (sensitivity / 100)   — lux where ceiling is reached
///   target   = clamp(min + (lux / max_lux) * (max - min), min, max)
///   smoothed = current + (target - current) * (1 - smoothing/100)
///
/// Mode:
///   "curve" — always use the fixed formula above.
///   "smart" — use a learned user-preference curve (mean per lux bucket with
///             std-dev band) where enough samples exist; fall back to the
///             formula on buckets without data.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AiBrightnessConfig {
    /// Whether our adaptive loop should run.
    pub enabled: bool,
    /// Floor: brightness will never drop below this % (5-80, default 10).
    pub min_brightness: u8,
    /// Ceiling: brightness will never exceed this % (20-100, default 100).
    pub max_brightness: u8,
    /// Reactivity: 100 = full range at 2000 lux, 200 = at 1000 lux (more), 50 = at 4000 lux (less).
    pub sensitivity: u8,
    /// Transition smoothing 0-90: 0 = instant, 30 = default (fast), 90 = very gradual.
    pub smoothing: u8,
    /// "curve" (fixed formula) or "smart" (learned user-preference curve).
    pub mode: String,
}

impl Default for AiBrightnessConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            min_brightness: 10,
            max_brightness: 100,
            sensitivity: 100,
            smoothing: 30,
            mode: "curve".into(),
        }
    }
}

// ── User-override offset for the adaptive loop ────────────────────────────────
//
// When the user manually adjusts brightness while auto-brightness is active,
// we compute the delta between their chosen value and what the loop would have
// produced at the current lux level.  This offset is added to every future
// loop iteration so the curve shifts to match the user's preference without
// disabling automation entirely.
//
// The offset is:
//   • stored as a signed integer
//   • bounded by its ORIGIN — ±100 pp for an explicit user action
//     (MAX_USER_OFFSET_PP), ±30 pp for an offset captured from the loop
//     (MAX_CAPTURED_OFFSET_PP)
//   • applied before the final clamp(min, max) — the final target is the only
//     thing the backlight sees clamped
//   • reset whenever the user disables auto-brightness or changes its config

/// Last lux-based target (before offset) stored so we can compute the delta.
static AUTO_LAST_TARGET: AtomicU8 = AtomicU8::new(50);
/// Signed offset (percentage points) to add to the loop's raw target.
static AUTO_OFFSET: AtomicI16 = AtomicI16::new(0);
/// Whether the offset was explicitly set by the user (false = use 0).
static AUTO_OFFSET_ACTIVE: AtomicBool = AtomicBool::new(false);
/// Origin of the current `AUTO_OFFSET` (`0` = captured, `1` = user intent).
/// Kept separate so `AUTO_OFFSET` remains the single source of the value; the
/// origin only decides which bound applies to it.
static AUTO_OFFSET_ORIGIN: AtomicU8 = AtomicU8::new(0);

/// Hard limit on an offset the **user** asked for (slider, OSD, hotkey,
/// Windows slider), in percentage points.
///
/// The offset is `user_value - raw_curve_target`, so asking for 100 % while the
/// curve sits at 26 % needs **+74 pp**. Bounding this at ±30 pp (the previous
/// `MAX_OVERRIDE_OFFSET_PP`) capped the reachable brightness at `raw + 30` —
/// 56 % at 29 lux — and made the top 44 % of the slider decorative: the value
/// was silently trimmed and the loop reverted it once the grace window
/// expired. Production log: `brightness offset +74 pp out of range — clamped
/// to +30 pp`, `target=56.1`, panel at 57 %.
///
/// The user's intent is absolute, so this bound is the whole 0–100 scale
/// (±100 pp). The *final* target is still clamped to
/// `[min_brightness, max_brightness]` by the loop ([`effective_target`]) — that
/// clamp is the only one the backlight is allowed to see.
const MAX_USER_OFFSET_PP: i16 = 100;

/// Hard limit on offsets **captured automatically** from the loop, in
/// percentage points.
///
/// The loop treats any ≥2 pp difference between what it last wrote and what the
/// panel reports as an external change (Fn key, Windows slider, another app)
/// and shifts the curve to match. That path is a feedback loop: every capture
/// is measured *after* the previous offset was applied, so an unbounded offset
/// ratchets upwards until the curve is pinned at max (production reached
/// `offset: 54`, which is why a clamp was introduced). ±30 pp is wide enough
/// for a genuine preference shift and small enough that a capture bug or a
/// corrupted state file cannot saturate the backlight.
const MAX_CAPTURED_OFFSET_PP: i16 = 30;

/// Where a stored offset came from. The two origins have different bounds (see
/// [`MAX_USER_OFFSET_PP`] / [`MAX_CAPTURED_OFFSET_PP`]) and must survive a
/// crash/relaunch, so the origin is persisted alongside the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
enum OffsetOrigin {
    /// Captured automatically by the adaptive loop (`diff >= 2` path).
    ///
    /// Also the default for state files written before the split: every offset
    /// those builds persisted came from the capture path, so treating them as
    /// captured keeps the old anti-runaway guard.
    #[default]
    Captured,
    /// Derived from an explicit user action (slider / OSD / hotkey).
    User,
}

impl OffsetOrigin {
    /// Bound to apply to an offset of this origin, in percentage points.
    fn limit(self) -> i16 {
        match self {
            OffsetOrigin::User => MAX_USER_OFFSET_PP,
            OffsetOrigin::Captured => MAX_CAPTURED_OFFSET_PP,
        }
    }

    fn label(self) -> &'static str {
        match self {
            OffsetOrigin::User => "user",
            OffsetOrigin::Captured => "captured",
        }
    }

    fn as_u8(self) -> u8 {
        match self {
            OffsetOrigin::User => 1,
            OffsetOrigin::Captured => 0,
        }
    }

    fn from_u8(value: u8) -> Self {
        if value == 1 {
            OffsetOrigin::User
        } else {
            OffsetOrigin::Captured
        }
    }
}

/// Clamp an offset to ±`limit`, logging any trim so a bad input (corrupt file,
/// capture feedback) shows up in the log instead of being silently absorbed.
fn clamp_offset(offset: i16, limit: i16, origin: &str) -> i16 {
    let clamped = offset.clamp(-limit, limit);
    if clamped != offset {
        log::warn!(
            "[display] {origin} brightness offset {offset:+} pp out of range — clamped to \
             {clamped:+} pp (limit ±{limit})"
        );
    }
    clamped
}

/// Clamp an explicit user-intent offset (full 0–100 scale, ±100 pp).
fn clamp_user_offset(offset: i16) -> i16 {
    clamp_offset(offset, MAX_USER_OFFSET_PP, OffsetOrigin::User.label())
}

/// Clamp an offset captured automatically by the loop (±30 pp).
fn clamp_captured_offset(offset: i16) -> i16 {
    clamp_offset(
        offset,
        MAX_CAPTURED_OFFSET_PP,
        OffsetOrigin::Captured.label(),
    )
}

/// Clamp an offset according to its recorded origin.
fn clamp_offset_for(offset: i16, origin: OffsetOrigin) -> i16 {
    clamp_offset(offset, origin.limit(), origin.label())
}

/// The brightness the loop should drive to: the raw curve target plus the
/// override offset, clamped to the configured range.
///
/// Kept pure and separate from the loop so the "user intent reaches 100 %"
/// guarantee is unit-testable without touching the backlight.
fn effective_target(raw: f32, offset: f32, min: u8, max: u8) -> f32 {
    (raw + offset).clamp(min as f32, max as f32)
}

/// How many percentage points an observed brightness may differ from the value
/// the current user offset already explains before the loop treats it as a new
/// preference rather than its own write coming back.
///
/// The loop writes the target and reads it back one iteration later; a
/// smoothing step, driver quantisation or a slightly stale WMI read can all
/// move the readback by a point or two without any user action.
const OFFSET_ECHO_TOLERANCE_PP: i16 = 3;

/// What an observed brightness change means for the curve offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CaptureOutcome {
    /// Offset to apply to the curve.
    offset: i16,
    /// Bound to store it under.
    origin: OffsetOrigin,
    /// True when the observation was our own output (keep the curve as-is).
    echo: bool,
}

/// Decide what an observed brightness change (`diff >= 2` against the value the
/// loop last wrote) means, given the override currently in force.
///
/// Two very different things can produce that difference:
///   • the user acted (slider, Fn key, Windows slider) — a new preference; or
///   • the loop's own write came back a step late, quantised, or mid-smoothing.
///
/// While an explicit **user** offset is in force (±100 pp), re-deriving it with
/// the capture bound would silently undo the top of the slider: at 29 lux the
/// curve gives 26 %, the user picks 100 % (+74 pp), the next iteration reads
/// 100 %, and `clamp_captured(100 − 26 = 74)` = +30 → 56 %. That is exactly the
/// regression being fixed, so a user offset is classified with the user bound
/// and an observation that reproduces it is ignored (echo). A value that does
/// not reproduce it is a genuinely new preference and is recorded as user
/// intent, still at full scale.
///
/// With no user offset in force, the old feedback rule applies unchanged: the
/// capture path keeps ±30 pp so repeated captures cannot ratchet the curve to
/// max (production reached `offset: 54` that way).
fn classify_capture(
    observed: u8,
    raw_target: u8,
    current_offset: i16,
    origin: OffsetOrigin,
    active: bool,
) -> CaptureOutcome {
    let candidate = observed as i16 - raw_target as i16;
    if active && origin == OffsetOrigin::User {
        let bounded = clamp_user_offset(candidate);
        if (bounded - current_offset).abs() <= OFFSET_ECHO_TOLERANCE_PP {
            // Our own write, or a rounding/smoothing step: leave the curve.
            CaptureOutcome {
                offset: current_offset,
                origin: OffsetOrigin::User,
                echo: true,
            }
        } else {
            CaptureOutcome {
                offset: bounded,
                origin: OffsetOrigin::User,
                echo: false,
            }
        }
    } else {
        CaptureOutcome {
            offset: clamp_captured_offset(candidate),
            origin: OffsetOrigin::Captured,
            echo: false,
        }
    }
}

// ── Manual-override grace period ─────────────────────────────────────────────
//
// S32-005: When the user manually changes brightness (MiControl slider, OSD,
// hotkey, Windows slider, Fn key) and a crash/relaunch happens shortly after,
// the relaunched process must NOT immediately revert the user's brightness.
// Before the last manual change is older than MANUAL_OVERRIDE_GRACE_MS, the
// adaptive loop is frozen at the user's value. This also protects against the
// "app recovers brightness instantly after I adjusted it" complaint.

/// Milliseconds the adaptive loop must stay hands-off after a manual change.
const MANUAL_OVERRIDE_GRACE_MS: u64 = 120_000; // 2 minutes
/// Wall-clock (ms since UNIX epoch) of the last manual brightness change.
static AUTO_LAST_MANUAL_MS: AtomicU64 = AtomicU64::new(0);

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// True while the user's last manual brightness change is still within the
/// grace window — the adaptive loop must not touch the backlight.
pub fn user_override_in_grace() -> bool {
    let last = AUTO_LAST_MANUAL_MS.load(Ordering::Relaxed);
    last != 0 && now_ms().saturating_sub(last) < MANUAL_OVERRIDE_GRACE_MS
}

/// Called by the `set_brightness` Tauri command when auto-brightness is on.
/// Records the delta so future loop iterations honour the user's preference,
/// and stamps the manual-change time to enter the grace window.
///
/// Also records a (lux, brightness) sample into the smart model, so the
/// "Smart" mode can learn the user's preferred brightness per light level.
pub fn record_user_brightness_override(user_value: u8) {
    let last_target = AUTO_LAST_TARGET.load(Ordering::Relaxed);
    // Explicit user intent: the offset may be the whole scale (±100 pp) so the
    // full 10–100 slider is reachable whatever the curve is doing. Only the
    // final target is clamped (to min/max) by the loop.
    let offset = clamp_user_offset(user_value as i16 - last_target as i16);
    AUTO_OFFSET.store(offset, Ordering::Relaxed);
    AUTO_OFFSET_ORIGIN.store(OffsetOrigin::User.as_u8(), Ordering::Relaxed);
    AUTO_OFFSET_ACTIVE.store(true, Ordering::Relaxed);
    AUTO_LAST_MANUAL_MS.store(now_ms(), Ordering::Relaxed);
    // Persist so a crash/relaunch right after the manual change does not
    // forget it (the relaunched loop would steal the brightness back).
    persist_override_state();
    log::debug!(
        "auto_brightness: user override {user_value}% \
         (last_target={last_target}%, offset={offset:+}) — grace {MANUAL_OVERRIDE_GRACE_MS}ms"
    );

    // Smart-mode learning: log the manual brightness change with the current
    // ambient lux (if the sensor is available).
    if let Some(lux) = get_ambient_lux().filter(|v| *v > MIN_PLAUSIBLE_LUX) {
        record_smart_sample(lux, user_value);
    }
}

/// Reset the offset — call when auto-brightness is toggled or config changes.
pub fn clear_user_brightness_override() {
    AUTO_OFFSET.store(0, Ordering::Relaxed);
    AUTO_OFFSET_ORIGIN.store(OffsetOrigin::Captured.as_u8(), Ordering::Relaxed);
    AUTO_OFFSET_ACTIVE.store(false, Ordering::Relaxed);
    AUTO_LAST_MANUAL_MS.store(0, Ordering::Relaxed);
    persist_override_state();
}

// ── Manual-override persistence (crash/relaunch survival) ────────────────────
//
// The adaptive loop's manual-override state (offset + last manual change) is
// process-local. After a crash (WebView2 BEX64 → watchdog relaunch), a brand
// new process starts with AUTO_LAST_MANUAL_MS=0 and NO offset — the loop
// immediately sees the current brightness (which the user just set manually)
// as its baseline via seeding, BUT it keeps recomputing the curve and writing
// over the user's value 2 s later (the "app steals my brightness" regression).
//
// Fix: persist {last_manual_ms (epoch abs), offset, offset_active} through
// the brightness state store (C3: `util::state_store`) on every manual change
// and restore it when the adaptive loop starts in a new process. As long as
// the user's manual change happened < MANUAL_OVERRIDE_GRACE_MS ago, the
// relaunched loop stays hands-off. The store's declared migration imports the
// legacy `brightness_override.json` (kept until the first store save, which
// retires it), so the migration here is the same code the store tests cover.

/// Persist the current manual-override statics through the brightness store
/// (crash survival). Store failures downgrade to the same warn the direct
/// write produced; the in-memory statics remain authoritative.
fn persist_override_state() {
    let state = state_store::BrightnessOverrideState {
        last_manual_ms: AUTO_LAST_MANUAL_MS.load(Ordering::Relaxed),
        offset: AUTO_OFFSET.load(Ordering::Relaxed),
        offset_active: AUTO_OFFSET_ACTIVE.load(Ordering::Relaxed),
        origin: match OffsetOrigin::from_u8(AUTO_OFFSET_ORIGIN.load(Ordering::Relaxed)) {
            OffsetOrigin::Captured => state_store::BrightnessOffsetOrigin::Captured,
            OffsetOrigin::User => state_store::BrightnessOffsetOrigin::User,
        },
    };
    if let Err(e) = state_store::save_brightness_override(&state) {
        log::warn!("[display] Failed to persist brightness override: {e}");
    }
}

/// Restore the manual-override statics (new process after crash). Stale
/// entries (> grace window) restore nothing so a fresh boot never gets stuck
/// in a perpetual hands-off state. Corrupt state or store IO failure logs and
/// starts clean; it never panics and never leaves the statics half-set.
///
/// On the first call after the C3 migration this also CONSUMES the legacy
/// `brightness_override.json` (import into the store), and the next
/// [`persist_override_state`] save retires the legacy file.
pub fn restore_override_state() {
    // One-time legacy import. Importing before load means the first restore
    // already sees the migrated payload; the legacy file is only removed by
    // the next successful save (store contract), never here.
    if let Err(e) = state_store::migrate_legacy_brightness_override() {
        log::warn!("[display] brightness override legacy migration skipped: {e}");
    }
    let loaded = match state_store::brightness_store().and_then(|store| store.load()) {
        Ok(loaded) => loaded,
        Err(e) => {
            // Corrupt state or IO failure: start clean rather than wedging
            // the adaptive loop (a bad state file must not take brightness
            // control down with it).
            log::warn!("[display] brightness override state unreadable, starting clean: {e}");
            return;
        }
    };
    let Some(state) = loaded else { return };
    if state.last_manual_ms == 0 {
        return;
    }
    let age = now_ms().saturating_sub(state.last_manual_ms);
    if age >= MANUAL_OVERRIDE_GRACE_MS {
        // Manual change too old to matter — start clean.
        log::debug!("[adaptive_brightness] persisted override too old ({age}ms) — ignoring");
        return;
    }
    let origin = OffsetOrigin::from_u8(state.origin.as_u8());
    AUTO_LAST_MANUAL_MS.store(state.last_manual_ms, Ordering::Relaxed);
    // Clamp on restore with the bound of the offset's ORIGIN: a user-intent
    // offset may legitimately be ±100 pp (the slider's full range), while a
    // captured offset stays at ±30 pp. State files written before the split
    // carry no origin and came from the capture path → ±30 pp, exactly as the
    // old build did. Never apply an offset unclamped: the historical `54` in
    // production came from the capture feedback loop and pinned the curve.
    let offset = clamp_offset_for(state.offset, origin);
    AUTO_OFFSET.store(offset, Ordering::Relaxed);
    AUTO_OFFSET_ORIGIN.store(origin.as_u8(), Ordering::Relaxed);
    AUTO_OFFSET_ACTIVE.store(state.offset_active, Ordering::Relaxed);
    log::info!(
        "[adaptive_brightness] restored manual override from crash \
         (age={age}ms, offset={}, origin={}, active={}) — grace window until manual change",
        offset,
        origin.label(),
        state.offset_active
    );
}

/// Read the current display brightness from WMI (ground truth) or IGCL.
/// WmiMonitorBrightness.CurrentBrightness is what Windows Display Settings
/// reads, so it is the correct source for "what Windows thinks the brightness is".
fn read_current_brightness() -> Option<u8> {
    get_brightness_wmi().or_else(|_| get_brightness_igcl()).ok()
}

/// Lightweight brightness read (no full DisplayInfo) for the gesture thread.
pub fn current_brightness() -> u8 {
    read_current_brightness().unwrap_or(80)
}

pub fn get_display_info() -> HardwareResult<DisplayInfo> {
    // WMI brightness = what Windows Display Settings slider shows (ground truth).
    let brightness = get_brightness_wmi().unwrap_or_else(|_| get_brightness_igcl().unwrap_or(80));
    // P10: make sure `brightness_source` reflects the LIVE backend instead of
    // the value persisted by the previous session.
    probe_brightness_source_once();
    let hdr_displays = hdr_displays();
    let hdr_enabled = hdr_displays.iter().any(|d| d.enabled);
    let hdr_supported = hdr_displays.iter().any(|d| d.supported);
    // One enumeration, one display: `refresh_rate_hz` and
    // `available_refresh_rates` must describe the SAME device, otherwise the
    // UI shows a current rate from one monitor and chips from another (the
    // audit's "120 Hz" over a [50, 85] list, where no chip was active).
    let (refresh_rate_hz, available_refresh_rates) = match refresh_rate_snapshot() {
        Ok(snapshot) => snapshot,
        Err(e) => {
            // S25-009: Don't let refresh-rate enumeration failure nuke the
            // entire display info — fall back to the (GPU-level) WMI rate.
            log::warn!(
                target: "hw::display",
                "refresh-rate enumeration failed: {e}, falling back to the WMI controller rate"
            );
            let hz = get_refresh_rate_wmi().unwrap_or(120);
            (hz, vec![hz])
        }
    };
    // DRR is active when the display is set to its highest supported refresh rate.
    let dynamic_refresh_rate_capable = available_refresh_rates
        .last()
        .map(|&max| max == refresh_rate_hz)
        .unwrap_or(false);
    let adaptive_refresh_rate = get_intel_drrs();
    let ai_brightness_config = get_ai_brightness_config();
    let ai_brightness = ai_brightness_config.enabled;
    let ambient_lux = get_ambient_lux().filter(|&v| v > 0.5);
    Ok(DisplayInfo {
        brightness,
        hdr_enabled,
        hdr_supported,
        hdr_displays,
        refresh_rate_hz,
        available_refresh_rates,
        dynamic_refresh_rate_capable,
        adaptive_refresh_rate,
        adaptive_refresh_rate_pending_reboot: drrs_pending_reboot_now(),
        ai_brightness,
        ai_brightness_config,
        ambient_lux,
        ambient_sensor: ambient_sensor_info(),
        brightness_source: brightness_source().to_string(),
    })
}

pub fn set_brightness(level: u8) -> HardwareResult<()> {
    let level = level.clamp(10, 100);
    // P10: do not disable IGCL for the whole process on the first failure.
    // Retry it slowly (IGCL_RETRY_SECS) so a driver re-enumeration can restore
    // hardware brightness without restarting the app.
    if IGCL_SET_AVAILABLE.load(Ordering::Relaxed) || igcl_retry_due() {
        match set_brightness_igcl(level) {
            Ok(()) => {
                report_igcl_available("IGCL brightness write succeeded");
            }
            Err(e) => {
                IGCL_SET_AVAILABLE.store(false, Ordering::Relaxed);
                // One clear transition line (set_brightness_source logs only
                // when the source actually changes) instead of a WARN per write.
                set_brightness_source(
                    SOURCE_WMI_FALLBACK,
                    &format!("IGCL brightness write failed: {e}"),
                );
                schedule_igcl_retry();
                set_brightness_wmi(level)?;
            }
        }
    } else {
        set_brightness_wmi(level)?;
    }
    Ok(())
}

pub fn set_hdr(enabled: bool) -> HardwareResult<()> {
    set_hdr_ccd(enabled)
}

pub fn set_ai_brightness(enabled: bool) -> HardwareResult<()> {
    // Toggle the enabled flag while preserving all other config values.
    let mut cfg = get_ai_brightness_config();
    cfg.enabled = enabled;
    set_ai_brightness_config(cfg).map_err(|e| {
        log::error!("[display] set_ai_brightness_config failed: {e}");
        e
    })?;
    if enabled {
        // Windows has its own ALS-based adaptive brightness (ADAPTBRIGHT power plan setting).
        // If both are active they fight over the same backlight knob, causing the 90% cap
        // symptom. Disable Windows adaptive brightness while our loop is in charge.
        disable_windows_adaptive_brightness();
    }
    log::info!(
        "[display] AI brightness {}",
        if enabled { "enabled" } else { "disabled" }
    );
    Ok(())
}

// ── Adaptive brightness config ────────────────────────────────────────────────

fn read_display_dword(name: &str, default: u32) -> u32 {
    #[cfg(windows)]
    {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
        if let Ok(Some(key)) = RegKeyGuard::open_read(HKEY_LOCAL_MACHINE, AI_BRIGHTNESS_REG_KEY) {
            if let Ok(Some(v)) = key.read_u32(name) {
                return v;
            }
        }
    }
    default
}

fn write_display_dword(name: &str, value: u32) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
        let key = RegKeyGuard::create_write(HKEY_LOCAL_MACHINE, AI_BRIGHTNESS_REG_KEY)
            .map_err(|e| HardwareError::Registry(format!("create display settings key: {e}")))?;
        key.write_u32(name, value)
            .map_err(HardwareError::Registry)?;
    }
    Ok(())
}

// ── Curve/smart mode persistence (state store, registry imported once) ────────
//
// The mode used to live ONLY in the HKLM `AiBrightnessMode` value, which a
// non-elevated user process cannot write: the choice silently reverted on
// machines where the write was refused. The `brightness_mode` state store is
// the source of truth now; the HKLM value is imported once (read-only) on the
// first load and mirrored best-effort on every change (external tools may
// still read it), never the other way around.

/// In-process cache of the persisted mode: `get_ai_brightness_config` runs
/// every 2 s in the adaptive loop, and each read would otherwise re-open the
/// state file. `None` = not loaded yet.
static BRIGHTNESS_MODE_CACHE: std::sync::OnceLock<std::sync::Mutex<Option<String>>> =
    std::sync::OnceLock::new();

/// Registry DWORD → mode string (pure, unit-tested).
fn mode_from_registry_dword(value: u32) -> &'static str {
    if value == 1 {
        "smart"
    } else {
        "curve"
    }
}

/// Load the persisted mode, running the one-time HKLM import when the store
/// is empty. Store failures degrade to the default ("curve") and never wedge
/// the adaptive loop.
fn load_brightness_mode() -> String {
    let store = match state_store::brightness_mode_store() {
        Ok(store) => store,
        Err(e) => {
            log::warn!("[display] brightness mode store unavailable: {e}");
            return "curve".to_string();
        }
    };
    match store.load() {
        Ok(Some(state)) => state.mode,
        Ok(None) => {
            // One-time import: whatever the HKLM value says (0 curve / 1
            // smart); a missing value means the user never chose, so the
            // default curve is imported and becomes the persisted baseline.
            let mode = mode_from_registry_dword(read_display_dword(AI_BRIGHTNESS_MODE_VALUE, 0));
            let state = state_store::BrightnessModeState {
                mode: mode.to_string(),
                updated_at_ms: now_ms(),
            };
            if let Err(e) = store.save(&state) {
                log::warn!("[display] brightness mode import save failed: {e}");
            } else {
                log::info!(
                    "[display] imported brightness mode '{mode}' from the registry into the state store"
                );
            }
            mode.to_string()
        }
        Err(e) => {
            log::warn!("[display] brightness mode state unreadable, using default: {e}");
            "curve".to_string()
        }
    }
}

/// Cached persisted mode ("curve" | "smart"). Test helper
/// [`reset_brightness_mode_cache_for_tests`] simulates a restart by clearing it.
fn brightness_mode_cached() -> String {
    let cache = BRIGHTNESS_MODE_CACHE.get_or_init(|| std::sync::Mutex::new(None));
    let mut guard = crate::util::panic::lock_or_recover(cache);
    if guard.is_none() {
        *guard = Some(load_brightness_mode());
    }
    guard
        .as_ref()
        .expect("brightness mode cache initialised")
        .clone()
}

/// Persist the mode through the store and refresh the in-process cache.
fn save_brightness_mode(mode: &str) -> HardwareResult<()> {
    let state = state_store::BrightnessModeState {
        mode: mode.to_string(),
        updated_at_ms: now_ms(),
    };
    let store = state_store::brightness_mode_store()
        .map_err(|e| HardwareError::Registry(format!("brightness mode store unavailable: {e}")))?;
    store
        .save(&state)
        .map_err(|e| HardwareError::Registry(format!("brightness mode save failed: {e}")))?;
    let cache = BRIGHTNESS_MODE_CACHE.get_or_init(|| std::sync::Mutex::new(None));
    *crate::util::panic::lock_or_recover(cache) = Some(mode.to_string());
    log::info!("[display] brightness mode persisted: {mode}");
    Ok(())
}

/// Clear the in-process mode cache (tests use this to simulate a restart:
/// the next read must come from the persisted store again).
#[cfg(test)]
fn reset_brightness_mode_cache_for_tests() {
    if let Some(cache) = BRIGHTNESS_MODE_CACHE.get() {
        if let Ok(mut guard) = cache.lock() {
            *guard = None;
        }
    }
}

pub fn get_ai_brightness_config() -> AiBrightnessConfig {
    let enabled = get_ai_brightness_registry().unwrap_or(false);
    let min_b = (read_display_dword(AI_BRIGHTNESS_MIN_VALUE, 10) as u8).clamp(5, 80);
    let max_b = (read_display_dword(AI_BRIGHTNESS_MAX_VALUE, 100) as u8).clamp(min_b + 5, 100);
    let mode = brightness_mode_cached();
    AiBrightnessConfig {
        enabled,
        min_brightness: min_b,
        max_brightness: max_b,
        sensitivity: (read_display_dword(AI_BRIGHTNESS_SENS_VALUE, 100) as u8).clamp(10, 200),
        smoothing: (read_display_dword(AI_BRIGHTNESS_SMTH_VALUE, 30) as u8).min(90),
        mode,
    }
}

pub fn set_ai_brightness_config(config: AiBrightnessConfig) -> HardwareResult<()> {
    // Mode FIRST, through the user-writable state store: every later HKLM
    // write (the enabled flag, the mirror, min/max/sens/smoothing) can be
    // refused for a non-elevated process, and the old order silently dropped
    // the user's Smart/Formula choice exactly on those machines. The mode
    // save must not depend on any registry write succeeding.
    let mode = if config.mode == "smart" {
        "smart"
    } else {
        "curve"
    };
    save_brightness_mode(mode)?;
    persist_ai_brightness_registry(config.enabled)?;
    // Best-effort HKLM mirror for external tooling; a refusal is logged at
    // DEBUG and never fails the save.
    let mirror = write_display_dword(
        AI_BRIGHTNESS_MODE_VALUE,
        if config.mode == "smart" { 1 } else { 0 },
    );
    if let Err(e) = mirror {
        log::debug!("[display] brightness mode registry mirror skipped (best-effort): {e}");
    }
    write_display_dword(AI_BRIGHTNESS_MIN_VALUE, config.min_brightness as u32)?;
    write_display_dword(AI_BRIGHTNESS_MAX_VALUE, config.max_brightness as u32)?;
    write_display_dword(AI_BRIGHTNESS_SENS_VALUE, config.sensitivity as u32)?;
    write_display_dword(AI_BRIGHTNESS_SMTH_VALUE, config.smoothing as u32)?;
    Ok(())
}

// ── Smart brightness model (learned user preference) ─────────────────────────
//
// The "Smart" mode learns the user's brightness preference per ambient-light
// level and uses it to drive the adaptive loop instead of the fixed
// min/max formula.  Each manual brightness change (slider, hotkey, Fn key,
// external Windows change) is logged as a (lux, brightness) sample; the
// samples are aggregated into lux buckets and the loop uses the *mean*
// brightness inside the bucket, with a *standard-deviation band* used to
// flag outlier samples that are likely accidental (brightness changes made
// for other reasons, e.g. plugging in HDMI, sunny window).
//
// Persistence format (`smart_brightness.json`):
// {
//   "schema": 1,
//   "bins": {
//     "0":   { "n": 3, "mean": 42.0, "m2": 100.0, "min": 30, "max": 55 },  // lux range [0,20)
//     "20":  { "n": 5, "mean": 60.0, "m2": 200.0, "min": 45, "max": 80 }, // lux range [20,40)
//     ...
//   }
// }
//
// Bin size is 20 lux: bin index = floor(lux / 20), keyed by index * 20.
// `m2` is the sum of squared deviations (Welford's online mean/variance
// algorithm) so we can compute the std-dev without storing every sample.

/// Lux bucket width for the smart model (10 lux per bucket keeps memory tiny
/// while still capturing meaningful per-light-level preferences).
const SMART_BIN_LUX: f32 = 10.0;
/// Minimum number of samples in a bucket before the smart model is trusted
/// over the raw formula.  Fewer than this → use the standard formula.
const SMART_MIN_SAMPLES: u32 = 3;
/// Maximum number of buckets cap — protects the file from unbounded growth
/// if the sensor reports bizarre lux values.
const SMART_MAX_BUCKETS: usize = 200;
/// Upper bound on the **total** number of samples kept in
/// `smart_brightness.json`.
///
/// The audit found 1_315_938 samples — 30+ days of the adaptive loop writing a
/// sample every 2 s (30/min). That is not user preference, it is the loop
/// learning from its own output. A sane rolling window is 7 days:
/// 7 × 24 h × 60 min × 30 samples/min = 302_400 samples. When the loaded model
/// exceeds it, older mass is decayed (see [`prune_smart_model`]).
const SMART_MAX_TOTAL_SAMPLES: u32 = 302_400;
/// Upper bound per lux bucket. A single hot bucket (e.g. a desk lamp holding
/// the sensor at 4 lux) otherwise accumulates hundreds of thousands of samples
/// on its own — the audit's bin 4 lux had 283_355.
const SMART_MAX_SAMPLES_PER_BIN: u32 = 20_000;

/// One lux bucket in the smart brightness model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartBin {
    /// Number of samples in this bucket.
    pub n: u32,
    /// Running mean (Welford).
    pub mean: f32,
    /// Running M2 (sum of squared deviations from the mean).
    pub m2: f32,
    /// Minimum brightness observed in this bucket.
    pub min: u8,
    /// Maximum brightness observed in this bucket.
    pub max: u8,
}

/// The full smart brightness model (serialized to `smart_brightness.json`).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SmartModel {
    /// Schema version for future migrations.
    pub schema: u32,
    /// Map of bucket index → stats. The bucket index is `floor(lux / SMART_BIN_LUX)`.
    pub bins: std::collections::HashMap<u32, SmartBin>,
}

impl SmartBin {
    /// Halve the sample count while preserving the mean and the variance.
    ///
    /// `m2` is a *sum* of squared deviations, so it scales linearly with `n`:
    /// halving both keeps `m2 / (n - 1)` — the variance the UI shows — intact.
    /// This is the only way to "forget" old samples in an aggregate-only model:
    /// Welford's statistics carry no timestamps, so a time window cannot be
    /// applied retroactively. Exponential halving weights recent samples more
    /// than old ones, which is what a rolling window approximates.
    fn decay(&mut self) {
        self.n = (self.n / 2).max(SMART_MIN_SAMPLES);
        self.m2 *= 0.5;
    }
}

impl SmartModel {
    fn new() -> Self {
        Self {
            schema: 1,
            bins: std::collections::HashMap::new(),
        }
    }

    /// Number of buckets with at least `SMART_MIN_SAMPLES`.
    fn mature_bucket_count(&self) -> usize {
        self.bins
            .values()
            .filter(|b| b.n >= SMART_MIN_SAMPLES)
            .count()
    }

    /// Total number of samples across all buckets.
    fn total_samples(&self) -> u32 {
        self.bins.values().map(|b| b.n).sum()
    }
}

/// Trim a loaded model back inside the retention bounds.
///
/// Returns `true` when anything changed, so the caller can rewrite the file
/// once (migration on load) instead of carrying a 1.3 M-sample JSON forever.
///
/// Strategy — exponential decay, not deletion, because the model is
/// aggregate-only (mean/m2, no per-sample timestamps):
/// 1. every bucket above [`SMART_MAX_SAMPLES_PER_BIN`] is halved until it fits
///    (283_355 → 17_709 in four halvings), preserving mean and variance;
/// 2. while the total exceeds [`SMART_MAX_TOTAL_SAMPLES`], the largest bucket is
///    halved — repeated halving is a geometric down-weighting of old mass and
///    keeps the *shape* of the distribution (which buckets are "hot") intact.
///
/// `min`/`max` are historical extremes and are kept: they are informational
/// (the stats panel shows the observed range) and cannot be recomputed without
/// the dropped samples.
fn prune_smart_model(model: &mut SmartModel) -> bool {
    let before = model.total_samples();
    let mut changed = false;

    for bin in model.bins.values_mut() {
        while bin.n > SMART_MAX_SAMPLES_PER_BIN {
            bin.decay();
            changed = true;
        }
    }

    while model.total_samples() > SMART_MAX_TOTAL_SAMPLES {
        let hottest = model.bins.values().map(|b| b.n).max().unwrap_or(0);
        if hottest <= SMART_MIN_SAMPLES {
            break; // nothing left worth decaying
        }
        match model.bins.iter_mut().max_by_key(|(_, b)| b.n) {
            Some((_, bin)) => {
                bin.decay();
                changed = true;
            }
            None => break,
        }
    }

    if changed {
        log::info!(
            "[display] smart model pruned on load: {before} → {} samples across {} buckets \
             (caps: {SMART_MAX_TOTAL_SAMPLES} total, {SMART_MAX_SAMPLES_PER_BIN} per bucket)",
            model.total_samples(),
            model.bins.len()
        );
    }
    changed
}

/// In-memory cache of the smart model. Loaded once at startup (or on demand)
/// and updated on every sample; persisted on change.
static SMART_MODEL_CACHE: std::sync::OnceLock<std::sync::Mutex<Option<SmartModel>>> =
    std::sync::OnceLock::new();

fn load_smart_model() -> SmartModel {
    let store = match smart_model_store() {
        Ok(store) => store,
        Err(e) => {
            log::warn!("[display] smart model store unavailable, starting empty: {e}");
            return SmartModel::new();
        }
    };
    match store.load() {
        Err(e) => {
            // Corrupt store file: fall back to the default model and keep
            // going. Startup must never die on a state file.
            log::warn!("[display] smart model state unreadable, starting empty: {e}");
            SmartModel::new()
        }
        Ok(Some(mut model)) => {
            // Migration: an unbounded model (1.3 M samples in production) is
            // trimmed on load and written back once, so every subsequent start
            // reads the small version.
            if prune_smart_model(&mut model) {
                persist_smart_model(&model);
            }
            model
        }
        Ok(None) => {
            // One-time legacy import: the pre-store `smart_brightness.json`.
            // Pruned BEFORE the store save so an oversized pre-C3 model still
            // fits the store's declared ceiling; the legacy file is retired
            // only after that save succeeded.
            let legacy_path = state_store::legacy_smart_model_path();
            let Ok(data) = std::fs::read_to_string(&legacy_path) else {
                return SmartModel::new();
            };
            match serde_json::from_str::<SmartModel>(&data) {
                Ok(mut model) => {
                    prune_smart_model(&mut model);
                    if let Err(e) = store.save(&model) {
                        // Keep the pruned model for this session and leave the
                        // legacy file in place (it is still the only copy).
                        log::warn!("[display] smart model legacy import failed: {e}");
                        return model;
                    }
                    state_store::retire_legacy_smart_model();
                    log::info!(
                        "[display] migrated legacy smart_brightness.json into the state store"
                    );
                    model
                }
                Err(e) => {
                    log::warn!(
                        "[display] legacy smart_brightness.json unreadable, starting empty: {e}"
                    );
                    SmartModel::new()
                }
            }
        }
    }
}

fn persist_smart_model(model: &SmartModel) {
    let store = match smart_model_store() {
        Ok(store) => store,
        Err(e) => {
            log::warn!("[display] smart model store unavailable: {e}");
            return;
        }
    };
    if let Err(e) = store.save(model) {
        log::warn!("[display] Failed to persist smart model: {e}");
        return;
    }
    // Retire the legacy file only after a successful store save, so a crash
    // mid-migration always leaves one valid copy behind.
    state_store::retire_legacy_smart_model();
}

/// Access to the smart model (locked).  Loads from disk on first access.
fn with_smart_model<T>(f: impl FnOnce(&mut SmartModel) -> T) -> T {
    let cache = SMART_MODEL_CACHE.get_or_init(|| std::sync::Mutex::new(None));
    let mut guard = crate::util::panic::lock_or_recover(cache);
    if guard.is_none() {
        *guard = Some(load_smart_model());
    }
    let model = guard.as_mut().expect("smart model cache initialized");
    let result = f(model);

    // Opportunistic: only write back when the model changed.  We can't easily
    // detect churn, so write on every public mutation call that uses this fn
    // (record_log_sample / reset_smart_model). Kept simple: callers that only
    // *read* must not go through here (they use `read_smart_model()`).
    result
}

/// Read the current smart model without mutating it (for the frontend stats bar).
pub fn get_smart_model() -> SmartModel {
    let cache = SMART_MODEL_CACHE.get_or_init(|| std::sync::Mutex::new(None));
    let guard = crate::util::panic::lock_or_recover(cache);
    match guard.as_ref() {
        Some(m) => m.clone(),
        None => load_smart_model(),
    }
}

/// Clear the entire smart model (user clicked "Reset learning").
pub fn reset_smart_model() {
    // Purge through the store (data + tmp + lock); the legacy
    // `smart_brightness.json` is gone for good on a reset, so remove it too.
    match smart_model_store() {
        Ok(store) => {
            if let Err(e) = store.purge() {
                log::warn!("[display] smart model purge failed: {e}");
            }
        }
        Err(e) => log::warn!("[display] smart model store unavailable on reset: {e}"),
    }
    let _ = std::fs::remove_file(state_store::legacy_smart_model_path());
    if let Some(cache) = SMART_MODEL_CACHE.get() {
        if let Ok(mut guard) = cache.lock() {
            *guard = Some(SmartModel::new());
        }
    }
    log::info!("[display] Smart brightness model reset");
}

/// Log a (lux, brightness) sample into the smart model and persist it.
///
/// Outliers (values more than 2 std-dev from the bucket mean, when the bucket
/// already has enough data) are still recorded — they shift the mean — but
/// they are logged separately so the stats panel can show "X outlier-adjusted
/// samples".  The loop uses the *mean* which is robust to occasional outliers.
pub fn record_smart_sample(lux: f32, brightness: u8) {
    if !lux.is_finite() || lux < 0.0 {
        return;
    }
    let bin_idx = (lux / SMART_BIN_LUX).floor() as u32;
    if bin_idx as usize > SMART_MAX_BUCKETS {
        return; // absurd lux value — ignore
    }
    with_smart_model(|m| {
        let bin = m.bins.entry(bin_idx).or_insert(SmartBin {
            n: 0,
            mean: 0.0,
            m2: 0.0,
            min: 100,
            max: 0,
        });
        // Welford's online algorithm.
        bin.n += 1;
        let delta = brightness as f32 - bin.mean;
        bin.mean += delta / bin.n as f32;
        let delta2 = brightness as f32 - bin.mean;
        bin.m2 += delta * delta2;
        bin.min = bin.min.min(brightness);
        bin.max = bin.max.max(brightness);
        let _ = bin;
        // Session guard: the load-time migration only trims an oversized
        // model at startup; a long-lived process can still grow past the
        // caps (30 samples/min at the audit's old rate). Prune BEFORE the
        // persist below so the store file never outgrows its ceiling
        // mid-session and the sample count stays a rolling window.
        prune_smart_model(m);
    });
    let model = get_smart_model();
    // Persist outside the lock to avoid holding it during file I/O.
    persist_smart_model(&model);
    log::debug!(
        "[display] Smart sample: lux={lux:.0} brightness={brightness}% \
         → bucket={bin_idx} (total={})",
        model.total_samples()
    );
}

/// Whether a captured brightness value may be fed to the smart model.
///
/// While a user-override offset is active, the loop's own output already
/// contains that offset. A readback that differs from what we set by ≥2 pp is
/// then almost always our own write coming back (driver quantisation, smoothing
/// step, another app echoing the value), not a new user preference. Learning it
/// teaches the model the curve we just applied, and the next iteration adds the
/// offset on top of the learned mean — the self-reinforcing loop that produced
/// 1_315_938 samples and `offset: 54` in production.
fn should_learn_from_capture() -> bool {
    if AUTO_OFFSET_ACTIVE.load(Ordering::Relaxed) {
        log::debug!(
            "[display] smart sample skipped — brightness_override is active, the captured \
             value contains the loop's own offset"
        );
        return false;
    }
    true
}

/// Record a sample captured from an *external* brightness change, unless the
/// captured value is the loop's own overlaid output.
fn record_external_change_sample(lux: f32, brightness: u8) {
    if should_learn_from_capture() {
        record_smart_sample(lux, brightness);
    }
}

/// Compute the std-dev for a bucket (population std-dev using Welford's M2).
/// Used by `smart_model_stats` to report the confidence band in the UI.
pub fn bin_stddev(bin: &SmartBin) -> f32 {
    if bin.n < 2 {
        return 0.0;
    }
    (bin.m2 / bin.n as f32).sqrt()
}

/// Number of smart-bucket samples (for the stats UI).
pub fn smart_model_stats() -> (u32, usize) {
    let model = get_smart_model();
    (model.total_samples(), model.mature_bucket_count())
}

/// Compute the predicted (mean) brightness for a given lux value using the
/// learned model. Returns `None` when the bucket has too few samples or the
/// lux value is outside every known bucket — the caller falls back to the
/// fixed formula in that case.
pub fn smart_predict_brightness(lux: f32) -> Option<u8> {
    if !lux.is_finite() || lux < 0.0 {
        return None;
    }
    let bin_idx = (lux / SMART_BIN_LUX).floor() as u32;
    let model = get_smart_model();
    let bin = model.bins.get(&bin_idx)?;
    if bin.n < SMART_MIN_SAMPLES {
        return None;
    }
    Some(bin.mean.round().clamp(0.0, 100.0) as u8)
}

// ── Ambient light sensor ──────────────────────────────────────────────────────

/// Sensor availability, so the UI says "this device has no ambient light
/// sensor" (permanent) instead of "unavailable" (transient) for a machine
/// that simply has none, and so the adaptive loop stops burning reset
/// requests on a sensor that exists on paper but never delivers data.
///
/// States, classified by [`classify_sensor_probe`] from one fresh probe of
/// the process's committed backend (COM and WinRT are never mixed in one
/// process, see `LuxBackend`):
///   Available    — the backend returned a fresh, plausible reading.
///   NoSensor     — the backend cannot even see a sensor device (no
///                  enumeration / GetDefault() fails). Permanent in practice:
///                  the machine has no ALS hardware or its driver is disabled.
///   FrozenReport — sensor devices enumerate and answer, but no fresh data
///                  arrives (timestamp does not advance, lux stays at the
///                  0-lux placeholder). The hardware exists but the
///                  driver/fusion stack never delivers data; found in
///                  production on the TCS3408 (reports frozen for 4+ days,
///                  surviving device restarts and reset requests). Treating
///                  this as a NEW probe every cycle (the old behaviour) made
///                  the health supervisor and the loop request sensor resets
///                  forever; treating it as a state lets the UI say "sensor
///                  present but not reporting" and lets the loop idle.
///   Unknown      — not probed yet in this process; settled lazily.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AmbientSensorState {
    /// A backend returned a fresh, plausible lux reading.
    Available,
    /// No sensor device visible to the committed backend (no ALS here).
    NoSensor,
    /// Sensor devices enumerate but never deliver a fresh report (frozen
    /// timestamp / permanent 0 lux).
    FrozenReport,
    /// Not probed yet in this process (the honest unset value).
    #[default]
    Unknown,
}

impl AmbientSensorState {
    /// Dense wire code for the process-local atomic (`0` Unknown, `1`
    /// Available, `2` NoSensor, `3` FrozenReport).
    #[must_use]
    pub const fn code(self) -> u8 {
        match self {
            AmbientSensorState::Available => 1,
            AmbientSensorState::NoSensor => 2,
            AmbientSensorState::FrozenReport => 3,
            AmbientSensorState::Unknown => 0,
        }
    }

    pub const fn from_code(v: u8) -> Self {
        match v {
            1 => AmbientSensorState::Available,
            2 => AmbientSensorState::NoSensor,
            3 => AmbientSensorState::FrozenReport,
            _ => AmbientSensorState::Unknown,
        }
    }

    /// States whose diagnosis is stable enough that poking the stack with
    /// reset requests is provably useless (production: the resets fired every
    /// 30 s for days against a frozen TCS3408 and changed nothing).
    pub fn is_diagnosed(self) -> bool {
        matches!(
            self,
            AmbientSensorState::FrozenReport | AmbientSensorState::NoSensor
        )
    }
}

/// One probe of the committed backend: the lux reading (already passed
/// through the per-sensor selection) plus whether the backend could see
/// sensor DEVICES at all (the NoSensor vs FrozenReport discriminator).
#[derive(Debug, Clone, Copy, PartialEq)]
struct AmbientProbe {
    lux: Option<f32>,
    devices_seen: bool,
}

/// Latest probe of the persistent sensor state.
static SENSOR_STATE: AtomicU8 = AtomicU8::new(AmbientSensorState::Unknown.code());
/// Wall-clock ms of the last REAL backend probe (suppression-window base).
static SENSOR_LAST_PROBE_MS: AtomicU64 = AtomicU64::new(0);

/// How long a diagnosed (`FrozenReport`/`NoSensor`) sensor goes without a
/// real backend probe. Between probes `get_ambient_lux` answers `None`
/// without touching COM/WinRT, so a dead sensor stops costing a stack
/// round-trip every 2 s; one probe per window still notices a driver fix.
const SENSOR_DIAGNOSED_REPROBE_SECS: u64 = 900;

/// Latest sensor diagnosis, for the loop / supervisor / `DisplayInfo`.
pub fn ambient_sensor_state() -> AmbientSensorState {
    AmbientSensorState::from_code(SENSOR_STATE.load(Ordering::Relaxed))
}

/// Pure classification of one probe (unit-testable, no COM/WinRT contact):
/// fresh lux wins; devices seen but no fresh data = frozen; nothing
/// enumerated = no sensor on this machine.
fn classify_sensor_probe(lux: Option<f32>, devices_seen: bool) -> AmbientSensorState {
    match lux {
        Some(v) if v.is_finite() && v > MIN_PLAUSIBLE_LUX => AmbientSensorState::Available,
        _ if devices_seen => AmbientSensorState::FrozenReport,
        _ => AmbientSensorState::NoSensor,
    }
}

/// Pure suppression-window check: a diagnosed sensor is re-probed at most
/// once per `SENSOR_DIAGNOSED_REPROBE_SECS`; `last_probe_ms == 0` means
/// "never probed" and always probes (the first diagnosis must be real).
fn probe_suppressed(state: AmbientSensorState, last_probe_ms: u64, now_ms: u64) -> bool {
    if !state.is_diagnosed() {
        return false;
    }
    last_probe_ms != 0
        && now_ms.saturating_sub(last_probe_ms) < SENSOR_DIAGNOSED_REPROBE_SECS * 1000
}

/// Store a settled state and log TRANSITIONS only (the Available probe runs
/// every 2 s; logging every settle would spam the log the old resets did).
fn settle_sensor_state(probe: AmbientProbe) -> AmbientSensorState {
    let settled = classify_sensor_probe(probe.lux, probe.devices_seen);
    let previous = SENSOR_STATE.swap(settled.code(), Ordering::Relaxed);
    if previous != settled.code() {
        log::info!(
            "[display] ambient sensor state: {:?} (was {:?}, lux={:?} devices_seen={})",
            settled,
            AmbientSensorState::from_code(previous),
            probe.lux,
            probe.devices_seen
        );
    }
    settled
}

// ── Last fresh reading (persisted, hour resolution) ───────────────────────────

/// Wall-clock ms of the last FRESH lux reading this process has seen. `0`
/// until hydration from the `ambient_sensor` store (so the frozen-sensor
/// message can show a date from a previous session, e.g. "18 Nov").
static AMBIENT_LAST_FRESH_MS: AtomicU64 = AtomicU64::new(0);
/// One-time hydration guard for the persisted last-fresh timestamp.
static AMBIENT_FRESH_HYDRATED: AtomicBool = AtomicBool::new(false);
/// Wall-clock ms last WRITTEN to the store; throttles persistence to one
/// write per hour (a healthy sensor produces a fresh reading every 2 s and
/// must not turn an atomic-rename state file into a write hotspot).
static AMBIENT_FRESH_PERSISTED_MS: AtomicU64 = AtomicU64::new(0);

/// Persist the last-fresh stamp at most once per hour (`prev == 0` = never
/// persisted, always allowed). Pure, so tests can pin the throttle.
fn should_persist_fresh_stamp(prev_persisted_ms: u64, now_ms: u64) -> bool {
    prev_persisted_ms == 0 || now_ms.saturating_sub(prev_persisted_ms) >= 3_600_000
}

/// Persisted last-fresh timestamp for `DisplayInfo` / the UI message. Loads
/// the store once per process, then tracks the in-session value.
pub fn ambient_last_fresh_ms() -> Option<u64> {
    if !AMBIENT_FRESH_HYDRATED.swap(true, Ordering::Relaxed) {
        match state_store::ambient_sensor_store().and_then(|store| store.load()) {
            Ok(Some(record)) if record.last_fresh_ms > 0 => {
                // Never overwrite a value a fresh probe already recorded.
                AMBIENT_LAST_FRESH_MS
                    .compare_exchange(
                        0,
                        record.last_fresh_ms,
                        Ordering::Relaxed,
                        Ordering::Relaxed,
                    )
                    .ok();
                AMBIENT_FRESH_PERSISTED_MS.store(record.last_fresh_ms, Ordering::Relaxed);
            }
            Ok(_) => {}
            Err(e) => log::warn!("[display] ambient sensor record unreadable: {e}"),
        }
    }
    let v = AMBIENT_LAST_FRESH_MS.load(Ordering::Relaxed);
    (v != 0).then_some(v)
}

/// Record a fresh (plausible) lux reading and persist the wall-clock stamp
/// through the `ambient_sensor` store at most once per hour. Failures only
/// log: the timestamp is diagnostic metadata, never control state.
fn note_fresh_lux(lux: f32) {
    if !lux.is_finite() || lux <= MIN_PLAUSIBLE_LUX {
        return;
    }
    let now = now_ms();
    AMBIENT_LAST_FRESH_MS.store(now, Ordering::Relaxed);
    if !should_persist_fresh_stamp(AMBIENT_FRESH_PERSISTED_MS.load(Ordering::Relaxed), now) {
        return;
    }
    AMBIENT_FRESH_PERSISTED_MS.store(now, Ordering::Relaxed);
    let record = state_store::AmbientSensorRecord { last_fresh_ms: now };
    match state_store::ambient_sensor_store().and_then(|store| store.save(&record)) {
        Ok(()) => log::debug!("[display] persisted ambient last-fresh stamp ({now}ms)"),
        Err(e) => log::warn!("[display] could not persist ambient last-fresh stamp: {e}"),
    }
}

/// Wire shape for `DisplayInfo.ambient_sensor`: the diagnosis plus the
/// persisted last-fresh timestamp the UI renders as "last reading D Mon".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct AmbientSensorInfo {
    /// The current diagnosis (available / no_sensor / frozen_report / unknown).
    pub state: AmbientSensorState,
    /// Wall-clock ms of the last fresh lux reading (survives restarts, hour
    /// resolution). `None` = no fresh reading ever recorded.
    pub last_fresh_ms: Option<u64>,
}

/// Snapshot for `get_display_info`.
fn ambient_sensor_info() -> AmbientSensorInfo {
    AmbientSensorInfo {
        state: ambient_sensor_state(),
        last_fresh_ms: ambient_last_fresh_ms(),
    }
}

/// Global flag to force sensor re-initialization on next read.
/// Set to `true` when a power-resume event is detected so the sensor
/// instance is re-created instead of returning stale `None`.
static SENSOR_RESET_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Request a sensor reset on the next polling cycle.
///
/// Callers own the *reason*: a real resume (power listener) is INFO, a
/// degraded/stuck sensor (health supervisor, adaptive-brightness watchdog) is
/// DEBUG noise. This used to log "(power resume event)" for every caller,
/// which put six fake resumes in a 7 h window with no suspend at all.
///
/// Diagnosed-stable states (FrozenReport / NoSensor) suppress the request
/// entirely: re-creating those stacks cannot make them deliver data
/// (production proof: the TCS3408 froze for 4+ days while reset requests kept
/// firing every 5 cycles). Gating here protects every caller, including the
/// resume path: a healthy sensor still resets normally on resume, a frozen
/// one just wastes the attempt.
pub fn request_sensor_reset(reason: &str) {
    let state = ambient_sensor_state();
    if state.is_diagnosed() {
        log::debug!(
            "[display] sensor reset suppressed ({reason}): ambient sensor state is {state:?}"
        );
        return;
    }
    SENSOR_RESET_REQUESTED.store(true, Ordering::SeqCst);
    log::debug!("[display] Sensor reset requested ({reason})");
}

/// Cached LightSensor instance — re-created only when a reset is requested.
/// This avoids calling `LightSensor::GetDefault()` on every 2-second poll,
/// which can be slow and may return `None` transiently after sleep/wake.
#[cfg(windows)]
static CACHED_LIGHT_SENSOR: std::sync::OnceLock<
    std::sync::Mutex<Option<windows::Devices::Sensors::LightSensor>>,
> = std::sync::OnceLock::new();

/// Fallback ambient-light reader using the WinRT `Windows.Devices.Sensors.LightSensor` API.
/// This works well in UWP/WinUI apps, but in an unpackaged desktop app it frequently
/// returns `None` even when a HID ALS sensor is present in Device Manager.
/// Kill switch for the ambient-light readers (diagnostics, CI and kiosks with a
/// broken sensor stack). `MICONTROL_DISABLE_AMBIENT_SENSOR` accepts:
///   "1" / "all"  → both readers off
///   "com"        → only the classic ISensorManager path off
///   "winrt"      → only the WinRT LightSensor path off
/// `0` or empty leaves both enabled.
#[cfg(windows)]
fn ambient_sensor_disabled(path: &str) -> bool {
    static DISABLED: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    let v = DISABLED.get_or_init(|| {
        std::env::var("MICONTROL_DISABLE_AMBIENT_SENSOR")
            .unwrap_or_default()
            .to_ascii_lowercase()
    });
    match v.as_str() {
        "" | "0" => false,
        "1" | "all" => true,
        other => other == path,
    }
}

#[cfg(windows)]
static WINRT_DEVICES_SEEN: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
fn get_ambient_lux_winrt() -> Option<f32> {
    use windows::Devices::Sensors::LightSensor;

    if ambient_sensor_disabled("winrt") {
        return None;
    }

    // Check if a reset was requested (e.g., after sleep/wake)
    let needs_reset = SENSOR_RESET_REQUESTED.swap(false, Ordering::SeqCst);

    let cache = CACHED_LIGHT_SENSOR.get_or_init(|| std::sync::Mutex::new(None));
    let mut guard = cache.lock().ok()?;

    if needs_reset || guard.is_none() {
        // Re-create the sensor instance
        match LightSensor::GetDefault() {
            Ok(sensor) => {
                log::debug!(
                    "[display] WinRT ambient light sensor {}",
                    if needs_reset {
                        "re-initialized (reset)"
                    } else {
                        "found"
                    }
                );
                WINRT_DEVICES_SEEN.store(true, Ordering::Relaxed);
                *guard = Some(sensor);
            }
            Err(_) => {
                log::debug!("[display] WinRT ambient light sensor not available");
                WINRT_DEVICES_SEEN.store(false, Ordering::Relaxed);
                *guard = None;
                return None;
            }
        }
    }

    let sensor = guard.as_ref()?;
    let reading = sensor.GetCurrentReading().ok()?;
    let lux = reading.IlluminanceInLux().ok()?;
    log::debug!("[display] WinRT ambient lux: {lux}");
    Some(lux)
}

// Per-thread cache of the classic COM light sensor stack.
//
// `ISensorManager` (COM) is not `Send`/`Sync`, so we cannot put it in a
// `static`. This mirrors `wmi_cache.rs`'s thread_local pattern: each worker
// thread lazily creates the manager + sensor collection once, then reuses it
// for every 2-second poll. The entry is left as `None` (a) before first use,
// (b) when a poll fails — the next poll retries from scratch (re-enumerating
// hot-plugged sensors) — and (c) forever on machines without the sensor.
#[cfg(windows)]
thread_local! {
    static COM_SENSOR_MANAGER: std::cell::RefCell<Option<ComSensorStack>> = const {
        std::cell::RefCell::new(None)
    };
}

/// Whether the LAST COM stack creation succeeded in enumerating at least one
/// ambient-light sensor. The WinRT side sets the same flag when
/// `GetDefault()` returns a sensor. This is the NoSensor-vs-FrozenReport
/// discriminator of `classify_sensor_probe`.
#[cfg(windows)]
static COM_DEVICES_SEEN: AtomicBool = AtomicBool::new(false);

/// Lazily-created COM sensor manager + ambient-light collection.
#[cfg(windows)]
struct ComSensorStack {
    /// Sensor collection (owns the enumeration). Manager is kept only for its
    /// lifetime semantics; both are released together when the thread exits.
    collection: windows::Win32::Devices::Sensors::ISensorCollection,
}

#[cfg(windows)]
impl ComSensorStack {
    /// (Re)create the sensor stack. Returns None on any failure path so the
    /// caller falls back to WinRT / sensor-off.
    ///
    /// Whether the backend can see sensor DEVICES at all is recorded
    /// process-wide (`COM_DEVICES_SEEN`: the stack itself is per-thread, the
    /// diagnosis is not) so `classify_sensor_probe` can tell NoSensor
    /// (nothing enumerated) from FrozenReport (devices present, no data).
    fn new() -> Option<Self> {
        use windows::Win32::Devices::Sensors::{
            ISensorManager, SensorManager as CLSID_SensorManager, SENSOR_TYPE_AMBIENT_LIGHT,
        };
        use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL};

        COM_DEVICES_SEEN.store(false, Ordering::Relaxed);
        // SAFETY: COM objects released on drop; pointers remain valid within the call.
        unsafe {
            let manager: ISensorManager = CoCreateInstance(&CLSID_SensorManager, None, CLSCTX_ALL)
                .map_err(|e| {
                    log::debug!("[display] COM SensorManager not available: {e}");
                    e
                })
                .ok()?;
            let collection = manager.GetSensorsByType(&SENSOR_TYPE_AMBIENT_LIGHT).ok()?;
            let count = collection.GetCount().ok()?;
            if count == 0 {
                log::debug!("[display] COM SensorManager: no ambient light sensors found");
                return None;
            }
            log::debug!("[display] COM SensorManager cached ({count} sensors)");
            COM_DEVICES_SEEN.store(true, Ordering::Relaxed);
            Some(Self { collection })
        }
    }

    /// Read every sensor's current lux value. Returns None if the whole stack
    /// must be recreated (the caller invalidates the cache).
    fn read_all(&self) -> Option<Vec<Option<f32>>> {
        use windows::Win32::Devices::Sensors::SENSOR_DATA_TYPE_LIGHT_LEVEL_LUX;
        use windows::Win32::Foundation::{E_FAIL, S_OK};

        // SAFETY: sensor/report/VARIANT pointers valid within the call; released
        // by their wrappers on drop.
        unsafe {
            let count = self.collection.GetCount().ok()?;
            let mut readings: Vec<Option<f32>> = Vec::with_capacity(count as usize);
            for i in 0..count {
                let Ok(sensor) = self.collection.GetAt(i) else {
                    // A sensor appeared/disappeared → invalidate cache.
                    return None;
                };
                let Ok(report) = sensor.GetData() else {
                    // Sensor momentarily unresponsive: report as None this poll,
                    // but don't tear down the whole stack for one glitch.
                    readings.push(None);
                    continue;
                };
                let pv = report.GetSensorValue(&SENSOR_DATA_TYPE_LIGHT_LEVEL_LUX);
                match pv {
                    Ok(v) => {
                        let lux = f64::try_from(&v).ok()? as f32;
                        log::debug!("[display] COM sensor[{i}] ambient lux: {lux}");
                        readings.push(if lux.is_finite() { Some(lux) } else { None });
                    }
                    Err(e) => {
                        let hr = e.code();
                        if hr == E_FAIL || hr == S_OK {
                            readings.push(None);
                        } else {
                            // Unusual HRESULT → invalidate the whole stack.
                            return None;
                        }
                    }
                }
            }
            Some(readings)
        }
    }
}

/// Primary ambient-light reader using the classic COM Sensor API (`ISensorManager`).
/// This is the API that Windows itself uses for the "Adaptive brightness" power-plan
/// setting and it works for unpackaged desktop applications, unlike the WinRT API.
#[cfg(windows)]
fn get_ambient_lux_com() -> Option<f32> {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    if ambient_sensor_disabled("com") {
        return None;
    }

    // S32-003 rationale (sensor selection) lives in `select_responsive_sensor`.
    //
    // S26-006: The manager+enumerator are cached per-thread (this runs on the
    // tokio blocking pool, whose threads are long-lived). The previous
    // implementation called `CoCreateInstance` + `GetSensorsByType` + `GetAt`
    // on EVERY 2-second poll, churning COM allocations and MTA registrations
    // for the whole app lifetime — an unnecessary leak/stress vector. Caching
    // mirrors the `wmi_cache` thread_local pattern; the cache is invalidated
    // when a poll fails so hot-plugged sensors are re-enumerated.

    // SAFETY: `CoInitializeEx` is idempotent for the same apartment model and is safe
    // to call from each thread. If the thread is already in a different apartment model
    // the call returns RPC_E_CHANGED_MODE, which we ignore because the runtime may have
    // already initialized COM for us.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };

    let readings = COM_SENSOR_MANAGER.with(|cell| {
        let mut borrow = cell.borrow_mut();
        let stack = match borrow.as_ref() {
            Some(s) => s,
            None => {
                let s = ComSensorStack::new()?;
                *borrow = Some(s);
                borrow.as_ref()?
            }
        };
        let r = stack.read_all();
        if r.is_none() {
            // Invalidate on failure so the next poll re-enumerates.
            *borrow = None;
        }
        r
    });
    let readings = readings?;
    let lux = select_responsive_sensor(&readings);
    log::debug!("[display] COM selected lux: {lux:?}");
    lux
}

/// Per-sensor last-read cache used to detect which ALS sensor actually responds
/// to light (the others are placeholders that return a fixed value).
#[cfg(windows)]
static COM_LUX_HISTORY: std::sync::OnceLock<std::sync::Mutex<Vec<Option<f32>>>> =
    std::sync::OnceLock::new();

/// A sensor reading is only trustworthy when it is a plausible, finite
/// illuminance value. Values at or below this floor are physically impossible
/// with the panel on and mean either a placeholder sensor, an uninitialised
/// driver value, or a stale/unresponsive sensor (e.g. stuck at 1 lux after the
/// app was closed and reopened). The adaptive loop uses the same threshold.
const MIN_PLAUSIBLE_LUX: f32 = 1.5;

/// Choose the most responsive ambient-light sensor from a set of simultaneous
/// readings, using cached previous readings to measure variability.
///
/// Strategy:
///   1. Compute the absolute delta between each sensor's current and previous
///      reading. The sensor with the largest delta is the real one.
///   2. If all deltas are 0 (steady light), reuse the index of the sensor that
///      was responsive in the last poll (cached).
///   3. If never responsive, fall back to the first finite reading.
#[cfg(windows)]
fn select_responsive_sensor(readings: &[Option<f32>]) -> Option<f32> {
    select_responsive_sensor_impl(readings)
}

/// Test hook: public wrapper around the responsive-sensor selector so the
/// selection logic can be validated from integration tests.
#[cfg(windows)]
pub fn test_select_responsive(readings: Vec<Option<f32>>) -> Option<f32> {
    select_responsive_sensor_impl(&readings)
}

#[cfg(windows)]
fn select_responsive_sensor_impl(readings: &[Option<f32>]) -> Option<f32> {
    let history = COM_LUX_HISTORY.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    let mut guard = history.lock().ok()?;

    // Resize the history cache to match the sensor count.
    if guard.len() != readings.len() {
        guard.resize(readings.len(), None);
    }
    // Snapshot the PREVIOUS readings (before we overwrite them below) so we
    // can measure variability between this poll and the last poll.
    let prev: Vec<Option<f32>> = guard.clone();

    // 1. Find the sensor with the largest absolute change vs the previous poll.
    let mut best_index: Option<usize> = None;
    let mut best_delta: f32 = 0.0;
    for (i, reading) in readings.iter().enumerate() {
        if let (Some(cur), Some(prev_v)) = (reading, prev[i]) {
            let delta = (cur - prev_v).abs();
            if delta > best_delta {
                best_delta = delta;
                best_index = Some(i);
            }
        }
    }

    // 2. Update the history with ALL current readings (not just the chosen
    //    one) so every sensor accumulates history for future polls.
    for (i, reading) in readings.iter().enumerate() {
        guard[i] = *reading;
    }

    // 3. Return the most responsive sensor's current reading — but only when
    //    it is a plausible lux value. A "responsive" placeholder that wanders
    //    between 0.1 and 1 lux is not a light reading; treat it as unavailable
    //    so the loop idles instead of driving brightness to the floor.
    if let Some(idx) = best_index {
        if let Some(v) = readings[idx].filter(|v| *v >= MIN_PLAUSIBLE_LUX) {
            return Some(v);
        }
    }

    // 4. Steady light (nothing changed): reuse the last sensor that ever had
    //    a recorded value different from its first-seen value. Simpler and
    //    robust enough: prefer the sensor whose history is non-empty and whose
    //    value differs from the very first reading ever stored for it. We
    //    approximate "responsive ever" by preferring the LAST finite sensor,
    //    since the placeholder is typically index 0. Reject implausible values
    //    here too — a stuck "1 lux" must never be trusted just because it is
    //    the last finite reading.
    let last_finite = readings.iter().rposition(|r| r.is_some());
    if let Some(idx) = last_finite {
        return readings[idx].filter(|v| *v >= MIN_PLAUSIBLE_LUX);
    }

    None
}

/// Which ambient-light backend this process has committed to.
///
/// 2026-11-18 (AV in the display tests): calling the classic COM
/// `ISensorManager` and then the WinRT `LightSensor` in the SAME poll crashed
/// the process with STATUS_ACCESS_VIOLATION right after the COM stack
/// returned "no usable reading" — reproduced twice, and each path in
/// isolation passed. Mixing the two sensor stacks (the WinRT activation
/// re-enters the same sensorservice singleton the COM manager just created)
/// is the trigger. The fix is to commit to ONE backend per process: the first
/// backend that yields a reading wins, and the other is never touched again.
/// A backend that fails to produce a reading is simply not adopted, so a
/// machine where WinRT is unavailable still ends up on COM.
#[cfg(windows)]
#[derive(Clone, Copy, PartialEq, Eq)]
enum LuxBackend {
    Winrt,
    Com,
}

#[cfg(windows)]
fn lux_backend_override() -> Option<LuxBackend> {
    static FORCED: std::sync::OnceLock<Option<LuxBackend>> = std::sync::OnceLock::new();
    *FORCED.get_or_init(|| {
        match std::env::var("MICONTROL_AMBIENT_SENSOR_BACKEND")
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "winrt" => Some(LuxBackend::Winrt),
            "com" => Some(LuxBackend::Com),
            _ => None,
        }
    })
}

#[cfg(windows)]
fn run_lux_backend(b: LuxBackend) -> Option<f32> {
    match b {
        LuxBackend::Winrt => get_ambient_lux_winrt(),
        LuxBackend::Com => get_ambient_lux_com(),
    }
}

/// True when the classic COM sensor stack can be created on this thread
/// (manager + at least one ambient-light sensor). Probed ONCE per process.
#[cfg(windows)]
fn com_sensor_stack_available() -> bool {
    static AVAILABLE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *AVAILABLE.get_or_init(|| {
        COM_SENSOR_MANAGER.with(|cell| {
            let mut borrow = cell.borrow_mut();
            if borrow.is_none() {
                *borrow = ComSensorStack::new();
            }
            borrow.is_some()
        })
    })
}

/// One classified probe of the committed lux backend.
#[cfg(windows)]
fn probe_ambient_backend(backend: LuxBackend) -> AmbientProbe {
    AmbientProbe {
        lux: run_lux_backend(backend),
        devices_seen: COM_DEVICES_SEEN.load(Ordering::Relaxed)
            || WINRT_DEVICES_SEEN.load(Ordering::Relaxed),
    }
}

/// The process's committed ambient-light backend. One decision, one scope:
/// COM and WinRT both reach the same sensorservice singleton and mixing the
/// two stacks in one process is the access-violation trigger, so every real
/// probe goes through this single choice.
#[cfg(windows)]
static COMMITTED_BACKEND: std::sync::OnceLock<LuxBackend> = std::sync::OnceLock::new();

#[cfg(windows)]
fn get_ambient_lux() -> Option<f32> {
    // The backend is decided once, on the first probe, and never revisited:
    // an explicit MICONTROL_AMBIENT_SENSOR_BACKEND wins, otherwise the
    // classic COM stack is adopted when it enumerates, with WinRT as the
    // fallback for machines where it does not.
    let backend = *COMMITTED_BACKEND.get_or_init(|| {
        lux_backend_override().unwrap_or_else(|| {
            if com_sensor_stack_available() {
                LuxBackend::Com
            } else {
                LuxBackend::Winrt
            }
        })
    });

    // Diagnosed-stale gate: while the sensor is diagnosed FrozenReport or
    // NoSensor, only one REAL probe per re-probe window reaches the sensor
    // stack; every other call in the window answers `None` without touching
    // COM/WinRT. This is what ends both cost centres of the old probe-per-
    // call behaviour: the health supervisor's every-30 s reset requests and
    // the loop's per-2 s stack round-trips against a sensor that does not
    // deliver data.
    let now = now_ms();
    let last = SENSOR_LAST_PROBE_MS.swap(now, Ordering::Relaxed);
    if probe_suppressed(ambient_sensor_state(), last, now) {
        return None;
    }

    let probe = probe_ambient_backend(backend);
    settle_sensor_state(probe);
    let lux = probe
        .lux
        .filter(|v| v.is_finite() && *v > MIN_PLAUSIBLE_LUX);
    if let Some(v) = lux {
        // Persist the last-fresh stamp (hour resolution) so the frozen-
        // sensor message can later show a real date ("last reading 18 Nov").
        note_fresh_lux(v);
    }
    lux
}

#[cfg(not(windows))]
fn get_ambient_lux() -> Option<f32> {
    // No COM/WinRT stacks on this platform: honestly NoSensor.
    settle_sensor_state(AmbientProbe {
        lux: None,
        devices_seen: false,
    });
    None
}

// ── Adaptive brightness background loop ──────────────────────────────────────

/// Returns `true` when the monitor is powered on.
///
/// Uses `GetSystemMetrics(SM_MONITORPOWER)`:
/// -1 = display is on
///  1 = display is going to power-off
///  2 = display is off
///  0 = some systems return this when the display is on (not documented but observed)
///
/// On non-Windows or if the call fails, we assume the display is on.
#[cfg(windows)]
fn is_display_on() -> bool {
    use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SYSTEM_METRICS_INDEX};
    // SM_MONITORPOWER is not exposed by the windows 0.58 crate; the raw value is 112.
    const SM_MONITORPOWER: i32 = 112;
    // SAFETY: GetSystemMetrics is a thread-safe Win32 API call.
    let val = unsafe { GetSystemMetrics(SYSTEM_METRICS_INDEX(SM_MONITORPOWER)) };
    // -1 and 0 mean the display is on; 1 means transitioning to off; 2 means off.
    val == -1 || val == 0
}

#[cfg(not(windows))]
fn is_display_on() -> bool {
    true
}

/// Spawned once at startup. Every 2 s it reads the ambient light sensor and
/// adjusts screen brightness according to the user-configured sensitivity curve.
/// Config changes are picked up automatically on each iteration.
pub async fn adaptive_brightness_loop() {
    log::info!("[adaptive_brightness] loop started");
    // S38: A crash/relaunch resets all process-local statics. If the user
    // manually adjusted brightness shortly before the crash, restore the
    // override so the loop stays hands-off instead of immediately stealing
    // the user's value back (the brightness regression after relaunch).
    restore_override_state();
    let mut smoothed: Option<f32> = None;
    let mut no_sensor_warned = false;
    // Last value we applied so we can detect external changes (Fn keys, OS).
    let mut last_set: Option<u8> = None;
    // Track whether we have already disabled Windows ADAPTBRIGHT for the
    // current "enabled session".  Reset when adaptive brightness is turned off
    // so we re-disable it if the user re-enables.
    let mut adaptbright_suppressed = false;
    // Counter of consecutive invalid/stuck sensor reads. When a sensor gets
    // stuck (e.g. returning 1 lux forever after the app was closed and
    // reopened), we request a sensor reset instead of fighting the backlight.
    let mut consecutive_invalid_reads: u32 = 0;
    const STUCK_SENSOR_RESET_AFTER: u32 = 5; // ~10 s of invalid reads
    loop {
        tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;

        // S32-003: While the display color calibration wizard is open, do NOT
        // touch the backlight. Writing brightness while dccw.exe is saving its
        // gamma ramp causes "Access is denied" / "Close other programs".
        if CALIBRATION_IN_PROGRESS.load(Ordering::Relaxed) {
            log::debug!("[adaptive_brightness] paused — color calibration in progress");
            continue;
        }

        // Skip the iteration when the display is off (lid closed, sleep, etc.)
        // to avoid wasting CPU and fighting with the OS power manager.
        // S69: FIXED pool (no tokio blocking-pool churn → no handle leak).
        let display_on = crate::util::fixed_pool::run_blocking(is_display_on)
            .await
            .unwrap_or(true);
        if !display_on {
            continue;
        }

        // Run blocking hardware calls on the fixed pool to avoid starving the
        // tokio runtime worker threads.
        let Ok((cfg, brightness_actual)) = crate::util::fixed_pool::run_blocking(|| {
            let cfg = get_ai_brightness_config();
            let brightness_actual = if cfg.enabled {
                read_current_brightness()
            } else {
                None
            };
            (cfg, brightness_actual)
        })
        .await
        else {
            log::warn!("adaptive_brightness: blocking(config) failed");
            continue;
        };

        if !cfg.enabled {
            smoothed = None;
            last_set = None;
            adaptbright_suppressed = false;
            consecutive_invalid_reads = 0;
            log::debug!("[adaptive_brightness] disabled — skipping iteration");
            continue;
        }

        log::debug!(
            "[adaptive_brightness] enabled: min={} max={} sens={} smooth={} actual_brightness={:?}",
            cfg.min_brightness,
            cfg.max_brightness,
            cfg.sensitivity,
            cfg.smoothing,
            brightness_actual
        );

        // Ensure Windows' own ADAPTBRIGHT (power-plan adaptive brightness) is
        // off — if both run simultaneously they fight over the backlight,
        // causing the brightness-near-zero oscillation symptom.
        if !adaptbright_suppressed {
            let _ =
                crate::util::fixed_pool::run_blocking(disable_windows_adaptive_brightness).await;
            adaptbright_suppressed = true;
        }

        // ── Detect external brightness changes (Fn keys, Windows sliders) ──
        // If the actual brightness differs from what we last set by ≥ 2 pp,
        // someone else changed it.  Treat it as a user preference shift:
        // compute a new offset so the loop keeps the adjusted baseline.
        // S32-005: Also enter the manual-override grace window so the loop
        // stays hands-off while the user finishes adjusting.
        if let (Some(prev), Some(actual)) = (last_set, brightness_actual) {
            let diff = (actual as i16 - prev as i16).abs();
            if diff >= 2 {
                let raw = AUTO_LAST_TARGET.load(Ordering::Relaxed);
                let current_offset = AUTO_OFFSET.load(Ordering::Relaxed);
                let active = AUTO_OFFSET_ACTIVE.load(Ordering::Relaxed);
                let origin = OffsetOrigin::from_u8(AUTO_OFFSET_ORIGIN.load(Ordering::Relaxed));
                // Classify before writing: an active user offset (±100 pp) must
                // not be re-derived with the capture bound (±30 pp) — that is
                // what silently trimmed "100 %" to 56 % within one iteration.
                // A value that reproduces the current offset is our own write
                // coming back and leaves the curve alone.
                let outcome = classify_capture(actual, raw, current_offset, origin, active);

                AUTO_OFFSET.store(outcome.offset, Ordering::Relaxed);
                AUTO_OFFSET_ORIGIN.store(outcome.origin.as_u8(), Ordering::Relaxed);
                AUTO_OFFSET_ACTIVE.store(true, Ordering::Relaxed);
                AUTO_LAST_MANUAL_MS.store(now_ms(), Ordering::Relaxed);
                // S38: persist so a crash/relaunch within the grace window
                // does not steal the user's brightness back.
                persist_override_state();
                // Snap smoothed to actual so we don't animate back.
                smoothed = Some(actual as f32);
                // Self-clearing: the next iteration must not re-detect the
                // difference we just absorbed (`last_set` is otherwise only
                // refreshed by an actual write, which the grace window skips).
                last_set = Some(actual);
                log::debug!(
                    "auto_brightness: external change detected \
                     prev={prev}% actual={actual}% raw={raw}% origin={} → offset={:+}{}, grace armed",
                    outcome.origin.label(),
                    outcome.offset,
                    if outcome.echo { " (echo — curve unchanged)" } else { "" }
                );
                // Smart-mode learning: this external change (Fn key, Windows
                // slider) carries the user's preference for the current light
                // level — log it into the smart model, but never while the
                // override offset is active (the captured value then contains
                // the loop's own output, which is the feedback loop that
                // produced 1.3 M samples and `offset: 54` in production).
                if let Some(lux) = get_ambient_lux().filter(|v| *v > MIN_PLAUSIBLE_LUX) {
                    record_external_change_sample(lux, actual);
                }
            }
        }
        // After a crash/relaunch (or first run), seed last_set from the actual
        // current brightness so the FIRST iteration does not immediately treat
        // the real user value as an "external change" and move the backlight.
        if last_set.is_none() {
            if let Some(actual) = brightness_actual {
                last_set = Some(actual);
                smoothed = Some(actual as f32);
            }
        }

        // S32-005: While the user is still within the manual-override grace
        // window, do NOT touch the backlight at all — the user just set it and
        // may still be tweaking. This is the guarantee the user asked for:
        // "if I adjust brightness manually, it must not snap back".
        if user_override_in_grace() {
            log::debug!(
                "[adaptive_brightness] in manual-override grace window \
                 (last manual change < {MANUAL_OVERRIDE_GRACE_MS}ms) — skipping write"
            );
            continue;
        }

        let lux = match crate::util::fixed_pool::run_blocking(get_ambient_lux).await {
            // A reading ≤ MIN_PLAUSIBLE_LUX lux with the screen on is physically
            // impossible; it means a placeholder sensor, an uninitialised value
            // (common at process startup on this hardware), or a stuck sensor
            // (e.g. fixed 1 lux after app close/reopen). Track consecutive bad
            // reads so we can reset a wedged sensor instead of trusting it.
            Ok(Some(v)) if v > MIN_PLAUSIBLE_LUX => v,
            Ok(Some(_)) => {
                consecutive_invalid_reads = consecutive_invalid_reads.saturating_add(1);
                if consecutive_invalid_reads >= STUCK_SENSOR_RESET_AFTER {
                    let state = ambient_sensor_state();
                    if state.is_diagnosed() {
                        // {FrozenReport, NoSensor}: resetting the stacks cannot
                        // make a diagnosed-dead sensor deliver data (production
                        // proof: the TCS3408 froze for 4+ days while the reset
                        // requests kept firing every 5 cycles). Idle quietly.
                        log::debug!(
                            "[adaptive_brightness] sensor diagnosed {state:?} — reset suppressed"
                        );
                    } else {
                        log::warn!(
                            "[adaptive_brightness] sensor stuck ({} consecutive invalid \
                             readings) — requesting sensor reset",
                            consecutive_invalid_reads
                        );
                        request_sensor_reset("sensor stuck (invalid readings)");
                    }
                    consecutive_invalid_reads = 0;
                }
                continue;
            }
            Ok(None) => {
                consecutive_invalid_reads = consecutive_invalid_reads.saturating_add(1);
                if consecutive_invalid_reads >= STUCK_SENSOR_RESET_AFTER {
                    let state = ambient_sensor_state();
                    if state.is_diagnosed() {
                        log::debug!(
                            "[adaptive_brightness] sensor diagnosed {state:?} — reset suppressed"
                        );
                    } else {
                        log::warn!(
                            "[adaptive_brightness] no valid sensor reading for {} cycles — \
                             requesting sensor reset",
                            consecutive_invalid_reads
                        );
                        request_sensor_reset("sensor stuck (no reading)");
                    }
                    consecutive_invalid_reads = 0;
                }
                if !no_sensor_warned {
                    // One line per session, honest about WHICH absence this is:
                    // a machine with no ALS at all is not the same failure as a
                    // present-but-frozen sensor.
                    match ambient_sensor_state() {
                        AmbientSensorState::FrozenReport => log::info!(
                            "[adaptive_brightness] ambient sensor present but not reporting — \
                             loop idle (no reset requests)"
                        ),
                        _ => log::warn!(
                            "adaptive_brightness: no ambient light sensor found — loop idle. \
                             LightSensor::GetDefault() returned None. \
                             Check: (1) sensor driver installed, (2) sensor enabled in Device \
                             Manager, (3) Devices_Sensors feature in Cargo.toml (confirmed present)."
                        ),
                    }
                    no_sensor_warned = true;
                }
                continue;
            }
            Err(e) => {
                log::warn!("adaptive_brightness: get_ambient_lux task failed: {e}");
                continue;
            }
        };
        no_sensor_warned = false;
        consecutive_invalid_reads = 0;
        // sensitivity=100 → reaches ceiling at 2000 lux
        // sensitivity=200 → reaches ceiling at 1000 lux  (more reactive)
        // sensitivity=50  → reaches ceiling at 4000 lux  (less reactive)
        let max_lux = 2000.0_f32 * (100.0 / cfg.sensitivity.max(1) as f32);
        let range = cfg.max_brightness as f32 - cfg.min_brightness as f32;
        // CURVE_BOOST lifts the entire brightness curve by this many percentage
        // points without changing the slope or the user-configurable min/max.
        const CURVE_BOOST: f32 = 20.0;
        let raw_target = if cfg.mode == "smart" {
            // Smart mode: use the learned user-preference mean for this lux
            // level where enough samples exist; otherwise fall back to the
            // fixed curve so new light levels still behave sensibly.
            match smart_predict_brightness(lux) {
                Some(learned) => {
                    let learned = learned as f32;
                    log::debug!(
                        "[adaptive_brightness] smart target: lux={lux:.0} \
                         learned={learned:.0}% (bucketed mean)"
                    );
                    learned.clamp(cfg.min_brightness as f32, cfg.max_brightness as f32)
                }
                None => (cfg.min_brightness as f32 + (lux / max_lux) * range + CURVE_BOOST)
                    .clamp(cfg.min_brightness as f32, cfg.max_brightness as f32),
            }
        } else {
            (cfg.min_brightness as f32 + (lux / max_lux) * range + CURVE_BOOST)
                .clamp(cfg.min_brightness as f32, cfg.max_brightness as f32)
        };

        // Persist raw target so set_brightness can compute the correct offset.
        AUTO_LAST_TARGET.store(raw_target.round() as u8, Ordering::Relaxed);

        // Apply user-override offset: shifts the entire curve up/down so that
        // when the user manually sets brightness the automation respects that
        // preference and only adjusts relative to it as light changes.
        //
        // Bound by ORIGIN, never by the capture limit: a user-intent offset of
        // +74 pp (100 % chosen at 29 lux, raw target 26 %) is legitimate and
        // must reach the panel. Only the final target is clamped, to
        // [min_brightness, max_brightness].
        let offset = if AUTO_OFFSET_ACTIVE.load(Ordering::Relaxed) {
            let origin = OffsetOrigin::from_u8(AUTO_OFFSET_ORIGIN.load(Ordering::Relaxed));
            // Defensive clamp at the point of use: a stale process (or an older
            // build) may have left an out-of-range value in the atomic.
            clamp_offset_for(AUTO_OFFSET.load(Ordering::Relaxed), origin) as f32
        } else {
            0.0
        };
        let target = effective_target(raw_target, offset, cfg.min_brightness, cfg.max_brightness);

        let current = match smoothed {
            Some(s) => s,
            None => {
                // First valid lux reading: seed the smoother from actual current
                // brightness so we never jump immediately to the computed target.
                // read_current_brightness → get_brightness_wmi is a blocking COM
                // call, so it must run on the fixed pool (S69).
                crate::util::fixed_pool::run_blocking(move || {
                    read_current_brightness()
                        .map(|b| b as f32)
                        .unwrap_or(target)
                })
                .await
                .unwrap_or(target)
            }
        };
        let sf = cfg.smoothing.min(95) as f32 / 100.0;
        let next = current + (target - current) * (1.0 - sf);
        smoothed = Some(next);
        let value = next.round() as u8;
        // Hysteresis: skip the write if the new value is the same as last
        // (or within 1 pp) to avoid constant low-amplitude oscillations.
        if last_set.is_some_and(|prev| (value as i16 - prev as i16).abs() < 2) {
            continue;
        }
        let set_value = value;
        log::info!(
            "[adaptive_brightness] setting brightness to {set_value}% (lux={lux:.0}, target={target:.1}, smoothed={next:.1})"
        );
        match crate::util::fixed_pool::run_blocking(move || set_brightness(set_value)).await {
            Ok(Ok(())) => {
                last_set = Some(set_value);
            }
            Ok(Err(e)) => {
                log::warn!("adaptive_brightness: set_brightness error: {e}");
            }
            Err(e) => {
                log::warn!("adaptive_brightness: set_brightness task failed: {e}");
            }
        }
    }
}

// ── IGCL FFI ────────────────────────────────────────────────────────────────

#[cfg(windows)]
mod igcl {
    use std::ffi::c_void;

    #[repr(C)]
    pub struct CtlInitArgs {
        pub size: u32,
        pub app_version: u64,
        pub flags: u32,
    }

    /// Matches Intel IGCL `ctl_brightness_settings_t` (sizeof = 32).
    /// Fields: Size(4) | Version(1) + 3-pad | TargetBrightness(8) |
    ///         SmoothTransitionTargetBrightness(8) | SmoothTransitionTime(4) + 4-pad
    #[repr(C)]
    pub struct CtlBrightnessArgs {
        pub size: u32,
        pub version: u8,
        // [3 bytes C-alignment padding before f64]
        pub target_brightness: f64,
        pub smooth_target_brightness: f64,
        pub smooth_time_ms: u32,
        // [4 bytes C-alignment trailing padding]
    }

    pub type CtlApiHandle = *mut c_void;
    pub type CtlDeviceHandle = *mut c_void;
    pub type CtlResult = u32; // 0 = success

    // Function pointer types
    pub type FnCtlInit = unsafe extern "C" fn(*mut CtlInitArgs, *mut CtlApiHandle) -> CtlResult;
    pub type FnCtlEnumerateDevices =
        unsafe extern "C" fn(CtlApiHandle, *mut u32, *mut CtlDeviceHandle) -> CtlResult;
    pub type FnCtlGetBrightnessSetting =
        unsafe extern "C" fn(CtlDeviceHandle, *mut CtlBrightnessArgs) -> CtlResult;
    pub type FnCtlSetBrightnessSetting =
        unsafe extern "C" fn(CtlDeviceHandle, *mut CtlBrightnessArgs) -> CtlResult;
}

#[cfg(windows)]
fn get_brightness_igcl() -> HardwareResult<u8> {
    use igcl::*;
    with_igcl_device_pub(|device, lib| unsafe {
        // SAFETY: device is a valid IGCL device handle obtained from ctlEnumerateDevices.
        // CtlBrightnessArgs is POD with correctly set size field; the IGCL function only reads
        // args and writes back the brightness value.
        let get_brightness: libloading::Symbol<FnCtlGetBrightnessSetting> = lib
            .get(b"ctlGetBrightnessSetting\0")
            .context("ctlGetBrightnessSetting")?;
        let mut args = CtlBrightnessArgs {
            size: std::mem::size_of::<CtlBrightnessArgs>() as u32,
            version: 0,
            target_brightness: 0.0,
            smooth_target_brightness: 0.0,
            smooth_time_ms: 0,
        };
        let rc = get_brightness(device as CtlDeviceHandle, &mut args);
        if rc != 0 && (rc & 0x8000_0000) != 0 {
            return Err(HardwareError::Display(format!(
                "ctlGetBrightnessSetting failed: {rc:#x}"
            )));
        }
        if rc != 0 {
            // Intel warning HRESULT (severity bit clear) — treat as success.
            log::debug!(
                "[igcl] ctlGetBrightnessSetting warning rc=0x{rc:08X} — using returned value"
            );
        }
        Ok(args.target_brightness.clamp(0.0, 100.0) as u8)
    })
}

#[cfg(not(windows))]
fn get_brightness_igcl() -> HardwareResult<u8> {
    Err(HardwareError::NotSupported(
        "IGCL not available on non-Windows".to_string(),
    ))
}

#[cfg(windows)]
fn set_brightness_igcl(level: u8) -> HardwareResult<()> {
    use igcl::*;
    with_igcl_device_pub(|device, lib| unsafe {
        // S64: a null device makes EVERY IGCL call a silent no-op. Intel
        // returns a warning HRESULT with the severity bit CLEAR (observed
        // 0x40000001) and writes nothing — the old code treated that as
        // success, so brightness never changed and the WMI fallback was never
        // reached. That is the "brightness control does nothing" regression.
        // Fail loudly instead so the caller falls back to WMI.
        if device.is_null() {
            return Err(HardwareError::Display(
                "IGCL device unavailable (null device) — brightness not applied".into(),
            ));
        }
        // SAFETY: device is a valid IGCL device handle. The brightness args struct is sized
        // correctly and both target_brightness and smooth_target_brightness are initialized.
        let set_brightness: libloading::Symbol<FnCtlSetBrightnessSetting> = lib
            .get(b"ctlSetBrightnessSetting\0")
            .context("ctlSetBrightnessSetting")?;
        let mut args = CtlBrightnessArgs {
            size: std::mem::size_of::<CtlBrightnessArgs>() as u32,
            version: 0,
            target_brightness: level as f64,
            smooth_target_brightness: level as f64,
            smooth_time_ms: 0,
        };
        let rc = set_brightness(device as CtlDeviceHandle, &mut args);
        if rc != 0 && (rc & 0x8000_0000) != 0 {
            return Err(HardwareError::Display(format!(
                "ctlSetBrightnessSetting failed: {rc:#x}"
            )));
        }
        if rc != 0 {
            // Intel warning HRESULT (severity bit clear) — treat as success.
            log::debug!(
                "[igcl] ctlSetBrightnessSetting warning rc=0x{rc:08X} — treated as applied"
            );
        }
        Ok(())
    })
}

#[cfg(not(windows))]
fn set_brightness_igcl(_level: u8) -> HardwareResult<()> {
    Err(HardwareError::NotSupported(
        "IGCL not available on non-Windows".to_string(),
    ))
}

// ── WMI fallback ────────────────────────────────────────────────────────────

fn get_brightness_wmi() -> HardwareResult<u8> {
    #[cfg(windows)]
    {
        use crate::hw::wmi_cache;
        use crate::util::wmi_extract;
        use std::collections::HashMap;

        let res = wmi_cache::with_wmi(|wmi| {
            let results: Vec<HashMap<String, wmi::Variant>> = wmi
                .raw_query("SELECT CurrentBrightness FROM WmiMonitorBrightness")
                .context("WmiMonitorBrightness")?;
            let first = results.first().context("No monitor")?;
            Ok(wmi_extract::extract_u32(first, "CurrentBrightness")
                .map(|v| v as u8)
                .unwrap_or(80))
        });
        res
    }
    #[cfg(not(windows))]
    {
        Ok(80)
    }
}

fn set_brightness_wmi(level: u8) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        // The spawn helper applies CREATE_NO_WINDOW itself; the timeout
        // bounds a hung PowerShell WMI call (tree-killed on expiry).
        const WMI_SET_BRIGHTNESS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
        // WmiSetBrightness requires:
        //  1. Targeting a specific CIM *instance* (not just the class name)
        //  2. Brightness typed as [byte] (UInt8), Timeout as [uint32]
        // Using -ClassName without -InputObject returns "Invalid method Parameter(s)".
        let script = format!(
            "$i=Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightnessMethods; \
             Invoke-CimMethod -InputObject $i -MethodName WmiSetBrightness \
             -Arguments @{{Timeout=[uint32]1;Brightness=[byte]{}}}",
            level
        );
        let mut cmd = std::process::Command::new("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
        let output = process_spawn::run_capture(&mut cmd, WMI_SET_BRIGHTNESS_TIMEOUT)
            .map_err(|e| HardwareError::Display(format!("WmiSetBrightness spawn failed: {e}")))?;
        if !output.success() {
            return Err(HardwareError::Display(format!(
                "WmiSetBrightness exited with {}",
                output.status
            )));
        }
    }
    Ok(())
}

// ── Windows built-in adaptive brightness (ADAPTBRIGHT) ───────────────────────
//
// Windows has its own ALS-based adaptive brightness in the active power plan
// (power setting ADAPTBRIGHT = fbd9aa66-9553-4097-ba44-ed6e9d65eab8).
// When it is enabled it intercepts every brightness request and adjusts the
// value based on its own sensor reading, producing the well-known "caps at 90%"
// symptom where the user sets 100% but Windows immediately dials it back.
// MiControl provides its own, better-calibrated loop, so the two must not run
// concurrently.  This function silently disables ADAPTBRIGHT for the current
// power scheme on both AC and DC.  It is best-effort (no error returned) — if
// powercfg is unavailable the loop still works, just with occasional fighting.
fn disable_windows_adaptive_brightness() {
    #[cfg(windows)]
    {
        // Best-effort: errors are logged, never propagated. The helper
        // applies CREATE_NO_WINDOW and bounds the child wait (tree-kill).
        const POWERCFG_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
        let scheme = "SCHEME_CURRENT";
        let sub = "SUB_VIDEO";
        let guid = "ADAPTBRIGHT";
        for flag in ["/SETACVALUEINDEX", "/SETDCVALUEINDEX"] {
            let mut cmd = std::process::Command::new("powercfg");
            cmd.args([flag, scheme, sub, guid, "0"]);
            if let Err(e) = process_spawn::run_capture(&mut cmd, POWERCFG_TIMEOUT) {
                log::warn!("adaptive_brightness: powercfg {flag} failed: {e}");
            }
        }
        // Activate the scheme so the change takes effect immediately.
        let mut cmd = std::process::Command::new("powercfg");
        cmd.args(["/setactive", scheme]);
        if let Err(e) = process_spawn::run_capture(&mut cmd, POWERCFG_TIMEOUT) {
            log::warn!("adaptive_brightness: powercfg /setactive failed: {e}");
        }
        log::info!("adaptive_brightness: disabled Windows ADAPTBRIGHT (power plan)");
    }
}

// ── HDR state via Windows CCD API ────────────────────────────────────────────
//
// Windows stores HDR (Advanced Color / Wide Color Gamut) state per-display
// in the Connected Displays API (CCD).  We use the windows crate's typed
// bindings for type-safety and correct struct layout.
//
// GetDisplayConfigBufferSizes → sizes → QueryDisplayConfig → paths[] →
// DisplayConfigGetDeviceInfo(GET_ADVANCED_COLOR_INFO) → read bit 1 for HDR on
// DisplayConfigSetDeviceInfo(SET_ADVANCED_COLOR_STATE) → write bit 0 to toggle
//
// None of these calls require administrator privileges.
// A retry loop handles the rare race where display config changes between the
// GetDisplayConfigBufferSizes and QueryDisplayConfig calls (ERROR_INSUFFICIENT_BUFFER).

#[cfg(windows)]
use windows::Win32::Devices::Display::{
    DisplayConfigGetDeviceInfo, DisplayConfigSetDeviceInfo, GetDisplayConfigBufferSizes,
    QueryDisplayConfig, DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
    DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
    DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_DEVICE_INFO_SET_ADVANCED_COLOR_STATE,
    DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
    DISPLAYCONFIG_SET_ADVANCED_COLOR_STATE, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
    DISPLAYCONFIG_TARGET_DEVICE_NAME, QDC_ONLY_ACTIVE_PATHS,
};
#[cfg(windows)]
use windows::Win32::Foundation::{ERROR_INSUFFICIENT_BUFFER, ERROR_SUCCESS};

/// Call GetDisplayConfigBufferSizes + QueryDisplayConfig with retry on
/// ERROR_INSUFFICIENT_BUFFER (display config may change between the two calls).
///
/// # Safety
///
/// This function calls raw Win32 CCD display API functions. The caller must ensure that
/// the returned path/mode vectors are not modified while the underlying display config
/// handle (which does not exist here — this is a one-shot query) remains in use.
#[cfg(windows)]
unsafe fn query_display_config_retry() -> HardwareResult<(
    u32,
    u32,
    Vec<DISPLAYCONFIG_PATH_INFO>,
    Vec<DISPLAYCONFIG_MODE_INFO>,
)> {
    for _ in 0..5 {
        let mut np = 0u32;
        let mut nm = 0u32;
        let rc = GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut np, &mut nm);
        if rc != ERROR_SUCCESS {
            return Err(HardwareError::Display(format!(
                "GetDisplayConfigBufferSizes failed: {}",
                rc.0
            )));
        }
        let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); np as usize];
        let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); nm as usize];
        let rc = QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut np,
            paths.as_mut_ptr(),
            &mut nm,
            modes.as_mut_ptr(),
            None,
        );
        if rc == ERROR_INSUFFICIENT_BUFFER {
            continue; // retry with fresh buffer sizes
        }
        if rc != ERROR_SUCCESS {
            return Err(HardwareError::Display(format!(
                "QueryDisplayConfig failed: {}",
                rc.0
            )));
        }
        return Ok((np, nm, paths, modes));
    }
    Err(HardwareError::Display(
        "QueryDisplayConfig: too many retries (display config keeps changing)".to_string(),
    ))
}

// ── CCD path enumeration ─────────────────────────────────────────────────────
//
// One `QueryDisplayConfig` pass feeds every consumer that needs to know *which*
// physical displays exist: the per-display HDR state below and the "internal
// panel" flag used by the refresh-rate control. A single enumeration keeps the
// two from disagreeing about the topology.

/// One active CCD path, resolved to the GDI device that drives it.
#[cfg(windows)]
#[derive(Debug, Clone)]
struct CcdPath {
    adapter_id: windows::Win32::Foundation::LUID,
    target_id: u32,
    /// GDI source name (`\\.\DISPLAY1`) — joins CCD data to the GDI enumeration.
    gdi_device: String,
    /// Monitor friendly name when Windows exposes one, else empty.
    label: String,
    /// The target is an embedded output (laptop panel).
    is_embedded: bool,
    /// `monitorDevicePath` (`\\?\DISPLAY#EDO4503#4&…`) for the monitor on this
    /// target. Joins CCD to `WmiMonitorID.InstanceName` (EDID panel identity).
    monitor_device_path: String,
}

/// Active display paths with their GDI source name and monitor name.
///
/// Errors are propagated instead of swallowed: a caller that writes (HDR) must
/// be able to tell "the topology could not be read" from "no display supports
/// this", and only the former is retryable.
#[cfg(windows)]
fn ccd_paths() -> HardwareResult<Vec<CcdPath>> {
    // SAFETY: `paths` comes from a successful QueryDisplayConfig; every request
    // struct's header points at its own base address and outlives the call
    // (DisplayConfigGetDeviceInfo does not retain it).
    let (np, _nm, paths, _modes) = unsafe { query_display_config_retry() }?;
    let mut out = Vec::with_capacity(np as usize);
    for path in paths.iter().take(np as usize) {
        let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
            header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                size: std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                adapterId: path.sourceInfo.adapterId,
                id: path.sourceInfo.id,
            },
            ..Default::default()
        };
        if unsafe { DisplayConfigGetDeviceInfo(&mut source.header as *mut _) } != 0 {
            continue;
        }
        let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME {
            header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
                size: std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
                adapterId: path.targetInfo.adapterId,
                id: path.targetInfo.id,
            },
            ..Default::default()
        };
        let (label, is_embedded, monitor_device_path) =
            if unsafe { DisplayConfigGetDeviceInfo(&mut target.header as *mut _) } == 0 {
                (
                    wide_to_string(&target.monitorFriendlyDeviceName),
                    is_embedded_output_technology(target.outputTechnology),
                    wide_to_string(&target.monitorDevicePath),
                )
            } else {
                (String::new(), false, String::new())
            };
        out.push(CcdPath {
            adapter_id: path.targetInfo.adapterId,
            target_id: path.targetInfo.id,
            gdi_device: wide_to_string(&source.viewGdiDeviceName),
            label,
            is_embedded,
            monitor_device_path,
        });
    }
    Ok(out)
}

/// Monitor device instance path (`\\?\DISPLAY#EDO4503#4&…`) per GDI display.
///
/// One CCD pass for every consumer: the colour tab needs it to join a display
/// to its EDID panel (`WmiMonitorID`), and doing that per display would run
/// `QueryDisplayConfig` N times.
pub fn monitor_device_paths() -> std::collections::HashMap<String, String> {
    #[cfg(windows)]
    {
        match ccd_paths() {
            Ok(paths) => paths
                .into_iter()
                .filter(|p| !p.monitor_device_path.is_empty())
                .map(|p| (p.gdi_device, p.monitor_device_path))
                .collect(),
            Err(e) => {
                log::debug!("[edid] CCD monitor device paths unavailable: {e}");
                std::collections::HashMap::new()
            }
        }
    }
    #[cfg(not(windows))]
    {
        std::collections::HashMap::new()
    }
}

/// Per-display Windows advanced-color (HDR / wide colour gamut) state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HdrDisplayState {
    /// GDI device name (`\\.\DISPLAY1`).
    pub device: String,
    /// Monitor friendly name when Windows exposes one, else empty.
    pub label: String,
    /// `advancedColorSupported` — the display can do HDR / WCG.
    pub supported: bool,
    /// `advancedColorEnabled` — HDR is currently on for this display.
    pub enabled: bool,
}

/// `(advancedColorSupported, advancedColorEnabled)` for one CCD target.
///
/// `None` means the target did not answer — unknown, not "unsupported".
#[cfg(windows)]
fn advanced_color_info(
    adapter_id: windows::Win32::Foundation::LUID,
    target_id: u32,
) -> Option<(bool, bool)> {
    let mut info = DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: DISPLAYCONFIG_DEVICE_INFO_GET_ADVANCED_COLOR_INFO,
            size: std::mem::size_of::<DISPLAYCONFIG_GET_ADVANCED_COLOR_INFO>() as u32,
            adapterId: adapter_id,
            id: target_id,
        },
        ..Default::default()
    };
    // SAFETY: the struct is correctly initialised and the header is its own
    // base address; the API only reads it during the call. The union field is
    // the API's bitfield, read immediately after a successful call.
    if unsafe { DisplayConfigGetDeviceInfo(&mut info.header as *mut _) } != 0 {
        return None;
    }
    // Anonymous union bitfield: bit 0 = supported, bit 1 = enabled.
    let value = unsafe { info.Anonymous.value };
    Some((value & 0x1 != 0, value & 0x2 != 0))
}

/// Advanced-color state of every active display (HDR supported / on).
///
/// Read-only and cheap enough for `get_display_info`: one CCD query, one device
/// info call per path. A path whose target does not answer is reported as not
/// supported so the UI can explain why a toggle is off.
pub fn hdr_displays() -> Vec<HdrDisplayState> {
    #[cfg(windows)]
    {
        let Ok(paths) = ccd_paths() else {
            log::warn!("[hdr] display topology unavailable — HDR state unknown");
            return Vec::new();
        };
        let mut out = Vec::with_capacity(paths.len());
        for path in paths {
            // SAFETY: adapter id / target id come from a live CCD path.
            let state = advanced_color_info(path.adapter_id, path.target_id);
            if state.is_none() {
                log::debug!(
                    "[hdr] {} ({}) did not answer GET_ADVANCED_COLOR_INFO",
                    path.gdi_device,
                    path.label
                );
            }
            let (supported, enabled) = state.unwrap_or((false, false));
            out.push(HdrDisplayState {
                device: path.gdi_device,
                label: path.label,
                supported,
                enabled,
            });
        }
        out
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// True when at least one connected display supports HDR (advanced color).
pub fn hdr_supported() -> bool {
    hdr_displays().iter().any(|d| d.supported)
}

/// Read the real HDR (Advanced Color) state — true when *any* display has it on.
pub fn get_hdr_state() -> bool {
    hdr_displays().iter().any(|d| d.enabled)
}

/// One `SET_ADVANCED_COLOR_STATE` outcome for a single display path.
#[derive(Debug, Clone, PartialEq, Eq)]
struct HdrWriteResult {
    device: String,
    /// `advancedColorSupported` for this path.
    supported: bool,
    /// `Some(0)` = applied, `Some(rc)` = Windows refused, `None` = not attempted
    /// (unsupported path, reported for the log).
    rc: Option<i32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct HdrWriteSummary {
    /// Paths that report advanced-color support.
    supported: usize,
    applied: usize,
    failed: usize,
    /// Paths left untouched because they do not support advanced color.
    skipped: usize,
}

/// Short per-path label for the single log line of a HDR toggle.
fn hdr_result_label(result: &HdrWriteResult) -> &'static str {
    match result.rc {
        None => "unsupported",
        Some(0) => "ok",
        Some(_) => "failed",
    }
}

/// Decide the outcome of a HDR toggle from its per-path results.
///
/// `Ok` when at least one *supporting* display accepted the change — a laptop
/// whose panel says yes while two external monitors say no is a success, not an
/// error (the old code returned `Err` for that, so the UI rolled back a change
/// the hardware had already applied). When no connected display supports
/// advanced color the toggle is refused with a message the UI can show verbatim,
/// so the switch cannot pretend to have done something.
fn summarize_hdr_write(results: &[HdrWriteResult]) -> HardwareResult<HdrWriteSummary> {
    let mut summary = HdrWriteSummary::default();
    for result in results {
        if !result.supported {
            summary.skipped += 1;
            continue;
        }
        summary.supported += 1;
        match result.rc {
            Some(0) => summary.applied += 1,
            Some(_) => summary.failed += 1,
            None => {}
        }
    }
    if summary.supported == 0 {
        return Err(HardwareError::NotSupported(
            "no connected display supports HDR (advanced color) — nothing to change".to_string(),
        ));
    }
    if summary.applied == 0 {
        return Err(HardwareError::Display(format!(
            "HDR could not be changed on any of the {} display(s) that support it",
            summary.supported
        )));
    }
    Ok(summary)
}

/// Enable or disable HDR (Advanced Color) on the displays that support it.
///
/// Uses `DisplayConfigSetDeviceInfo` — operates on the current user's
/// interactive session and does NOT require administrator privileges.
///
/// Only paths whose `advancedColorSupported` bit is set are written; the rest
/// are left alone. One `info` line records what happened per display, because
/// this path used to be completely silent (audit §5: no way to prove HDR was
/// ever toggled).
fn set_hdr_ccd(enabled: bool) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        let paths = ccd_paths()?;
        let mut results = Vec::with_capacity(paths.len());
        for path in &paths {
            // SAFETY: adapter id / target id come from a live CCD path, and the
            // SET struct's header points at its own base address.
            let supported = matches!(
                advanced_color_info(path.adapter_id, path.target_id),
                Some((true, _))
            );
            if !supported {
                results.push(HdrWriteResult {
                    device: path.gdi_device.clone(),
                    supported: false,
                    rc: None,
                });
                continue;
            }
            let mut state = DISPLAYCONFIG_SET_ADVANCED_COLOR_STATE {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_SET_ADVANCED_COLOR_STATE,
                    size: std::mem::size_of::<DISPLAYCONFIG_SET_ADVANCED_COLOR_STATE>() as u32,
                    adapterId: path.adapter_id,
                    id: path.target_id,
                },
                ..Default::default()
            };
            // bit 0 = enableAdvancedColor
            state.Anonymous.value = enabled as u32;
            // SAFETY: the struct is initialised above and only read during the call.
            let rc = unsafe { DisplayConfigSetDeviceInfo(&state.header as *const _) };
            results.push(HdrWriteResult {
                device: path.gdi_device.clone(),
                supported: true,
                rc: Some(rc),
            });
        }
        let summary = summarize_hdr_write(&results)?;
        let per_path: Vec<String> = results
            .iter()
            .map(|r| format!("{}={}", r.device, hdr_result_label(r)))
            .collect();
        log::info!(
            "set_hdr({enabled}) → supported={} applied={} failed={} skipped={} [{}]",
            summary.supported,
            summary.applied,
            summary.failed,
            summary.skipped,
            per_path.join(", ")
        );
        if summary.failed > 0 {
            log::warn!(
                "set_hdr({enabled}): {} display(s) refused the change",
                summary.failed
            );
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        log::info!("set_hdr({enabled}) — stub on non-Windows");
        Ok(())
    }
}

// ── Intel PSR2 DRRS (Display Refresh Rate Switching) ─────────────────────────
//
// Intel's PSR2 DRRS is a driver-level feature distinct from the Windows 11
// "Dynamic Refresh Rate" (DRR) API.  It lets the Intel Arc GPU driver
// automatically switch the panel between 60 Hz (idle) and the max rate
// (active content) without Windows involvement.
//
// The Xiaomi laptop BIOS/firmware marks this feature as supported.
// Windows says "Variable refresh rate: Not Supported" because that refers to
// the hardware VRR (FreeSync/G-Sync) capability — a different, faster mechanism.
// PSR2 DRRS works on fixed-rate panels by switching between pre-defined modes.
//
// Controlled via the Intel Arc driver registry key:
// HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-...}\####\Psr2DrrsEnable
//
// Writing requires elevation. Changes take effect after driver restart (brief
// screen flash) or system reboot.

const INTEL_GPU_CLASS: &str =
    r"SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}";
const DRRS_REG_VALUE: &str = "Psr2DrrsEnable";

#[cfg(windows)]
fn find_intel_arc_driver_key() -> Option<String> {
    use crate::util::registry::RegKeyGuard;
    use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
    // Check that the GPU class key exists before iterating subkeys.
    let _ = RegKeyGuard::open_read(HKEY_LOCAL_MACHINE, INTEL_GPU_CLASS).ok()?;
    for i in 0..=9u32 {
        let name = format!("{:04}", i);
        if let Ok(Some(sub)) =
            RegKeyGuard::open_read(HKEY_LOCAL_MACHINE, &format!("{INTEL_GPU_CLASS}\\{name}"))
        {
            if let Ok(Some(desc)) = sub.read_string("DriverDesc") {
                let dl = desc.to_lowercase();
                if dl.contains("intel")
                    && (dl.contains("arc") || dl.contains("uhd") || dl.contains("iris"))
                {
                    return Some(format!("{}\\{}", INTEL_GPU_CLASS, name));
                }
            }
        }
    }
    None
}

/// Read Intel PSR2 DRRS state from the Arc driver registry key.
pub fn get_intel_drrs() -> bool {
    #[cfg(windows)]
    {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
        if let Some(path) = find_intel_arc_driver_key() {
            if let Ok(Some(key)) = RegKeyGuard::open_read(HKEY_LOCAL_MACHINE, &path) {
                if let Ok(Some(v)) = key.read_u32(DRRS_REG_VALUE) {
                    return v != 0;
                }
            }
        }
    }
    true // default: assume enabled when registry is unreadable
}

// ── PSR2 DRRS: "applied — effective after reboot" ────────────────────────────
//
// The registry value is read by the Intel driver when it initialises the panel,
// so a write has NO observable effect until the next boot: Windows keeps
// reporting the configured Hz, and there is no API that exposes the driver's
// actual switching state. Reading Psr2DrrsEnable back therefore proves only that
// the *intent* was stored — the audit found exactly that: reg=1, zero
// observable effect, reboot required.
//
// To stop the UI from claiming "active", the timestamp of the write is persisted
// and compared with the current boot time: while the write belongs to this boot
// session the toggle reads "applied — takes effect after reboot". After a reboot
// the same file answers "not pending" and the value in the registry is what the
// driver loaded at start-up.

const DRRS_STATE_FILE: &str = "psr2_drrs.json";

/// Clock skew allowed between the write and the boot-time estimate (both derive
/// from the wall clock, so a NTP correction must not flip the flag).
const DRRS_CLOCK_SLACK_MS: u64 = 60_000;

#[derive(Debug, Serialize, Deserialize)]
struct DrrsState {
    /// Unix ms at which the registry value was written.
    applied_at_ms: u64,
}

/// `%LOCALAPPDATA%\MiControl\psr2_drrs.json`.
///
/// Tests override this via `MICONTROL_DRRS_STATE_PATH` so they never touch the
/// user's real state.
fn drrs_state_path() -> std::path::PathBuf {
    if let Ok(path) = std::env::var("MICONTROL_DRRS_STATE_PATH") {
        return std::path::PathBuf::from(path);
    }
    let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| ".".into());
    std::path::PathBuf::from(base)
        .join("MiControl")
        .join(DRRS_STATE_FILE)
}

#[cfg(windows)]
fn boot_time_ms() -> Option<u64> {
    // SAFETY: GetTickCount64 is a pure read of the system tick counter.
    let uptime_ms = unsafe { windows::Win32::System::SystemInformation::GetTickCount64() };
    Some(now_ms().saturating_sub(uptime_ms))
}

#[cfg(not(windows))]
fn boot_time_ms() -> Option<u64> {
    None
}

/// True when a DRRS write happened during the current boot session.
///
/// Pure so the boundary is unit-testable without touching the clock: a write
/// at or after the boot instant is still waiting for the driver to reload.
fn drrs_pending_reboot(applied_at_ms: Option<u64>, boot_time_ms: Option<u64>) -> bool {
    match (applied_at_ms, boot_time_ms) {
        (Some(applied), Some(boot)) => applied >= boot.saturating_sub(DRRS_CLOCK_SLACK_MS),
        // Unknown timestamps: do not invent a state.
        _ => false,
    }
}

fn read_drrs_state() -> Option<DrrsState> {
    let raw = std::fs::read_to_string(drrs_state_path()).ok()?;
    serde_json::from_str(&raw).ok()
}

/// Record that the PSR2 DRRS registry value was just written.
///
/// Called from the (unelevated) app process after the elevated bridge succeeds,
/// because the SYSTEM helper's `LOCALAPPDATA` is not the user's.
pub fn mark_drrs_applied() {
    let path = drrs_state_path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let state = DrrsState {
        applied_at_ms: now_ms(),
    };
    match serde_json::to_string(&state) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                log::warn!(
                    "[drrs] could not persist applied state to {}: {e}",
                    path.display()
                );
            }
        }
        Err(e) => log::warn!("[drrs] could not serialise applied state: {e}"),
    }
}

/// True while the DRRS value written this session has not been loaded by the
/// driver yet (i.e. a reboot is still required for it to take effect).
pub fn drrs_pending_reboot_now() -> bool {
    drrs_pending_reboot(read_drrs_state().map(|s| s.applied_at_ms), boot_time_ms())
}

/// Write Intel PSR2 DRRS state to the Arc driver registry key.
/// Requires an elevated (admin) process — called from elevated.rs.
/// Changes take effect after the display driver restarts or system reboots.
pub fn set_intel_drrs(enabled: bool) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
        let path = find_intel_arc_driver_key().ok_or_else(|| {
            HardwareError::Display("Intel Arc driver registry key not found".to_string())
        })?;
        // RegKeyGuard::create_write opens with KEY_ALL_ACCESS which includes KEY_WRITE.
        let key = RegKeyGuard::create_write(HKEY_LOCAL_MACHINE, &path).map_err(|e| {
            HardwareError::Registry(format!("open Intel Arc driver key for write: {e}"))
        })?;
        key.write_u32(DRRS_REG_VALUE, enabled as u32)
            .map_err(|e| HardwareError::Registry(format!("set Psr2DrrsEnable: {e}")))?;
        // The registry is the only observable surface: say so once, with the
        // caveat, so a log reader does not mistake this for a live effect.
        log::info!(
            "[drrs] Psr2DrrsEnable={} written to {path} — applied in the registry, \
             effective only after a reboot/driver restart (not observable by the OS)",
            enabled as u32
        );
    }
    Ok(())
}

// ── Refresh rate (per display device) ─────────────────────────────────────────
//
// Refresh rate is a PER-DISPLAY property, but the old implementation mixed two
// sources: `get_available_refresh_rates()` enumerated the GDI *primary* display
// (`EnumDisplaySettingsExW(NULL)` → the external LG, 3440×1440@85 → [50, 85])
// while `get_refresh_rate()` read `Win32_VideoController` (the internal panel,
// 3120×2080@120). On a laptop + external monitor that produced a UI showing
// "120 Hz" with chips [50, 85] — no chip active — and clicking a chip called
// `ChangeDisplaySettingsExW(NULL)`, i.e. it changed the EXTERNAL monitor while
// the user was looking at the laptop panel. The panel's 60 Hz mode was
// unreachable from the app.
//
// Fix: enumerate every attached GDI device once, mark the internal panel from
// the CCD topology (`outputTechnology`), and route *both* the reads and the
// write through [`select_refresh_target`], so the current rate, the chips and
// the write always describe the same device. Until the UI grows a display
// selector (audit 2.7), the edited display is the laptop's own panel, falling
// back to the GDI primary on desktops.

/// Refresh-rate capabilities of one attached GDI display device.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DisplayRefreshModes {
    /// GDI device name (`\\.\DISPLAY1`) — the handle used by both the read and
    /// the write (`EnumDisplaySettingsExW` / `ChangeDisplaySettingsExW`).
    device: String,
    /// Monitor / adapter description, for logs only.
    label: String,
    /// Refresh rate currently in effect for this device.
    current_hz: u32,
    /// Refresh rates available at the device's current resolution.
    rates: Vec<u32>,
    /// Windows reports an embedded output (CCD `outputTechnology`), i.e. the
    /// laptop's built-in panel.
    is_internal: bool,
    /// This device is the GDI primary (what `EnumDisplaySettingsW(NULL)` means).
    is_primary: bool,
}

/// The display the refresh-rate control edits: the internal panel when there is
/// one, else the GDI primary, else the first attached device.
///
/// Everything that touches refresh rate goes through this function — reading
/// the current rate, listing the supported rates and changing the mode — so the
/// three can never disagree about which monitor they mean.
fn select_refresh_target(modes: &[DisplayRefreshModes]) -> Option<&DisplayRefreshModes> {
    modes
        .iter()
        .find(|m| m.is_internal)
        .or_else(|| modes.iter().find(|m| m.is_primary))
        .or_else(|| modes.first())
}

/// Resolve the device `set_refresh_rate` will write to.
///
/// Pure and shared with the read path (`select_refresh_target`), so a chip can
/// only ever move the display whose rates produced that chip: a rate the target
/// does not advertise is refused instead of being applied to another monitor.
fn resolve_refresh_write(
    modes: &[DisplayRefreshModes],
    hz: u32,
) -> Result<&DisplayRefreshModes, String> {
    let target = select_refresh_target(modes)
        .ok_or_else(|| "no attached display device could be enumerated".to_string())?;
    if !target.rates.contains(&hz) {
        return Err(format!(
            "{hz} Hz is not supported by {} ({}) — available: {:?}",
            target.device, target.label, target.rates
        ));
    }
    Ok(target)
}

/// Enumerate, pick the display the UI edits, and return `(current_hz, rates)`.
///
/// One enumeration for both values: calling two functions that each enumerate
/// their own device is exactly the bug being fixed.
fn refresh_rate_snapshot() -> HardwareResult<(u32, Vec<u32>)> {
    let modes = enumerate_refresh_modes()?;
    let target = select_refresh_target(&modes).ok_or_else(|| {
        HardwareError::Display("no attached display device could be enumerated".to_string())
    })?;
    if target.rates.is_empty() {
        return Err(HardwareError::Display(format!(
            "No refresh rates found for {} at its current mode",
            target.device
        )));
    }
    log::debug!(
        target: "hw::display",
        "refresh-rate source: {} ({}) current={} Hz rates={:?} internal={} primary={}",
        target.device,
        target.label,
        target.current_hz,
        target.rates,
        target.is_internal,
        target.is_primary
    );
    Ok((target.current_hz, target.rates.clone()))
}

/// All Hz values supported by the display the refresh-rate control edits.
///
/// Uses `EnumDisplaySettingsExW` (Win32 GDI) — the same source the Windows
/// Display Settings page uses for its "Choose a refresh rate" dropdown — but
/// per device instead of `NULL` (⇒ GDI primary).
pub fn get_available_refresh_rates() -> HardwareResult<Vec<u32>> {
    refresh_rate_snapshot().map(|(_, rates)| rates)
}

#[cfg(windows)]
fn wide_to_string(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// True for CCD output technologies that are soldered to the machine (laptop
/// panel): internal, eDP, LVDS or UDI-embedded.
#[cfg(windows)]
fn is_embedded_output_technology(
    tech: windows::Win32::Devices::Display::DISPLAYCONFIG_VIDEO_OUTPUT_TECHNOLOGY,
) -> bool {
    use windows::Win32::Devices::Display::{
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED,
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL, DISPLAYCONFIG_OUTPUT_TECHNOLOGY_LVDS,
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED,
    };
    matches!(
        tech,
        DISPLAYCONFIG_OUTPUT_TECHNOLOGY_INTERNAL
            | DISPLAYCONFIG_OUTPUT_TECHNOLOGY_DISPLAYPORT_EMBEDDED
            | DISPLAYCONFIG_OUTPUT_TECHNOLOGY_UDI_EMBEDDED
            | DISPLAYCONFIG_OUTPUT_TECHNOLOGY_LVDS
    )
}

/// CCD paths keyed by GDI device name.
///
/// Joins CCD metadata (monitor friendly name, embedded-panel flag) onto the GDI
/// enumeration. Best effort: if the CCD query fails the map is empty and
/// [`select_refresh_target`] falls back to the GDI primary.
#[cfg(windows)]
fn ccd_devices_by_gdi_name() -> std::collections::HashMap<String, CcdPath> {
    ccd_paths()
        .unwrap_or_default()
        .into_iter()
        .map(|path| (path.gdi_device.clone(), path))
        .collect()
}

/// One display device attached to the desktop, as GDI sees it.
///
/// This is the single per-device enumeration behind the colour tab, the
/// refresh-rate control and the calibration-wizard target — replacing the old
/// `for i in 0..=7` that invented eight displays (audit 2.9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AttachedDisplayDevice {
    /// GDI device name (`DISPLAY1` style, with the `\\.\` prefix).
    pub device: String,
    /// Adapter / monitor description, for display only.
    pub label: String,
    /// CCD reports an embedded output (laptop panel).
    pub is_internal: bool,
    /// GDI primary — what `EnumDisplaySettingsW(NULL)` and `dccw.exe` mean.
    pub is_primary: bool,
}

/// Every display device attached to the desktop, in GDI order.
///
/// Only devices Windows says are attached are returned, so a machine with three
/// monitors yields three rows. The mirroring pseudo-driver is skipped (it has no
/// modes and no panel of its own).
#[cfg(windows)]
fn enumerate_gdi_devices() -> Vec<AttachedDisplayDevice> {
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayDevicesW, DISPLAY_DEVICEW, DISPLAY_DEVICE_ATTACHED_TO_DESKTOP,
        DISPLAY_DEVICE_MIRRORING_DRIVER, DISPLAY_DEVICE_PRIMARY_DEVICE,
    };

    let ccd = ccd_devices_by_gdi_name();
    let mut out = Vec::new();
    let mut idx = 0u32;
    loop {
        // SAFETY: POD struct with `cb` set; EnumDisplayDevicesW writes into it
        // and does not retain the pointer.
        let mut dd = DISPLAY_DEVICEW {
            cb: std::mem::size_of::<DISPLAY_DEVICEW>() as u32,
            ..Default::default()
        };
        if !unsafe { EnumDisplayDevicesW(PCWSTR::null(), idx, &mut dd, 0).as_bool() } {
            break;
        }
        idx += 1;
        if dd.StateFlags & DISPLAY_DEVICE_ATTACHED_TO_DESKTOP == 0 {
            continue;
        }
        if dd.StateFlags & DISPLAY_DEVICE_MIRRORING_DRIVER != 0 {
            continue;
        }
        let device = wide_to_string(&dd.DeviceName);
        let path = ccd.get(&device);
        // Prefer the monitor name from CCD: `DISPLAY_DEVICEW.DeviceString` is the
        // *adapter* ("Intel(R) Arc(TM) B390 GPU") on this machine, which would
        // label every card identically.
        let ccd_label = path.map(|p| p.label.clone()).unwrap_or_default();
        out.push(AttachedDisplayDevice {
            is_internal: path.is_some_and(|p| p.is_embedded),
            is_primary: dd.StateFlags & DISPLAY_DEVICE_PRIMARY_DEVICE != 0,
            label: if ccd_label.is_empty() {
                wide_to_string(&dd.DeviceString)
            } else {
                ccd_label
            },
            device,
        });
    }
    out
}

/// Every display device attached to the desktop (empty on non-Windows).
pub(crate) fn attached_display_devices() -> Vec<AttachedDisplayDevice> {
    #[cfg(windows)]
    {
        enumerate_gdi_devices()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

/// Enumerate the attached display devices with their current and supported
/// refresh rates at the current resolution.
#[cfg(windows)]
fn enumerate_refresh_modes() -> HardwareResult<Vec<DisplayRefreshModes>> {
    use std::collections::HashSet;
    use windows::core::PCWSTR;
    use windows::Win32::Graphics::Gdi::{
        EnumDisplaySettingsExW, DEVMODEW, ENUM_CURRENT_SETTINGS, ENUM_DISPLAY_SETTINGS_FLAGS,
        ENUM_DISPLAY_SETTINGS_MODE,
    };

    let mut out: Vec<DisplayRefreshModes> = Vec::new();
    for dev in enumerate_gdi_devices() {
        // NUL-terminated copy: the PCWSTR must outlive every call below.
        let device: Vec<u16> = dev
            .device
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let name = PCWSTR::from_raw(device.as_ptr());

        // SAFETY: DEVMODEW is a POD with `dmSize` set; the API writes into it
        // and does not retain the pointer. `name` outlives both calls.
        let mut cur = DEVMODEW {
            dmSize: std::mem::size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        if !unsafe {
            EnumDisplaySettingsExW(
                name,
                ENUM_CURRENT_SETTINGS,
                &mut cur,
                ENUM_DISPLAY_SETTINGS_FLAGS(0),
            )
        }
        .as_bool()
        {
            continue;
        }
        let (w, h, bpp) = (cur.dmPelsWidth, cur.dmPelsHeight, cur.dmBitsPerPel);
        let current_hz = cur.dmDisplayFrequency;

        let mut seen: HashSet<u32> = HashSet::new();
        let mut mode_idx = 0u32;
        loop {
            let mut m = DEVMODEW {
                dmSize: std::mem::size_of::<DEVMODEW>() as u16,
                ..Default::default()
            };
            if !unsafe {
                EnumDisplaySettingsExW(
                    name,
                    ENUM_DISPLAY_SETTINGS_MODE(mode_idx),
                    &mut m,
                    ENUM_DISPLAY_SETTINGS_FLAGS(0),
                )
            }
            .as_bool()
            {
                break;
            }
            if m.dmPelsWidth == w
                && m.dmPelsHeight == h
                && m.dmBitsPerPel == bpp
                && m.dmDisplayFrequency > 0
            {
                seen.insert(m.dmDisplayFrequency);
            }
            mode_idx += 1;
        }
        let mut rates: Vec<u32> = seen.into_iter().collect();
        rates.sort_unstable();
        if rates.is_empty() && current_hz > 0 {
            // Devices that report no enumerable modes still have a current one.
            rates.push(current_hz);
        }

        out.push(DisplayRefreshModes {
            is_internal: dev.is_internal,
            is_primary: dev.is_primary,
            label: dev.label,
            device: dev.device,
            current_hz,
            rates,
        });
    }
    Ok(out)
}

#[cfg(not(windows))]
fn enumerate_refresh_modes() -> HardwareResult<Vec<DisplayRefreshModes>> {
    Ok(vec![DisplayRefreshModes {
        device: r"\\.\DISPLAY1".to_string(),
        label: "display".to_string(),
        current_hz: 60,
        rates: vec![60, 120],
        is_internal: true,
        is_primary: true,
    }])
}

/// Change the refresh rate of the display the UI edits.
///
/// The target device is resolved by [`resolve_refresh_write`] — the same
/// selection the current rate and the chips come from — and passed explicitly
/// to `ChangeDisplaySettingsExW`, so a chip can never move a different monitor
/// (the old code passed `NULL` = GDI primary, i.e. the external LG, while the
/// UI showed the internal panel's rate).
///
/// `hz` must be one of the values returned by `get_available_refresh_rates()`;
/// anything else is refused before touching the display. The change is
/// persisted to the registry (`CDS_UPDATEREGISTRY`) so it survives reboots.
pub fn set_refresh_rate(hz: u32) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        use windows::core::PCWSTR;
        use windows::Win32::Graphics::Gdi::{
            ChangeDisplaySettingsExW, EnumDisplaySettingsExW, CDS_TYPE, DEVMODEW,
            DEVMODE_FIELD_FLAGS, DISP_CHANGE, ENUM_CURRENT_SETTINGS, ENUM_DISPLAY_SETTINGS_FLAGS,
        };

        const DM_DISPLAYFREQUENCY: u32 = 0x00400000;
        const CDS_UPDATEREGISTRY_VAL: u32 = 0x00000001;

        let modes = enumerate_refresh_modes()?;
        let target = resolve_refresh_write(&modes, hz).map_err(HardwareError::Display)?;
        // NUL-terminated copy: the PCWSTR must outlive every call below.
        let device: Vec<u16> = target
            .device
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let name = PCWSTR::from_raw(device.as_ptr());
        log::info!(
            "set_refresh_rate: {hz} Hz → {} ({}, currently {} Hz, supported {:?})",
            target.device,
            target.label,
            target.current_hz,
            target.rates
        );

        unsafe {
            // SAFETY: DEVMODEW is POD with dmSize set. EnumDisplaySettingsExW populates the
            // current mode of `device`; we modify dmDisplayFrequency and dmFields before
            // passing it to ChangeDisplaySettingsExW which does not retain the pointer.
            let mut mode = DEVMODEW {
                dmSize: std::mem::size_of::<DEVMODEW>() as u16,
                ..Default::default()
            };
            if !EnumDisplaySettingsExW(
                name,
                ENUM_CURRENT_SETTINGS,
                &mut mode,
                ENUM_DISPLAY_SETTINGS_FLAGS(0),
            )
            .as_bool()
            {
                return Err(HardwareError::Display(format!(
                    "EnumDisplaySettingsExW(CURRENT) failed for {}",
                    target.device
                )));
            }
            mode.dmDisplayFrequency = hz;
            // Tell Windows we're only changing the refresh rate field.
            mode.dmFields = DEVMODE_FIELD_FLAGS(DM_DISPLAYFREQUENCY);

            let result = ChangeDisplaySettingsExW(
                name,
                Some(&mode),
                None,
                CDS_TYPE(CDS_UPDATEREGISTRY_VAL),
                None,
            );
            if result == DISP_CHANGE(0) {
                Ok(())
            } else {
                Err(HardwareError::Display(format!(
                    "ChangeDisplaySettingsExW({}) failed ({result:?}); requested {hz} Hz",
                    target.device
                )))
            }
        }
    }
    #[cfg(not(windows))]
    {
        log::info!("set_refresh_rate({hz}) — stub on non-Windows");
        Ok(())
    }
}

/// Last-resort refresh-rate read, used only when the per-device GDI
/// enumeration fails.
///
/// `Win32_VideoController.CurrentRefreshRate` describes the GPU, not a display:
/// on the measured machine it reported the internal panel (120 Hz) while the
/// GDI primary was the external LG (85 Hz) — the mismatch that made the UI show
/// "120 Hz" over a [50, 85] chip list. It must never be the primary source; it
/// is kept so a failed enumeration still yields a number instead of 0.
fn get_refresh_rate_wmi() -> HardwareResult<u32> {
    #[cfg(windows)]
    {
        use crate::hw::wmi_cache;
        use std::collections::HashMap;

        if let Ok(Some(hz)) = wmi_cache::with_cimv2(|wmi| {
            let results: Vec<HashMap<String, wmi::Variant>> = match wmi
                .raw_query("SELECT CurrentRefreshRate FROM Win32_VideoController")
            {
                Ok(r) => r,
                Err(e) => {
                    log::debug!(target: "hw::display", "Win32_VideoController raw_query error: {e}");
                    return Ok(None);
                }
            };
            if let Some(row) = results.first() {
                match row.get("CurrentRefreshRate") {
                    Some(wmi::Variant::UI4(v)) => Ok(Some(*v)),
                    _ => Ok(None),
                }
            } else {
                Ok(None)
            }
        }) {
            return Ok(hz);
        }
    }
    Ok(120)
}

fn get_ai_brightness_registry() -> HardwareResult<bool> {
    #[cfg(windows)]
    {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
        if let Ok(Some(key)) = RegKeyGuard::open_read(HKEY_LOCAL_MACHINE, AI_BRIGHTNESS_REG_KEY) {
            if let Ok(Some(v)) = key.read_u32(AI_BRIGHTNESS_REG_VALUE) {
                return Ok(v != 0);
            }
        }
        Ok(false)
    }
    #[cfg(not(windows))]
    {
        Ok(false)
    }
}

fn persist_ai_brightness_registry(enabled: bool) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
        let key = RegKeyGuard::create_write(HKEY_LOCAL_MACHINE, AI_BRIGHTNESS_REG_KEY)
            .map_err(|e| HardwareError::Registry(format!("Create display settings key: {e}")))?;
        let val: u32 = if enabled { 1 } else { 0 };
        key.write_u32(AI_BRIGHTNESS_REG_VALUE, val)
            .map_err(|e| HardwareError::Registry(format!("Write AI brightness: {e}")))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serializes tests that share the `COM_LUX_HISTORY` global so parallel
    /// test execution cannot interleave `clear()` with dependent polls.
    ///
    /// The smart-model tests also take this lock: `manual_override_grace_window`
    /// calls `record_user_brightness_override`, which on a machine with a live
    /// ambient-light sensor records a smart sample into the shared model — so
    /// any test touching either the lux history or the smart model must be
    /// serialized together.
    static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Point the smart-model path at a temp file so tests never touch the
    /// user's real learned model, and remove any stale file from a previous
    /// test run.
    fn isolate_smart_model() {
        let temp = std::env::temp_dir().join("micontrol_test_smart_brightness.json");
        std::env::set_var("MICONTROL_SMART_MODEL_PATH", &temp);
        // Isolate the manual-override state too: `record_user_brightness_override`
        // persists it on every call, and unit tests must never rewrite the user's
        // real `%LOCALAPPDATA%\MiControl\brightness_override.json`.
        let override_temp = std::env::temp_dir().join("micontrol_test_brightness_override.json");
        std::env::set_var("MICONTROL_BRIGHTNESS_OVERRIDE_PATH", &override_temp);
        let _ = std::fs::remove_file(&override_temp);
        // C3: persistence goes through the state store. Redirect the whole
        // user state root to a fresh scratch dir per call. The variable is
        // set and never removed (a parallel test may be reading it); every
        // store is one file under `<root>\MiControl\state\`, so concurrent
        // tests never collide with each other's payloads.
        static STATE_ROOT_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = STATE_ROOT_SEQ.fetch_add(1, Ordering::Relaxed);
        let state_root = std::env::temp_dir().join(format!(
            "micontrol_test_state_root_{}_{}",
            std::process::id(),
            seq
        ));
        let _ = std::fs::remove_dir_all(&state_root);
        std::env::set_var("MICONTROL_STATE_ROOT", &state_root);
        // Also isolate any already-cached model so `reset_smart_model()` starts
        // from the (empty) file at the new path.
        if let Some(cache) = SMART_MODEL_CACHE.get() {
            if let Ok(mut guard) = cache.lock() {
                *guard = Some(SmartModel::new());
            }
        }
        let _ = std::fs::remove_file(&temp);
    }

    /// The scratch state root `isolate_smart_model` redirected the store to.
    fn isolated_state_root() -> std::path::PathBuf {
        std::env::var_os("MICONTROL_STATE_ROOT")
            .map(std::path::PathBuf::from)
            .expect("isolate_smart_model must run before this assertion")
    }

    /// Remove a scratch state root at the end of a test (the env var stays
    /// set; the next isolate call hands out a fresh root anyway).
    fn cleanup_state_root(root: &std::path::Path) {
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(windows)]
    #[test]
    fn responsive_sensor_prefers_largest_delta() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Clear any cached history from previous tests so the first poll has no
        // previous values to diff against.
        if let Some(history) = COM_LUX_HISTORY.get() {
            if let Ok(mut guard) = history.lock() {
                guard.clear();
            }
        }
        // First poll with empty history: no deltas, so the last finite
        // plausible reading wins (sensor[1] = 420 lux).
        let first = test_select_responsive(vec![Some(1000.0), Some(420.0)]);
        assert_eq!(first, Some(420.0), "empty history → last finite fallback");
        // Second poll: sensor[1] moved 420→120 (delta 300) — the real one,
        // while the placeholder stayed at 1000. Must win via largest delta.
        let second = test_select_responsive(vec![Some(1000.0), Some(120.0)]);
        assert_eq!(second, Some(120.0));
    }

    #[cfg(windows)]
    #[test]
    fn implausible_values_are_rejected() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // A stuck sensor reporting 1 lux (or a placeholder wandering near 0)
        // must never pass the plausibility floor, even when it is the only
        // finite reading available.
        if let Some(history) = COM_LUX_HISTORY.get() {
            if let Ok(mut guard) = history.lock() {
                guard.clear();
            }
        }
        // Stuck at 1 lux — physically impossible, must be rejected.
        assert_eq!(test_select_responsive(vec![Some(1.0)]), None);
        // Below the floor.
        assert_eq!(test_select_responsive(vec![Some(0.4)]), None);
        // A plausible reading wins over the stuck sensor.
        assert_eq!(
            test_select_responsive(vec![Some(1.0), Some(2.5)]),
            Some(2.5),
            "a plausible reading wins over the stuck sensor"
        );
    }

    #[test]
    fn manual_override_grace_window() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        isolate_smart_model();
        // Clean slate.
        AUTO_OFFSET.store(0, Ordering::Relaxed);
        AUTO_OFFSET_ACTIVE.store(false, Ordering::Relaxed);
        AUTO_LAST_MANUAL_MS.store(0, Ordering::Relaxed);

        // No manual change yet → no grace active.
        assert!(!user_override_in_grace());

        // Record an override → grace becomes active.
        AUTO_LAST_TARGET.store(50, Ordering::Relaxed);
        record_user_brightness_override(80);
        assert!(
            user_override_in_grace(),
            "grace must be armed right after a manual change"
        );

        // After the clock moves past the grace window, grace expires.
        let expired = now_ms().saturating_sub(MANUAL_OVERRIDE_GRACE_MS + 1);
        AUTO_LAST_MANUAL_MS.store(expired, Ordering::Relaxed);
        assert!(
            !user_override_in_grace(),
            "grace must expire after the window"
        );

        // clear resets both offset and grace.
        record_user_brightness_override(60);
        assert!(user_override_in_grace());
        clear_user_brightness_override();
        assert!(!user_override_in_grace());
        assert_eq!(AUTO_OFFSET.load(Ordering::Relaxed), 0);
        assert!(!AUTO_OFFSET_ACTIVE.load(Ordering::Relaxed));
    }

    #[test]
    fn smart_model_learns_and_predicts() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        isolate_smart_model();
        reset_smart_model();
        // Seed the model directly (without touching the sensor).
        record_smart_sample(50.0, 30);
        record_smart_sample(51.0, 32);
        record_smart_sample(52.0, 34);
        record_smart_sample(53.0, 36);
        // All in bucket floor(50/10)=5 → mean = 33, n=4 ≥ 3.
        let predicted = smart_predict_brightness(51.0);
        assert_eq!(
            predicted,
            Some(33),
            "bucket mean (30+32+34+36)/4 = 33 predicted with n=4"
        );
        // Different lux in same bucket → same prediction.
        assert_eq!(smart_predict_brightness(58.0), Some(33));
        // Empty bucket → falls back (None) — loop uses the formula.
        assert_eq!(smart_predict_brightness(200.0), None);
        // Outlier (very different brightness) updates the mean but stays in range.
        record_smart_sample(52.0, 90);
        let predicted2 = smart_predict_brightness(52.0);
        assert!(predicted2.is_some());
        assert!(
            (predicted2.unwrap() as f32 - ((30.0 + 32.0 + 34.0 + 36.0 + 90.0) / 5.0)).abs() < 1.0
        );
        // Std-dev of the bucket (population) can be derived from m2.
        let model = get_smart_model();
        let bin = model.bins.get(&5).expect("bucket 5 exists");
        assert_eq!(bin.n, 5);
        let sd = bin_stddev(bin);
        assert!(
            (sd - 22.89).abs() < 0.5,
            "population std-dev ≈ 22.89, got {sd}"
        );
        reset_smart_model();
    }

    #[test]
    fn smart_model_ignores_implausible_lux() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        isolate_smart_model();
        reset_smart_model();
        // Negative / NaN lux must not create buckets.
        record_smart_sample(-5.0, 50);
        record_smart_sample(f32::NAN, 50);
        record_smart_sample(f32::INFINITY, 50);
        // Still falls back everywhere.
        assert_eq!(smart_predict_brightness(10.0), None);
        let model = get_smart_model();
        assert!(model.bins.is_empty(), "no valid samples added");
        // A valid sample then works.
        record_smart_sample(10.0, 40);
        record_smart_sample(11.0, 42);
        record_smart_sample(12.0, 44);
        assert_eq!(smart_predict_brightness(10.5), Some(42));
        reset_smart_model();
    }

    // ── P10: brightness source state ────────────────────────────────────────

    /// The source string is part of the `DisplayInfo` JSON contract (UI).
    #[test]
    fn brightness_source_names_are_stable() {
        assert_eq!(brightness_source_name(SOURCE_IGCL), "igcl");
        assert_eq!(brightness_source_name(SOURCE_WMI_FALLBACK), "wmi-fallback");
        assert_eq!(brightness_source_name(SOURCE_UNKNOWN), "unknown");
    }

    /// A source change must persist `source` + `reason` so a post-mortem (or
    /// the next session's UI) can explain why brightness is on WMI.
    #[test]
    fn brightness_source_transition_is_persisted_with_reason() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let path = std::env::temp_dir().join("micontrol_test_brightness_source.json");
        let _ = std::fs::remove_file(&path);
        std::env::set_var("MICONTROL_BRIGHTNESS_SOURCE_PATH", &path);

        set_brightness_source(
            SOURCE_WMI_FALLBACK,
            "ctlEnumerateDevices returned 0 devices (test)",
        );

        let data = std::fs::read_to_string(&path).expect("state file must be written");
        let state: BrightnessSourceState =
            serde_json::from_str(&data).expect("state file must be valid JSON");
        assert_eq!(state.source, "wmi-fallback");
        assert!(
            state.reason.contains("0 devices"),
            "reason must survive: {}",
            state.reason
        );
        assert!(state.updated_at_ms > 0);

        // Reset the live state and clean up.
        BRIGHTNESS_SOURCE.store(SOURCE_UNKNOWN, Ordering::Relaxed);
        let _ = std::fs::remove_file(&path);
        std::env::remove_var("MICONTROL_BRIGHTNESS_SOURCE_PATH");
    }

    /// The slow retry is a 10-minute window: IGCL must not be retried on every
    /// brightness write while the driver has no device (P10).
    #[test]
    fn igcl_retry_window_is_ten_minutes_and_blocks_until_then() {
        assert_eq!(IGCL_RETRY_SECS, 600);
        let now = std::time::Instant::now();
        let deadline = now + std::time::Duration::from_secs(IGCL_RETRY_SECS);

        assert!(
            !retry_due(Some(deadline), now),
            "fresh failure blocks retry"
        );
        assert!(
            !retry_due(
                Some(deadline),
                now + std::time::Duration::from_secs(IGCL_RETRY_SECS - 1)
            ),
            "still inside the window"
        );
        assert!(
            retry_due(
                Some(deadline),
                now + std::time::Duration::from_secs(IGCL_RETRY_SECS)
            ),
            "retry becomes due exactly at the deadline"
        );
        assert!(!retry_due(None, now), "no scheduled retry = not due");
    }
    // ── brightness offset clamp (audit: `offset: 54` unclamped; regression:
    //    user intent pinned at +30 pp → top 44 % of the slider reverted) ────

    #[test]
    fn captured_offset_clamp_limits_range() {
        // Offsets captured by the loop keep the anti-runaway ±30 pp bound.
        assert_eq!(clamp_captured_offset(0), 0);
        assert_eq!(clamp_captured_offset(29), 29);
        assert_eq!(clamp_captured_offset(MAX_CAPTURED_OFFSET_PP), 30);
        assert_eq!(clamp_captured_offset(-MAX_CAPTURED_OFFSET_PP), -30);
        // The historical production value (from the capture feedback loop).
        assert_eq!(clamp_captured_offset(54), 30);
        assert_eq!(clamp_captured_offset(-54), -30);
        assert_eq!(clamp_captured_offset(i16::MAX), 30);
        assert_eq!(clamp_captured_offset(i16::MIN), -30);
    }

    #[test]
    fn user_offset_clamp_keeps_full_scale() {
        // Explicit user intent spans the whole 0–100 slider: 26 % curve + 74 pp
        // = 100 %. The old ±30 pp clamp is what made >56 % unreachable.
        assert_eq!(clamp_user_offset(74), 74);
        assert_eq!(clamp_user_offset(54), 54);
        assert_eq!(clamp_user_offset(MAX_USER_OFFSET_PP), 100);
        assert_eq!(clamp_user_offset(-95), -95);
        // Still bounded so a corrupt value cannot overflow the atomic's range
        // or produce nonsense at the point of use.
        assert_eq!(clamp_user_offset(140), 100);
        assert_eq!(clamp_user_offset(-140), -100);
        assert_eq!(clamp_user_offset(i16::MAX), 100);
        assert_eq!(clamp_user_offset(i16::MIN), -100);
    }

    #[test]
    fn effective_target_reaches_100_and_respects_config_range() {
        // Regression case: 29 lux gives raw 26.1 %; the user picks 100 %.
        assert_eq!(effective_target(26.1, 74.0, 5, 100), 100.0);
        // The final clamp still applies — the backlight never exceeds max.
        assert_eq!(effective_target(100.0, 74.0, 5, 100), 100.0);
        assert_eq!(effective_target(26.1, -30.0, 5, 100), 5.0);
        assert_eq!(effective_target(26.1, 0.0, 10, 40), 26.1); // inside range: untouched
        assert_eq!(effective_target(56.1, 0.0, 10, 40), 40.0);
        assert_eq!(effective_target(26.1, 74.0, 10, 40), 40.0);
    }

    #[test]
    fn capture_echo_does_not_undo_a_user_offset() {
        // The regression: the user picks 100 % at 29 lux (raw 26 %) → +74 pp.
        // One iteration later the loop reads 100 % back. Re-deriving the offset
        // with the capture bound would store +30 → 56 %, i.e. the user's choice
        // is thrown away within 2 s, long before the grace window expires.
        let outcome = classify_capture(100, 26, 74, OffsetOrigin::User, true);
        assert!(outcome.echo, "the loop's own output must be recognised");
        assert_eq!(outcome.offset, 74);
        assert_eq!(outcome.origin, OffsetOrigin::User);
        assert_eq!(
            effective_target(26.0, outcome.offset as f32, 5, 100),
            100.0,
            "the user's 100 % must survive the capture pass"
        );
    }

    #[test]
    fn capture_echo_within_tolerance_is_ignored() {
        // Rounding / smoothing / stale read: 98 % instead of 100 %.
        let outcome = classify_capture(98, 26, 74, OffsetOrigin::User, true);
        assert!(outcome.echo);
        assert_eq!(outcome.offset, 74);
    }

    #[test]
    fn capture_of_a_new_value_while_user_offset_active_keeps_full_range() {
        // A genuinely new preference (Fn key / Windows slider) while a user
        // offset is active is recorded as user intent, not clamped to ±30 pp.
        let outcome = classify_capture(60, 26, 74, OffsetOrigin::User, true);
        assert!(!outcome.echo);
        assert_eq!(outcome.offset, 34);
        assert_eq!(outcome.origin, OffsetOrigin::User);
        assert_eq!(effective_target(26.0, outcome.offset as f32, 5, 100), 60.0);
    }

    #[test]
    fn capture_without_user_offset_keeps_the_capture_clamp() {
        // No override yet: this is the loop's feedback path (production reached
        // +54 pp and pinned the curve), so ±30 pp still applies.
        let outcome = classify_capture(100, 26, 0, OffsetOrigin::Captured, false);
        assert!(!outcome.echo);
        assert_eq!(outcome.offset, MAX_CAPTURED_OFFSET_PP);
        assert_eq!(outcome.origin, OffsetOrigin::Captured);

        // Same with a captured offset already active.
        let outcome = classify_capture(100, 26, 20, OffsetOrigin::Captured, true);
        assert!(!outcome.echo);
        assert_eq!(outcome.offset, MAX_CAPTURED_OFFSET_PP);
    }

    #[test]
    fn user_override_offset_is_not_capped_at_the_capture_limit() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        isolate_smart_model();
        AUTO_OFFSET.store(0, Ordering::Relaxed);
        AUTO_OFFSET_ACTIVE.store(false, Ordering::Relaxed);
        AUTO_LAST_MANUAL_MS.store(0, Ordering::Relaxed);

        // 29 lux: raw curve target 26 %, user asks 100 % → +74 pp (not +30).
        AUTO_LAST_TARGET.store(26, Ordering::Relaxed);
        record_user_brightness_override(100);
        assert_eq!(
            AUTO_OFFSET.load(Ordering::Relaxed),
            74,
            "the +74 pp needed to reach 100 % from a 26 % curve must survive"
        );
        assert!(AUTO_OFFSET_ACTIVE.load(Ordering::Relaxed));
        assert_eq!(
            AUTO_OFFSET_ORIGIN.load(Ordering::Relaxed),
            OffsetOrigin::User.as_u8(),
            "an explicit user value is recorded as user intent"
        );
        assert_eq!(
            effective_target(26.0, AUTO_OFFSET.load(Ordering::Relaxed) as f32, 5, 100),
            100.0
        );

        // Negative direction: raw 90 %, user asks 5 % → -85 pp.
        AUTO_LAST_TARGET.store(90, Ordering::Relaxed);
        record_user_brightness_override(5);
        assert_eq!(AUTO_OFFSET.load(Ordering::Relaxed), -85);

        // In-range deltas are untouched.
        AUTO_LAST_TARGET.store(50, Ordering::Relaxed);
        record_user_brightness_override(62);
        assert_eq!(AUTO_OFFSET.load(Ordering::Relaxed), 12);

        clear_user_brightness_override();
    }

    #[test]
    fn captured_offset_keeps_the_anti_runaway_clamp() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // The loop's capture path (external change ≥2 pp) must not inherit the
        // ±100 pp user bound: it is a feedback loop and ratchets.
        let raw: i16 = 26;
        let captured = clamp_captured_offset(100 - raw); // user asked 100 %
        assert_eq!(captured, MAX_CAPTURED_OFFSET_PP);
        // And a captured offset is recorded as such, so a restart re-clamps it.
        AUTO_OFFSET.store(captured, Ordering::Relaxed);
        AUTO_OFFSET_ORIGIN.store(OffsetOrigin::Captured.as_u8(), Ordering::Relaxed);
        assert_eq!(
            clamp_offset_for(AUTO_OFFSET.load(Ordering::Relaxed), OffsetOrigin::Captured),
            30
        );
        AUTO_OFFSET.store(0, Ordering::Relaxed);
        AUTO_OFFSET_ORIGIN.store(OffsetOrigin::Captured.as_u8(), Ordering::Relaxed);
    }

    #[test]
    fn restore_override_state_clamps_legacy_captured_offset() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        isolate_smart_model();
        // C3: the override state lives in the store under the redirected
        // root; the exact-but-legacy pre-store wire is written as the LEGACY
        // file so this test also covers the migration path.
        let state_root = isolated_state_root();
        let legacy = state_store::legacy_brightness_override_path();
        // The exact shape found in production: offset 54, still inside the
        // 120 s grace window, and NO origin field (written before the
        // user/captured split). Every offset those builds persisted came from
        // the capture loop, so it must keep the ±30 pp bound.
        std::fs::create_dir_all(legacy.parent().expect("legacy parent dir")).unwrap();
        std::fs::write(
            &legacy,
            format!(
                "{{\"last_manual_ms\":{},\"offset\":54,\"offset_active\":true}}",
                now_ms()
            ),
        )
        .unwrap();

        AUTO_OFFSET.store(0, Ordering::Relaxed);
        AUTO_OFFSET_ACTIVE.store(false, Ordering::Relaxed);
        AUTO_LAST_MANUAL_MS.store(0, Ordering::Relaxed);

        restore_override_state();

        assert_eq!(
            AUTO_OFFSET.load(Ordering::Relaxed),
            MAX_CAPTURED_OFFSET_PP,
            "a legacy (origin-less) 54 pp offset came from the capture loop and must stay clamped"
        );
        assert_eq!(
            AUTO_OFFSET_ORIGIN.load(Ordering::Relaxed),
            OffsetOrigin::Captured.as_u8()
        );
        assert!(AUTO_OFFSET_ACTIVE.load(Ordering::Relaxed));
        assert!(user_override_in_grace());

        clear_user_brightness_override();
        cleanup_state_root(&state_root);
    }

    #[test]
    fn restore_override_state_keeps_persisted_user_offset() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        isolate_smart_model();
        let state_root = isolated_state_root();
        // C3: seed the STORE (post-migration shape) directly. A user-intent
        // offset (+74 pp = 100 % at 29 lux) must survive a crash/relaunch:
        // the relaunched loop may not trim it back to +30.
        let store = state_store::brightness_store().expect("store from redirected root");
        store
            .save(&state_store::BrightnessOverrideState {
                last_manual_ms: now_ms(),
                offset: 74,
                offset_active: true,
                origin: state_store::BrightnessOffsetOrigin::User,
            })
            .expect("seed store with user offset");

        AUTO_OFFSET.store(0, Ordering::Relaxed);
        AUTO_OFFSET_ORIGIN.store(OffsetOrigin::Captured.as_u8(), Ordering::Relaxed);
        AUTO_OFFSET_ACTIVE.store(false, Ordering::Relaxed);
        AUTO_LAST_MANUAL_MS.store(0, Ordering::Relaxed);

        restore_override_state();

        assert_eq!(
            AUTO_OFFSET.load(Ordering::Relaxed),
            74,
            "a persisted user-intent offset is valid up to ±100 pp"
        );
        assert_eq!(
            AUTO_OFFSET_ORIGIN.load(Ordering::Relaxed),
            OffsetOrigin::User.as_u8()
        );
        assert!(AUTO_OFFSET_ACTIVE.load(Ordering::Relaxed));

        clear_user_brightness_override();
        cleanup_state_root(&state_root);
    }

    /// C3a: a corrupt store file must not take brightness down with it. The
    /// restore logs, starts clean (no offset restored) and the next persist
    /// overwrites the bad file.
    #[test]
    fn restore_override_state_survives_corrupt_store_file() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        isolate_smart_model();
        let state_root = isolated_state_root();
        let store = state_store::brightness_store().expect("store from redirected root");
        std::fs::create_dir_all(store.path().parent().expect("state dir")).unwrap();
        std::fs::write(store.path(), b"not json at all").unwrap();

        AUTO_OFFSET.store(9, Ordering::Relaxed);
        AUTO_OFFSET_ACTIVE.store(false, Ordering::Relaxed);
        AUTO_LAST_MANUAL_MS.store(0, Ordering::Relaxed);

        // Must not panic; must restore nothing (fresh offset state).
        restore_override_state();
        assert_eq!(AUTO_OFFSET.load(Ordering::Relaxed), 9);
        assert!(!AUTO_OFFSET_ACTIVE.load(Ordering::Relaxed));
        assert_eq!(AUTO_LAST_MANUAL_MS.load(Ordering::Relaxed), 0);

        // The next save overwrites the corrupt file and succeeds.
        record_user_brightness_override(80);
        let loaded = store
            .load()
            .expect("reload after overwrite")
            .expect("state present");
        assert!(loaded.offset_active, "corrupt file replaced by valid state");

        clear_user_brightness_override();
        cleanup_state_root(&state_root);
    }

    // ── smart-model prune (audit: 1_315_938 samples) ────────────────────────

    /// A bucket with `n` samples at a fixed value has `m2 = 0`, so decay must
    /// preserve the mean exactly and the variance at 0.
    #[test]
    fn smart_model_prune_caps_hot_bucket() {
        let mut model = SmartModel::new();
        model.bins.insert(
            4,
            SmartBin {
                n: 283_355,
                mean: 76.0,
                m2: 0.0,
                min: 40,
                max: 100,
            },
        );

        assert!(prune_smart_model(&mut model), "prune must report a change");

        let bin = model.bins.get(&4).expect("bucket survives pruning");
        assert!(
            bin.n <= SMART_MAX_SAMPLES_PER_BIN,
            "bucket n={} must be ≤ {}",
            bin.n,
            SMART_MAX_SAMPLES_PER_BIN
        );
        // 283_355 → 141_677 → 70_838 → 35_419 → 17_709 (4 halvings).
        assert_eq!(bin.n, 17_709);
        assert!((bin.mean - 76.0).abs() < 1e-4, "mean preserved by decay");
        assert_eq!(bin.m2, 0.0);
        // Historical extremes are kept for the stats panel.
        assert_eq!((bin.min, bin.max), (40, 100));
    }

    #[test]
    fn smart_model_prune_caps_total_across_buckets() {
        let mut model = SmartModel::new();
        // 40 buckets × 20_000 = 800_000 samples, all below the per-bin cap but
        // far above the 302_400 total cap.
        for idx in 0..40u32 {
            model.bins.insert(
                idx,
                SmartBin {
                    n: 20_000,
                    mean: 50.0,
                    m2: 20_000.0 * 4.0, // sd = 2
                    min: 40,
                    max: 60,
                },
            );
        }
        assert!(prune_smart_model(&mut model));
        let total = model.bins.values().map(|b| b.n).sum::<u32>();
        assert!(
            total <= SMART_MAX_TOTAL_SAMPLES,
            "total {total} must be ≤ {SMART_MAX_TOTAL_SAMPLES}"
        );
        // Shape preserved: the 40 buckets all shrink together (largest-first
        // halving) rather than some being deleted.
        assert_eq!(model.bins.len(), 40);
        for bin in model.bins.values() {
            // Variance (m2 / (n - 1)) unchanged by a balanced halving.
            let var = bin.m2 / (bin.n - 1) as f32;
            assert!((var - 4.0).abs() < 0.05, "variance drifted to {var}");
        }
    }

    #[test]
    fn smart_model_prune_is_noop_within_bounds() {
        let mut model = SmartModel::new();
        model.bins.insert(
            1,
            SmartBin {
                n: 120,
                mean: 33.0,
                m2: 12.0,
                min: 10,
                max: 60,
            },
        );
        assert!(
            !prune_smart_model(&mut model),
            "small model must not be rewritten"
        );
        assert_eq!(model.bins.get(&1).unwrap().n, 120);
    }

    /// The migration path: an oversized file on disk is trimmed and rewritten
    /// when the model is first loaded.
    #[test]
    fn smart_model_prunes_on_load_and_rewrites_file() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        isolate_smart_model();
        let path = std::env::temp_dir().join("micontrol_test_smart_brightness.json");

        // 201 buckets × 6_550 samples ≈ 1.316 M — the production shape.
        let mut json = String::from("{\"schema\":1,\"bins\":{");
        for idx in 0..201u32 {
            if idx > 0 {
                json.push(',');
            }
            json.push_str(&format!(
                "\"{idx}\":{{\"n\":6550,\"mean\":76.0,\"m2\":6550.0,\"min\":0,\"max\":100}}"
            ));
        }
        json.push_str("}}");
        std::fs::write(&path, &json).unwrap();

        // Bust any cached model so `get_smart_model` reloads from disk.
        if let Some(cache) = SMART_MODEL_CACHE.get() {
            if let Ok(mut guard) = cache.lock() {
                *guard = None;
            }
        } else {
            // First initialisation wins the OnceLock; the load still prunes.
        }

        let model = get_smart_model();
        let total = model.bins.values().map(|b| b.n).sum::<u32>();
        assert!(
            total <= SMART_MAX_TOTAL_SAMPLES,
            "loaded model still has {total} samples (cap {SMART_MAX_TOTAL_SAMPLES})"
        );
        // Decay keeps every bucket (and therefore every lux level's shape);
        // it only makes them lighter.
        assert_eq!(model.bins.len(), 201, "decay must not delete buckets");
        assert!(
            model.bins.values().any(|b| b.n < 6550),
            "hot buckets must actually decay"
        );

        // The trimmed model was written back through the store, so the next
        // start reads it small (from the store, not the retired legacy file).
        let store = state_store::Store::<SmartModel>::new(state_store::smart_model_spec_in(
            &isolated_state_root(),
        ));
        let on_disk = store
            .load()
            .expect("store load after prune")
            .expect("pruned model persisted in the store");
        let disk_total = on_disk.bins.values().map(|b| b.n).sum::<u32>();
        assert!(
            disk_total <= SMART_MAX_TOTAL_SAMPLES,
            "file was not rewritten with the pruned model ({disk_total} samples)"
        );
        // The legacy file (same path this test seeded) was retired by the
        // store's save.
        assert!(
            !std::path::Path::new(&path).exists(),
            "legacy smart_brightness.json must be retired after the store save"
        );

        reset_smart_model();
    }

    // ── anti-feedback guard (audit: loop learns from its own output) ────────

    #[test]
    fn external_capture_is_not_learned_while_offset_is_active() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        isolate_smart_model();
        reset_smart_model();

        // Override active → the captured brightness already contains our offset,
        // so it must not become a training sample.
        AUTO_OFFSET.store(25, Ordering::Relaxed);
        AUTO_OFFSET_ACTIVE.store(true, Ordering::Relaxed);
        record_external_change_sample(50.0, 80);
        assert!(
            get_smart_model().bins.is_empty(),
            "no sample may be learned while brightness_override is active"
        );

        // No override → a genuine external change (Fn key, Windows slider) is
        // exactly what the model should learn.
        AUTO_OFFSET.store(0, Ordering::Relaxed);
        AUTO_OFFSET_ACTIVE.store(false, Ordering::Relaxed);
        record_external_change_sample(50.0, 80);
        let model = get_smart_model();
        assert_eq!(model.total_samples(), 1);
        assert_eq!(model.bins.get(&5).map(|b| b.n), Some(1));

        reset_smart_model();
    }

    // ── refresh-rate device selection (audit 2.7: chips changed the wrong
    //    monitor because current and rates came from different displays) ────

    /// The machine as measured in the audit: internal panel `\\.\DISPLAY1`
    /// 3120×2080 @120 (rates [60, 120]) and the *external* LG ULTRAGEAR as the
    /// GDI primary @85 (rates [50, 85]). This is the exact layout that made the
    /// UI show "120 Hz" with chips [50, 85] and no active chip.
    fn measured_refresh_layout() -> Vec<DisplayRefreshModes> {
        vec![
            DisplayRefreshModes {
                device: r"\\.\DISPLAY3".to_string(),
                label: "LG ULTRAGEAR".to_string(),
                current_hz: 85,
                rates: vec![50, 85],
                is_internal: false,
                is_primary: true,
            },
            DisplayRefreshModes {
                device: r"\\.\DISPLAY1".to_string(),
                label: "EDO".to_string(),
                current_hz: 120,
                rates: vec![60, 120],
                is_internal: true,
                is_primary: false,
            },
            DisplayRefreshModes {
                device: r"\\.\DISPLAY2".to_string(),
                label: "LG ULTRAWIDE".to_string(),
                current_hz: 60,
                rates: vec![50, 60, 75],
                is_internal: false,
                is_primary: false,
            },
        ]
    }

    #[test]
    fn refresh_target_is_the_internal_panel_not_the_gdi_primary() {
        let modes = measured_refresh_layout();
        let target = select_refresh_target(&modes).expect("an attached display");
        assert_eq!(target.device, r"\\.\DISPLAY1");
        assert_eq!(target.current_hz, 120);
        assert_eq!(target.rates, vec![60, 120]);
        // The bug in one line: the current rate must be one of the offered
        // rates (it used to be 120 Hz over a [50, 85] chip list).
        assert!(
            target.rates.contains(&target.current_hz),
            "current rate {} Hz must appear among {:?}",
            target.current_hz,
            target.rates
        );
    }

    #[test]
    fn refresh_target_falls_back_to_gdi_primary_without_internal_panel() {
        let modes: Vec<DisplayRefreshModes> = measured_refresh_layout()
            .into_iter()
            .map(|mut m| {
                m.is_internal = false;
                m
            })
            .collect();
        let target = select_refresh_target(&modes).expect("an attached display");
        assert_eq!(target.device, r"\\.\DISPLAY3");
        assert_eq!(target.current_hz, 85);
        assert_eq!(target.rates, vec![50, 85]);
        assert!(target.rates.contains(&target.current_hz));
    }

    #[test]
    fn refresh_target_without_any_attached_device_is_none() {
        assert!(select_refresh_target(&[]).is_none());
        assert!(resolve_refresh_write(&[], 60).is_err());
    }

    #[test]
    fn refresh_write_targets_the_same_display_the_ui_shows() {
        let modes = measured_refresh_layout();
        let shown = select_refresh_target(&modes).expect("an attached display");
        let shown_device = shown.device.clone();
        let shown_rates = shown.rates.clone();

        // A rate the edited display advertises is written to that display.
        let plan = resolve_refresh_write(&modes, 60).expect("60 Hz is supported by the panel");
        assert_eq!(plan.device, shown_device);
        assert!(shown_rates.contains(&60));

        // The old chips (50/85 Hz, from the external LG) must be refused, not
        // silently applied to the external monitor.
        for hz in [50u32, 85] {
            let err = resolve_refresh_write(&modes, hz).unwrap_err();
            assert!(
                err.contains("is not supported by") && err.contains(r"\\.\DISPLAY1"),
                "unexpected error for {hz} Hz: {err}"
            );
        }
    }

    /// Read-only smoke test against the live topology: never writes a mode, but
    /// proves the enumeration returns coherent data on the machine running the
    /// tests (skips silently on a headless host with no attached displays).
    #[test]
    fn live_refresh_enumeration_is_coherent() {
        let modes = enumerate_refresh_modes().expect("enumeration must not fail");
        for m in &modes {
            // Diagnostic only: visible with `cargo test -- --nocapture`.
            println!(
                "{} '{}' {} Hz {:?} internal={} primary={}",
                m.device, m.label, m.current_hz, m.rates, m.is_internal, m.is_primary
            );
        }
        if modes.is_empty() {
            return; // headless CI runner
        }
        for m in &modes {
            assert!(!m.device.is_empty());
            assert!(
                m.rates.is_empty() || m.rates.contains(&m.current_hz),
                "{} current {} Hz not in {:?}",
                m.device,
                m.current_hz,
                m.rates
            );
        }
        if let Some(target) = select_refresh_target(&modes) {
            assert!(
                !target.rates.is_empty(),
                "the edited display {} must advertise at least one rate",
                target.device
            );
        }
    }

    // ── HDR: check advanced-color support before writing (audit 2.5) ─────────

    fn hdr_path(device: &str, supported: bool, rc: Option<i32>) -> HdrWriteResult {
        HdrWriteResult {
            device: device.to_string(),
            supported,
            rc,
        }
    }

    #[test]
    fn hdr_write_is_refused_when_no_display_supports_it() {
        let results = [
            hdr_path("DISPLAY1", false, None),
            hdr_path("DISPLAY2", false, None),
        ];
        let err = summarize_hdr_write(&results).unwrap_err();
        assert!(
            matches!(&err, HardwareError::NotSupported(_)),
            "expected NotSupported, got {err:?}"
        );
        assert!(
            err.to_string()
                .contains("no connected display supports HDR"),
            "message must name the reason: {err}"
        );
    }

    #[test]
    fn hdr_write_succeeds_when_one_display_of_three_accepts_it() {
        // The audit's machine: internal panel + two monitors, only one path
        // supports advanced color. The old code returned Err because the other
        // two failed, so the UI rolled back a change the panel had applied.
        let results = [
            hdr_path("DISPLAY1", true, Some(0)),
            hdr_path("DISPLAY2", false, None),
            hdr_path("DISPLAY3", false, None),
        ];
        let summary = summarize_hdr_write(&results).expect("one supporting display is enough");
        assert_eq!(summary.supported, 1);
        assert_eq!(summary.applied, 1);
        assert_eq!(summary.skipped, 2);
        assert_eq!(summary.failed, 0);
    }

    #[test]
    fn hdr_write_reports_failure_only_when_every_supported_display_refused() {
        let denied = 0x8007_0005u32 as i32; // E_ACCESSDENIED
        let mixed = [
            hdr_path("DISPLAY1", true, Some(0)),
            hdr_path("DISPLAY2", true, Some(denied)),
        ];
        let summary = summarize_hdr_write(&mixed).expect("one display was changed");
        assert_eq!(summary.applied, 1);
        assert_eq!(summary.failed, 1);

        let all_failed = [hdr_path("DISPLAY1", true, Some(denied))];
        let err = summarize_hdr_write(&all_failed).unwrap_err();
        assert!(matches!(&err, HardwareError::Display(_)), "got {err:?}");
    }

    #[test]
    fn hdr_log_labels_distinguish_skipped_from_failed() {
        assert_eq!(hdr_result_label(&hdr_path("D", false, None)), "unsupported");
        assert_eq!(hdr_result_label(&hdr_path("D", true, Some(0))), "ok");
        assert_eq!(
            hdr_result_label(&hdr_path("D", true, Some(0x8007_0005u32 as i32))),
            "failed"
        );
    }

    /// Read-only: one HDR entry per active CCD path, each joined to a GDI
    /// device name. Never writes a state. Skips silently on a headless host.
    #[test]
    fn live_hdr_enumeration_is_per_display() {
        let devices = attached_display_devices();
        let hdr = hdr_displays();
        println!(
            "attached={} ccd_paths={} hdr_supported_any={}",
            devices.len(),
            hdr.len(),
            hdr.iter().any(|d| d.supported)
        );
        for d in &hdr {
            println!(
                "  {} '{}' supported={} enabled={}",
                d.device, d.label, d.supported, d.enabled
            );
        }
        if devices.is_empty() {
            return; // headless CI runner
        }
        assert!(
            !hdr.is_empty(),
            "CCD returned no path for {} attached display(s)",
            devices.len()
        );
        for d in &hdr {
            assert!(
                d.device.contains("DISPLAY"),
                "CCD path is not joined to a GDI device: {d:?}"
            );
        }
        // No duplicate devices: the aggregate flag must not be inflated.
        let mut names: Vec<&str> = devices.iter().map(|d| d.device.as_str()).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), devices.len(), "duplicate GDI devices");
    }

    // ── PSR2 DRRS: "applied, effective after reboot" (audit 2.8) ────────────

    #[test]
    fn drrs_write_is_pending_until_the_next_boot() {
        let boot = 10_000_000u64;
        // Written ten minutes into this boot: the driver has not reloaded it.
        assert!(drrs_pending_reboot(Some(boot + 600_000), Some(boot)));
        // Written just before the boot-time estimate (wall-clock noise).
        assert!(drrs_pending_reboot(Some(boot - 200), Some(boot)));
    }

    #[test]
    fn drrs_write_stops_pending_after_a_reboot() {
        let boot = 10_000_000u64;
        // Written two hours before this boot: the driver loaded it at start-up.
        assert!(!drrs_pending_reboot(Some(boot - 7_200_000), Some(boot)));
        // Unknown timestamps never invent a state.
        assert!(!drrs_pending_reboot(None, Some(boot)));
        assert!(!drrs_pending_reboot(Some(boot + 1), None));
    }

    // ── Ambient sensor states: Available / NoSensor / FrozenReport ───────────

    #[test]
    fn sensor_probe_classification() {
        // Fresh, plausible lux → Available.
        assert_eq!(
            classify_sensor_probe(Some(120.0), true),
            AmbientSensorState::Available
        );
        // Fresh lux wins even before the devices flag is set.
        assert_eq!(
            classify_sensor_probe(Some(120.0), false),
            AmbientSensorState::Available
        );
        // Devices enumerate but no fresh data → FrozenReport (the TCS3408 case:
        // 0.0 lux placeholder, frozen timestamp).
        assert_eq!(
            classify_sensor_probe(Some(0.0), true),
            AmbientSensorState::FrozenReport
        );
        assert_eq!(
            classify_sensor_probe(None, true),
            AmbientSensorState::FrozenReport
        );
        // Nothing enumerated at all → NoSensor.
        assert_eq!(
            classify_sensor_probe(None, false),
            AmbientSensorState::NoSensor
        );
        // A NaN reading is not fresh data.
        assert_eq!(
            classify_sensor_probe(Some(f32::NAN), false),
            AmbientSensorState::NoSensor
        );
        // The placeholder at/under the plausibility floor with devices present
        // is frozen, not available.
        assert_eq!(
            classify_sensor_probe(Some(MIN_PLAUSIBLE_LUX), true),
            AmbientSensorState::FrozenReport
        );
    }

    #[test]
    fn sensor_state_code_roundtrip() {
        for state in [
            AmbientSensorState::Available,
            AmbientSensorState::NoSensor,
            AmbientSensorState::FrozenReport,
            AmbientSensorState::Unknown,
        ] {
            assert_eq!(
                AmbientSensorState::from_code(state.code()),
                state,
                "{state:?} code must round-trip"
            );
        }
        // Distinct codes: the atomic cannot alias two states.
        let codes = [
            AmbientSensorState::Available,
            AmbientSensorState::NoSensor,
            AmbientSensorState::FrozenReport,
            AmbientSensorState::Unknown,
        ]
        .map(|s| s.code());
        for i in 0..codes.len() {
            for j in (i + 1)..codes.len() {
                assert_ne!(codes[i], codes[j]);
            }
        }
    }

    #[test]
    fn diagnosed_states_suppress_probe_and_resets() {
        // Suppression window: a diagnosed sensor is re-probed at most once
        // per window; Unknown/Available are never suppressed.
        assert!(!probe_suppressed(
            AmbientSensorState::Available,
            1_000,
            2_000
        ));
        assert!(!probe_suppressed(AmbientSensorState::Unknown, 1_000, 2_000));
        // Never probed: the first diagnosis must be real.
        assert!(!probe_suppressed(
            AmbientSensorState::FrozenReport,
            0,
            1_000_000
        ));
        let last = 1_000_000u64;
        let window_ms = SENSOR_DIAGNOSED_REPROBE_SECS * 1000;
        assert!(probe_suppressed(
            AmbientSensorState::FrozenReport,
            last,
            last + window_ms - 1
        ));
        assert!(!probe_suppressed(
            AmbientSensorState::FrozenReport,
            last,
            last + window_ms
        ));
        assert!(!probe_suppressed(
            AmbientSensorState::NoSensor,
            last,
            last + window_ms
        ));
    }

    #[test]
    fn reset_requests_stop_for_diagnosed_sensors() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let previous = SENSOR_STATE.load(Ordering::Relaxed);
        SENSOR_RESET_REQUESTED.store(false, Ordering::SeqCst);
        // FrozenReport: every 5 cycles the loop used to re-request a reset
        // that provably could not help. It must be refused.
        SENSOR_STATE.store(AmbientSensorState::FrozenReport.code(), Ordering::Relaxed);
        request_sensor_reset("unit test frozen");
        assert!(
            !SENSOR_RESET_REQUESTED.load(Ordering::SeqCst),
            "a FrozenReport sensor must not receive reset requests"
        );
        // NoSensor: nothing exists to reset.
        SENSOR_STATE.store(AmbientSensorState::NoSensor.code(), Ordering::Relaxed);
        request_sensor_reset("unit test no sensor");
        assert!(!SENSOR_RESET_REQUESTED.load(Ordering::SeqCst));
        // Available/Unknown: reset requests keep working (real recovery path).
        SENSOR_STATE.store(AmbientSensorState::Available.code(), Ordering::Relaxed);
        request_sensor_reset("unit test available");
        assert!(SENSOR_RESET_REQUESTED.load(Ordering::SeqCst));
        // Restore the global for the other tests in this binary.
        SENSOR_STATE.store(previous, Ordering::Relaxed);
        SENSOR_RESET_REQUESTED.store(false, Ordering::SeqCst);
    }

    #[test]
    fn last_fresh_stamp_persists_at_most_once_per_hour() {
        assert!(
            should_persist_fresh_stamp(0, 1_000),
            "never persisted → write"
        );
        let persisted = 10_000_000u64;
        assert!(!should_persist_fresh_stamp(persisted, persisted + 60_000));
        assert!(should_persist_fresh_stamp(persisted, persisted + 3_600_000));
    }

    // ── Curve/smart mode persistence ──────────────────────────────────────────

    #[test]
    fn mode_from_registry_dword_maps_known_values() {
        assert_eq!(mode_from_registry_dword(0), "curve");
        assert_eq!(mode_from_registry_dword(1), "smart");
        // Anything else is not a mode the UI offers; curve is the default.
        assert_eq!(mode_from_registry_dword(7), "curve");
    }

    #[test]
    fn brightness_mode_persists_across_restart() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        isolate_smart_model();
        reset_brightness_mode_cache_for_tests();

        // The user picks Smart in the UI: the config save persists the mode.
        save_brightness_mode("smart").expect("mode save");
        // The store file exists under the redirected state root.
        let root = isolated_state_root();
        let store_path = state_store::StoreRegistry::resolve_in(
            state_store::StoreRoot::User,
            &root,
            "brightness_mode",
        );
        assert!(
            store_path.exists(),
            "store file missing: {}",
            store_path.display()
        );

        // RESTART: a new process has an empty cache; the mode must come back
        // from the store, not from the (unwritable-by-users) HKLM registry.
        reset_brightness_mode_cache_for_tests();
        assert_eq!(
            brightness_mode_cached(),
            "smart",
            "smart mode must survive a restart"
        );
        // And the raw store agrees.
        let store = state_store::brightness_mode_store_in(&root);
        let loaded = store.load().expect("reload store").expect("mode present");
        assert_eq!(loaded.mode, "smart");

        // Switching back to Formula persists too.
        save_brightness_mode("curve").expect("mode save curve");
        reset_brightness_mode_cache_for_tests();
        assert_eq!(brightness_mode_cached(), "curve");
        // Leave the cache clean for the other tests in this binary.
        reset_brightness_mode_cache_for_tests();
        cleanup_state_root(&root);
    }

    #[test]
    fn set_config_persists_the_mode_before_any_registry_write() {
        // Order proof for the production defect: the mode is written to the
        // store BEFORE the HKLM writes (the mirror + min/max/sens/smth) that
        // a non-elevated process cannot perform. Exercised through the real
        // config path, then read back from the store itself.
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        isolate_smart_model();
        reset_brightness_mode_cache_for_tests();
        set_ai_brightness_config(AiBrightnessConfig {
            enabled: false,
            min_brightness: 10,
            max_brightness: 100,
            sensitivity: 100,
            smoothing: 30,
            mode: "smart".into(),
        })
        .unwrap_or_else(|e| {
            // An HKLM refusal (non-elevated host) is acceptable AFTER the
            // mode save; anything else is a real regression.
            let msg = e.to_string();
            assert!(
                msg.contains("display settings key") || msg.contains("AI brightness"),
                "unexpected config failure (the mode save must have succeeded first): {e}"
            );
            log::debug!("HKLM write refused in the test host: {e}");
        });
        let root = isolated_state_root();
        let store = state_store::brightness_mode_store_in(&root);
        let loaded = store.load().expect("reload store").expect("mode present");
        assert_eq!(
            loaded.mode, "smart",
            "the mode must persist in the store even when the HKLM writes are refused"
        );
        reset_brightness_mode_cache_for_tests();
        cleanup_state_root(&root);
    }

    // ── Smart-model prune: production size fits, oversize is trimmed ──────────

    /// The production model shape measured on 2026-11-21: 201 buckets,
    /// 295 920 samples, 15 441 B serialized. 200 buckets at 1 472 samples
    /// plus one at 1 520.
    fn production_shaped_model() -> SmartModel {
        let mut model = SmartModel::new();
        for idx in 0..200u32 {
            model.bins.insert(
                idx * 10,
                SmartBin {
                    n: 1_472,
                    mean: 42.0,
                    m2: 100.0,
                    min: 30,
                    max: 55,
                },
            );
        }
        model.bins.insert(
            2_000,
            SmartBin {
                n: 1_520,
                mean: 60.0,
                m2: 200.0,
                min: 45,
                max: 80,
            },
        );
        model
    }

    #[test]
    fn prune_leaves_the_production_sized_model_untouched() {
        let mut model = production_shaped_model();
        assert_eq!(model.total_samples(), 295_920);
        assert_eq!(model.bins.len(), 201);
        assert!(
            !prune_smart_model(&mut model),
            "a model inside both caps (295 920 ≤ 302 400 total, every bucket ≤ 20 000) \
             must not be decayed"
        );
        assert_eq!(model.total_samples(), 295_920, "no samples lost");
        assert_eq!(model.bins.len(), 201, "no buckets dropped");
        // The serialized payload fits the store ceiling with room to spare
        // (production measured 15 441 B against a 64 KiB max_bytes).
        let bytes = serde_json::to_vec(&model).expect("serialize").len() as u64;
        assert!(
            bytes < state_store::DEFAULT_STATE_MAX_BYTES,
            "serialized model is {bytes} B, must stay under the {} B store ceiling",
            state_store::DEFAULT_STATE_MAX_BYTES
        );
        assert!(bytes >= 10_000, "shape sanity: got {bytes} B");
    }

    #[test]
    fn prune_decays_a_hot_bucket_in_exact_halvings() {
        // The audit's hot bucket: 283 355 samples at 4 lux. The per-bin cap
        // is 20 000: 283 355 → 141 677 → 70 838 → 35 419 → 17 709 (four
        // halvings), mean and variance preserved.
        let mut model = SmartModel::new();
        model.bins.insert(
            0,
            SmartBin {
                n: 283_355,
                mean: 4.0,
                m2: 8_000.0,
                min: 0,
                max: 30,
            },
        );
        assert!(prune_smart_model(&mut model));
        let bin = model.bins.get(&0).expect("hot bucket kept");
        assert_eq!(bin.n, 17_709);
        assert!(bin.n <= SMART_MAX_SAMPLES_PER_BIN);
        assert_eq!(bin.mean, 4.0, "decay must not move the bucket mean");
    }

    #[test]
    fn prune_pulls_the_oversized_audit_model_inside_both_caps() {
        // Audit shape: 1 315 938-ish samples, one hot bucket 283 355, the
        // rest spread across 200 buckets.
        let mut model = SmartModel::new();
        model.bins.insert(
            0,
            SmartBin {
                n: 283_355,
                mean: 4.0,
                m2: 8_000.0,
                min: 0,
                max: 30,
            },
        );
        for idx in 1..=200u32 {
            model.bins.insert(
                idx * 10,
                SmartBin {
                    n: 5_163,
                    mean: 40.0,
                    m2: 120.0,
                    min: 20,
                    max: 70,
                },
            );
        }
        let before = model.total_samples();
        assert_eq!(before, 283_355 + 200 * 5_163);
        assert!(prune_smart_model(&mut model), "oversized model must change");
        assert!(
            model.total_samples() <= SMART_MAX_TOTAL_SAMPLES,
            "total {} exceeds the {} cap",
            model.total_samples(),
            SMART_MAX_TOTAL_SAMPLES
        );
        assert_eq!(model.bins.len(), 201, "decay never deletes buckets");
        for (idx, bin) in &model.bins {
            assert!(
                bin.n <= SMART_MAX_SAMPLES_PER_BIN,
                "bucket {idx} holds {} samples over the {} cap",
                bin.n,
                SMART_MAX_SAMPLES_PER_BIN
            );
            // Decay preserves the learned mean per bucket: the model keeps
            // its shape (which buckets are hot), only the mass shrinks.
            let expected = if *idx == 0 { 4.0 } else { 40.0 };
            assert_eq!(bin.mean, expected, "bucket {idx} mean drifted");
        }
    }

    #[test]
    fn record_smart_sample_keeps_a_long_session_inside_the_caps() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        isolate_smart_model();
        // Seed the cache with a model already over the per-bin cap, as if it
        // had grown past it in a long-lived session (the load-time migration
        // only runs at startup).
        let cache = SMART_MODEL_CACHE.get_or_init(|| std::sync::Mutex::new(None));
        {
            let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
            *guard = Some(SmartModel {
                schema: 1,
                bins: [(
                    4u32,
                    SmartBin {
                        n: 302_401,
                        mean: 42.0,
                        m2: 100.0,
                        min: 20,
                        max: 70,
                    },
                )]
                .into_iter()
                .collect(),
            });
        }
        record_smart_sample(45.0, 50);
        let model = get_smart_model();
        assert!(
            model.total_samples() <= SMART_MAX_TOTAL_SAMPLES,
            "session growth must be pruned: total {}",
            model.total_samples()
        );
        for (idx, bin) in &model.bins {
            assert!(
                bin.n <= SMART_MAX_SAMPLES_PER_BIN,
                "bucket {idx} holds {} samples over the cap",
                bin.n
            );
        }
        // The persisted file fits the store ceiling too.
        let path = state_store::smart_model_spec().expect("spec").path;
        let size = std::fs::metadata(&path)
            .map(|m| m.len())
            .unwrap_or_default();
        assert!(
            size < state_store::DEFAULT_STATE_MAX_BYTES,
            "persisted smart model is {size} B, over the {} B ceiling",
            state_store::DEFAULT_STATE_MAX_BYTES
        );
        cleanup_state_root(&isolated_state_root());
    }
}
