//! System-level microphone mute via Windows Core Audio (WASAPI).
//!
//! Sets hardware mute on the default capture endpoint so no application can
//! record while muted, regardless of per-app volume settings.
//!
//! Every operation here reports the state the endpoint **actually** holds after
//! the call (read-back), never the intended one. A mic-mute OSD that disagrees
//! with the hardware mute flag is worse than no OSD at all, and the consumer-key
//! path cannot know the starting state, so it must read it.
//!
//! All public functions return immediately: the COM work happens on a dedicated
//! thread so callers (WMI dispatch, hot-key hooks) are never blocked and never
//! inherit an incompatible COM apartment model.

#[cfg(windows)]
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;

/// Set the capture endpoint mute flag (fire-and-forget).
///
/// Kept for callers that already know the target state; the OSD variants below
/// are preferred because they show the state the endpoint reports back.
#[cfg(windows)]
pub fn set_system_mic_mute(muted: bool) {
    run_on_capture_endpoint(
        "set mute",
        move |endpoint| unsafe {
            endpoint.SetMute(muted, std::ptr::null())?;
            let actual = endpoint.GetMute()?.as_bool();
            log::info!("[mic] System mic mute set to {actual}");
            Ok(actual)
        },
        |_| {},
    );
}

#[cfg(not(windows))]
pub fn set_system_mic_mute(_muted: bool) {}

/// Set the capture endpoint mute flag and show the OSD with the state the
/// endpoint reports back. No OSD when the endpoint cannot be reached — silence
/// is honest, a wrong icon is not.
#[cfg(windows)]
pub fn set_system_mic_mute_osd(muted: bool) {
    run_on_capture_endpoint(
        "set mute (osd)",
        move |endpoint| unsafe {
            endpoint.SetMute(muted, std::ptr::null())?;
            Ok(endpoint.GetMute()?.as_bool())
        },
        report_mic_state,
    );
}

#[cfg(not(windows))]
pub fn set_system_mic_mute_osd(_muted: bool) {}

/// Flip the capture endpoint mute based on its *real* state, then show the OSD
/// with the resulting state.
///
/// Used by the HID consumer-key path, which receives a key press without any
/// state information. Reading first removes the old internal-boolean guess that
/// drifted out of sync with the endpoint after any external mute change.
#[cfg(windows)]
pub fn toggle_system_mic_mute_osd() {
    run_on_capture_endpoint(
        "toggle mute",
        |endpoint| unsafe {
            let current = endpoint.GetMute()?.as_bool();
            endpoint.SetMute(!current, std::ptr::null())?;
            Ok(endpoint.GetMute()?.as_bool())
        },
        report_mic_state,
    );
}

#[cfg(not(windows))]
pub fn toggle_system_mic_mute_osd() {}

/// Show the mic OSD for the state the endpoint reported.
#[cfg(windows)]
fn report_mic_state(actual: Option<bool>) {
    match actual {
        Some(muted) => crate::hw::osd::show_mic_mute_osd(muted),
        None => log::warn!("[mic] Endpoint unreachable — no OSD shown (state unknown)"),
    }
}

/// Run `f` against the default capture endpoint on a dedicated thread, then
/// hand the result (`None` = endpoint unreachable) to `report` on that thread.
#[cfg(windows)]
fn run_on_capture_endpoint<F, R>(label: &'static str, f: F, report: R)
where
    F: FnOnce(&IAudioEndpointVolume) -> windows::core::Result<bool> + Send + 'static,
    R: FnOnce(Option<bool>) + Send + 'static,
{
    let spawned = std::thread::Builder::new()
        .name("mic-endpoint".into())
        .spawn(move || {
            // SAFETY: endpoint_call initialises COM on this dedicated thread,
            // uses the COM objects only inside its scope and uninitialises
            // before returning. Nothing is shared with other threads.
            let actual = unsafe { endpoint_call(label, f) };
            report(actual);
        });

    if let Err(e) = spawned {
        log::warn!("[mic] Endpoint thread spawn failed: {e}");
    }
}

/// COM scaffolding for a single capture-endpoint operation.
#[cfg(windows)]
unsafe fn endpoint_call<F>(label: &'static str, f: F) -> Option<bool>
where
    F: FnOnce(&IAudioEndpointVolume) -> windows::core::Result<bool>,
{
    use windows::Win32::Media::Audio::{
        eCapture, eConsole, IMMDeviceEnumerator, MMDeviceEnumerator,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    };

    let hr = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
    // S_OK (0) = initialised, S_FALSE (1) = already init on this thread — both fine.
    // RPC_E_CHANGED_MODE (0x80010106) means another apartment model owns this thread.
    if hr.is_err() {
        log::warn!("[mic] CoInitializeEx failed: {:?}", hr);
        return None;
    }

    let result = (|| -> windows::core::Result<bool> {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = enumerator.GetDefaultAudioEndpoint(eCapture, eConsole)?;
        let endpoint: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
        f(&endpoint)
    })();

    CoUninitialize();

    match result {
        Ok(value) => Some(value),
        Err(e) => {
            log::warn!("[mic] {label} failed: {e:?}");
            None
        }
    }
}
