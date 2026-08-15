//! ICredentialProvider implementation — owns the face-unlock tile.

use std::sync::Mutex;
use windows::Win32::Foundation::{BOOL, E_INVALIDARG, E_OUTOFMEMORY};
use windows::Win32::System::Com::CoTaskMemAlloc;
use windows::Win32::UI::Shell::{
    ICredentialProvider, ICredentialProviderCredential, ICredentialProviderEvents,
    ICredentialProvider_Impl, CPFT_SMALL_TEXT, CPFT_TILE_IMAGE, CPUS_INVALID, CPUS_LOGON,
    CPUS_UNLOCK_WORKSTATION, CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR,
    CREDENTIAL_PROVIDER_USAGE_SCENARIO,
};
use windows_core::{implement, PWSTR};

/// Number of tile fields: [0]=tile image, [1]=label text.
pub const FIELD_TILE: u32 = 0;
pub const FIELD_LABEL: u32 = 1;
pub const FIELD_COUNT: u32 = 2;

/// Provider that exposes one face-unlock credential tile.
#[implement(ICredentialProvider)]
pub struct FaceProvider {
    usage_scenario: Mutex<CREDENTIAL_PROVIDER_USAGE_SCENARIO>,
    events: Mutex<Option<ICredentialProviderEvents>>,
}

impl FaceProvider {
    pub fn new() -> Self {
        Self {
            usage_scenario: Mutex::new(CPUS_INVALID),
            events: Mutex::new(None),
        }
    }
}

impl Default for FaceProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl ICredentialProvider_Impl for FaceProvider_Impl {
    fn SetUsageScenario(
        &self,
        cpus: CREDENTIAL_PROVIDER_USAGE_SCENARIO,
        _dwflags: u32,
    ) -> windows_core::Result<()> {
        log::info!("SetUsageScenario: {:?}", cpus);
        match cpus {
            CPUS_LOGON | CPUS_UNLOCK_WORKSTATION => {
                *self.usage_scenario.lock().unwrap() = cpus;
                Ok(())
            }
            _ => Err(E_INVALIDARG.into()),
        }
    }

    fn SetSerialization(
        &self,
        _pcpcs: *const windows::Win32::UI::Shell::CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION,
    ) -> windows_core::Result<()> {
        Ok(())
    }

    fn Advise(
        &self,
        pcpe: Option<&ICredentialProviderEvents>,
        _upadvisecontext: usize,
    ) -> windows_core::Result<()> {
        *self.events.lock().unwrap() = pcpe.cloned();
        Ok(())
    }

    fn UnAdvise(&self) -> windows_core::Result<()> {
        *self.events.lock().unwrap() = None;
        Ok(())
    }

    fn GetFieldDescriptorCount(&self) -> windows_core::Result<u32> {
        Ok(FIELD_COUNT)
    }

    fn GetFieldDescriptorAt(
        &self,
        dwindex: u32,
    ) -> windows_core::Result<*mut CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR> {
        if dwindex >= FIELD_COUNT {
            return Err(E_INVALIDARG.into());
        }
        unsafe {
            let fd = CoTaskMemAlloc(std::mem::size_of::<CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR>())
                as *mut CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR;
            if fd.is_null() {
                return Err(E_OUTOFMEMORY.into());
            }
            (*fd).dwFieldID = dwindex;
            (*fd).cpft = if dwindex == FIELD_TILE {
                CPFT_TILE_IMAGE
            } else {
                CPFT_SMALL_TEXT
            };
            (*fd).pszLabel = if dwindex == FIELD_TILE {
                PWSTR::null()
            } else {
                let label: Vec<u16> = "Face Unlock\0".encode_utf16().collect();
                let ptr = CoTaskMemAlloc(label.len() * 2) as *mut u16;
                if !ptr.is_null() {
                    std::ptr::copy_nonoverlapping(label.as_ptr(), ptr, label.len());
                }
                PWSTR(ptr)
            };
            Ok(fd)
        }
    }

    // COM ABI: the `*mut` out-params are required by the COM interface and
    // cannot be marked unsafe; deref is intentional (null-checked below).
    #[allow(clippy::not_unsafe_ptr_arg_deref)]
    fn GetCredentialCount(
        &self,
        pdwcount: *mut u32,
        pdwdefault: *mut u32,
        pbautologonwithdefault: *mut BOOL,
    ) -> windows_core::Result<()> {
        unsafe {
            if !pdwcount.is_null() {
                *pdwcount = FIELD_COUNT;
            }
            if !pdwdefault.is_null() {
                *pdwdefault = 0;
            }
            if !pbautologonwithdefault.is_null() {
                *pbautologonwithdefault = BOOL(0);
            }
        }
        Ok(())
    }

    fn GetCredentialAt(&self, dwindex: u32) -> windows_core::Result<ICredentialProviderCredential> {
        if dwindex >= FIELD_COUNT {
            return Err(E_INVALIDARG.into());
        }
        let cred: ICredentialProviderCredential = crate::credential::FaceCredential::new().into();
        Ok(cred)
    }
}
