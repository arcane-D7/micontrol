// hw/audio.rs
//
// Audio device enumeration and control via Windows Core Audio API.
// Provides device listing, volume control, and mute toggle.

#[cfg(windows)]
use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioDevice {
    pub name: String,
    pub id: String,
    pub direction: String,
    pub is_default: bool,
    pub volume: u8,
    pub muted: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioDeviceList {
    pub playback: Vec<AudioDevice>,
    pub capture: Vec<AudioDevice>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioVolumeResult {
    pub success: bool,
    pub volume: u8,
    pub muted: bool,
}

#[cfg(windows)]
pub fn list_audio_devices() -> Result<AudioDeviceList> {
    use windows::Win32::Media::Audio::{
        eCapture, eRender, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    // SAFETY: CoInitializeEx initializes the COM library for this thread; safe because we call CoUninitialize before returning.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }

    let result = (|| -> Result<AudioDeviceList> {
        // SAFETY: CoCreateInstance creates a COM object from a known CLSID; the returned interface pointer is valid and we consume it within this scope.
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let playback = enumerate_devices(&enumerator, eRender)?;
            let capture = enumerate_devices(&enumerator, eCapture)?;
            Ok(AudioDeviceList { playback, capture })
        }
    })();

    // SAFETY: CoUninitialize shuts down COM on this thread; safe because we initialized it with CoInitializeEx above.
    unsafe {
        CoUninitialize();
    }
    result
}

#[cfg(not(windows))]
pub fn list_audio_devices() -> Result<AudioDeviceList> {
    Ok(AudioDeviceList {
        playback: vec![],
        capture: vec![],
    })
}

#[cfg(windows)]
pub fn get_playback_volume() -> Result<AudioVolumeResult> {
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    // SAFETY: CoInitializeEx initializes COM for this thread; safe because we call CoUninitialize before returning.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }

    let result = (|| -> Result<AudioVolumeResult> {
        // SAFETY: CoCreateInstance creates a known COM object; the returned interface pointers are valid for the scope of this closure.
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
            let endpoint: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
            let volume = endpoint.GetMasterVolumeLevelScalar()?;
            let muted = endpoint.GetMute()?;
            Ok(AudioVolumeResult {
                success: true,
                volume: (volume * 100.0) as u8,
                muted: muted.as_bool(),
            })
        }
    })();

    // SAFETY: CoUninitialize shuts down COM on this thread; safe because it was initialized above.
    unsafe {
        CoUninitialize();
    }
    result
}

#[cfg(not(windows))]
pub fn get_playback_volume() -> Result<AudioVolumeResult> {
    Ok(AudioVolumeResult {
        success: false,
        volume: 0,
        muted: false,
    })
}

#[cfg(windows)]
pub fn set_playback_volume(volume: u8) -> Result<AudioVolumeResult> {
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    let volume = volume.min(100);
    let scalar = volume as f32 / 100.0;

    // SAFETY: CoInitializeEx initializes COM for this thread; safe because we call CoUninitialize before returning.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }

    let result = (|| -> Result<AudioVolumeResult> {
        // SAFETY: CoCreateInstance creates a known COM object; the returned interface pointers are valid for the scope of this closure. SetMasterVolumeLevelScalar takes a raw pointer for notifications; passing null is safe per the Windows API contract.
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
            let endpoint: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
            endpoint.SetMasterVolumeLevelScalar(scalar, std::ptr::null())?;
            let muted = endpoint.GetMute()?;
            Ok(AudioVolumeResult {
                success: true,
                volume,
                muted: muted.as_bool(),
            })
        }
    })();

    // SAFETY: CoUninitialize shuts down COM on this thread; safe because it was initialized above.
    unsafe {
        CoUninitialize();
    }
    result
}

#[cfg(not(windows))]
pub fn set_playback_volume(_volume: u8) -> Result<AudioVolumeResult> {
    Ok(AudioVolumeResult {
        success: false,
        volume: 0,
        muted: false,
    })
}

#[cfg(windows)]
pub fn set_playback_mute(muted: bool) -> Result<AudioVolumeResult> {
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{
        eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    // SAFETY: CoInitializeEx initializes COM for this thread; safe because we call CoUninitialize before returning.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }

    let result = (|| -> Result<AudioVolumeResult> {
        // SAFETY: CoCreateInstance creates a known COM object; the returned interface pointers are valid. SetMute takes a raw pointer for notifications; passing null is safe per Windows API contract.
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
            let endpoint: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
            endpoint.SetMute(muted, std::ptr::null())?;
            let volume = endpoint.GetMasterVolumeLevelScalar()?;
            Ok(AudioVolumeResult {
                success: true,
                volume: (volume * 100.0) as u8,
                muted,
            })
        }
    })();

    // SAFETY: CoUninitialize shuts down COM on this thread; safe because it was initialized above.
    unsafe {
        CoUninitialize();
    }
    result
}

#[cfg(not(windows))]
pub fn set_playback_mute(_muted: bool) -> Result<AudioVolumeResult> {
    Ok(AudioVolumeResult {
        success: false,
        volume: 0,
        muted: false,
    })
}

// ── Private helpers ──────────────────────────────────────────────────────────

#[cfg(windows)]
fn enumerate_devices(
    enumerator: &windows::Win32::Media::Audio::IMMDeviceEnumerator,
    data_flow: windows::Win32::Media::Audio::EDataFlow,
) -> Result<Vec<AudioDevice>> {
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::Media::Audio::{eConsole, DEVICE_STATE_ACTIVE};

    // SAFETY: All COM calls are made through the windows crate's safe wrappers. The IMMDeviceEnumerator and its children are valid COM pointers obtained from CoCreateInstance / EnumAudioEndpoints.
    unsafe {
        let collection = enumerator.EnumAudioEndpoints(data_flow, DEVICE_STATE_ACTIVE)?;
        let count = collection.GetCount()?;
        let mut devices = Vec::with_capacity(count as usize);
        let default_device = enumerator.GetDefaultAudioEndpoint(data_flow, eConsole).ok();

        for i in 0..count {
            let device = collection.Item(i)?;
            let id = device.GetId()?.to_string()?;
            let name =
                get_device_friendly_name(&device).unwrap_or_else(|_| format!("Audio Device {}", i));

            let is_default = default_device.as_ref().map_or(false, |d| {
                d.GetId().map(|s| s.to_string().unwrap_or_default()) == Ok(id.clone())
            });

            let (volume, muted) = get_device_volume(&device).unwrap_or((50, false));

            devices.push(AudioDevice {
                name,
                id,
                direction: if data_flow == windows::Win32::Media::Audio::eRender {
                    "playback".to_string()
                } else {
                    "capture".to_string()
                },
                is_default,
                volume,
                muted,
            });
        }
        Ok(devices)
    }
}

#[cfg(windows)]
fn get_device_friendly_name(device: &windows::Win32::Media::Audio::IMMDevice) -> Result<String> {
    // SAFETY: device.GetId() is a COM method call through the windows crate; the IMMDevice pointer is valid as it was obtained from EnumAudioEndpoints.
    let id = unsafe { device.GetId()?.to_string()? };
    Ok(id)
}

#[cfg(windows)]
fn get_device_volume(device: &windows::Win32::Media::Audio::IMMDevice) -> Result<(u8, bool)> {
    use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
    use windows::Win32::System::Com::CLSCTX_ALL;

    // SAFETY: device.Activate returns a valid IAudioEndpointVolume COM pointer; calls to GetMasterVolumeLevelScalar and GetMute through the windows crate are safe given a valid interface pointer.
    unsafe {
        let endpoint: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
        let volume = endpoint.GetMasterVolumeLevelScalar()?;
        let muted = endpoint.GetMute()?;
        Ok(((volume * 100.0) as u8, muted.as_bool()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_device_list() {
        let result = list_audio_devices();
        assert!(result.is_ok());
    }

    #[test]
    fn test_volume_range() {
        let result = get_playback_volume();
        if let Ok(vol) = result {
            assert!(vol.volume <= 100);
        }
    }
}
