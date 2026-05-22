//! ecram_shim — IoTDriver ECRAM helper
//!
//! This binary is deployed to the IoTDriver DriverStore directory so that its
//! image path satisfies IoTDriver.sys's `RtlPrefixUnicodeString` security check:
//!
//!   Driver path: \...\DriverStore\FileRepository\miiotdrv.inf_amd64_XXXXXX\IoTDriver.sys
//!   Prefix kept: \...\DriverStore\FileRepository\miiotdrv.inf_amd64
//!   Shim path:   \...\DriverStore\FileRepository\miiotdrv.inf_amd64_XXXXXX\ecram_shim.exe
//!   Passes check: shim dir STARTS WITH driver prefix ✓
//!
//! Usage:
//!   ecram_shim.exe <phys_addr_hex> <byte_count_dec>
//!   ecram_shim.exe read <phys_addr_hex> <byte_count_dec>
//!   ecram_shim.exe write <phys_addr_hex> <hex_data>
//!   ecram_shim.exe read-region <ERAM|SMA2|IOT_STATUS|IOT_SENSORS>
//! Stdout: JSON
//!
//! The binary intentionally does NOT link against micontrol_lib to stay small.

#![windows_subsystem = "console"]
#![cfg(windows)]

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::process;

use windows::{
    core::PCWSTR,
    Win32::{
        Devices::DeviceAndDriverInstallation::{
            SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsW,
            SetupDiGetDeviceInterfaceDetailW, DIGCF_DEVICEINTERFACE, DIGCF_PRESENT,
            SP_DEVICE_INTERFACE_DATA, SP_DEVICE_INTERFACE_DETAIL_DATA_W,
        },
        Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        },
        System::IO::DeviceIoControl,
    },
};

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
const ERAM_BASE: u64 = 0xFE0B0300;
const ERAM_SIZE: usize = 0x100;
const SMA2_BASE: u64 = 0xFE0B0A00;
const SMA2_SIZE: usize = 0x100;
const IOT_STATUS_BASE: u64 = 0xFE0B0F00;
const IOT_STATUS_SIZE: usize = 0x08;
const IOT_SENSORS_BASE: u64 = 0xFE0B0F08;
const IOT_SENSORS_SIZE: usize = 0x78;

#[repr(C)]
struct EcramBuf {
    physical_address: u64,
    byte_count: u64,
    data: [u8; 0x100],
}
const _: () = assert!(std::mem::size_of::<EcramBuf>() == IOCTL_BUF_SIZE);

enum ShimCommand {
    Read { addr: u64, count: usize },
    Write { addr: u64, data: Vec<u8> },
    ReadRegion { name: &'static str, addr: u64, count: usize },
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let command = match parse_args(&args) {
        Ok(command) => command,
        Err(e) => {
            print_error(&e);
            process::exit(1);
        }
    };

    match command {
        ShimCommand::Read { addr, count } => match read_ecram(addr, count) {
            Ok(data) => print_read_ok(None, addr, count, &data),
            Err(e) => {
                print_error(&e);
                process::exit(1);
            }
        },
        ShimCommand::ReadRegion { name, addr, count } => match read_ecram(addr, count) {
            Ok(data) => print_read_ok(Some(name), addr, count, &data),
            Err(e) => {
                print_error(&e);
                process::exit(1);
            }
        },
        ShimCommand::Write { addr, data } => match write_ecram(addr, &data) {
            Ok(()) => {
                println!(
                    r#"{{"ok":true,"operation":"write","address":"{addr:#010x}","bytes_written":{}}}"#,
                    data.len()
                );
            }
            Err(e) => {
                print_error(&e);
                process::exit(1);
            }
        },
    }
}

fn parse_args(args: &[String]) -> Result<ShimCommand, String> {
    fn parse_addr(raw: &str) -> Result<u64, String> {
        u64::from_str_radix(raw.trim_start_matches("0x"), 16)
            .map_err(|e| format!("bad addr: {e}"))
    }

    fn parse_count(raw: &str) -> Result<usize, String> {
        let count = raw.parse::<usize>().map_err(|e| format!("bad count: {e}"))?;
        if count == 0 || count > 0x100 {
            return Err("count must be 1..256".into());
        }
        Ok(count)
    }

    match args.get(1).map(String::as_str) {
        Some("read") => {
            if args.len() != 4 {
                return Err("usage: ecram_shim read <addr_hex> <count_dec>".into());
            }
            Ok(ShimCommand::Read {
                addr: parse_addr(&args[2])?,
                count: parse_count(&args[3])?,
            })
        }
        Some("write") => {
            if args.len() != 4 {
                return Err("usage: ecram_shim write <addr_hex> <hex_data>".into());
            }
            Ok(ShimCommand::Write {
                addr: parse_addr(&args[2])?,
                data: parse_hex_bytes(&args[3])?,
            })
        }
        Some("read-region") => {
            if args.len() != 3 {
                return Err("usage: ecram_shim read-region <ERAM|SMA2|IOT_STATUS|IOT_SENSORS>".into());
            }
            let (name, addr, count) = lookup_region(&args[2])?;
            Ok(ShimCommand::ReadRegion { name, addr, count })
        }
        Some(_) if args.len() == 3 => Ok(ShimCommand::Read {
            addr: parse_addr(&args[1])?,
            count: parse_count(&args[2])?,
        }),
        _ => Err(
            "usage: ecram_shim <addr_hex> <count_dec> | read <addr_hex> <count_dec> | write <addr_hex> <hex_data> | read-region <ERAM|SMA2|IOT_STATUS|IOT_SENSORS>".into(),
        ),
    }
}

fn parse_hex_bytes(raw: &str) -> Result<Vec<u8>, String> {
    let normalized: String = raw
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != ',' && *c != '-')
        .collect();

    if normalized.is_empty() || normalized.len() % 2 != 0 {
        return Err("hex_data must contain an even number of hex digits".into());
    }

    let bytes = (0..normalized.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&normalized[i..i + 2], 16))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("bad hex_data: {e}"))?;

    if bytes.is_empty() || bytes.len() > 0x100 {
        return Err("hex_data must decode to 1..256 bytes".into());
    }

    Ok(bytes)
}

fn lookup_region(name: &str) -> Result<(&'static str, u64, usize), String> {
    match name.to_ascii_uppercase().as_str() {
        "ERAM" => Ok(("ERAM", ERAM_BASE, ERAM_SIZE)),
        "SMA2" => Ok(("SMA2", SMA2_BASE, SMA2_SIZE)),
        "IOT_STATUS" => Ok(("IOT_STATUS", IOT_STATUS_BASE, IOT_STATUS_SIZE)),
        "IOT_SENSORS" => Ok(("IOT_SENSORS", IOT_SENSORS_BASE, IOT_SENSORS_SIZE)),
        _ => Err("unknown region; expected ERAM, SMA2, IOT_STATUS or IOT_SENSORS".into()),
    }
}

fn print_read_ok(region: Option<&str>, addr: u64, count: usize, data: &[u8]) {
    let hex: String = data.iter().map(|b| format!("{b:02x}")).collect();
    match region {
        Some(region) => println!(
            r#"{{"ok":true,"operation":"read","region":"{region}","address":"{addr:#010x}","size":{count},"data":"{hex}"}}"#
        ),
        None => println!(
            r#"{{"ok":true,"operation":"read","address":"{addr:#010x}","size":{count},"data":"{hex}"}}"#
        ),
    }
}

fn print_error(error: &str) {
    let msg = error.replace('"', "'");
    println!(r#"{{"ok":false,"error":"{msg}"}}"#);
}

fn read_ecram(phys_addr: u64, byte_count: usize) -> Result<Vec<u8>, String> {
    let device_path = find_iot_device_path()?;
    read_ecram_inner(&device_path, phys_addr, byte_count)
}

fn write_ecram(phys_addr: u64, data: &[u8]) -> Result<(), String> {
    let device_path = find_iot_device_path()?;
    write_ecram_inner(&device_path, phys_addr, data)
}

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

        let ok = SetupDiEnumDeviceInterfaces(dev_info, None, &IOT_GUID, 0, &mut iface);
        if ok.is_err() {
            let _ = SetupDiDestroyDeviceInfoList(dev_info);
            return Err("No IoT device interface found (GUID {AB7924A1-...})".into());
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
        (*detail_ptr).cbSize =
            std::mem::size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_W>() as u32;

        let detail_ok = SetupDiGetDeviceInterfaceDetailW(
            dev_info,
            &iface,
            Some(detail_ptr),
            required,
            None,
            None,
        );
        let _ = SetupDiDestroyDeviceInfoList(dev_info);
        detail_ok.map_err(|e| format!("SetupDiGetDeviceInterfaceDetailW: {e}"))?;

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
            .map_err(|e| format!("UTF-16 device path: {e}"))
    }
}

fn read_ecram_inner(device_path: &str, phys_addr: u64, byte_count: usize) -> Result<Vec<u8>, String> {
    let path_w: Vec<u16> = OsStr::new(device_path).encode_wide().chain(Some(0)).collect();

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

        let mut returned = 0u32;
        let result = DeviceIoControl(
            handle,
            IOCTL_ECRAM_READ,
            Some(&in_buf as *const EcramBuf as *const _),
            IOCTL_BUF_SIZE as u32,
            Some(&mut out_buf as *mut EcramBuf as *mut _),
            IOCTL_BUF_SIZE as u32,
            Some(&mut returned),
            None,
        );
        let _ = CloseHandle(handle);

        result.map_err(|e| format!("DeviceIoControl ECRAM_READ: {e}"))?;

        // Driver writes EC data starting at byte offset 0x10 in the output buffer
        Ok(out_buf.data[..byte_count].to_vec())
    }
}

fn write_ecram_inner(device_path: &str, phys_addr: u64, data: &[u8]) -> Result<(), String> {
    let path_w: Vec<u16> = OsStr::new(device_path).encode_wide().chain(Some(0)).collect();

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

        let mut returned = 0u32;
        let result = DeviceIoControl(
            handle,
            IOCTL_ECRAM_WRITE,
            Some(&in_buf as *const EcramBuf as *const _),
            IOCTL_BUF_SIZE as u32,
            Some(&mut out_buf as *mut EcramBuf as *mut _),
            IOCTL_BUF_SIZE as u32,
            Some(&mut returned),
            None,
        );
        let _ = CloseHandle(handle);

        result.map_err(|e| format!("DeviceIoControl ECRAM_WRITE: {e}"))?;
        Ok(())
    }
}
