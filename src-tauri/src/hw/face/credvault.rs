//! LSA Secret vault — stores/reads the Windows sign-in password used by the
//! Credential Provider to unlock the workstation.
//!
//! Port of the reference `face_hello/cred_vault.py`:
//! - Secret name: `L$FaceHello_<user>` (the `L$` prefix marks LSA Secrets).
//! - **Write** requires `POLICY_CREATE_SECRET` (admin/elevated console).
//! - **Read** requires `POLICY_GET_PRIVATE_INFORMATION` (SYSTEM service / CP).
//! - The password is UTF-16LE, stored in the secret's data buffer.
//!
//! Security note (mirrors the reference): the password never crosses the
//! named pipe — the CP reads it itself in SYSTEM context at unlock time.

use crate::hw::face::config::LSA_SECRET_PREFIX;
use crate::hw::face::errors::{FaceError, FaceResult};

/// Build the LSA Secret name for a Windows SAM account name.
pub fn secret_name_for_user(user: &str) -> String {
    format!("{LSA_SECRET_PREFIX}{user}")
}

#[cfg(windows)]
mod lsa_impl {
    use super::*;
    use windows::core::PWSTR;
    use windows::Win32::Foundation::{LocalFree, NTSTATUS};
    use windows::Win32::Security::Authentication::Identity::{
        LsaClose, LsaOpenPolicy, LsaRetrievePrivateData, LsaStorePrivateData, LSA_HANDLE,
        LSA_OBJECT_ATTRIBUTES, LSA_UNICODE_STRING,
    };

    const POLICY_CREATE_SECRET: u32 = 0x0020;
    const POLICY_GET_PRIVATE_INFORMATION: u32 = 0x0004;

    fn to_lsa_string(s: &str) -> LSA_UNICODE_STRING {
        let wide: Vec<u16> = s.encode_utf16().collect();
        LSA_UNICODE_STRING {
            Length: (wide.len() * 2) as u16,
            MaximumLength: (wide.len() * 2) as u16,
            Buffer: PWSTR(wide.as_ptr() as *mut u16),
        }
    }

    fn open_policy(access: u32) -> FaceResult<LSA_HANDLE> {
        let mut attrs = LSA_OBJECT_ATTRIBUTES::default();
        // SAFETY: valid object attributes; name is null (local policy).
        let mut policy = LSA_HANDLE::default();
        let status = unsafe { LsaOpenPolicy(None, &mut attrs, access, &mut policy) };
        if status != NTSTATUS(0) {
            return Err(FaceError::CredVault(format!(
                "LsaOpenPolicy failed: 0x{:08X}",
                status.0
            )));
        }
        Ok(policy)
    }

    /// Store the password for a user (elevated).
    pub fn store_password(user: &str, password: &str) -> FaceResult<()> {
        let secret_name = secret_name_for_user(user);
        let name_str = to_lsa_string(&secret_name);
        let wide: Vec<u16> = password.encode_utf16().collect();
        let data = LSA_UNICODE_STRING {
            Length: (wide.len() * 2) as u16,
            MaximumLength: (wide.len() * 2) as u16,
            Buffer: PWSTR(wide.as_ptr() as *mut u16),
        };

        let policy = open_policy(POLICY_CREATE_SECRET)?;
        // SAFETY: policy valid; name valid; data valid for the call duration.
        let status = unsafe { LsaStorePrivateData(policy, &name_str, Some(&data as *const _)) };
        // SAFETY: close policy handle (log-and-ignore best-effort NTSTATUS).
        let _close_status = unsafe { LsaClose(policy) };
        if status != NTSTATUS(0) {
            return Err(FaceError::CredVault(format!(
                "LsaStorePrivateData failed: 0x{:08X}",
                status.0
            )));
        }
        Ok(())
    }

    /// Retrieve the password for a user (SYSTEM / elevated).
    pub fn read_password(user: &str) -> FaceResult<String> {
        let secret_name = secret_name_for_user(user);
        let name_str = to_lsa_string(&secret_name);
        let policy = open_policy(POLICY_GET_PRIVATE_INFORMATION)?;
        let mut data_ptr: *mut LSA_UNICODE_STRING = std::ptr::null_mut();
        // SAFETY: policy valid; name valid; output pointer valid.
        let status = unsafe { LsaRetrievePrivateData(policy, &name_str, &mut data_ptr) };
        // SAFETY: close policy handle (log-and-ignore best-effort NTSTATUS).
        let _close_status = unsafe { LsaClose(policy) };
        if status != NTSTATUS(0) {
            return Err(FaceError::CredVault(format!(
                "LsaRetrievePrivateData failed: 0x{:08X}",
                status.0
            )));
        }
        if data_ptr.is_null() {
            return Err(FaceError::CredVault("secret not found".into()));
        }
        // SAFETY: data_ptr is a valid LSA-allocated LSA_UNICODE_STRING.
        let data = unsafe { &*data_ptr };
        let len_u16 = (data.Length as usize) / 2;
        // SAFETY: Buffer is a valid pointer with Length bytes.
        let password = unsafe {
            std::slice::from_raw_parts(data.Buffer.0 as *const u16, len_u16)
                .iter()
                .map(|&c| c)
                .collect::<Vec<u16>>()
        };
        let password = String::from_utf16_lossy(&password);
        // SAFETY: LSA-allocated buffer must be freed with LsaFreeReturnBuffer via LocalFree.
        unsafe { LocalFree(windows::Win32::Foundation::HLOCAL(data_ptr as *mut _)) };
        Ok(password)
    }

    /// Delete the secret (uninstall / forget).
    pub fn delete_password(user: &str) -> FaceResult<()> {
        let secret_name = secret_name_for_user(user);
        let name_str = to_lsa_string(&secret_name);
        let policy = open_policy(POLICY_CREATE_SECRET)?;
        // SAFETY: policy valid; name valid; data null = delete.
        let status = unsafe { LsaStorePrivateData(policy, &name_str, None) };
        let _close_status = unsafe { LsaClose(policy) };
        if status != NTSTATUS(0) {
            return Err(FaceError::CredVault(format!(
                "LsaStorePrivateData(delete) failed: 0x{:08X}",
                status.0
            )));
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod lsa_impl {
    use super::*;
    pub fn store_password(_u: &str, _p: &str) -> FaceResult<()> {
        Err(FaceError::NotSupported(
            "LSA secrets require Windows".into(),
        ))
    }
    pub fn read_password(_u: &str) -> FaceResult<String> {
        Err(FaceError::NotSupported(
            "LSA secrets require Windows".into(),
        ))
    }
    pub fn delete_password(_u: &str) -> FaceResult<()> {
        Err(FaceError::NotSupported(
            "LSA secrets require Windows".into(),
        ))
    }
}

pub use lsa_impl::{delete_password, read_password, store_password};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_name_format() {
        assert_eq!(secret_name_for_user("alice"), "L$FaceHello_alice");
        assert_eq!(
            secret_name_for_user("user@example.com"),
            "L$FaceHello_user@example.com"
        );
    }
}
