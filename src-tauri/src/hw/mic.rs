//! System-level microphone mute via Windows Core Audio (WASAPI).
//!
//! Sets hardware mute on the default capture endpoint so no application
//! can record while muted, regardless of per-app volume settings.

#[cfg(windows)]
pub fn set_system_mic_mute(muted: bool) {
    // Spawn a dedicated thread so we don't inherit the WMI COM apartment model.
    std::thread::spawn(move || unsafe { set_mute_inner(muted) });
}

#[cfg(not(windows))]
pub fn set_system_mic_mute(_muted: bool) {}

#[cfg(windows)]
unsafe fn set_mute_inner(muted: bool) {
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
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
        return;
    }

    let result = (|| -> windows::core::Result<()> {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
        let device = enumerator.GetDefaultAudioEndpoint(eCapture, eConsole)?;
        let endpoint: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
        endpoint.SetMute(muted, std::ptr::null())?;
        log::info!("[mic] System mic mute set to {}", muted);
        Ok(())
    })();

    if let Err(e) = result {
        log::warn!("[mic] Failed to set mic mute: {:?}", e);
    }

    CoUninitialize();
}
