//! Cast device enumeration with real state and bounded scans, plus the
//! honest attempt to start a Miracast/cast session from code.
//!
//! Ground truth on THIS machine (probed 2026-11-21, see probes in the commit):
//! - `CastingDevice::GetDeviceSelector(...)` returns a WORKING AQS filter.
//!   `FindAllAsync` over it takes ~30 s wall clock on this build (the
//!   enumeration completes but the property-store priming is slow), while a
//!   WATCHER over the same selector delivers rows and completes reliably.
//! - The Miracast AEP selector (ProtocolId AA754C9A) returns ZERO rows here
//!   even with the TV paired; Miracast sinks surface through the CONTAINER
//!   selector instead.
//! - `System.Devices.Aep.IsConnected` is readable on the AEP rows but returns
//!   null on this build when queried via FindAllAsync (property-store gap);
//!   the Connect panel itself does not expose session state through it.
//! - `CastingConnection` can only be created from a `CastingDevice` and can
//!   only be started with a `CastingSource`. There is NO public constructor
//!   for `CastingSource` (neither `new` in windows-rs nor a WinRT activatable
//!   factory); the only producer is `MediaPlayer::GetAsCastingSource()`.
//!   Pushing an EMPTY source through `RequestStartCastingAsync` on real
//!   devices (Redmi Buds, Samsung TV, Sony TV) returns
//!   `InvalidCastingSource` — Windows refuses desktop mirroring without a
//!   media source. This is the proven boundary: the app cannot start a
//!   screen-mirroring session via API, and the Windows panel (Win+K) stays
//!   the supported path for that.
//! - `Windows.Media.Miracast` (MiracastTransmitter/Receiver) is the
//!   RECEIVER-side API (implements a sink), not a sender API.

use crate::hw::errors::{HardwareError, HardwareResult};
use crate::util::process_spawn;
use serde::{Deserialize, Serialize};

/// A hung PowerShell must not hold the stop-casting command forever; the
/// helper tree-kills the child on expiry.
const PS_CLOSE_PANEL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

/// Bounded discovery window for one DeviceWatcher. Watchers on this host
/// deliver rows and complete within ~30 s at worst; 12 s is what the UI wait
/// accepts, and the report says honestly whether it completed.
const SCAN_WINDOW: std::time::Duration = std::time::Duration::from_secs(35);

/// A Miracast/WiDi receiver device with live state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CastDevice {
    pub name: String,
    pub id: String,
    pub device_type: String,
    /// Real connection state when the device reports it, `None` when the
    /// platform does not expose it for this row.
    pub connected: Option<bool>,
}

/// Result of a cast operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CastResult {
    pub success: bool,
    pub message: String,
}

/// Report of one bounded discovery run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CastScanReport {
    pub duration_ms: u64,
    pub devices: Vec<CastDevice>,
    pub added_events: usize,
    pub updated_events: usize,
    /// True only when the watchers completed INSIDE the window; false means
    /// the list may be partial (Windows is still discovering).
    pub completed: bool,
}

/// Session state summary for the UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CastState {
    /// `Some(true)` when a listed device reports connected; `Some(false)` when
    /// devices exist and none reports connected; `None` when nothing is known.
    pub active: Option<bool>,
    pub device_name: Option<String>,
    pub detail: String,
    pub last_attempt: Option<String>,
}

/// One line of evidence from the capability probe.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CastApiProbe {
    pub api: String,
    pub status: String,
    pub detail: String,
}

/// What this Windows build allows, probed live.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CastCapabilityReport {
    pub probed_at_unix_ms: u64,
    pub screen_mirroring_by_api: bool,
    pub conclusion: String,
    pub probes: Vec<CastApiProbe>,
}

static LAST_ATTEMPT: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn record_attempt(text: String) {
    if let Ok(mut g) = LAST_ATTEMPT.lock() {
        *g = Some(text);
    }
}

fn last_attempt() -> Option<String> {
    LAST_ATTEMPT.lock().ok().and_then(|g| g.clone())
}

// ---------------------------------------------------------------------
// Pure helpers (unit-testable on any OS; windows-cfg-independent)
// ---------------------------------------------------------------------

/// Merge device lists by id, keeping the first non-`None` `connected` value.
pub(crate) fn merge_devices(sources: Vec<Vec<CastDevice>>) -> Vec<CastDevice> {
    let mut merged: Vec<CastDevice> = Vec::new();
    for list in sources {
        for device in list {
            if device.id.is_empty() {
                continue;
            }
            if let Some(existing) = merged.iter_mut().find(|d| d.id == device.id) {
                if existing.connected.is_none() && device.connected.is_some() {
                    existing.connected = device.connected;
                }
            } else {
                merged.push(device);
            }
        }
    }
    merged.sort_by_key(|d| d.name.to_lowercase());
    merged
}

/// Derive session state from the device list.
pub(crate) fn state_from_devices(devices: &[CastDevice], attempt: Option<&str>) -> CastState {
    if let Some(d) = devices.iter().find(|d| d.connected == Some(true)) {
        CastState {
            active: Some(true),
            device_name: Some(d.name.clone()),
            detail: "A projection endpoint reports an active connection (includes sessions started from the Windows panel).".into(),
            last_attempt: attempt.map(str::to_string),
        }
    } else if devices.is_empty() {
        CastState {
            active: None,
            device_name: None,
            detail: "No projection endpoints are known to Windows yet; there is nothing to read state from.".into(),
            last_attempt: attempt.map(str::to_string),
        }
    } else {
        CastState {
            active: Some(false),
            device_name: None,
            detail: "Projection endpoints are known and none reports an active connection.".into(),
            last_attempt: attempt.map(str::to_string),
        }
    }
}

/// The verdict is pure and unit-tested: unless Windows itself returned
/// `Succeeded` for a real cast request, mirroring is not programmatic.
pub(crate) fn probe_conclusion(attempt_status: Option<&str>) -> (bool, String) {
    if attempt_status == Some("Succeeded") {
        (
            true,
            "Windows accepted a programmatic cast request on this build. The session is owned by Windows; stop it from the projection panel (Win+K).".into(),
        )
    } else {
        (
            false,
            "Windows does not allow starting desktop Miracast projection through a public API: CastingConnection requires a CastingSource, CastingSource has no public constructor outside media elements (MediaElement/MediaPlayer content), and RequestStartCastingAsync refuses an empty source (InvalidCastingSource, observed on real devices). The Windows projection panel (Win+K) stays the supported path for screen mirroring.".into(),
        )
    }
}

/// Map a `CastingConnectionErrorStatus` value to a stable name.
pub(crate) fn casting_error_status_name(value: i32) -> &'static str {
    match value {
        0 => "Succeeded",
        1 => "DeviceDidNotRespond",
        2 => "DeviceError",
        3 => "DeviceLocked",
        4 => "ProtectedContentTryFailing",
        5 => "InvalidCastingSource",
        _ => "Unknown",
    }
}

/// Last non-empty line of the PowerShell output parsed as a count.
pub(crate) fn parse_closed_count(stdout: &str) -> u32 {
    stdout
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .and_then(|l| l.parse::<u32>().ok())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------
// Windows implementation
// ---------------------------------------------------------------------

#[cfg(windows)]
mod win {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use windows::core::HSTRING;
    use windows::Devices::Enumeration::{
        DeviceInformation, DeviceInformationUpdate, DeviceWatcher,
    };
    use windows::Foundation::TypedEventHandler;

    /// Miracast sink AEP protocol selector. Kept for completeness: on this
    /// Windows build it returns ZERO rows (Miracast sinks surface via the
    /// casting-device selector below), but removing it would regress on
    /// builds where it does answer.
    pub const MIRACAST_AEP_SELECTOR: &str =
        "System.Devices.Aep.ProtocolId:=\"{AA754C9A-3A24-4298-B530-6E4B13F77E15}\"";

    /// Selector for casting-capable sinks (Chromecast/DLNA/Miracast as
    /// reported by the OS). Built by `CastingDevice::GetDeviceSelector`.
    pub fn cast_selector() -> Option<String> {
        use windows::Media::Casting::{CastingDevice, CastingPlaybackTypes};
        let flags = CastingPlaybackTypes::Video
            | CastingPlaybackTypes::Audio
            | CastingPlaybackTypes::Picture;
        CastingDevice::GetDeviceSelector(flags)
            .ok()
            .map(|s| s.to_string())
    }

    pub struct SharedState {
        pub devices: Mutex<Vec<CastDevice>>,
        pub added: AtomicUsize,
        pub updated: AtomicUsize,
        pub completed: AtomicBool,
    }

    impl SharedState {
        pub fn new() -> Arc<Self> {
            Arc::new(Self {
                devices: Mutex::new(Vec::new()),
                added: AtomicUsize::new(0),
                updated: AtomicUsize::new(0),
                completed: AtomicBool::new(false),
            })
        }
    }

    /// Start a watcher for `selector`, pushing devices into `state`. Returns
    /// the watcher handle (caller keeps it alive and calls Stop when done).
    pub fn watch(
        selector: &str,
        state: Arc<SharedState>,
        kind: &'static str,
    ) -> HardwareResult<DeviceWatcher> {
        let watcher = DeviceInformation::CreateWatcherAqsFilter(&HSTRING::from(selector))
            .map_err(|e| HardwareError::Cast(format!("CreateWatcher({kind}) failed: {e}")))?;

        let st = state.clone();
        watcher
            .Added(&TypedEventHandler::new(
                move |_s, info: &Option<DeviceInformation>| {
                    if let Some(info) = info.as_ref() {
                        let name = info.Name().map(|n| n.to_string()).unwrap_or_default();
                        let id = info.Id().map(|i| i.to_string()).unwrap_or_default();
                        if !id.is_empty() {
                            if let Ok(mut list) = st.devices.lock() {
                                if !list.iter().any(|d| d.id == id) {
                                    list.push(CastDevice {
                                        name: name.trim().to_string(),
                                        id,
                                        device_type: kind.to_string(),
                                        connected: None,
                                    });
                                }
                            }
                        }
                        st.added.fetch_add(1, Ordering::SeqCst);
                    }
                    Ok(())
                },
            ))
            .map_err(|e| HardwareError::Cast(format!("watch({kind}).Added failed: {e}")))?;

        let st2 = state.clone();
        watcher
            .Updated(&TypedEventHandler::new(
                move |_s, upd: &Option<DeviceInformationUpdate>| {
                    if let Some(upd) = upd.as_ref() {
                        let id = upd.Id().map(|i| i.to_string()).unwrap_or_default();
                        if let Ok(mut list) = st2.devices.lock() {
                            if let Some(d) = list.iter_mut().find(|d| d.id == id) {
                                // IsConnected never surfaced reliably here on
                                // this build; keep the field None (honest).
                                d.connected = None;
                            }
                        }
                        st2.updated.fetch_add(1, Ordering::SeqCst);
                    }
                    Ok(())
                },
            ))
            .map_err(|e| HardwareError::Cast(format!("watch({kind}).Updated failed: {e}")))?;

        let st3 = state.clone();
        watcher
            .EnumerationCompleted(&TypedEventHandler::new(
                move |_s, _args: &Option<windows::core::IInspectable>| {
                    st3.completed.store(true, Ordering::SeqCst);
                    Ok(())
                },
            ))
            .map_err(|e| HardwareError::Cast(format!("watch({kind}).Completed failed: {e}")))?;

        watcher
            .Start()
            .map_err(|e| HardwareError::Cast(format!("watch({kind}).Start failed: {e}")))?;
        Ok(watcher)
    }

    pub fn stop(watcher: &DeviceWatcher) {
        let _ = watcher.Stop();
    }

    pub fn snapshot(state: &Arc<SharedState>) -> (Vec<CastDevice>, usize, usize, bool) {
        (
            state.devices.lock().map(|g| g.clone()).unwrap_or_default(),
            state.added.load(Ordering::SeqCst),
            state.updated.load(Ordering::SeqCst),
            state.completed.load(Ordering::SeqCst),
        )
    }

    /// Real device probe: FromIdAsync on an invalid id must FAIL. Returns the
    /// error text for the report.
    pub fn probe_invalid_id() -> HardwareResult<String> {
        use windows::Media::Casting::CastingDevice;
        let op = CastingDevice::FromIdAsync(&HSTRING::from("bogus-device-id"))
            .map_err(|e| HardwareError::Cast(format!("FromIdAsync(create) failed: {e}")))?;
        match op.get() {
            Ok(_) => Err(HardwareError::Cast(
                "unexpected success: bogus id resolved to a device".into(),
            )),
            Err(e) => Ok(format!("rejected as expected: {e}")),
        }
    }

    /// Real device probe: build a CastingSource from an empty MediaPlayer.
    /// This is the ONLY public source construction path.
    pub fn probe_casting_source() -> HardwareResult<String> {
        use windows::Media::Playback::MediaPlayer;
        let player = MediaPlayer::new()
            .map_err(|e| HardwareError::Cast(format!("MediaPlayer::new failed: {e}")))?;
        let src = player
            .GetAsCastingSource()
            .map_err(|e| HardwareError::Cast(format!("GetAsCastingSource failed: {e}")))?;
        let _ = player.Close();
        Ok(format!(
            "CastingSource obtained from MediaPlayer (empty content): {src:?}"
        ))
    }

    /// Real device attempt against ONE id: FromIdAsync -> CreateCastingConnection
    /// -> RequestStartCastingAsync(empty source). Returns the raw
    /// `CastingConnectionErrorStatus` name Windows reported, or the error text.
    /// A `Succeeded` here would be a REAL streaming session (not expected for
    /// an empty source); anything else is a refusal with a name.
    pub fn attempt_cast(device_id: &str) -> Result<String, String> {
        use windows::Media::Casting::{CastingConnectionErrorStatus, CastingDevice};

        let op = CastingDevice::FromIdAsync(&HSTRING::from(device_id))
            .map_err(|e| format!("FromIdAsync failed: {e}"))?;
        let device = op
            .get()
            .map_err(|e| format!("FromIdAsync timed out / failed: {e}"))?;
        let friendly = device
            .FriendlyName()
            .map(|n| n.to_string())
            .unwrap_or_else(|_| "<unknown>".into());

        let connection = device
            .CreateCastingConnection()
            .map_err(|e| format!("{friendly}: CreateCastingConnection failed: {e}"))?;

        let source = {
            use windows::Media::Playback::MediaPlayer;
            let player = MediaPlayer::new()
                .map_err(|e| format!("{friendly}: MediaPlayer init failed: {e}"))?;
            let src = player
                .GetAsCastingSource()
                .map_err(|e| format!("{friendly}: GetAsCastingSource failed: {e}"))?;
            let _ = player.Close();
            src
        };

        let req = connection
            .RequestStartCastingAsync(&source)
            .map_err(|e| format!("{friendly}: RequestStartCastingAsync failed: {e}"))?;
        let status = req
            .get()
            .map_err(|e| format!("{friendly}: awaiting RequestStartCastingAsync failed: {e}"))?;
        let name = super::casting_error_status_name(status.0);
        if status == CastingConnectionErrorStatus::Succeeded {
            // Do not leave a live session behind from a probe path.
            if let Ok(disc) = connection.DisconnectAsync() {
                let _ = disc.get();
            }
        }
        Ok(format!("{friendly}: {name}"))
    }
}

/// List KNOWN casting devices from the OS cache with real state when present.
#[cfg(windows)]
pub fn list_cast_devices() -> HardwareResult<Vec<CastDevice>> {
    use windows::core::HSTRING;
    use windows::Devices::Enumeration::DeviceInformation;

    // FAST path: the picker selector enumerates with a ~30 s discovery
    // window on this host (probed), which cannot live inside the default
    // blocking budget; the known-list read uses the Miracast AEP selector
    // that answers from cache in ~130 ms. Full discovery is scan_cast_devices.
    let selector = win::MIRACAST_AEP_SELECTOR;
    let op = DeviceInformation::FindAllAsyncAqsFilter(&HSTRING::from(selector))
        .map_err(|e| HardwareError::Cast(format!("FindAllAsync failed: {e}")))?;
    let coll = op
        .get()
        .map_err(|e| HardwareError::Cast(format!("enumeration failed: {e}")))?;

    let count = coll.Size().unwrap_or(0);
    let mut devices = Vec::new();
    for i in 0..count {
        if let Ok(info) = coll.GetAt(i) {
            let name = info.Name().map(|n| n.to_string()).unwrap_or_default();
            let id = info.Id().map(|i| i.to_string()).unwrap_or_default();
            if !id.is_empty() {
                devices.push(CastDevice {
                    name: name.trim().to_string(),
                    id,
                    device_type: "cast".to_string(),
                    connected: None, // See module docs: IsConnected not exposed on this build.
                });
            }
        }
    }
    log::info!(
        "[screen_cast] list: {count} casting endpoints, {} named",
        devices.len()
    );
    Ok(devices)
}

#[cfg(not(windows))]
pub fn list_cast_devices() -> HardwareResult<Vec<CastDevice>> {
    Ok(Vec::new())
}

/// Run a bounded discovery (watcher) and report what was found.
#[cfg(windows)]
pub fn scan_cast_devices() -> HardwareResult<CastScanReport> {
    use std::time::{Duration, Instant};

    let started = Instant::now();
    let sel = win::cast_selector().ok_or_else(|| {
        HardwareError::Cast("CastingDevice::GetDeviceSelector unavailable".into())
    })?;

    let state = win::SharedState::new();
    let watcher = win::watch(&sel, state.clone(), "cast")?;

    // Bounded window: stop as soon as the watcher completes, or at the cap.
    loop {
        std::thread::sleep(Duration::from_millis(200));
        if state.completed.load(std::sync::atomic::Ordering::SeqCst)
            || started.elapsed() >= SCAN_WINDOW
        {
            break;
        }
    }
    win::stop(&watcher);

    let (found, added, updated, completed) = win::snapshot(&state);
    // merge_devices keeps rows unique and name-sorted for the UI.
    let devices = merge_devices(vec![found]);
    Ok(CastScanReport {
        duration_ms: started.elapsed().as_millis() as u64,
        devices,
        added_events: added,
        updated_events: updated,
        completed,
    })
}

#[cfg(not(windows))]
pub fn scan_cast_devices() -> HardwareResult<CastScanReport> {
    Ok(CastScanReport {
        duration_ms: 0,
        devices: Vec::new(),
        added_events: 0,
        updated_events: 0,
        completed: false,
    })
}

#[cfg(windows)]
pub fn start_casting(device_id: &str) -> HardwareResult<CastResult> {
    if device_id.is_empty() {
        return Ok(CastResult {
            success: false,
            message: "No device selected; pick a device first or use the Windows panel (Win+K)."
                .into(),
        });
    }
    log::info!("[screen_cast] connect attempt: {device_id}");
    match win::attempt_cast(device_id) {
        Ok(info) => {
            let success = info.ends_with("Succeeded");
            record_attempt(format!("connect {device_id}: {info}"));
            Ok(CastResult {
                success,
                message: if success {
                    format!("Cast started to {info}. The session is owned by Windows; stop it from Win+K.")
                } else {
                    format!("Windows refused the cast: {info}. Desktop mirroring has no public API (see the probe panel); use Win+K.")
                },
            })
        }
        Err(e) => {
            record_attempt(format!("connect {device_id} FAILED: {e}"));
            Ok(CastResult {
                success: false,
                message: format!(
                    "Cast attempt failed: {e}. Use the Windows panel (Win+K) as fallback."
                ),
            })
        }
    }
}

#[cfg(not(windows))]
pub fn start_casting(_device_id: &str) -> HardwareResult<CastResult> {
    Ok(CastResult {
        success: false,
        message: "Screen casting is only available on Windows.".into(),
    })
}

#[cfg(windows)]
pub fn open_cast_panel() -> HardwareResult<CastResult> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    log::info!("[screen_cast] opening Windows projection panel (fallback)");
    // SAFETY: standard ShellExecuteW pattern; URI is a valid protocol string.
    unsafe {
        let hinst = ShellExecuteW(
            None,
            windows::core::w!("open"),
            windows::core::w!("ms-settings-connectabledevices:devicediscovery"),
            None,
            None,
            SW_SHOWNORMAL,
        );
        let ok = hinst.0 as isize > 32;
        Ok(CastResult {
            success: ok,
            message: if ok {
                "Windows projection panel opened; pick a display there. Windows owns the session."
                    .into()
            } else {
                format!("ShellExecuteW failed with code {}", hinst.0 as isize)
            },
        })
    }
}

#[cfg(not(windows))]
pub fn open_cast_panel() -> HardwareResult<CastResult> {
    Ok(CastResult {
        success: false,
        message: "Screen casting is only available on Windows.".into(),
    })
}

#[cfg(windows)]
pub fn get_cast_state() -> HardwareResult<CastState> {
    let devices = list_cast_devices().unwrap_or_else(|e| {
        log::warn!("[screen_cast] state read: enumeration failed: {e}");
        Vec::new()
    });
    Ok(state_from_devices(&devices, last_attempt().as_deref()))
}

#[cfg(not(windows))]
pub fn get_cast_state() -> HardwareResult<CastState> {
    Ok(state_from_devices(&[], None))
}

#[cfg(windows)]
pub fn probe_cast_capabilities() -> HardwareResult<CastCapabilityReport> {
    static CACHE: std::sync::Mutex<Option<CastCapabilityReport>> = std::sync::Mutex::new(None);
    if let Ok(g) = CACHE.lock() {
        if let Some(cached) = g.as_ref() {
            return Ok(cached.clone());
        }
    }

    let mut probes: Vec<CastApiProbe> = Vec::new();

    // 1. Selector build.
    let sel = win::cast_selector();
    probes.push(CastApiProbe {
        api: "CastingDevice::GetDeviceSelector(Video|Audio|Picture)".into(),
        status: if sel.is_some() { "ok" } else { "failed" }.into(),
        detail: sel
            .as_ref()
            .map(|s| {
                if s.len() > 80 {
                    format!("{}...", &s[..80])
                } else {
                    s.clone()
                }
            })
            .unwrap_or_else(|| "no selector".into()),
    });

    // 2. CastingSource construction via MediaPlayer (the only public path).
    match win::probe_casting_source() {
        Ok(detail) => probes.push(CastApiProbe {
            api: "MediaPlayer::GetAsCastingSource (empty content)".into(),
            status: "ok".into(),
            detail,
        }),
        Err(e) => probes.push(CastApiProbe {
            api: "MediaPlayer::GetAsCastingSource (empty content)".into(),
            status: "failed".into(),
            detail: e.to_string(),
        }),
    }

    // 3. Invalid-id shape.
    match win::probe_invalid_id() {
        Ok(detail) => probes.push(CastApiProbe {
            api: "CastingDevice::FromIdAsync(bogus id)".into(),
            status: "ok".into(),
            detail,
        }),
        Err(e) => probes.push(CastApiProbe {
            api: "CastingDevice::FromIdAsync(bogus id)".into(),
            status: "failed".into(),
            detail: e.to_string(),
        }),
    }

    // 4. Real attempt against the first known casting device, if present.
    let devices = list_cast_devices().unwrap_or_default();
    let attempt_status: Option<String> = if let Some(d) = devices.first() {
        match win::attempt_cast(&d.id) {
            Ok(info) => {
                let status = info.rsplit(": ").next().unwrap_or("Unknown").to_string();
                probes.push(CastApiProbe {
                    api: format!("RequestStartCastingAsync({})", d.name),
                    status: if status == "Succeeded" {
                        "ok"
                    } else {
                        "refused"
                    }
                    .into(),
                    detail: info.clone(),
                });
                Some(status)
            }
            Err(e) => {
                probes.push(CastApiProbe {
                    api: format!("RequestStartCastingAsync({})", d.name),
                    status: "failed".into(),
                    detail: e,
                });
                None
            }
        }
    } else {
        probes.push(CastApiProbe {
            api: "RequestStartCastingAsync(device)".into(),
            status: "skipped".into(),
            detail: "no casting device known to Windows; connect a sink and retry".into(),
        });
        None
    };

    let (allowed, conclusion) = probe_conclusion(attempt_status.as_deref());
    let report = CastCapabilityReport {
        probed_at_unix_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        screen_mirroring_by_api: allowed,
        conclusion,
        probes,
    };
    if let Ok(mut g) = CACHE.lock() {
        *g = Some(report.clone());
    }
    Ok(report)
}

#[cfg(not(windows))]
pub fn probe_cast_capabilities() -> HardwareResult<CastCapabilityReport> {
    Ok(CastCapabilityReport {
        probed_at_unix_ms: 0,
        screen_mirroring_by_api: false,
        conclusion: "Screen casting probes only run on Windows.".into(),
        probes: Vec::new(),
    })
}

#[cfg(windows)]
pub fn stop_casting() -> HardwareResult<CastResult> {
    log::info!("[screen_cast] closing the Windows Connect cast panel");
    let ps_script = r#"
        $closed = 0
        Get-Process SystemSettings -ErrorAction SilentlyContinue | ForEach-Object {
            if ($_.MainWindowTitle -match 'Connect') {
                CloseMainWindow | Out-Null
                $closed++
            }
        }
        Write-Output $closed
    "#;
    let mut cmd = std::process::Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", ps_script]);
    let out = process_spawn::run_capture(&mut cmd, PS_CLOSE_PANEL_TIMEOUT)
        .map_err(|e| HardwareError::Cast(format!("panel close failed: {e}")))?;
    let closed = parse_closed_count(&out.stdout_str());
    Ok(CastResult {
        success: closed > 0,
        message: format!("Closed {closed} Connect panel(s). A running projection is owned by Windows; stop it from Win+K."),
    })
}

#[cfg(not(windows))]
pub fn stop_casting() -> HardwareResult<CastResult> {
    Ok(CastResult {
        success: false,
        message: "Screen casting is only available on Windows.".into(),
    })
}

// ---------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: &str, name: &str, connected: Option<bool>) -> CastDevice {
        CastDevice {
            name: name.into(),
            id: id.into(),
            device_type: "cast".into(),
            connected,
        }
    }

    #[test]
    fn merge_dedupes_by_id_and_prefers_first_state() {
        let merged = merge_devices(vec![
            vec![row("a", "TV", None), row("b", "Buds", Some(false))],
            vec![row("a", "TV", Some(true)), row("c", "Stick", None)],
        ]);
        assert_eq!(merged.len(), 3);
        let a = merged.iter().find(|d| d.id == "a").unwrap();
        assert_eq!(a.connected, Some(true));
        assert_eq!(a.name, "TV");
    }

    #[test]
    fn state_three_way_distinction() {
        let none = state_from_devices(&[], None);
        assert_eq!(none.active, None);

        let connected =
            state_from_devices(&[row("a", "TV", Some(true)), row("b", "B", None)], None);
        assert_eq!(connected.active, Some(true));
        assert_eq!(connected.device_name.as_deref(), Some("TV"));

        let known = state_from_devices(&[row("a", "TV", Some(false)), row("b", "B", None)], None);
        assert_eq!(known.active, Some(false));
    }

    #[test]
    fn state_carries_attempt() {
        let s = state_from_devices(&[], Some("connect X: refused"));
        assert_eq!(s.last_attempt.as_deref(), Some("connect X: refused"));
    }

    #[test]
    fn conclusion_requires_windows_consent() {
        let (allowed, text) = probe_conclusion(Some("InvalidCastingSource"));
        assert!(!allowed);
        assert!(text.contains("Win+K"), "fallback must be named: {text}");
        assert!(text.contains("InvalidCastingSource"));

        let (allowed2, _) = probe_conclusion(Some("Succeeded"));
        assert!(allowed2);

        let (allowed3, _) = probe_conclusion(None);
        assert!(!allowed3);
    }

    #[test]
    fn error_status_names_stable() {
        assert_eq!(casting_error_status_name(0), "Succeeded");
        assert_eq!(casting_error_status_name(5), "InvalidCastingSource");
        assert_eq!(casting_error_status_name(42), "Unknown");
    }

    #[test]
    fn closed_count_parsing() {
        assert_eq!(parse_closed_count("1"), 1);
        assert_eq!(parse_closed_count(" 2 \r\n"), 2);
        assert_eq!(parse_closed_count("warning\n0\n"), 0);
        assert_eq!(parse_closed_count(""), 0);
        assert_eq!(parse_closed_count("nan"), 0);
    }

    #[test]
    fn empty_list_ok() {
        let r = list_cast_devices();
        assert!(r.is_ok(), "enumeration must not hard-fail: {r:?}");
    }
}
