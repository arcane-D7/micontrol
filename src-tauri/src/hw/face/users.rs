//! Local Windows user enumeration + Windows Hello consent gate.
//!
//! # Users dropdown
//!
//! Enumerates local user accounts via [`NetUserEnum`] (NET API 32), returning
//! each account's name + SID (via [`LookupAccountNameW`] against the local
//! machine). Used by the enrollment wizard to let the user pick which Windows
//! account the face-unlock credential should unlock, instead of free-typing a
//! name.
//!
//! `IUserConsentVerifierInterop::RequestVerificationForWindowAsync` is the
//! sanctioned desktop-API to confirm "it's really you" using the user's
//! configured **Windows Hello** (PIN, fingerprint or face). It is used here as
//! an enrollment *gate*: before storing a password in the LSA Secret, we ask
//! the user to authenticate with their existing Hello factor. This matches
//! Microsoft's docs — the API confirms presence in the *logged-in* session and
//! does **not** unlock the workstation — which is exactly the enrollment-time
//! semantics we want.
//!
//! The consent call must run on a thread with an active core-window/message
//! pump associated with the calling HWND; we run it on a dedicated thread with
//! the app's main window handle (Tauri `WebviewWindow::hwnd()`).

use crate::hw::face::errors::{FaceError, FaceResult};

/// A local Windows account usable for interactive sign-in.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LocalUser {
    pub name: String,
    pub sid: Option<String>,
    pub enabled: bool,
}

#[cfg(windows)]
mod win {
    use super::*;
    use windows::core::PCWSTR;

    /// Maximum buffer count for `NetUserEnum` (5,000 users is plenty).
    const MAX_PREFERRED_LENGTH: u32 = 0xFFFF_FFFF;
    const FILTER_NORMAL_ACCOUNT: u32 = 0x0000_0002; // UF_NORMAL_ACCOUNT
    const NERR_SUCCESS: i32 = 0;

    unsafe extern "system" {
        fn NetUserEnum(
            servername: PCWSTR,
            level: u32,
            filter: u32,
            bufptr: *mut *mut u8,
            prefmaxlen: u32,
            entriesread: *mut u32,
            totalentries: *mut u32,
            resume_handle: *mut u32,
        ) -> i32;
        fn NetApiBufferFree(buffer: *mut u8) -> i32;
        fn LookupAccountNameW(
            lpsystemname: PCWSTR,
            lpaccountname: PCWSTR,
            sid: *mut u8,
            cbsid: *mut u32,
            referenceddomainname: *mut u16,
            cchreferenceddomainname: *mut u32,
            peuse: *mut i32,
        ) -> i32;
    }

    #[repr(C)]
    struct USER_INFO_0 {
        usri0_name: *mut u16,
    }

    fn to_wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn account_sid(name: &str) -> Option<String> {
        // First call with null SID to get required size.
        let mut cb_sid: u32 = 0;
        let mut cb_domain: u32 = 0;
        let name_w = to_wide(name);
        let mut use_kind: i32 = 0;
        // SAFETY: sizing call — output pointers real, buffers null.
        let r1 = unsafe {
            LookupAccountNameW(
                PCWSTR::null(),
                PCWSTR(name_w.as_ptr()),
                std::ptr::null_mut(),
                &mut cb_sid,
                std::ptr::null_mut(),
                &mut cb_domain,
                &mut use_kind,
            )
        };
        if r1 == 0 && cb_sid == 0 {
            return None;
        }
        let mut sid = vec![0u8; cb_sid as usize];
        let mut domain = vec![0u16; cb_domain.max(1) as usize];
        // SAFETY: buffers sized per the previous call.
        let r2 = unsafe {
            LookupAccountNameW(
                PCWSTR::null(),
                PCWSTR(name_w.as_ptr()),
                sid.as_mut_ptr(),
                &mut cb_sid,
                domain.as_mut_ptr(),
                &mut cb_domain,
                &mut use_kind,
            )
        };
        if r2 == 0 {
            return None;
        }
        // Convert binary SID to the string form via ConvertSidToStringSidW.
        let mut str_sid: *mut u16 = std::ptr::null_mut();
        extern "system" {
            fn ConvertSidToStringSidW(sid: *const u8, stringsid: *mut *mut u16) -> i32;
            fn LocalFree(h: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
        }
        // SAFETY: sid is a valid SID of cb_sid bytes.
        let ok = unsafe { ConvertSidToStringSidW(sid.as_ptr(), &mut str_sid) };
        if ok == 0 || str_sid.is_null() {
            return None;
        }
        // SAFETY: str_sid is null-terminated wide string allocated by RtlAllocateHeap.
        let s = unsafe { PCWSTR(str_sid).to_string().ok() };
        // SAFETY: free the LocalAlloc buffer.
        unsafe { LocalFree(str_sid as *mut core::ffi::c_void) };
        s
    }

    /// Enumerate local users (level 0) and filter to interactive-capable accounts.
    pub(super) fn local_users() -> FaceResult<Vec<LocalUser>> {
        let mut buf: *mut u8 = std::ptr::null_mut();
        let mut entries_read: u32 = 0;
        let mut total: u32 = 0;
        let mut resume: u32 = 0;
        // SAFETY: all out-pointers valid; system null → local machine.
        let rc = unsafe {
            NetUserEnum(
                PCWSTR::null(),
                0,
                FILTER_NORMAL_ACCOUNT,
                &mut buf,
                MAX_PREFERRED_LENGTH,
                &mut entries_read,
                &mut total,
                &mut resume,
            )
        };
        if rc != NERR_SUCCESS {
            return Err(FaceError::Users(format!("NetUserEnum failed: {rc}")));
        }
        let mut users = Vec::new();
        if !buf.is_null() {
            // SAFETY: buf points to entries_read USER_INFO_0 structs.
            let slice = unsafe {
                std::slice::from_raw_parts(buf as *const USER_INFO_0, entries_read as usize)
            };
            for entry in slice {
                if entry.usri0_name.is_null() {
                    continue;
                }
                // SAFETY: null-terminated wide string.
                let name = unsafe { PCWSTR(entry.usri0_name).to_string().unwrap_or_default() };
                let sid = account_sid(&name);
                let enabled = true; // NetUserEnum level 0 doesn't carry flags
                users.push(LocalUser { name, sid, enabled });
            }
            // SAFETY: free the NetApiBuffer.
            unsafe { NetApiBufferFree(buf) };
        }
        // Sort by name; keep "Administrator" at the end like Windows does not —
        // just alphabetical, deterministic for the UI dropdown.
        users.sort_by_key(|a| a.name.to_lowercase());
        Ok(users)
    }
}

#[cfg(not(windows))]
fn local_users() -> FaceResult<Vec<LocalUser>> {
    Err(FaceError::NotSupported(
        "local user enumeration requires Windows".into(),
    ))
}

/// Public entry point — list local interactive user accounts.
pub fn list_local_users() -> FaceResult<Vec<LocalUser>> {
    #[cfg(windows)]
    {
        win::local_users()
    }
    #[cfg(not(windows))]
    {
        local_users()
    }
}

// ── Windows Hello (IUserConsentVerifierInterop) consent gate ───────────────

/// Result of the Windows Hello consent gate.
#[derive(Debug, Clone, serde::Serialize)]
pub struct HelloResult {
    /// One of: verified | canceled | device_not_present | not_configured |
    /// device_busy | retries_exhausted | failed.
    pub status: String,
    /// True when `status == "verified"`.
    pub verified: bool,
    /// Human-readable message for the UI.
    pub message: String,
}

#[cfg(windows)]
mod hello_win {
    use super::*;
    use windows::core::{factory, HSTRING};
    use windows::Foundation::IAsyncOperation;
    use windows::Security::Credentials::UI::{
        UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability,
    };
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::WinRT::IUserConsentVerifierInterop;

    /// Blocking call: asks the OS to present the user's configured Windows
    /// Hello factor (PIN / fingerprint / face) in a modal attached to `hwnd`.
    ///
    /// Must run on a thread whose HWND is associated to a message queue; we do
    /// this on a dedicated thread, and `RequestVerificationForWindowAsync`
    /// handles cross-thread marshalling internally.
    pub(super) fn verify_with_window(message: &str, hwnd: HWND) -> FaceResult<HelloResult> {
        // Cross-thread run: the interop requires a UI thread with a window
        // message pump associated. The Tauri main window lives on the main
        // thread, but the command handler runs on a Tauri async worker — so
        // spawn and join a dedicated thread.
        let message = message.to_string();
        let hwnd = hwnd.0 as usize;
        std::thread::Builder::new()
            .name("face-hello-consent".into())
            .spawn(move || verify_impl(&message, HWND(hwnd as *mut core::ffi::c_void)))
            .map_err(|e| FaceError::Hello(format!("spawn consent thread: {e}")))?
            .join()
            .map_err(|_| FaceError::Hello("consent thread panicked".into()))?
    }

    fn verify_impl(message: &str, hwnd: HWND) -> FaceResult<HelloResult> {
        // Safety-check availability first (cheap, non-interactive).
        let availability = match UserConsentVerifier::CheckAvailabilityAsync() {
            Ok(op) => match op.get() {
                Ok(a) => a,
                Err(e) => {
                    return Ok(HelloResult {
                        status: "failed".into(),
                        verified: false,
                        message: format!("CheckAvailabilityAsync: {e}"),
                    });
                }
            },
            Err(e) => {
                return Ok(HelloResult {
                    status: "not_supported".into(),
                    verified: false,
                    message: format!("Windows Hello not available: {e}"),
                });
            }
        };

        match availability {
            UserConsentVerifierAvailability::Available => {}
            UserConsentVerifierAvailability::NotConfiguredForUser => {
                return Ok(HelloResult {
                    status: "not_configured".into(),
                    verified: false,
                    message: "Windows Hello (PIN / fingerprint / face) is not set up for this \
                              user yet. Open Settings → Accounts → Sign-in options and set up \
                              a PIN or fingerprint, then try again."
                        .into(),
                });
            }
            UserConsentVerifierAvailability::DeviceNotPresent => {
                return Ok(HelloResult {
                    status: "device_not_present".into(),
                    verified: false,
                    message: "No Windows Hello capabable device (fingerprint reader, IR camera) \
                              is present on this machine."
                        .into(),
                });
            }
            UserConsentVerifierAvailability::DeviceBusy => {
                return Ok(HelloResult {
                    status: "device_busy".into(),
                    verified: false,
                    message: "The Windows Hello device is busy right now — wait a moment and try \
                              again."
                        .into(),
                });
            }
            UserConsentVerifierAvailability::DisabledByPolicy => {
                return Ok(HelloResult {
                    status: "disabled_by_policy".into(),
                    verified: false,
                    message: "Windows Hello is disabled by policy on this machine.".into(),
                });
            }
            other => {
                return Ok(HelloResult {
                    status: "failed".into(),
                    verified: false,
                    message: format!("Windows Hello availability: {other:?}"),
                });
            }
        }

        // Present the consent dialog on the app window. Use the Win32 interop
        // interface (`IUserConsentVerifierInterop`) which is the desktop-
        // friendly path (Windows 11 build 22000+).
        let hstring = HSTRING::from(message);
        let result = make_consent_call(hwnd, &hstring);
        match result {
            Ok(op) => match op.get() {
                Ok(UserConsentVerificationResult::Verified) => Ok(HelloResult {
                    status: "verified".into(),
                    verified: true,
                    message: "Windows Hello confirmed the user.".into(),
                }),
                Ok(UserConsentVerificationResult::Canceled) => Ok(HelloResult {
                    status: "canceled".into(),
                    verified: false,
                    message: "Windows Hello verification was canceled.".into(),
                }),
                Ok(UserConsentVerificationResult::RetriesExhausted) => Ok(HelloResult {
                    status: "retries_exhausted".into(),
                    verified: false,
                    message: "Windows Hello verification retries exhausted.".into(),
                }),
                Ok(UserConsentVerificationResult::DeviceNotPresent) => Ok(HelloResult {
                    status: "device_not_present".into(),
                    verified: false,
                    message: "Windows Hello device not present.".into(),
                }),
                Ok(UserConsentVerificationResult::DeviceBusy) => Ok(HelloResult {
                    status: "device_busy".into(),
                    verified: false,
                    message: "The Windows Hello device is busy.".into(),
                }),
                Ok(_) => Ok(HelloResult {
                    status: "failed".into(),
                    verified: false,
                    message: "Windows Hello verification failed.".into(),
                }),
                Err(e) => Ok(HelloResult {
                    status: "failed".into(),
                    verified: false,
                    message: format!("Windows Hello verification error: {e}"),
                }),
            },
            Err(e) => Ok(HelloResult {
                status: "failed".into(),
                verified: false,
                message: format!("Windows Hello dialog error: {e}"),
            }),
        }
    }

    /// `RoGetActivationFactory(Windows.Security.Credentials.UI.UserConsentVerifier)`
    /// → cast to `IUserConsentVerifierInterop` → call `RequestVerificationForWindowAsync`.
    fn make_consent_call(
        hwnd: HWND,
        message: &HSTRING,
    ) -> windows_core::Result<IAsyncOperation<UserConsentVerificationResult>> {
        // SAFETY: `factory` returns a valid COM interface pointer or an error.
        let interop: IUserConsentVerifierInterop =
            factory::<UserConsentVerifier, IUserConsentVerifierInterop>()?;
        // SAFETY: hwnd is a valid top-level window handle; message valid for the duration
        // of the (blocking) call.
        unsafe { interop.RequestVerificationForWindowAsync(hwnd, message) }
    }
}

#[cfg(not(windows))]
fn verify_with_window(_message: &str, _hwnd: *mut core::ffi::c_void) -> FaceResult<HelloResult> {
    Err(FaceError::NotSupported(
        "Windows Hello consent requires Windows".into(),
    ))
}

/// Run the Windows Hello consent gate for the given main-window HWND.
pub fn verify_hello(message: &str, hwnd: *mut core::ffi::c_void) -> FaceResult<HelloResult> {
    #[cfg(windows)]
    {
        hello_win::verify_with_window(message, windows::Win32::Foundation::HWND(hwnd))
    }
    #[cfg(not(windows))]
    {
        verify_with_window(message, hwnd)
    }
}
