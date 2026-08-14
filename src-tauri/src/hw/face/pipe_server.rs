//! Named-pipe server for the face auth service.
//!
//! Port of the reference `face_hello/service.py` pipe protocol:
//! - Message-mode pipe, one JSON request per message.
//! - DACL: SYSTEM + Administrators only.
//! - `FILE_FLAG_FIRST_PIPE_INSTANCE` anti-squatting.
//! - Commands: `ping`, `auth_start`, `auth_poll` (production).
//! - The sign-in password never crosses the pipe.
//!
//! The service is single-instance serial: it accepts one client, handles the
//! request, responds, disconnects, then re-creates the pipe instance.

use crate::hw::face::config::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_NAME};
use crate::hw::face::errors::{FaceError, FaceResult};
use serde_json::{json, Value};

/// Response to a request.
pub type PipeResponse = Value;

#[cfg(windows)]
mod winpipe {
    use super::*;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
    use windows::Win32::Security::{
        Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1},
        PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
    };
    use windows::Win32::Storage::FileSystem::{
        FlushFileBuffers, ReadFile, WriteFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_FLAG_OVERLAPPED,
        PIPE_ACCESS_DUPLEX,
    };
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_MESSAGE,
        PIPE_TYPE_MESSAGE, PIPE_WAIT,
    };
    use windows::Win32::System::Threading::{CreateEventW, ResetEvent, WaitForSingleObject};
    use windows::Win32::System::IO::CancelIoEx;

    const PIPE_BUF_SIZE: u32 = 65536;
    const MAX_CONNECTIONS: u32 = 1; // serial: one client at a time

    /// Build SECURITY_ATTRIBUTES with a DACL granting SYSTEM + Administrators.
    ///
    /// The DACL is built from the canonical SDDL string
    /// `D:(A;;GA;;;SY)(A;;GA;;;BA)` (SYSTEM → Generic All, Builtin
    /// Administrators → Generic All) via
    /// `ConvertStringSecurityDescriptorToSecurityDescriptorW` — the same
    /// deterministic approach used by `micontrol_bridge` and `ecram_service`.
    ///
    /// Previous iterations used `BuildExplicitAccessWithNameW` +
    /// `SetSecurityDescriptorDacl` on a default-initialized
    /// `SECURITY_DESCRIPTOR` (all-zeros, never through
    /// `InitializeSecurityDescriptor`), so the DACL was silently not applied:
    /// the pipe inherited the process DACL (SYSTEM-only in practice when the
    /// service runs as SYSTEM, and *any* process's default DACL otherwise).
    /// SDDL parsing avoids all of that. The returned SECURITY_DESCRIPTOR is
    /// heap-allocated by Windows and intentionally leaked (must outlive the
    /// SECURITY_ATTRIBUTES usages).
    fn build_security() -> Option<SECURITY_ATTRIBUTES> {
        unsafe {
            let sddl: Vec<u16> = "D:(A;;GA;;;SY)(A;;GA;;;BA)"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let mut psd: PSECURITY_DESCRIPTOR = PSECURITY_DESCRIPTOR(std::ptr::null_mut());

            let result = ConvertStringSecurityDescriptorToSecurityDescriptorW(
                windows::core::PCWSTR(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut psd,
                None,
            );
            let psd_ptr = psd.0;
            if result.is_err() || psd_ptr.is_null() {
                let last_err = std::io::Error::last_os_error();
                log::warn!(
                    "face pipe: ConvertStringSecurityDescriptorToSecurityDescriptorW failed: {last_err} — pipe will be process-default DACL"
                );
                return None;
            }

            // psd is a heap allocation owned by the caller now; intentionally
            // never freed (lives for the whole pipe-server process).
            Some(SECURITY_ATTRIBUTES {
                nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: psd_ptr,
                bInheritHandle: false.into(),
            })
        }
    }

    /// Accept one client, invoke `handle`, respond, disconnect.
    /// Returns Ok(true) if a client was served, Ok(false) on timeout/shutdown.
    pub fn serve_one<F>(handle: &F, shutdown: &std::sync::atomic::AtomicBool) -> FaceResult<bool>
    where
        F: Fn(&Value) -> Value,
    {
        let pipe_name_w: Vec<u16> = OsStr::new(PIPE_NAME).encode_wide().chain(Some(0)).collect();
        let security = build_security();

        let handle_win = unsafe {
            CreateNamedPipeW(
                PCWSTR(pipe_name_w.as_ptr()),
                PIPE_ACCESS_DUPLEX
                    | FILE_FLAG_OVERLAPPED
                    | FILE_FLAGS_AND_ATTRIBUTES(FILE_FLAG_FIRST_PIPE_INSTANCE),
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
                MAX_CONNECTIONS,
                PIPE_BUF_SIZE,
                PIPE_BUF_SIZE,
                0,
                security.as_ref().map(|s| s as *const SECURITY_ATTRIBUTES),
            )
        };

        if handle_win == INVALID_HANDLE_VALUE {
            // Pipe name already in use (squatted or previous instance alive).
            return Err(FaceError::Pipe(format!(
                "CreateNamedPipe failed: {}",
                std::io::Error::last_os_error()
            )));
        }

        let event = unsafe {
            CreateEventW(None, true, false, PCWSTR::null())
                .map_err(|e| FaceError::Pipe(format!("CreateEventW: {e}")))?
        };

        let mut overlapped = unsafe {
            windows::Win32::System::IO::OVERLAPPED {
                hEvent: event,
                ..std::mem::zeroed()
            }
        };

        // Connect (non-blocking, overlapped) with a 500 ms timeout loop.
        let _ = unsafe { ConnectNamedPipe(handle_win, Some(&mut overlapped)) };

        loop {
            if shutdown.load(std::sync::atomic::Ordering::SeqCst) {
                unsafe {
                    CancelIoEx(handle_win, Some(&overlapped)).ok();
                    CloseHandle(event).ok();
                    CloseHandle(handle_win).ok();
                }
                return Ok(false);
            }
            let wait = unsafe { WaitForSingleObject(event, 500) };
            if wait == WAIT_OBJECT_0 {
                break;
            }
            // timeout → retry connect
            unsafe {
                CancelIoEx(handle_win, Some(&overlapped)).ok();
                ResetEvent(event).ok();
                let _ = ConnectNamedPipe(handle_win, Some(&mut overlapped));
            }
        }

        // Read the request.
        let mut buf = [0u8; 65536];
        let mut bytes_read = 0u32;
        let read_ok = unsafe { ReadFile(handle_win, Some(&mut buf), Some(&mut bytes_read), None) };
        if read_ok.is_err() || bytes_read == 0 {
            unsafe {
                DisconnectNamedPipe(handle_win).ok();
                CloseHandle(event).ok();
                CloseHandle(handle_win).ok();
            }
            return Ok(true);
        }

        // Parse JSON, invoke handler.
        let req: Value = serde_json::from_slice(&buf[..bytes_read as usize]).unwrap_or(Value::Null);
        let resp = handle(&req);
        let resp_bytes = serde_json::to_vec(&resp).unwrap_or_else(|_| b"{}".to_vec());

        unsafe {
            let mut written = 0u32;
            WriteFile(handle_win, Some(&resp_bytes), Some(&mut written), None).ok();
            FlushFileBuffers(handle_win).ok();
            DisconnectNamedPipe(handle_win).ok();
            CloseHandle(event).ok();
            CloseHandle(handle_win).ok();
        }
        Ok(true)
    }
}

#[cfg(not(windows))]
mod winpipe {
    use super::*;
    use std::sync::atomic::AtomicBool;

    pub fn serve_one<F>(_handle: &F, shutdown: &AtomicBool) -> FaceResult<bool>
    where
        F: Fn(&Value) -> Value,
    {
        let _ = shutdown;
        Ok(false)
    }
}

pub use winpipe::serve_one;

/// Build the standard error response.
pub fn err_response(reason: impl Into<String>) -> PipeResponse {
    json!({ "ok": false, "reason": reason.into() })
}
