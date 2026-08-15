//! Named-pipe client used by the credential provider.
//!
//! Connects to `\\.\pipe\micontrol_face`, verifies the server process is
//! LocalSystem (SID S-1-5-18), sends one JSON request, reads the JSON reply.

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::FromRawHandle;
use windows::Win32::Foundation::{
    CloseHandle, LocalFree, GENERIC_READ, GENERIC_WRITE, HANDLE, HLOCAL, INVALID_HANDLE_VALUE,
};
use windows::Win32::Security::{
    AllocateAndInitializeSid, EqualSid, GetTokenInformation, TokenUser, SID_IDENTIFIER_AUTHORITY,
    TOKEN_QUERY, TOKEN_USER,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
};
use windows::Win32::System::Pipes::{
    GetNamedPipeServerProcessId, SetNamedPipeHandleState, PIPE_READMODE_MESSAGE,
};
use windows::Win32::System::Threading::{
    OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

use crate::FACE_PIPE;

/// Verify that the pipe server process runs as LocalSystem.
fn server_is_localsystem(h: HANDLE) -> bool {
    unsafe {
        let mut server_pid: u32 = 0;
        if GetNamedPipeServerProcessId(h, &mut server_pid).is_err() || server_pid == 0 {
            return false;
        }
        let process = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, server_pid) {
            Ok(p) => p,
            Err(_) => return false,
        };
        let mut token = HANDLE::default();
        if OpenProcessToken(process, TOKEN_QUERY, &mut token).is_err() {
            let _ = CloseHandle(process);
            return false;
        }
        let mut size: u32 = 0;
        let _ = GetTokenInformation(token, TokenUser, None, 0, &mut size);
        let mut buf = vec![0u8; size as usize];
        if GetTokenInformation(
            token,
            TokenUser,
            Some(buf.as_mut_ptr() as *mut _),
            size,
            &mut size,
        )
        .is_err()
        {
            let _ = CloseHandle(token);
            let _ = CloseHandle(process);
            return false;
        }
        let _ = CloseHandle(token);
        let _ = CloseHandle(process);

        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let sid = user.User.Sid;
        let mut local_system: windows::Win32::Security::PSID =
            windows::Win32::Security::PSID(std::ptr::null_mut());
        let auth = SID_IDENTIFIER_AUTHORITY {
            Value: [0, 0, 0, 0, 0, 5],
        };
        if AllocateAndInitializeSid(&auth, 1, 18, 0, 0, 0, 0, 0, 0, 0, &mut local_system).is_err() {
            return false;
        }
        let is_ls = EqualSid(sid, local_system).is_ok();
        let _ = LocalFree(HLOCAL(local_system.0));
        is_ls
    }
}

/// Send a JSON request to the face auth service. Returns the JSON response.
pub fn send(request: &str) -> Result<String, String> {
    unsafe {
        let mut h: HANDLE = INVALID_HANDLE_VALUE;
        for _ in 0..30 {
            let wide: Vec<u16> = OsStr::new(FACE_PIPE).encode_wide().chain(Some(0)).collect();
            let r = CreateFileW(
                windows::core::PCWSTR(wide.as_ptr()),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                HANDLE::default(),
            );
            match r {
                Ok(handle) => {
                    h = handle;
                    break;
                }
                Err(_) => {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
            }
        }
        if h == INVALID_HANDLE_VALUE {
            return Err("auth service not running".into());
        }

        if !server_is_localsystem(h) {
            let _ = CloseHandle(h);
            return Err("auth service identity verification failed".into());
        }

        let mode = PIPE_READMODE_MESSAGE;
        let _ = SetNamedPipeHandleState(h, Some(&mode as *const _), None, None);

        let mut pipe = std::fs::File::from_raw_handle(h.0 as std::os::windows::io::RawHandle);
        pipe.write_all(request.as_bytes())
            .map_err(|e| e.to_string())?;
        pipe.flush().ok();

        let mut buf = vec![0u8; 65536];
        let mut total = 0usize;
        loop {
            if total >= buf.len() {
                break;
            }
            match pipe.read(&mut buf[total..]) {
                Ok(0) => break,
                Ok(n) => {
                    total += n;
                    if buf[..total].ends_with(b"}") {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        if total == 0 {
            return Err("no response from auth service".into());
        }
        Ok(String::from_utf8_lossy(&buf[..total]).to_string())
    }
}
