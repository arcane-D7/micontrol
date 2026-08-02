//! ICredentialProviderCredential implementation — the face tile behavior.

use std::sync::{Arc, Mutex};
use windows::Win32::Foundation::{BOOL, E_INVALIDARG, E_NOTIMPL, E_OUTOFMEMORY, NTSTATUS};
use windows::Win32::Graphics::Gdi::HBITMAP;
use windows::Win32::System::Com::CoTaskMemAlloc;
use windows::Win32::UI::Shell::{
    ICredentialProviderCredential, ICredentialProviderCredentialEvents,
    ICredentialProviderCredential_Impl, CPFIS_NONE, CPFS_DISPLAY_IN_BOTH,
    CPGSR_NO_CREDENTIAL_FINISHED, CPGSR_RETURN_CREDENTIAL_FINISHED,
    CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION, CREDENTIAL_PROVIDER_FIELD_INTERACTIVE_STATE,
    CREDENTIAL_PROVIDER_FIELD_STATE, CREDENTIAL_PROVIDER_GET_SERIALIZATION_RESPONSE,
    CREDENTIAL_PROVIDER_STATUS_ICON,
};
use windows_core::{implement, PCWSTR, PWSTR};

use crate::pipe_client;
use crate::provider::{FIELD_LABEL, FIELD_TILE};

/// Shared auth state (thread-safe; the COM credential wrapper cannot be
/// moved into a worker thread, so we share state via Arc).
#[derive(Default)]
struct AuthState {
    started: bool,
    success: bool,
    user: String,
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// The face credential: when selected, starts an auth attempt; when the
/// service reports success, GetSerialization reads the LSA password and
/// submits a Kerberos logon.
#[implement(ICredentialProviderCredential)]
pub struct FaceCredential {
    events: Mutex<Option<ICredentialProviderCredentialEvents>>,
    auth: Arc<Mutex<AuthState>>,
}

impl FaceCredential {
    pub fn new() -> Self {
        Self {
            events: Mutex::new(None),
            auth: Arc::new(Mutex::new(AuthState::default())),
        }
    }
}

impl Default for FaceCredential {
    fn default() -> Self {
        Self::new()
    }
}

/// Kick off an auth attempt and poll until done (max ~16 s).
/// Runs on a plain thread; state is shared via `Arc<Mutex<AuthState>>`.
fn run_auth_thread(auth: Arc<Mutex<AuthState>>) {
    {
        let mut a = auth.lock().unwrap();
        if a.started {
            return;
        }
        a.started = true;
    }

    // auth_start
    if pipe_client::send(r#"{"cmd":"auth_start"}"#).is_err() {
        log::warn!("FaceCP: auth_start pipe error");
        return;
    }

    // Poll auth_poll every 400 ms.
    for _ in 0..40 {
        std::thread::sleep(std::time::Duration::from_millis(400));
        match pipe_client::send(r#"{"cmd":"auth_poll"}"#) {
            Ok(json) => {
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) {
                    let done = v.get("done").and_then(|d| d.as_bool()).unwrap_or(false);
                    if done {
                        let success = v.get("success").and_then(|s| s.as_bool()).unwrap_or(false);
                        let user = v
                            .get("user")
                            .and_then(|u| u.as_str())
                            .unwrap_or("")
                            .to_string();
                        let mut a = auth.lock().unwrap();
                        a.success = success;
                        a.user = user;
                        return;
                    }
                }
            }
            Err(e) => {
                log::warn!("FaceCP: auth_poll error: {e}");
                return;
            }
        }
    }
    log::warn!("FaceCP: auth timed out");
}

impl ICredentialProviderCredential_Impl for FaceCredential_Impl {
    fn Advise(
        &self,
        pcpce: Option<&ICredentialProviderCredentialEvents>,
    ) -> windows_core::Result<()> {
        *self.events.lock().unwrap() = pcpce.map(|e| e.clone());
        Ok(())
    }

    fn UnAdvise(&self) -> windows_core::Result<()> {
        *self.events.lock().unwrap() = None;
        Ok(())
    }

    fn SetSelected(&self) -> windows_core::Result<BOOL> {
        log::info!("FaceCP: tile selected — starting auth");
        let auth = self.auth.clone();
        std::thread::spawn(move || run_auth_thread(auth));
        Ok(true.into())
    }

    fn SetDeselected(&self) -> windows_core::Result<()> {
        Ok(())
    }

    fn GetFieldState(
        &self,
        dwfieldid: u32,
        pcpfs: *mut CREDENTIAL_PROVIDER_FIELD_STATE,
        pcpfis: *mut CREDENTIAL_PROVIDER_FIELD_INTERACTIVE_STATE,
    ) -> windows_core::Result<()> {
        unsafe {
            match dwfieldid {
                FIELD_TILE | FIELD_LABEL => {
                    *pcpfs = CPFS_DISPLAY_IN_BOTH;
                    *pcpfis = CPFIS_NONE;
                    Ok(())
                }
                _ => Err(E_INVALIDARG.into()),
            }
        }
    }

    fn GetStringValue(&self, dwfieldid: u32) -> windows_core::Result<PWSTR> {
        let text = match dwfieldid {
            FIELD_LABEL => "Face Unlock",
            _ => "",
        };
        let wide = to_wide(text);
        unsafe {
            let ptr = CoTaskMemAlloc(wide.len() * 2) as *mut u16;
            if ptr.is_null() {
                return Err(E_OUTOFMEMORY.into());
            }
            std::ptr::copy_nonoverlapping(wide.as_ptr(), ptr, wide.len());
            Ok(PWSTR(ptr))
        }
    }

    fn GetBitmapValue(&self, _dwfieldid: u32) -> windows_core::Result<HBITMAP> {
        Ok(HBITMAP::default())
    }

    fn GetCheckboxValue(
        &self,
        _dwfieldid: u32,
        _pbchecked: *mut BOOL,
        _ppszlabel: *mut PWSTR,
    ) -> windows_core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn GetSubmitButtonValue(&self, _dwfieldid: u32) -> windows_core::Result<u32> {
        Err(E_NOTIMPL.into())
    }

    fn GetComboBoxValueCount(
        &self,
        _dwfieldid: u32,
        _pcitems: *mut u32,
        _pdwselecteditem: *mut u32,
    ) -> windows_core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn GetComboBoxValueAt(&self, _dwfieldid: u32, _dwitem: u32) -> windows_core::Result<PWSTR> {
        Err(E_NOTIMPL.into())
    }

    fn SetStringValue(&self, _dwfieldid: u32, _psz: &PCWSTR) -> windows_core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn SetCheckboxValue(&self, _dwfieldid: u32, _bchecked: BOOL) -> windows_core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn SetComboBoxSelectedValue(
        &self,
        _dwfieldid: u32,
        _dwselecteditem: u32,
    ) -> windows_core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn CommandLinkClicked(&self, _dwfieldid: u32) -> windows_core::Result<()> {
        let auth = self.auth.clone();
        std::thread::spawn(move || run_auth_thread(auth));
        Ok(())
    }

    /// Called when the system needs the actual credential to unlock.
    fn GetSerialization(
        &self,
        pcpgsr: *mut CREDENTIAL_PROVIDER_GET_SERIALIZATION_RESPONSE,
        pcpcs: *mut CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION,
        _ppszoptionalstatustext: *mut PWSTR,
        _pcpsioptionalstatusicon: *mut CREDENTIAL_PROVIDER_STATUS_ICON,
    ) -> windows_core::Result<()> {
        unsafe {
            let ok = self.auth.lock().unwrap().success;
            if !ok {
                log::warn!("FaceCP: GetSerialization without successful auth");
                *pcpgsr = CPGSR_NO_CREDENTIAL_FINISHED;
                return Ok(());
            }
            let user = self.auth.lock().unwrap().user.clone();
            log::info!("FaceCP: submitting credential for '{user}'");

            let password = match crate::credvault::read_password(&user) {
                Ok(p) => p,
                Err(e) => {
                    log::warn!("FaceCP: LSA read failed: {e}");
                    *pcpgsr = CPGSR_NO_CREDENTIAL_FINISHED;
                    return Ok(());
                }
            };

            let (buffer, size) = crate::kerb::pack_kerb_logon(&user, &password, "");

            (*pcpcs).clsidCredentialProvider = crate::FACE_PROVIDER_CLSID;
            (*pcpcs).rgbSerialization = buffer;
            (*pcpcs).cbSerialization = size;
            *pcpgsr = CPGSR_RETURN_CREDENTIAL_FINISHED;
            Ok(())
        }
    }

    fn ReportResult(
        &self,
        _ntsstatus: NTSTATUS,
        _ntssubstatus: NTSTATUS,
        _ppszoptionalstatustext: *mut PWSTR,
        _pcpsioptionalstatusicon: *mut CREDENTIAL_PROVIDER_STATUS_ICON,
    ) -> windows_core::Result<()> {
        Ok(())
    }
}
