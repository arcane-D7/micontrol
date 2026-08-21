//! miControl Face Unlock — Windows Credential Provider (COM DLL).
//!
//! A Credential Provider that shows a "Face Unlock" tile on the Windows
//! lock screen / sign-in. It talks to the `micontrol_face_svc` LocalSystem
//! service over the `\\.\pipe\micontrol_face` named pipe (auth_start /
//! auth_poll), and on success reads the sign-in password from the LSA
//! Secret (`L$FaceHello_<user>`) and submits a Kerberos interactive logon
//! to unlock — the same approach as the hardened reference implementation
//! (`everglow01/Windows-Face-Hello`).
//!
//! Security:
//! - The password NEVER crosses the named pipe (the service only returns
//!   `{ok, user, similarity}`).
//! - The CP verifies the pipe server process is LocalSystem before sending.
//! - The fallback password/PIN provider is never removed.

use std::ffi::c_void;
use std::sync::atomic::{AtomicI32, Ordering};
use windows::Win32::Foundation::{
    BOOL, CLASS_E_CLASSNOTAVAILABLE, CLASS_E_NOAGGREGATION, E_INVALIDARG, S_FALSE, S_OK,
};
use windows::Win32::System::Com::IClassFactory;
use windows::Win32::UI::Shell::ICredentialProvider;
use windows_core::{implement, Interface, GUID, HRESULT};

pub mod credential;
pub mod credvault;
pub mod kerb;
pub mod pipe_client;
pub mod provider;
pub mod settings;

/// CLSID for the face unlock credential provider.
/// {E071A7CE-5D7F-4063-9A10-AE39AEC64EE8} (from the reference; regenerate for
/// your own distribution).
pub const FACE_PROVIDER_CLSID: GUID = GUID::from_u128(0xE071A7CE_5D7F_4063_9A10_AE39AEC64EE8);

/// Named pipe used by the credential provider to reach the auth service.
pub const FACE_PIPE: &str = r"\\.\pipe\micontrol_face";

/// Global DLL refcount (DllCanUnloadNow).
static G_REF_COUNT: AtomicI32 = AtomicI32::new(0);

pub fn dll_add_ref() {
    G_REF_COUNT.fetch_add(1, Ordering::SeqCst);
}

pub fn dll_release() {
    G_REF_COUNT.fetch_sub(1, Ordering::SeqCst);
}

/// Simple logger to a file (SYSTEM can't easily write to user dirs).
pub fn init_log() {
    let _ = std::fs::create_dir_all(r"C:\ProgramData\MiControl\face");
    let _ = fern::Dispatch::new()
        .format(|out, message, record| out.finish(format_args!("[{}] {}", record.level(), message)))
        .level(log::LevelFilter::Info)
        .chain(fern::log_file(r"C:\ProgramData\MiControl\face\facecp.log").expect("open log"))
        .apply();
    log::info!("FaceCP loaded (v{})", env!("CARGO_PKG_VERSION"));
}

/// Class factory for the credential provider.
#[implement(IClassFactory)]
struct FaceClassFactory;

impl windows::Win32::System::Com::IClassFactory_Impl for FaceClassFactory_Impl {
    fn CreateInstance(
        &self,
        punkouter: Option<&windows_core::IUnknown>,
        riid: *const GUID,
        ppv_object: *mut *mut c_void,
    ) -> windows_core::Result<()> {
        // Aggregation is not supported.
        if punkouter.is_some() {
            return Err(CLASS_E_NOAGGREGATION.into());
        }
        unsafe {
            if ppv_object.is_null() {
                return Err(E_INVALIDARG.into());
            }
            let provider: ICredentialProvider = provider::FaceProvider::new().into();
            let result = provider.query(riid, ppv_object);
            if result.is_err() {
                Err(E_INVALIDARG.into())
            } else {
                Ok(())
            }
        }
    }

    fn LockServer(&self, flock: BOOL) -> windows_core::Result<()> {
        if flock.as_bool() {
            dll_add_ref();
        } else {
            dll_release();
        }
        Ok(())
    }
}

/// COM export: get the class factory for our CLSID.
///
/// # Safety
///
/// `rclsid`, `riid` and `ppv` must be valid pointers for the duration of the
/// call, as mandated by COM's `DllGetClassObject` ABI. `ppv` must point to
/// writable storage that receives the interface pointer on success.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllGetClassObject(
    rclsid: *const GUID,
    riid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if rclsid.is_null() || riid.is_null() || ppv.is_null() {
        return E_INVALIDARG;
    }
    if unsafe { *rclsid } == FACE_PROVIDER_CLSID {
        let factory: IClassFactory = FaceClassFactory.into();
        unsafe {
            let hr = factory.query(riid, ppv);
            if hr.is_ok() {
                S_OK
            } else {
                E_INVALIDARG
            }
        }
    } else {
        CLASS_E_CLASSNOTAVAILABLE
    }
}

/// COM export: can the DLL be unloaded?
///
/// # Safety
///
/// Safe when called as specified by COM's `DllCanUnloadNow` contract: it
/// reads only the process-global refcount and returns a status code.
#[unsafe(no_mangle)]
pub unsafe extern "system" fn DllCanUnloadNow() -> HRESULT {
    if G_REF_COUNT.load(Ordering::SeqCst) == 0 {
        S_OK
    } else {
        S_FALSE
    }
}
