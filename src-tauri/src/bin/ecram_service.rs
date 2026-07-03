//! ECRAM Service — a Windows service that provides EC RAM read/write access
//! via a named pipe interface.
//!
//! This binary is designed to replace Xiaomi's `IoTService.exe` in the
//! DriverStore directory. When installed as a Windows service (via SCM),
//! it runs as `NT AUTHORITY\SYSTEM` and is started by the Service Control
//! Manager, which satisfies the IoTDriver.sys security check.
//!
//! Once running, it creates a named pipe `\\.\pipe\ecram_service` and
//! accepts JSON commands:
//!   {"op":"read","addr":"0xFE0B0300","size":256}
//!   {"op":"write","addr":"0xFE0B0300","data":"DEADBEEF"}
//!   {"op":"read_region","region":"ERAM"}
//!
//! Responses are JSON:
//!   {"ok":true,"data":"HEXSTRING"}
//!   {"ok":false,"error":"message"}
//!
//! It also supports CLI mode for testing:
//!   ecram_service read-region ERAM
//!   ecram_service read 0xFE0B0300 256
//!   ecram_service write 0xFE0B0300 DEADBEEF

#![cfg(windows)]

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

// ── ECRAM low-level IOCTL access ─────────────────────────────────────────────

mod ecram {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Devices::DeviceAndDriverInstallation::{
        SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
        SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT,
        SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
    };
    use windows::Win32::Foundation::{
        CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows::Win32::System::IO::DeviceIoControl;

    /// IoT driver device interface GUID: {AB7924A1-3162-4010-B33B-837E87E25FBC}
    const IOT_GUID: windows::core::GUID = windows::core::GUID {
        data1: 0xAB7924A1,
        data2: 0x3162,
        data3: 0x4010,
        data4: [0xB3, 0x3B, 0x83, 0x7E, 0x87, 0xE2, 0x5F, 0xBC],
    };

    const IOCTL_ECRAM_READ: u32 = 0x22E000;
    const IOCTL_ECRAM_WRITE: u32 = 0x22E004;
    const IOCTL_BUF_SIZE: usize = 0x110;

    #[repr(C)]
    struct EcramBuf {
        physical_address: u64,
        byte_count: u64,
        data: [u8; 0x100],
    }

    const _: () = assert!(std::mem::size_of::<EcramBuf>() == IOCTL_BUF_SIZE);

    /// Known ECRAM regions
    pub const REGIONS: &[(&str, u64, usize)] = &[
        ("ERAM", 0xFE0B0300, 0x100),
        ("SMA2", 0xFE0B0A00, 0x100),
        ("IOT_STATUS", 0xFE0B0F00, 0x08),
        ("IOT_SENSORS", 0xFE0B0F08, 0x78),
    ];

    fn find_iot_device_path() -> Result<String, String> {
        unsafe {
            let dev_info = SetupDiGetClassDevsW(
                Some(&IOT_GUID),
                None,
                None,
                DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
            )
            .map_err(|e| format!("SetupDiGetClassDevsW: {e}"))?;

            let mut iface = SP_DEVICE_INTERFACE_DATA {
                cbSize: std::mem::size_of::<SP_DEVICE_INTERFACE_DATA>() as u32,
                ..std::mem::zeroed()
            };

            let enum_result = SetupDiEnumDeviceInterfaces(dev_info, None, &IOT_GUID, 0, &mut iface);
            if enum_result.is_err() {
                let _ = SetupDiDestroyDeviceInfoList(dev_info);
                return Err("No IoT device interface found".into());
            }

            let mut required = 0u32;
            let _ = SetupDiGetDeviceInterfaceDetailW(
                dev_info,
                &iface,
                None,
                0,
                Some(&mut required),
                None,
            );

            if required == 0 || required > 4096 {
                let _ = SetupDiDestroyDeviceInfoList(dev_info);
                return Err(format!("Invalid required size {required}"));
            }

            let mut buf = vec![0u8; required as usize];
            let detail_ptr = buf.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_W;
            (*detail_ptr).cbSize = std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;

            let detail_result = SetupDiGetDeviceInterfaceDetailW(
                dev_info,
                &iface,
                Some(detail_ptr),
                required,
                None,
                None,
            );
            let _ = SetupDiDestroyDeviceInfoList(dev_info);
            detail_result.map_err(|e| format!("SetupDiGetDeviceInterfaceDetailW: {e}"))?;

            let path_offset = 4usize;
            let wide_slice = std::slice::from_raw_parts(
                buf.as_ptr().add(path_offset) as *const u16,
                (required as usize - path_offset) / 2,
            );
            let null_pos = wide_slice
                .iter()
                .position(|&c| c == 0)
                .unwrap_or(wide_slice.len());
            String::from_utf16(&wide_slice[..null_pos])
                .map_err(|e| format!("Invalid UTF-16 device path: {e}"))
        }
    }

    /// Open the IoT driver device handle.
    fn open_device() -> Result<HANDLE, String> {
        let device_path = find_iot_device_path()?;
        let path_w: Vec<u16> = OsStr::new(&device_path)
            .encode_wide()
            .chain(Some(0))
            .collect();

        unsafe {
            let handle = CreateFileW(
                PCWSTR(path_w.as_ptr()),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                HANDLE::default(),
            )
            .map_err(|e| format!("Open IoT driver device: {e}"))?;

            if handle == INVALID_HANDLE_VALUE {
                return Err("INVALID_HANDLE_VALUE opening IoT driver device".into());
            }

            Ok(handle)
        }
    }

    /// Send the ReportLaptopStatus(IOT_WIN_READY) handshake.
    ///
    /// The handshake is a zeroed 0x110-byte buffer sent via IOCTL 0x22E000.
    /// The driver checks a global variable that is set when this IOCTL is received.
    /// Without this handshake, all ECRAM read/write IOCTLs return ACCESS_DENIED.
    pub fn send_handshake() -> Result<(), String> {
        let handle = open_device()?;

        unsafe {
            // Zeroed buffer — the handshake is just an all-zero 0x110 byte buffer
            let in_buf = EcramBuf {
                physical_address: 0,
                byte_count: 0,
                data: [0u8; 0x100],
            };

            let mut out_buf = EcramBuf {
                physical_address: 0,
                byte_count: 0,
                data: [0u8; 0x100],
            };

            let mut bytes_returned = 0u32;
            let result = DeviceIoControl(
                handle,
                IOCTL_ECRAM_READ, // 0x22E000 — same IOCTL for handshake and read
                Some((&raw const in_buf).cast()),
                IOCTL_BUF_SIZE as u32,
                Some((&raw mut out_buf).cast()),
                IOCTL_BUF_SIZE as u32,
                Some(&mut bytes_returned),
                None,
            );

            CloseHandle(handle).ok();
            result.map_err(|e| format!("DeviceIoControl handshake: {e}"))?;
        }

        Ok(())
    }

    pub fn read_ecram(phys_addr: u64, byte_count: usize) -> Result<Vec<u8>, String> {
        if byte_count == 0 || byte_count > 0x100 {
            return Err(format!("byte_count must be 1..256, got {byte_count}"));
        }

        let handle = open_device()?;

        unsafe {
            let in_buf = EcramBuf {
                physical_address: phys_addr,
                byte_count: byte_count as u64,
                data: [0u8; 0x100],
            };

            let mut out_buf = EcramBuf {
                physical_address: 0,
                byte_count: 0,
                data: [0u8; 0x100],
            };

            let mut bytes_returned = 0u32;
            let result = DeviceIoControl(
                handle,
                IOCTL_ECRAM_READ,
                Some((&raw const in_buf).cast()),
                IOCTL_BUF_SIZE as u32,
                Some((&raw mut out_buf).cast()),
                IOCTL_BUF_SIZE as u32,
                Some(&mut bytes_returned),
                None,
            );

            CloseHandle(handle).ok();
            result.map_err(|e| format!("DeviceIoControl ECRAM_READ: {e}"))?;

            Ok(out_buf.data[..byte_count].to_vec())
        }
    }

    pub fn write_ecram(phys_addr: u64, data: &[u8]) -> Result<usize, String> {
        if data.is_empty() || data.len() > 0x100 {
            return Err(format!("data size must be 1..256, got {}", data.len()));
        }

        let handle = open_device()?;

        unsafe {
            let mut in_buf = EcramBuf {
                physical_address: phys_addr,
                byte_count: data.len() as u64,
                data: [0u8; 0x100],
            };
            in_buf.data[..data.len()].copy_from_slice(data);

            let mut out_buf = EcramBuf {
                physical_address: 0,
                byte_count: 0,
                data: [0u8; 0x100],
            };

            let mut bytes_returned = 0u32;
            let result = DeviceIoControl(
                handle,
                IOCTL_ECRAM_WRITE,
                Some((&raw const in_buf).cast()),
                IOCTL_BUF_SIZE as u32,
                Some((&raw mut out_buf).cast()),
                IOCTL_BUF_SIZE as u32,
                Some(&mut bytes_returned),
                None,
            );

            CloseHandle(handle).ok();
            result.map_err(|e| format!("DeviceIoControl ECRAM_WRITE: {e}"))?;

            Ok(data.len())
        }
    }
}

// ── Named pipe server ────────────────────────────────────────────────────────

mod pipe_server {
    use super::ecram;
    use std::ffi::OsStr;
    use std::io;
    use std::os::windows::ffi::OsStrExt;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
    use windows::Win32::Storage::FileSystem::{
        FlushFileBuffers, ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX,
    };
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_WAIT,
    };
    use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
    use windows::Win32::System::IO::CancelIoEx;

    const PIPE_NAME: &str = r"\\.\pipe\ecram_service";
    const PIPE_BUF_SIZE: u32 = 4096;
    const BUFSIZE: u32 = 4096;

    /// Run the named pipe server. Blocks until `shutdown` is set.
    pub fn run_pipe_server(shutdown: Arc<AtomicBool>) {
        let pipe_name_w: Vec<u16> = OsStr::new(PIPE_NAME).encode_wide().chain(Some(0)).collect();

        eprintln!("[ecram_service] pipe server starting on {PIPE_NAME}");

        while !shutdown.load(Ordering::SeqCst) {
            let handle = unsafe {
                CreateNamedPipeW(
                    PCWSTR(pipe_name_w.as_ptr()),
                    PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                    1,
                    PIPE_BUF_SIZE,
                    PIPE_BUF_SIZE,
                    0,
                    None,
                )
            };

            if handle == INVALID_HANDLE_VALUE {
                eprintln!(
                    "[ecram_service] CreateNamedPipeW failed: {}",
                    io::Error::last_os_error()
                );
                std::thread::sleep(std::time::Duration::from_secs(1));
                continue;
            }

            let event = unsafe {
                CreateEventW(None, true, false, PCWSTR::null()).expect("CreateEventW failed")
            };

            let mut overlapped = unsafe {
                windows::Win32::System::IO::OVERLAPPED {
                    hEvent: event,
                    ..std::mem::zeroed()
                }
            };

            let _connect_result = unsafe { ConnectNamedPipe(handle, Some(&mut overlapped)) };

            let wait_result = unsafe { WaitForSingleObject(event, 500) };

            if wait_result != WAIT_OBJECT_0 {
                if shutdown.load(Ordering::SeqCst) {
                    unsafe {
                        CloseHandle(handle).ok();
                        CloseHandle(event).ok();
                    }
                    break;
                }
                unsafe {
                    CancelIoEx(handle, Some(&overlapped)).ok();
                }
                unsafe {
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

        eprintln!("[ecram_service] pipe server shutting down");
    }

    fn handle_client(handle: HANDLE) {
        let mut read_buf = [0u8; BUFSIZE as usize];
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

            let s = String::from_utf8_lossy(&read_buf[..total_read]);
            if s.trim_end().ends_with('}') {
                break;
            }
        }

        if total_read == 0 {
            return;
        }

        let request = String::from_utf8_lossy(&read_buf[..total_read]);
        let response = process_request(&request);

        let resp_bytes = response.as_bytes();
        let mut written = 0u32;
        unsafe {
            WriteFile(handle, Some(resp_bytes), Some(&mut written), None).ok();
            FlushFileBuffers(handle).ok();
        }
    }

    fn process_request(request: &str) -> String {
        let parsed: serde_json::Value = match serde_json::from_str(request.trim()) {
            Ok(v) => v,
            Err(e) => {
                return format!(r#"{{"ok":false,"error":"invalid JSON: {e}"}}"#);
            }
        };

        let op = parsed.get("op").and_then(|v| v.as_str()).unwrap_or("");

        match op {
            "read" => {
                let addr_str = parsed.get("addr").and_then(|v| v.as_str()).unwrap_or("");
                let size = parsed.get("size").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
                let addr = u64::from_str_radix(addr_str.trim_start_matches("0x"), 16).unwrap_or(0);

                // Send handshake before reading (idempotent — driver just sets a flag)
                let _ = ecram::send_handshake();
                match ecram::read_ecram(addr, size) {
                    Ok(data) => {
                        let hex: String = data.iter().map(|b| format!("{b:02X}")).collect();
                        format!(
                            r#"{{"ok":true,"addr":"0x{addr:08X}","size":{size},"data":"{hex}"}}"#
                        )
                    }
                    Err(e) => format!(r#"{{"ok":false,"error":"{e}"}}"#),
                }
            }
            "write" => {
                let addr_str = parsed.get("addr").and_then(|v| v.as_str()).unwrap_or("");
                let data_hex = parsed.get("data").and_then(|v| v.as_str()).unwrap_or("");
                let addr = u64::from_str_radix(addr_str.trim_start_matches("0x"), 16).unwrap_or(0);
                let data = match hex_decode(data_hex) {
                    Ok(d) => d,
                    Err(e) => return format!(r#"{{"ok":false,"error":"{e}"}}"#),
                };

                match ecram::write_ecram(addr, &data) {
                    Ok(n) => format!(r#"{{"ok":true,"addr":"0x{addr:08X}","bytes_written":{n}}}"#),
                    Err(e) => format!(r#"{{"ok":false,"error":"{e}"}}"#),
                }
            }
            "read_region" => {
                let region = parsed.get("region").and_then(|v| v.as_str()).unwrap_or("");
                let (addr, size) = ecram::REGIONS
                    .iter()
                    .find(|(name, _, _)| name.eq_ignore_ascii_case(region))
                    .map(|(_, a, s)| (*a, *s))
                    .unwrap_or((0, 0));

                if addr == 0 {
                    return format!(r#"{{"ok":false,"error":"unknown region '{region}'"}}"#);
                }

                // Send handshake before reading (idempotent — driver just sets a flag)
                let _ = ecram::send_handshake();
                match ecram::read_ecram(addr, size) {
                    Ok(data) => {
                        let hex: String = data.iter().map(|b| format!("{b:02X}")).collect();
                        format!(
                            r#"{{"ok":true,"region":"{region}","addr":"0x{addr:08X}","size":{size},"data":"{hex}"}}"#
                        )
                    }
                    Err(e) => format!(r#"{{"ok":false,"error":"{e}"}}"#),
                }
            }
            "ping" => r#"{"ok":true,"pong":true}"#.to_string(),
            _ => format!(r#"{{"ok":false,"error":"unknown op '{op}'"}}"#),
        }
    }

    fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
        let s = s.trim();
        if !s.len().is_multiple_of(2) {
            return Err("hex data must have even number of digits".into());
        }
        (0..s.len())
            .step_by(2)
            .map(|i| {
                u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| format!("invalid hex byte: {e}"))
            })
            .collect()
    }
}

// ── Windows Service implementation ────────────────────────────────────────────

mod service {
    use super::pipe_server;
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStrExt;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use windows::core::{PCWSTR, PWSTR};
    use windows::Win32::System::Services::{
        RegisterServiceCtrlHandlerW, SetServiceStatus, StartServiceCtrlDispatcherW,
        LPHANDLER_FUNCTION, SERVICE_ACCEPT_STOP, SERVICE_CONTROL_STOP, SERVICE_RUNNING,
        SERVICE_START_PENDING, SERVICE_STATUS, SERVICE_STOPPED, SERVICE_TABLE_ENTRYW,
        SERVICE_WIN32_OWN_PROCESS,
    };

    const SERVICE_NAME: &str = "IoTSvc";
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

                // Send the ReportLaptopStatus(IOT_WIN_READY) handshake.
                // This must be done before any ECRAM reads can succeed.
                // The handshake is a zeroed 0x110-byte buffer sent via IOCTL 0x22E000.
                // When running as a service via SCM, we are NT AUTHORITY\SYSTEM
                // and our process image path matches what the driver expects.
                match super::ecram::send_handshake() {
                    Ok(()) => {
                        eprintln!("[ecram_service] Handshake sent successfully");
                    }
                    Err(e) => {
                        eprintln!("[ecram_service] Handshake failed: {e}");
                        // Continue anyway — the pipe server can retry per-request
                    }
                }

                let shutdown = Arc::new(AtomicBool::new(false));
                let shutdown_clone = shutdown.clone();

                let pipe_thread = std::thread::spawn(move || {
                    pipe_server::run_pipe_server(shutdown_clone);
                });

                set_service_state(h, SERVICE_RUNNING);

                while !SHUTDOWN.load(Ordering::SeqCst) {
                    std::thread::sleep(std::time::Duration::from_millis(200));
                }

                shutdown.store(true, Ordering::SeqCst);
                let _ = pipe_thread.join();
                set_service_state(h, SERVICE_STOPPED);
            }
            Err(e) => {
                eprintln!("[ecram_service] RegisterServiceCtrlHandlerW failed: {e}");
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

// ── CLI mode ──────────────────────────────────────────────────────────────────

fn cli_mode(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("Usage: ecram_service <command> [args]");
        eprintln!("Commands:");
        eprintln!("  service                      Run as Windows service (via SCM)");
        eprintln!(
            "  handshake                    Send ReportLaptopStatus(IOT_WIN_READY) handshake"
        );
        eprintln!("  handshake-read-region <R>    Send handshake then read region");
        eprintln!("  read-region <REGION>         Read ECRAM region (ERAM, SMA2, IOT_STATUS, IOT_SENSORS)");
        eprintln!("  read <addr_hex> <count>      Read <count> bytes from address");
        eprintln!("  write <addr_hex> <hex_data>  Write hex data to address");
        eprintln!("  pipe-test                    Run pipe server in console (for testing)");
        return 1;
    }

    match args[0].as_str() {
        "service" => {
            if let Err(e) = service::run() {
                eprintln!("Service error: {e}");
                return 1;
            }
            0
        }
        "handshake" => {
            // Send the ReportLaptopStatus(IOT_WIN_READY) handshake
            match ecram::send_handshake() {
                Ok(()) => {
                    println!(r#"{{"ok":true,"msg":"handshake sent"}}"#);
                    0
                }
                Err(e) => {
                    println!(r#"{{"ok":false,"error":"{e}"}}"#);
                    1
                }
            }
        }
        "handshake-read-region" => {
            // Send handshake then immediately read a region
            if args.len() < 2 {
                eprintln!("Usage: handshake-read-region <ERAM|SMA2|IOT_STATUS|IOT_SENSORS>");
                return 1;
            }
            // Step 1: Send handshake
            if let Err(e) = ecram::send_handshake() {
                println!(r#"{{"ok":false,"error":"handshake failed: {e}"}}"#);
                return 1;
            }
            eprintln!("[ecram_service] Handshake sent, now reading region...");
            // Step 2: Read region
            let region = &args[1];
            let (addr, size) = ecram::REGIONS
                .iter()
                .find(|(name, _, _)| name.eq_ignore_ascii_case(region))
                .map(|(_, a, s)| (*a, *s))
                .unwrap_or((0, 0));
            if addr == 0 {
                eprintln!("Unknown region: {region}");
                return 1;
            }
            match ecram::read_ecram(addr, size) {
                Ok(data) => {
                    let hex: String = data.iter().map(|b| format!("{b:02X}")).collect();
                    println!(
                        r#"{{"ok":true,"region":"{region}","addr":"0x{addr:08X}","size":{size},"data":"{hex}"}}"#
                    );
                    0
                }
                Err(e) => {
                    println!(r#"{{"ok":false,"error":"read after handshake: {e}"}}"#);
                    1
                }
            }
        }
        "read-region" => {
            if args.len() < 2 {
                eprintln!("Usage: read-region <ERAM|SMA2|IOT_STATUS|IOT_SENSORS>");
                return 1;
            }
            let region = &args[1];
            let (addr, size) = ecram::REGIONS
                .iter()
                .find(|(name, _, _)| name.eq_ignore_ascii_case(region))
                .map(|(_, a, s)| (*a, *s))
                .unwrap_or((0, 0));
            if addr == 0 {
                eprintln!("Unknown region: {region}");
                return 1;
            }
            match ecram::read_ecram(addr, size) {
                Ok(data) => {
                    let hex: String = data.iter().map(|b| format!("{b:02X}")).collect();
                    println!(
                        r#"{{"ok":true,"region":"{region}","addr":"0x{addr:08X}","size":{size},"data":"{hex}"}}"#
                    );
                    0
                }
                Err(e) => {
                    println!(r#"{{"ok":false,"error":"{e}"}}"#);
                    1
                }
            }
        }
        "read" => {
            if args.len() < 3 {
                eprintln!("Usage: read <addr_hex> <count_dec>");
                return 1;
            }
            let addr = u64::from_str_radix(args[1].trim_start_matches("0x"), 16).unwrap_or(0);
            let size: usize = args[2].parse().unwrap_or(0);
            if size == 0 || size > 256 {
                eprintln!("count must be 1..256");
                return 1;
            }
            match ecram::read_ecram(addr, size) {
                Ok(data) => {
                    let hex: String = data.iter().map(|b| format!("{b:02X}")).collect();
                    println!(r#"{{"ok":true,"addr":"0x{addr:08X}","size":{size},"data":"{hex}"}}"#);
                    0
                }
                Err(e) => {
                    println!(r#"{{"ok":false,"error":"{e}"}}"#);
                    1
                }
            }
        }
        "write" => {
            if args.len() < 3 {
                eprintln!("Usage: write <addr_hex> <hex_data>");
                return 1;
            }
            let addr = u64::from_str_radix(args[1].trim_start_matches("0x"), 16).unwrap_or(0);
            let hex_data = &args[2];
            if !hex_data.len().is_multiple_of(2) {
                eprintln!("hex_data must have even number of digits");
                return 1;
            }
            let data: Vec<u8> = (0..hex_data.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex_data[i..i + 2], 16).unwrap_or(0))
                .collect();
            if data.is_empty() || data.len() > 256 {
                eprintln!("data must be 1..256 bytes");
                return 1;
            }
            match ecram::write_ecram(addr, &data) {
                Ok(n) => {
                    println!(r#"{{"ok":true,"addr":"0x{addr:08X}","bytes_written":{n}}}"#);
                    0
                }
                Err(e) => {
                    println!(r#"{{"ok":false,"error":"{e}"}}"#);
                    1
                }
            }
        }
        "pipe-test" => {
            let shutdown = Arc::new(AtomicBool::new(false));
            eprintln!("Starting pipe server on \\\\.\\pipe\\ecram_service");
            eprintln!("Press Ctrl+C to stop");
            pipe_server::run_pipe_server(shutdown);
            0
        }
        _ => {
            eprintln!("Unknown command: {}", args[0]);
            1
        }
    }
}

fn main() {
    // When started by SCM, there are no meaningful CLI args (or just the service name).
    // Try to connect to SCM first. If it succeeds, we're running as a service.
    // If it fails, we're running from a terminal — fall through to CLI mode.
    let args: Vec<String> = std::env::args().skip(1).collect();

    // If the user explicitly asked for CLI mode, skip the SCM attempt.
    if !args.is_empty() && args[0] != "service" {
        std::process::exit(cli_mode(&args));
    }

    // Try to start as a service (this blocks until the service stops).
    match service::run() {
        Ok(()) => {
            // Service ran and stopped normally.
            std::process::exit(0);
        }
        Err(e) => {
            // If the error is "service controller not available" (error 1063),
            // we're running from a terminal — fall through to CLI mode.
            if args.is_empty() {
                eprintln!("[ecram_service] Not started by SCM ({e}).");
                eprintln!("Usage: ecram_service <command> [args]");
                eprintln!("Commands:");
                eprintln!("  service                      Run as Windows service (via SCM)");
                eprintln!("  handshake                    Send ReportLaptopStatus(IOT_WIN_READY) handshake");
                eprintln!("  handshake-read-region <R>    Send handshake then read region");
                eprintln!("  read-region <REGION>         Read ECRAM region (ERAM, SMA2, IOT_STATUS, IOT_SENSORS)");
                eprintln!("  read <addr_hex> <count>      Read <count> bytes from address");
                eprintln!("  write <addr_hex> <hex_data>  Write hex data to address");
                eprintln!(
                    "  pipe-test                    Run pipe server in console (for testing)"
                );
                std::process::exit(1);
            }
            eprintln!("Service error: {e}");
            std::process::exit(1);
        }
    }
}
