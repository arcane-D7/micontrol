//! Audio device enumeration and control via Windows Core Audio API.
//! Provides device listing, volume control, and mute toggle.
#![allow(non_snake_case, dead_code)]

use crate::hw::errors::HardwareResult;
#[cfg(windows)]
use anyhow::Result;
use serde::{Deserialize, Serialize};
#[cfg(windows)]
use windows::core::{interface, IUnknown, IUnknown_Vtbl, Interface, GUID, HRESULT, PCWSTR};

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
pub fn list_audio_devices() -> HardwareResult<AudioDeviceList> {
    use windows::Win32::Media::Audio::{
        eCapture, eRender, IMMDeviceEnumerator, MMDeviceEnumerator,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    // SAFETY: CoInitializeEx initializes the COM library for this thread; safe because we call CoUninitialize before returning.
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }

    let result = (|| -> anyhow::Result<AudioDeviceList> {
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
    result.map_err(Into::into)
}

#[cfg(not(windows))]
pub fn list_audio_devices() -> HardwareResult<AudioDeviceList> {
    Ok(AudioDeviceList {
        playback: vec![],
        capture: vec![],
    })
}

#[cfg(windows)]
pub fn get_playback_volume() -> HardwareResult<AudioVolumeResult> {
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

    let result = (|| -> anyhow::Result<AudioVolumeResult> {
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
    result.map_err(Into::into)
}

#[cfg(not(windows))]
pub fn get_playback_volume() -> HardwareResult<AudioVolumeResult> {
    Ok(AudioVolumeResult {
        success: false,
        volume: 0,
        muted: false,
    })
}

#[cfg(windows)]
pub fn set_playback_volume(volume: u8) -> HardwareResult<AudioVolumeResult> {
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
    result.map_err(Into::into)
}

#[cfg(not(windows))]
pub fn set_playback_volume(_volume: u8) -> HardwareResult<AudioVolumeResult> {
    Ok(AudioVolumeResult {
        success: false,
        volume: 0,
        muted: false,
    })
}

#[cfg(windows)]
pub fn set_playback_mute(muted: bool) -> HardwareResult<AudioVolumeResult> {
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
    result.map_err(Into::into)
}

#[cfg(not(windows))]
pub fn set_playback_mute(_muted: bool) -> HardwareResult<AudioVolumeResult> {
    Ok(AudioVolumeResult {
        success: false,
        volume: 0,
        muted: false,
    })
}

/// Set the default audio playback device by device ID.
///
/// Uses the undocumented but stable `IPolicyConfig` COM interface
/// (IID `{f8679f50-850a-41cf-9c72-430f290290c8}`, available since Windows 8.1).
/// Calls `SetDefaultEndpoint` for all three roles (eConsole, eMultimedia,
/// eCommunications) to ensure the device becomes default across all audio
/// categories.
#[cfg(windows)]
const CLSID_POLICY_CONFIG: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

#[cfg(windows)]
#[interface("f8679f50-850a-41cf-9c72-430f290290c8")]
unsafe trait IPolicyConfig: IUnknown {
    fn GetMixFormat(&self, device_id: PCWSTR, format: *mut std::ffi::c_void) -> HRESULT;
    fn GetDeviceFormat(
        &self,
        device_id: PCWSTR,
        exclusive: i32,
        format: *mut std::ffi::c_void,
    ) -> HRESULT;
    fn ResetDeviceFormat(&self, device_id: PCWSTR) -> HRESULT;
    fn SetDeviceFormat(
        &self,
        device_id: PCWSTR,
        format: *const std::ffi::c_void,
        previous: *const std::ffi::c_void,
    ) -> HRESULT;
    fn GetProcessingPeriod(
        &self,
        device_id: PCWSTR,
        exclusive: i32,
        current: *mut i64,
        default: *mut i64,
    ) -> HRESULT;
    fn SetProcessingPeriod(&self, device_id: PCWSTR, period: *mut i64) -> HRESULT;
    fn GetShareMode(&self, device_id: PCWSTR, mode: *mut std::ffi::c_void) -> HRESULT;
    fn SetShareMode(&self, device_id: PCWSTR, mode: *const std::ffi::c_void) -> HRESULT;
    fn GetPropertyValue(
        &self,
        device_id: PCWSTR,
        property_format: i32,
        property: *const std::ffi::c_void,
        value: *mut std::ffi::c_void,
    ) -> HRESULT;
    fn SetPropertyValue(
        &self,
        device_id: PCWSTR,
        property_format: i32,
        property: *const std::ffi::c_void,
        value: *const std::ffi::c_void,
    ) -> HRESULT;
    // Slot 15: SetDefaultEndpoint — the method we need
    fn SetDefaultEndpoint(&self, device_id: PCWSTR, role: u32) -> HRESULT;
    fn SetEndpointVisibility(&self, device_id: PCWSTR, visible: i32) -> HRESULT;
}

#[cfg(windows)]
pub fn set_default_endpoint(device_id: &str) -> HardwareResult<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };

    // ERole constants
    const E_CONSOLE: u32 = 0;
    const E_MULTIMEDIA: u32 = 1;
    const E_COMMUNICATIONS: u32 = 2;

    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);

        let result = (|| -> windows::core::Result<()> {
            let unknown: IUnknown = CoCreateInstance(&CLSID_POLICY_CONFIG, None, CLSCTX_ALL)?;
            let policy: IPolicyConfig = unknown.cast()?;

            // Convert device_id to wide string
            let device_w: Vec<u16> = std::ffi::OsStr::new(device_id)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();
            let pcwstr = PCWSTR(device_w.as_ptr());

            // Set as default for all three roles
            for role in [E_CONSOLE, E_MULTIMEDIA, E_COMMUNICATIONS] {
                policy.SetDefaultEndpoint(pcwstr, role).ok()?;
            }
            Ok(())
        })();

        CoUninitialize();

        result.map_err(|e| {
            crate::hw::errors::HardwareError::Other(format!(
                "Failed to set default audio endpoint: {e}"
            ))
        })
    }
}

#[cfg(not(windows))]
pub fn set_default_endpoint(_device_id: &str) -> HardwareResult<()> {
    Err(crate::hw::errors::HardwareError::NotSupported(
        "Audio device switching is Windows-only".into(),
    ))
}

// ── Private helpers ──────────────────────────────────────────────────────────

#[cfg(windows)]
fn enumerate_devices(
    enumerator: &windows::Win32::Media::Audio::IMMDeviceEnumerator,
    data_flow: windows::Win32::Media::Audio::EDataFlow,
) -> Result<Vec<AudioDevice>> {
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

            let is_default = default_device.as_ref().is_some_and(|d| {
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
    use windows::Win32::System::Com::STGM_READ;
    use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, PROPERTYKEY};

    // PKEY_Device_FriendlyName = {a45c254e-df1c-4efd-8020-67d146a850e0}, 14
    const PKEY_DEVICE_FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
        fmtid: windows::core::GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
        pid: 14,
    };

    // SAFETY: device.OpenPropertyStore is a COM method call through the windows crate;
    // the IMMDevice pointer is valid as it was obtained from EnumAudioEndpoints.
    unsafe {
        let store: IPropertyStore = device.OpenPropertyStore(STGM_READ)?;

        let prop = store.GetValue(&PKEY_DEVICE_FRIENDLY_NAME)?;

        let bstr = windows::core::BSTR::try_from(&prop)
            .map_err(|e| anyhow::anyhow!("Failed to convert PROPVARIANT to string: {e}"))?;
        let name = bstr.to_string();

        if name.is_empty() {
            // Fallback to device ID if friendly name is unavailable
            let id = device.GetId()?.to_string()?;
            return Ok(id);
        }

        Ok(name)
    }
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
