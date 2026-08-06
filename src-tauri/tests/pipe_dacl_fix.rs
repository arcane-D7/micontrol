//! Temporary verification test for the fixed named-pipe DACL.
//!
//! Creates a pipe with the same SECURITY_ATTRIBUTES logic used by
//! `ecram_service` / `micontrol_bridge` (DACL granting Everyone GENERIC_READ
//! | GENERIC_WRITE, heap-leaked SD), then verifies that an unprivileged
//! open with GENERIC_READ|GENERIC_WRITE succeeds.
//!
//! Run: cargo test --manifest-path src-tauri/Cargo.toml --test pipe_dacl_fix -- --nocapture

#![cfg(windows)]

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};

const TEST_PIPE: &str = r"\\.\pipe\micontrol_dacl_verify";

/// Same logic as `build_pipe_security_attributes()` in micontrol_bridge/ecram_service.
fn build_test_security() -> Option<windows::Win32::Security::SECURITY_ATTRIBUTES> {
    use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
    use windows::Win32::Security::{
        Authorization::{
            BuildExplicitAccessWithNameW, SetEntriesInAclW, EXPLICIT_ACCESS_W, SET_ACCESS,
        },
        SetSecurityDescriptorDacl, ACE_FLAGS, ACL, SECURITY_DESCRIPTOR,
    };
    unsafe {
        let everyone_w: Vec<u16> = "Everyone".encode_utf16().chain(Some(0)).collect();
        let mut ea = EXPLICIT_ACCESS_W::default();
        BuildExplicitAccessWithNameW(
            &mut ea,
            PCWSTR(everyone_w.as_ptr()),
            GENERIC_READ.0 | GENERIC_WRITE.0,
            SET_ACCESS,
            ACE_FLAGS(0),
        );
        let entries = [ea];
        let mut new_acl: *mut ACL = std::ptr::null_mut();
        if SetEntriesInAclW(Some(&entries), None, &mut new_acl).is_err() || new_acl.is_null() {
            return None;
        }
        let mut sd = SECURITY_DESCRIPTOR::default();
        let sd_ptr = windows::Win32::Security::PSECURITY_DESCRIPTOR(
            (&mut sd as *mut SECURITY_DESCRIPTOR).cast(),
        );
        if SetSecurityDescriptorDacl(sd_ptr, true, Some(new_acl), false).is_err() {
            return None;
        }
        let sd_leaked = Box::leak(Box::new(sd)) as *mut SECURITY_DESCRIPTOR;
        Some(windows::Win32::Security::SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<windows::Win32::Security::SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd_leaked.cast(),
            bInheritHandle: false.into(),
        })
    }
}

fn open_rw(name: &str) -> Result<(), u32> {
    let path_w: Vec<u16> = OsStr::new(name).encode_wide().chain(Some(0)).collect();
    let handle = unsafe {
        CreateFileW(
            PCWSTR(path_w.as_ptr()),
            (windows::Win32::Foundation::GENERIC_READ | windows::Win32::Foundation::GENERIC_WRITE)
                .0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            HANDLE::default(),
        )
    };
    match handle {
        Ok(h) if h != INVALID_HANDLE_VALUE => {
            unsafe {
                CloseHandle(h).ok();
            }
            Ok(())
        }
        _ => Err(std::io::Error::last_os_error()
            .raw_os_error()
            .unwrap_or(999) as u32),
    }
}

#[test]
fn unprivileged_client_can_open_pipe_rw() {
    use windows::Win32::Storage::FileSystem::FILE_FLAG_FIRST_PIPE_INSTANCE;
    use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
    };

    // Clean up any leftover instance from a crashed previous run.
    let _ = open_rw(TEST_PIPE);

    let pipe_name_w: Vec<u16> = OsStr::new(TEST_PIPE).encode_wide().chain(Some(0)).collect();
    let security = build_test_security();

    let server = unsafe {
        CreateNamedPipeW(
            PCWSTR(pipe_name_w.as_ptr()),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
            1,
            4096,
            4096,
            0,
            security
                .as_ref()
                .map(|s| s as *const windows::Win32::Security::SECURITY_ATTRIBUTES),
        )
    };
    assert_ne!(
        server,
        INVALID_HANDLE_VALUE,
        "CreateNamedPipeW failed: {}",
        std::io::Error::last_os_error()
    );

    // Try to open with GENERIC_READ|GENERIC_WRITE (what the MiControl app does).
    let open_result = open_rw(TEST_PIPE);

    // Accept and cleanup the pending connection.
    let _ = unsafe { ConnectNamedPipe(server, None) };
    std::thread::sleep(std::time::Duration::from_millis(200));
    unsafe {
        CloseHandle(server).ok();
    }

    match open_result {
        Ok(()) => {
            eprintln!("[pass] unprivileged GENERIC_READ|GENERIC_WRITE open succeeded");
        }
        Err(code) => {
            panic!("unprivileged GENERIC_READ|GENERIC_WRITE open FAILED with error {code} (5=ACCESS_DENIED)");
        }
    }
}
