//! Credential serialization for the credential provider.
//!
//! Uses the official `CredPackAuthenticationBufferW` API (Windows SDK) to
//! build a `CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION`-compatible buffer,
//! avoiding fragile manual KERB struct packing. This is the approach used by
//! `facewinunlock`'s CP and is what Microsoft's own sample CP does.

use windows::Win32::Security::Credentials::{
    CredPackAuthenticationBufferW, CRED_PACK_PROTECTED_CREDENTIALS,
};
use windows::Win32::System::Com::CoTaskMemAlloc;

/// Pack a username+password+domain into a serialized credential buffer.
///
/// Returns `(buffer, size)` allocated via CoTaskMemAlloc (LogonUI frees it).
pub fn pack_kerb_logon(user: &str, password: &str, _domain: &str) -> (*mut u8, u32) {
    unsafe {
        let user_wide: Vec<u16> = user.encode_utf16().collect();
        let pass_wide: Vec<u16> = password.encode_utf16().collect();
        let user_pw = windows_core::PCWSTR(user_wide.as_ptr());
        let pass_pw = windows_core::PCWSTR(pass_wide.as_ptr());

        // First call: get required size.
        let mut size: u32 = 0;
        let _ = CredPackAuthenticationBufferW(
            CRED_PACK_PROTECTED_CREDENTIALS,
            user_pw,
            pass_pw,
            None,
            &mut size,
        );
        if size == 0 {
            return (std::ptr::null_mut(), 0);
        }

        let buffer = CoTaskMemAlloc(size as usize) as *mut u8;
        if buffer.is_null() {
            return (std::ptr::null_mut(), 0);
        }
        let mut filled = size;
        let ok = CredPackAuthenticationBufferW(
            CRED_PACK_PROTECTED_CREDENTIALS,
            user_pw,
            pass_pw,
            Some(buffer),
            &mut filled,
        );
        if ok.is_err() {
            let _ =
                windows::Win32::System::Com::CoTaskMemFree(Some(buffer as *mut core::ffi::c_void));
            return (std::ptr::null_mut(), 0);
        }
        (buffer, filled)
    }
}

/// Resolve the Negotiate auth package ID (needed for the CP's
/// `CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION.ulAuthenticationPackage`).
pub fn retrieve_negotiate_auth_package() -> Result<u32, String> {
    use windows::Win32::Foundation::{HANDLE, NTSTATUS};
    use windows::Win32::Security::Authentication::Identity::{
        LsaConnectUntrusted, LsaDeregisterLogonProcess, LsaLookupAuthenticationPackage, LSA_STRING,
    };

    unsafe {
        let mut lsa = HANDLE::default();
        let status = LsaConnectUntrusted(&mut lsa);
        if status != NTSTATUS(0) {
            return Err(format!("LsaConnectUntrusted 0x{:08X}", status.0));
        }
        let mut name = LSA_STRING::default();
        let negotiate = "Negotiate";
        let bytes = negotiate.as_bytes();
        name.Length = bytes.len() as u16;
        name.MaximumLength = (bytes.len() + 1) as u16;
        name.Buffer = windows_core::PSTR(bytes.as_ptr() as *mut u8);
        let mut package: u32 = 0;
        let status = LsaLookupAuthenticationPackage(lsa, &name, &mut package);
        let _ = LsaDeregisterLogonProcess(lsa);
        if status != NTSTATUS(0) {
            return Err(format!("LsaLookupAuthenticationPackage 0x{:08X}", status.0));
        }
        Ok(package)
    }
}
