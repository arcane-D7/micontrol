//! ecram_shim — IoTDriver ECRAM reader helper
//!
//! This binary is deployed to the IoTDriver DriverStore directory so that its
//! image path satisfies IoTDriver.sys's `RtlPrefixUnicodeString` security check:
//!
//!   Driver path: \...\DriverStore\FileRepository\miiotdrv.inf_amd64_XXXXXX\IoTDriver.sys
//!   Prefix kept: \...\DriverStore\FileRepository\miiotdrv.inf_amd64
//!   Shim path:   \...\DriverStore\FileRepository\miiotdrv.inf_amd64_XXXXXX\ecram_shim.exe
//!   Passes check: shim dir STARTS WITH driver prefix ✓
//!
//! Usage:  ecram_shim.exe <phys_addr_hex> <byte_count_dec>
//! Stdout: {"ok":true,"data":"AABBCC..."} | {"ok":false,"error":"..."}
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
const IOCTL_BUF_SIZE: usize = 0x110;

#[repr(C)]
struct EcramBuf {
    physical_address: u64,
    byte_count: u64,
    data: [u8; 0x100],
}
const _: () = assert!(std::mem::size_of::<EcramBuf>() == IOCTL_BUF_SIZE);

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        println!(r#"{{"ok":false,"error":"usage: ecram_shim <addr_hex> <count_dec>"}}"#);
        process::exit(1);
    }

    let addr = match u64::from_str_radix(args[1].trim_start_matches("0x"), 16) {
        Ok(v) => v,
        Err(e) => {
            println!(r#"{{"ok":false,"error":"bad addr: {e}"}}"#);
            process::exit(1);
        }
    };

    let count = match args[2].parse::<usize>() {
        Ok(v) => v,
        Err(e) => {
            println!(r#"{{"ok":false,"error":"bad count: {e}"}}"#);
            process::exit(1);
        }
    };

    if count == 0 || count > 0x100 {
        println!(r#"{{"ok":false,"error":"count must be 1..256"}}"#);
        process::exit(1);
    }

    match read_ecram(addr, count) {
        Ok(data) => {
            let hex: String = data.iter().map(|b| format!("{b:02x}")).collect();
            println!(r#"{{"ok":true,"data":"{hex}"}}"#);
        }
        Err(e) => {
            let msg = e.replace('"', "'");
            println!(r#"{{"ok":false,"error":"{msg}"}}"#);
            process::exit(1);
        }
    }
}

fn read_ecram(phys_addr: u64, byte_count: usize) -> Result<Vec<u8>, String> {
    let device_path = find_iot_device_path()?;
    read_ecram_inner(&device_path, phys_addr, byte_count)
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
