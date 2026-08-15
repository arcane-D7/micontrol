//! LSA Secret reader for the credential provider (SYSTEM context).
//!
//! Reads `L$FaceHello_<user>` — the sign-in password stored by the miControl
//! admin console. The CP runs as SYSTEM in LogonUI, which has
//! POLICY_GET_PRIVATE_INFORMATION.

use windows::Win32::Foundation::LocalFree;
use windows::Win32::Foundation::NTSTATUS;
use windows::Win32::Security::Authentication::Identity::{
    LsaClose, LsaOpenPolicy, LsaRetrievePrivateData, LSA_HANDLE, LSA_OBJECT_ATTRIBUTES,
    LSA_UNICODE_STRING,
};

const POLICY_GET_PRIVATE_INFORMATION: u32 = 0x0004;
const LSA_SECRET_PREFIX: &str = "L$FaceHello_";

fn to_lsa_string(s: &str) -> LSA_UNICODE_STRING {
    let wide: Vec<u16> = s.encode_utf16().collect();
    LSA_UNICODE_STRING {
        Length: (wide.len() * 2) as u16,
        MaximumLength: (wide.len() * 2) as u16,
        Buffer: windows::core::PWSTR(wide.as_ptr() as *mut u16),
    }
}

/// Read the stored sign-in password for a Windows account (SYSTEM/elevated).
pub fn read_password(user: &str) -> Result<String, String> {
    let secret_name = format!("{LSA_SECRET_PREFIX}{user}");
    let name = to_lsa_string(&secret_name);

    let attrs = LSA_OBJECT_ATTRIBUTES::default();
    let mut policy = LSA_HANDLE::default();
    let status =
        unsafe { LsaOpenPolicy(None, &attrs, POLICY_GET_PRIVATE_INFORMATION, &mut policy) };
    if status != NTSTATUS(0) {
        return Err(format!("LsaOpenPolicy 0x{:08X}", status.0));
    }

    let mut data_ptr: *mut LSA_UNICODE_STRING = std::ptr::null_mut();
    let status = unsafe { LsaRetrievePrivateData(policy, &name, &mut data_ptr) };
    unsafe {
        let _ = LsaClose(policy);
    };
    if status != NTSTATUS(0) {
        return Err(format!("LsaRetrievePrivateData 0x{:08X}", status.0));
    }
    if data_ptr.is_null() {
        return Err("secret not found".into());
    }

    let data = unsafe { &*data_ptr };
    let len = (data.Length as usize) / 2;
    let password = unsafe { std::slice::from_raw_parts(data.Buffer.0 as *const u16, len).to_vec() };
    let password = String::from_utf16_lossy(&password);
    unsafe {
        let _ = LocalFree(windows::Win32::Foundation::HLOCAL(data_ptr as *mut _));
    };
    Ok(password)
}
