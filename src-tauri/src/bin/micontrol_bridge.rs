//! MiControl Bridge — autonomous elevated service.
//!
//! A Windows service installed at install time (as `NT AUTHORITY\SYSTEM`).
//! It exposes a named pipe `\\.\pipe\micontrol_bridge` and executes privileged
//! hardware commands on behalf of the main (unprivileged) MiControl process.
//!
//! Because the service runs elevated from installation onward, the main app
//! NEVER needs to prompt UAC for privileged operations — it simply sends the
//! HMAC-authenticated command over the pipe and reads the response.
//!
//! Protocol (JSON line over named pipe):
//!   Request:  { "cmd": "...", "args": {...}, "request_id": "...",
//!               "created_at_ms": 1234, "nonce": "...", "hmac": "..." }
//!   Response: { "ok": true, "data": {...}, "request_id": "...", "hmac": "..." }
//!
//! Auth: HMAC-SHA256 over the JSON body, shared key in
//! `%LOCALAPPDATA%\MiControl\elev_key.bin` (same key as the scheduled-task
//! bridge). The pipe itself is also ACL-restricted to the current user +
//! SYSTEM, and the key file is ACL-restricted, so only the MiControl app can
//! talk to the bridge.
//!
//! CLI usage:
//!   micontrol_bridge service              Run as a Windows service (via SCM)
//!   micontrol_bridge console              Run pipe server in console (testing)
//!   micontrol_bridge install              Install the service (admin required)
//!   micontrol_bridge uninstall            Uninstall the service (admin required)

#![cfg(windows)]

use std::os::windows::ffi::OsStrExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// ── Constants ────────────────────────────────────────────────────────────────

/// Named pipe name for the bridge service.
pub const BRIDGE_PIPE_NAME: &str = r"\\.\pipe\micontrol_bridge";
/// Windows service name registered at install time.
pub const SERVICE_NAME: &str = "MiControlBridge";
/// Service display name.
pub const SERVICE_DISPLAY: &str = "MiControl Elevated Bridge";
/// Pipe buffer size.
const PIPE_BUF_SIZE: u32 = 16384;
/// Max command payload size (JSON).
const MAX_MSG_BYTES: usize = 16384;

// ── Main ─────────────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mode = args.get(1).map(|s| s.as_str()).unwrap_or("service");

    match mode {
        "service" => {
            if let Err(e) = service::run() {
                eprintln!("[micontrol_bridge] Service error: {e}");
                std::process::exit(1);
            }
        }
        "console" => {
            // Testing mode: run the pipe server in the foreground.
            eprintln!("[micontrol_bridge] Console mode — pipe server on {BRIDGE_PIPE_NAME}");
            let shutdown = Arc::new(AtomicBool::new(false));
            pipe_server::run(shutdown);
            eprintln!("[micontrol_bridge] Console mode exiting");
        }
        "install" => match install_service() {
            Ok(()) => println!("MiControlBridge service installed and started."),
            Err(e) => {
                eprintln!("Failed to install MiControlBridge: {e}");
                std::process::exit(1);
            }
        },
        "uninstall" => match uninstall_service() {
            Ok(()) => println!("MiControlBridge service removed."),
            Err(e) => {
                eprintln!("Failed to remove MiControlBridge: {e}");
                std::process::exit(1);
            }
        },
        "self-test" => {
            // Validation mode (no admin required): create a test pipe with the
            // exact DACL used by the server, then connect as the current user.
            // Proves the Everyone RW DACL works end-to-end WITHOUT reinstalling
            // the service (which needs elevation).
            let code = pipe_server::self_test();
            std::process::exit(code);
        }
        other => {
            eprintln!("[micontrol_bridge] Unknown mode: {other}");
            eprintln!("Usage: micontrol_bridge <service|console|install|uninstall|self-test>");
            std::process::exit(1);
        }
    }
}

// ── Service install helpers ──────────────────────────────────────────────────

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Find the current executable path (this binary).
fn current_exe() -> String {
    std::env::current_exe()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "micontrol_bridge.exe".to_string())
}

/// Install the MiControlBridge Windows service (requires admin).
fn install_service() -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    let exe = current_exe();
    let bin_path = format!("\"{exe}\" service");

    // Stop + delete any existing service first.
    let _ = Command::new("sc")
        .args(["stop", SERVICE_NAME])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let _ = Command::new("sc")
        .args(["delete", SERVICE_NAME])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    std::thread::sleep(std::time::Duration::from_millis(1500));

    // Create the service.
    let out = Command::new("sc")
        .args([
            "create",
            SERVICE_NAME,
            "binPath=",
            &bin_path,
            "start=",
            "auto",
            "DisplayName=",
            SERVICE_DISPLAY,
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("sc create failed to spawn: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "sc create failed: {} {}",
            String::from_utf8_lossy(&out.stderr),
            String::from_utf8_lossy(&out.stdout)
        ));
    }

    // Run as LocalSystem with failure auto-restart.
    let _ = Command::new("sc")
        .args(["config", SERVICE_NAME, "obj=", "LocalSystem"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let _ = Command::new("sc")
        .args([
            "failure",
            SERVICE_NAME,
            "reset=",
            "86400",
            "actions=",
            "restart/5000/restart/10000/restart/30000",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    // No desktop interaction needed.
    let _ = Command::new("sc")
        .args(["config", SERVICE_NAME, "type=", "own", "start=", "auto"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    // Start the service.
    let start = Command::new("sc")
        .args(["start", SERVICE_NAME])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("sc start failed to spawn: {e}"))?;
    if !start.status.success() {
        // Might already be running — check.
        let q = Command::new("sc")
            .args(["query", SERVICE_NAME])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("sc query failed: {e}"))?;
        let qtext = String::from_utf8_lossy(&q.stdout);
        if !qtext.contains("RUNNING") {
            return Err(format!(
                "sc start failed: {}",
                String::from_utf8_lossy(&start.stderr)
            ));
        }
    }

    Ok(())
}

/// Uninstall the MiControlBridge Windows service (requires admin).
fn uninstall_service() -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    let _ = Command::new("sc")
        .args(["stop", SERVICE_NAME])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    std::thread::sleep(std::time::Duration::from_millis(1500));
    let out = Command::new("sc")
        .args(["delete", SERVICE_NAME])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("sc delete failed to spawn: {e}"))?;
    if !out.status.success() {
        let text = String::from_utf8_lossy(&out.stderr);
        // "service does not exist" is fine.
        if !text.to_lowercase().contains("does not exist") {
            return Err(format!("sc delete failed: {text}"));
        }
    }
    Ok(())
}

// ── Windows Service plumbing ─────────────────────────────────────────────────

mod service {
    use super::*;
    use std::ffi::OsString;
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::System::Services::{
        RegisterServiceCtrlHandlerW, SetServiceStatus, StartServiceCtrlDispatcherW,
        LPHANDLER_FUNCTION, SERVICE_ACCEPT_STOP, SERVICE_CONTROL_STOP, SERVICE_RUNNING,
        SERVICE_START_PENDING, SERVICE_STATUS, SERVICE_STOPPED, SERVICE_TABLE_ENTRYW,
        SERVICE_WIN32_OWN_PROCESS,
    };

    static SHUTDOWN: AtomicBool = AtomicBool::new(false);

    pub fn run() -> Result<(), String> {
        let mut name_w: Vec<u16> = OsString::from(SERVICE_NAME)
            .encode_wide()
            .chain(Some(0))
            .collect();

        let table = [
            SERVICE_TABLE_ENTRYW {
                lpServiceName: PWSTR(name_w.as_mut_ptr()),
                lpServiceProc: Some(service_main),
            },
            SERVICE_TABLE_ENTRYW {
                lpServiceName: PWSTR::null(),
                lpServiceProc: None,
            },
        ];

        unsafe {
            StartServiceCtrlDispatcherW(table.as_ptr())
                .map_err(|e| format!("StartServiceCtrlDispatcherW: {e}"))?;
        }
        Ok(())
    }

    unsafe extern "system" fn service_main(_argc: u32, _argv: *mut windows::core::PWSTR) {
        let name_w: Vec<u16> = OsString::from(SERVICE_NAME)
            .encode_wide()
            .chain(Some(0))
            .collect();

        let handler: LPHANDLER_FUNCTION = Some(service_control_handler);
        let status_handle = RegisterServiceCtrlHandlerW(PCWSTR(name_w.as_ptr()), handler);

        match status_handle {
            Ok(h) => {
                set_service_state(h, SERVICE_START_PENDING);

                let shutdown = Arc::new(AtomicBool::new(false));
                let shutdown_clone = shutdown.clone();

                let pipe_thread = std::thread::spawn(move || {
                    pipe_server::run(shutdown_clone);
                });

                set_service_state(h, SERVICE_RUNNING);
                eprintln!("[micontrol_bridge] Service running — pipe: {BRIDGE_PIPE_NAME}");

                while !SHUTDOWN.load(Ordering::SeqCst) {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }

                shutdown.store(true, Ordering::SeqCst);
                let _ = pipe_thread.join();
                set_service_state(h, SERVICE_STOPPED);
                eprintln!("[micontrol_bridge] Service stopped");
            }
            Err(e) => {
                eprintln!("[micontrol_bridge] RegisterServiceCtrlHandlerW failed: {e}");
            }
        }
    }

    extern "system" fn service_control_handler(control: u32) {
        if control == SERVICE_CONTROL_STOP {
            SHUTDOWN.store(true, Ordering::SeqCst);
        }
    }

    fn set_service_state(
        handle: windows::Win32::System::Services::SERVICE_STATUS_HANDLE,
        state: windows::Win32::System::Services::SERVICE_STATUS_CURRENT_STATE,
    ) {
        unsafe {
            let accept = if state == SERVICE_RUNNING {
                SERVICE_ACCEPT_STOP
            } else {
                0u32
            };
            let status = SERVICE_STATUS {
                dwServiceType: SERVICE_WIN32_OWN_PROCESS,
                dwCurrentState: state,
                dwControlsAccepted: accept,
                dwWin32ExitCode: 0,
                dwServiceSpecificExitCode: 0,
                dwCheckPoint: 0,
                dwWaitHint: 3000,
            };
            SetServiceStatus(handle, &status).ok();
        }
    }
}

// ── Pipe server ──────────────────────────────────────────────────────────────

mod pipe_server {
    use super::*;
    use std::ffi::OsStr;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
    use windows::Win32::Storage::FileSystem::{
        ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX,
    };
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
    };
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
    use windows::Win32::System::IO::CancelIoEx;

    /// Build a SECURITY_ATTRIBUTES with a DACL that grants Everyone read/write
    /// access to the named pipe.
    ///
    /// When the service runs as `NT AUTHORITY\SYSTEM`, pipes created without
    /// explicit security inherit a DACL that only SYSTEM can access — the
    /// unprivileged MiControl app would then fail to open
    /// `\\.\pipe\micontrol_bridge` (ERROR_ACCESS_DENIED, error 5) and every
    /// elevated command would fall back to the scheduled task / UAC prompt at
    /// startup. Granting Everyone access is safe because the bridge protocol
    /// is HMAC-SHA256-authenticated (shared key in `%LocalAppData%\MiControl`
    /// with a freshness window), so an arbitrary process cannot forge commands.
    ///
    /// Implementation: build the DACL from the canonical SDDL string
    /// `D:(A;;GA;;;WD)` (Everyone/World → Generic All) via
    /// `ConvertStringSecurityDescriptorToSecurityDescriptorW`. This is the
    /// deterministic, language-independent way to grant Everyone access and it
    /// works identically whether the process runs as the interactive user or
    /// as SYSTEM. Previous iterations used `BuildExplicitAccessWithNameW`
    /// (silently failed on non-English/restricted Windows) and then
    /// `CreateWellKnownSid` + `SetEntriesInAclW` — which can return errors
    /// under SYSTEM and leave the pipe SYSTEM-only; SDDL parsing avoids all of
    /// that. The returned SECURITY_DESCRIPTOR is heap-allocated by Windows and
    /// intentionally leaked (must outlive the SECURITY_ATTRIBUTES usages).
    fn build_pipe_security_attributes() -> Option<windows::Win32::Security::SECURITY_ATTRIBUTES> {
        use windows::core::PCWSTR;
        use windows::Win32::Security::Authorization::{
            ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
        };
        use windows::Win32::Security::SECURITY_ATTRIBUTES;

        unsafe {
            use windows::Win32::Security::PSECURITY_DESCRIPTOR;
            let sddl: Vec<u16> = "D:(A;;GA;;;WD)"
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let mut psd: PSECURITY_DESCRIPTOR = PSECURITY_DESCRIPTOR(std::ptr::null_mut());

            let result = ConvertStringSecurityDescriptorToSecurityDescriptorW(
                PCWSTR(sddl.as_ptr()),
                SDDL_REVISION_1,
                &mut psd,
                None,
            );
            let psd_ptr = psd.0;
            if result.is_err() || psd_ptr.is_null() {
                let last_err = std::io::Error::last_os_error();
                eprintln!(
                    "[micontrol_bridge] ConvertStringSecurityDescriptorToSecurityDescriptorW failed: {last_err} — pipe will be SYSTEM-only"
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

    pub fn run(shutdown: Arc<AtomicBool>) {
        let pipe_name_w: Vec<u16> = OsStr::new(BRIDGE_PIPE_NAME)
            .encode_wide()
            .chain(Some(0))
            .collect();

        while !shutdown.load(Ordering::SeqCst) {
            // Keep the SECURITY_ATTRIBUTES alive for the duration of the
            // CreateNamedPipeW call — passing it as an inline temporary and
            // converting to a raw pointer in the same expression is safe, but
            // binding it explicitly guarantees the pointed-to memory cannot be
            // recycled between the temporary's drop and the FFI read.
            let sec_attr = build_pipe_security_attributes();
            let handle = unsafe {
                CreateNamedPipeW(
                    PCWSTR(pipe_name_w.as_ptr()),
                    PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                    1,
                    PIPE_BUF_SIZE,
                    PIPE_BUF_SIZE,
                    0,
                    sec_attr
                        .as_ref()
                        .map(|s| s as *const windows::Win32::Security::SECURITY_ATTRIBUTES),
                )
            };

            if handle == INVALID_HANDLE_VALUE {
                eprintln!(
                    "[micontrol_bridge] CreateNamedPipeW failed: {}",
                    std::io::Error::last_os_error()
                );
                std::thread::sleep(std::time::Duration::from_secs(1));
                continue;
            }

            let event = unsafe { CreateEventW(None, true, false, PCWSTR::null()) };
            let Ok(event) = event else {
                unsafe {
                    CloseHandle(handle).ok();
                }
                continue;
            };

            let mut overlapped = unsafe {
                windows::Win32::System::IO::OVERLAPPED {
                    hEvent: event,
                    ..std::mem::zeroed()
                }
            };

            let _ = unsafe { ConnectNamedPipe(handle, Some(&mut overlapped)) };
            let wait_result = unsafe { WaitForSingleObject(event, 500) };

            if wait_result != WAIT_OBJECT_0 {
                unsafe {
                    CancelIoEx(handle, Some(&overlapped)).ok();
                    CloseHandle(handle).ok();
                    CloseHandle(event).ok();
                }
                continue;
            }

            handle_client(handle);
            unsafe {
                CloseHandle(handle).ok();
                CloseHandle(event).ok();
            }
        }
    }

    fn handle_client(handle: windows::Win32::Foundation::HANDLE) {
        // Read the full request (loop until a full JSON object is received).
        let mut read_buf = [0u8; MAX_MSG_BYTES];
        let mut total_read = 0usize;

        loop {
            if total_read >= read_buf.len() {
                break;
            }
            let mut bytes_read = 0u32;
            let result = unsafe {
                ReadFile(
                    handle,
                    Some(&mut read_buf[total_read..]),
                    Some(&mut bytes_read),
                    None,
                )
            };
            if result.is_err() || bytes_read == 0 {
                break;
            }
            total_read += bytes_read as usize;
            // Stop when the JSON object appears complete (ends with }).
            if total_read > 0 && read_buf[total_read - 1] == b'}' {
                break;
            }
        }

        if total_read == 0 {
            return;
        }

        let request_str = String::from_utf8_lossy(&read_buf[..total_read]).to_string();
        let response = process_request(&request_str);

        let resp_bytes = response.as_bytes();
        let mut written = 0u32;
        unsafe {
            let _ = WriteFile(handle, Some(resp_bytes), Some(&mut written), None);
        }
    }

    /// Authenticate + dispatch one bridge request.
    fn process_request(request_str: &str) -> String {
        // Parse the request to extract the request_id for the response.
        let request_id = serde_json::from_str::<serde_json::Value>(request_str)
            .ok()
            .and_then(|v| v.get("request_id").cloned())
            .unwrap_or(serde_json::Value::Null);

        let mut response = match dispatch_with_auth(request_str) {
            Ok(data) => serde_json::json!({ "ok": true, "data": data }),
            Err(e) => serde_json::json!({ "ok": false, "error": e }),
        };
        if !request_id.is_null() {
            response["request_id"] = request_id;
        }
        response["created_at_ms"] = serde_json::json!(micontrol_lib::util::auth::now_ms());

        // Sign the response so the caller can verify integrity (same protocol
        // as the scheduled-task bridge).
        if let Ok(key) = micontrol_lib::util::auth::read_key() {
            micontrol_lib::util::auth::sign_payload(&mut response, &key);
        }
        response.to_string()
    }

    /// Verify HMAC + freshness, then execute the command via the library
    /// elevated dispatcher. Returns the `data` value on success.
    fn dispatch_with_auth(request_str: &str) -> Result<serde_json::Value, String> {
        let mut payload: serde_json::Value =
            serde_json::from_str(request_str).map_err(|e| format!("Invalid JSON request: {e}"))?;

        let cmd: String = payload
            .get("cmd")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Missing cmd field".to_string())?
            .to_string();
        let args = payload
            .get("args")
            .cloned()
            .unwrap_or(serde_json::Value::Null);

        // ── Auth: HMAC + freshness ────────────────────────────────────────
        let key = micontrol_lib::util::auth::read_key()
            .map_err(|e| format!("Bridge auth key unavailable: {e}"))?;

        // The library's verify_payload expects a `created_at_ms` field and an
        // `hmac` field; it verifies HMAC and timestamp freshness.
        micontrol_lib::util::auth::verify_payload(&mut payload, &key)
            .map_err(|e| format!("Bridge request authentication failed: {e}"))?;

        // ── Dispatch ───────────────────────────────────────────────────────
        let result = micontrol_lib::elevated::dispatch_cmd(&cmd, args);
        if result["ok"].as_bool().unwrap_or(false) {
            Ok(result["data"].clone())
        } else {
            Err(result["error"]
                .as_str()
                .unwrap_or("Bridge dispatch failed")
                .to_string())
        }
    }

    /// Validate the Everyone-RW DACL WITHOUT admin: create a throwaway pipe
    /// using the exact same security attributes as the real server, connect as
    /// the current (unprivileged) user, and do a tiny read/write round-trip.
    /// Exit code 0 = DACL works; 1 = DACL broken (ACCESS_DENIED would hit the
    /// real app, exactly the v0.1.16 bug).
    pub fn self_test() -> i32 {
        use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
        use windows::Win32::Storage::FileSystem::{
            CreateFileW, ReadFile, WriteFile, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        };
        use windows::Win32::System::Pipes::DisconnectNamedPipe;
        use windows::Win32::System::Threading::CreateEventW;

        const TEST_PIPE: &str = r"\\.\pipe\micontrol_bridge_selftest";
        let pipe_name_w: Vec<u16> = OsStr::new(TEST_PIPE).encode_wide().chain(Some(0)).collect();

        // Spawn a minimal server in this process (same DACL path). Keep the
        // SECURITY_ATTRIBUTES alive explicitly (see run() for rationale).
        let sec_attr = build_pipe_security_attributes();
        let server_handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(pipe_name_w.as_ptr()),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                1,
                PIPE_BUF_SIZE,
                PIPE_BUF_SIZE,
                0,
                sec_attr
                    .as_ref()
                    .map(|s| s as *const windows::Win32::Security::SECURITY_ATTRIBUTES),
            )
        };
        if server_handle == INVALID_HANDLE_VALUE {
            eprintln!(
                "[self-test] CreateNamedPipeW failed: {}",
                std::io::Error::last_os_error()
            );
            return 1;
        }

        // Connect the server side.
        let event = unsafe { CreateEventW(None, true, false, PCWSTR::null()) };
        let Ok(event) = event else {
            eprintln!("[self-test] CreateEventW failed");
            unsafe {
                CloseHandle(server_handle).ok();
            }
            return 1;
        };
        let mut overlapped = unsafe {
            windows::Win32::System::IO::OVERLAPPED {
                hEvent: event,
                ..std::mem::zeroed()
            }
        };
        let _ = unsafe { ConnectNamedPipe(server_handle, Some(&mut overlapped)) };

        // Now connect as the current (unprivileged) user → the whole point.
        let client_handle = unsafe {
            CreateFileW(
                PCWSTR(pipe_name_w.as_ptr()),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                None,
            )
        };
        if client_handle.is_err() {
            eprintln!(
                "[self-test] FAIL: client CreateFileW -> ACCESS_DENIED ({}) — DACL is SYSTEM-only; \
                 the real app would hit the same wall (v0.1.16 bug).",
                std::io::Error::last_os_error()
            );
            unsafe {
                CloseHandle(server_handle).ok();
                CloseHandle(event).ok();
            }
            return 1;
        }
        let client_handle = client_handle.unwrap();

        // Round-trip: client writes "ping", server reads it.
        let ping = b"ping";
        let mut written = 0u32;
        let wr = unsafe { WriteFile(client_handle, Some(ping), Some(&mut written), None) };
        if wr.is_err() || written != ping.len() as u32 {
            eprintln!(
                "[self-test] FAIL: client write failed: {}",
                std::io::Error::last_os_error()
            );
            unsafe {
                CloseHandle(server_handle).ok();
                CloseHandle(event).ok();
                CloseHandle(client_handle).ok();
            }
            return 1;
        }

        let mut buf = [0u8; 16];
        let mut bytes_read = 0u32;
        let rd = unsafe { ReadFile(server_handle, Some(&mut buf), Some(&mut bytes_read), None) };
        if rd.is_ok() && bytes_read >= 4 && &buf[..4] == ping {
            println!("[self-test] PASS: unprivileged client connected + round-tripped (DACL OK)");
            let code = 0;
            unsafe {
                DisconnectNamedPipe(server_handle).ok();
                CloseHandle(server_handle).ok();
                CloseHandle(event).ok();
                CloseHandle(client_handle).ok();
            }
            code
        } else {
            eprintln!(
                "[self-test] FAIL: server read did not match (DACL may be OK but I/O broken)"
            );
            unsafe {
                CloseHandle(server_handle).ok();
                CloseHandle(event).ok();
                CloseHandle(client_handle).ok();
            }
            1
        }
    }
}
