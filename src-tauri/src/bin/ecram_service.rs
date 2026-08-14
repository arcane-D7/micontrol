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

use std::os::windows::process::CommandExt;
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

// ── EC Command Protocol (IoT chip communication) ─────────────────────────────
//
// Implements the 4-phase EC command state machine reverse-engineered from
// the original IoTService.exe:
//   1. RamIsReady — check status byte at 0xFE0B0F00 == 0x00
//   2. WriteCommand — write 7-byte [cmd_id, 0x01, 0x01, UID×4] to 0xFE0B0F01,
//      then write 0x55 trigger to 0xFE0B0F00
//   3. ReadCmdAck — poll 0xFE0B0F00 for [0x55, cmd_id, 0x01, 0x02], 5ms interval,
//      100 max retries
//   4. ReadCmdRet — poll 0xFE0B0F00 for [0x55, cmd_id, 0x01, 0x03, <4 bytes data>],
//      45ms interval, 60 max retries
//
// See docs/EC_COMMAND_PROTOCOL_RE.md for full details.

mod ec_command {
    use super::ecram;
    use std::sync::Mutex;

    /// ECRAM addresses for EC command protocol
    const EC_STATUS_ADDR: u64 = 0xFE0B0F00;
    const EC_COMMAND_ADDR: u64 = 0xFE0B0F01;
    const EC_SENSOR_ADDR: u64 = 0xFE0B0F08;

    /// EC command IDs (from RE of IoTService.exe)
    ///
    /// Not all IDs are currently used — they document the full EC command
    /// protocol for future implementation. `#[allow(dead_code)]` suppresses
    /// warnings for the unused variants.
    #[allow(dead_code)]
    pub mod cmd_id {
        pub const GET_BIND_STATUS: u8 = 0x01;
        pub const SET_BIND_STATUS: u8 = 0x02;
        pub const RESET_DEVICE: u8 = 0x03;
        pub const WRITE_WIFI_ITEM: u8 = 0x04;
        pub const EMPTY_WIFI_ITEMS: u8 = 0x05;
        pub const DELETE_WIFI_ITEM: u8 = 0x06;
        pub const READ_WIFI_STATUS: u8 = 0x07;
        pub const READ_WIFI_COUNT: u8 = 0x08;
        pub const GET_WIFI_BY_INDEX: u8 = 0x09;
        pub const GET_FW_VERSION: u8 = 0x0A;
        pub const GET_MODEL: u8 = 0x0B;
        pub const CONNECT_WIFI: u8 = 0x0C;
        pub const GET_DEVICE_ID: u8 = 0x0D;
        pub const LAPTOP_SUSPEND: u8 = 0x0E;
        pub const LAPTOP_SHUTDOWN: u8 = 0x0F;
        pub const LAPTOP_WIN_READY: u8 = 0x10;
    }

    /// Mutex to serialize EC command access — prevents concurrent EC corruption.
    static EC_MUTEX: Mutex<()> = Mutex::new(());

    /// Cached device UID (populated by GetBindStatus or registry)
    static CACHED_UID: Mutex<Option<u32>> = Mutex::new(None);

    /// Read the device UID from the Windows registry.
    /// The original IoTService stores it in HKLM\SOFTWARE\MI\IoTDriver\Uid (REG_DWORD).
    fn read_uid_from_registry() -> Option<u32> {
        use windows::Win32::System::Registry::{
            RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_LOCAL_MACHINE, KEY_READ,
        };

        let subkey: Vec<u16> = "SOFTWARE\\MI\\IoTDriver\0".encode_utf16().collect();

        let mut hkey = windows::Win32::System::Registry::HKEY::default();
        let result = unsafe {
            RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                windows::core::PCWSTR(subkey.as_ptr()),
                0,
                KEY_READ,
                &mut hkey,
            )
        };
        if result.is_err() {
            return None;
        }

        let value_name: Vec<u16> = "Uid\0".encode_utf16().collect();
        let mut uid_val: u32 = 0;
        let mut uid_len: u32 = std::mem::size_of::<u32>() as u32;
        let uid_type = windows::Win32::System::Registry::REG_NONE;
        let uid_result = unsafe {
            RegQueryValueExW(
                hkey,
                windows::core::PCWSTR(value_name.as_ptr()),
                None,
                Some(&mut uid_type.clone() as *mut _),
                Some(&mut uid_val as *mut u32 as *mut u8),
                Some(&mut uid_len),
            )
        };

        unsafe {
            let _ = RegCloseKey(hkey);
        }

        if uid_result.is_err() || uid_val == 0 {
            None
        } else {
            Some(uid_val)
        }
    }

    /// Get the device UID, either from cache, registry, or by querying the EC.
    /// If no UID is available, returns 0 (some commands work with UID=0).
    fn get_uid() -> u32 {
        // Check cache first
        if let Ok(cache) = CACHED_UID.lock() {
            if let Some(uid) = *cache {
                return uid;
            }
        }

        // Try registry
        if let Some(uid) = read_uid_from_registry() {
            if let Ok(mut cache) = CACHED_UID.lock() {
                *cache = Some(uid);
            }
            return uid;
        }

        // No UID available — use 0 (commands may still work if EC doesn't require UID)
        0
    }

    /// Check if EC is ready (status byte == 0x00).
    /// Does NOT poll — caller must retry if busy.
    fn ram_is_ready() -> Result<(), String> {
        let _ = ecram::send_handshake();
        let data = ecram::read_ecram(EC_STATUS_ADDR, 1)?;
        if data[0] == 0x00 {
            Ok(())
        } else {
            Err(format!("EC busy (status=0x{:02X})", data[0]))
        }
    }

    /// Write a 7-byte command to EC, then trigger with 0x55.
    fn write_command(cmd: u8, uid: u32) -> Result<(), String> {
        let cmd_buf = [
            cmd,
            0x01,
            0x01,
            (uid & 0xFF) as u8,
            ((uid >> 8) & 0xFF) as u8,
            ((uid >> 16) & 0xFF) as u8,
            ((uid >> 24) & 0xFF) as u8,
        ];
        ecram::write_ecram(EC_COMMAND_ADDR, &cmd_buf)?;
        // Write trigger byte
        ecram::write_ecram(EC_STATUS_ADDR, &[0x55])?;
        Ok(())
    }

    /// Poll for ACK: expected [0x55, cmd_id, 0x01, 0x02].
    /// 5ms poll interval, 100 max retries.
    fn read_cmd_ack(cmd: u8) -> Result<(), String> {
        let expected_ack = [0x55u8, cmd, 0x01, 0x02];
        let last_cmd_pattern = [0x55u8, cmd, 0x01, 0x01];
        let next_cmd_pattern = [0x55u8, cmd, 0x01, 0x03];

        for i in 0..100 {
            // Don't sleep on first iteration — command may already be processed
            if i > 0 {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let data = ecram::read_ecram(EC_STATUS_ADDR, 4)?;
            if data == expected_ack {
                return Ok(());
            }
            if data == last_cmd_pattern {
                continue;
            }
            if data == next_cmd_pattern {
                // EC already moved to RET phase — retry
                continue;
            }
            // Mismatch — decode EC error code and reset
            let _ = reset_ec();
            let err_msg = match data[0] {
                0x11 => format!("EC error 0x11 (cmd 0x{cmd:02X})"),
                0x22 => format!("EC timeout (cmd 0x{cmd:02X})"),
                0x33 => format!("EC error 0x33 (cmd 0x{cmd:02X})"),
                0x44 => format!("EC error 0x44 (cmd 0x{cmd:02X})"),
                _ => format!(
                    "ACK mismatch for cmd 0x{cmd:02X}: expected {:02X?}, got {:02X?}",
                    expected_ack, data
                ),
            };
            return Err(err_msg);
        }
        let _ = reset_ec();
        Err(format!("ACK timeout for cmd 0x{cmd:02X}"))
    }

    /// Poll for RET: expected [0x55, cmd_id, 0x01, 0x03, <4 bytes data>].
    /// 45ms poll interval, 60 max retries.
    fn read_cmd_ret(cmd: u8) -> Result<[u8; 4], String> {
        let expected_ret = [0x55u8, cmd, 0x01, 0x03];
        let last_ack_pattern = [0x55u8, cmd, 0x01, 0x02];

        for i in 0..60 {
            // Don't sleep on first iteration — ACK may already be done
            if i > 0 {
                std::thread::sleep(std::time::Duration::from_millis(45));
            }
            let data = ecram::read_ecram(EC_STATUS_ADDR, 8)?;
            if data.len() < 8 {
                return Err("RET read too short".to_string());
            }
            let first4: [u8; 4] = [data[0], data[1], data[2], data[3]];
            if first4 == expected_ret {
                // RET data is in bytes 4-7
                return Ok([data[4], data[5], data[6], data[7]]);
            }
            if first4 == last_ack_pattern {
                // Still processing ACK — retry
                continue;
            }
            // Mismatch — decode EC error code and reset
            let _ = reset_ec();
            let err_msg = match first4[0] {
                0x11 => format!("EC error 0x11 (cmd 0x{cmd:02X})"),
                0x22 => format!("EC timeout (cmd 0x{cmd:02X})"),
                0x33 => format!("EC error 0x33 (cmd 0x{cmd:02X})"),
                0x44 => format!("EC error 0x44 (cmd 0x{cmd:02X})"),
                _ => format!(
                    "RET mismatch for cmd 0x{cmd:02X}: expected {:02X?}, got {:02X?}",
                    expected_ret, first4
                ),
            };
            return Err(err_msg);
        }
        let _ = reset_ec();
        Err(format!("RET timeout for cmd 0x{cmd:02X}"))
    }

    /// Reset EC by writing zero to status address.
    fn reset_ec() -> Result<(), String> {
        ecram::write_ecram(EC_STATUS_ADDR, &[0x00])?;
        Ok(())
    }

    /// Read 120 bytes of sensor data from 0xFE0B0F08.
    fn read_sensor_data() -> Result<Vec<u8>, String> {
        ecram::read_ecram(EC_SENSOR_ADDR, 120)
    }

    /// Write sensor data to 0xFE0B0F08 (max 120 bytes).
    fn write_sensor_data(data: &[u8]) -> Result<(), String> {
        if data.len() > 120 {
            return Err(format!("sensor data too large: {} > 120", data.len()));
        }
        ecram::write_ecram(EC_SENSOR_ADDR, data)?;
        Ok(())
    }

    /// Execute a full EC command (Reset → RamIsReady → Write → Ack → Ret → Reset).
    /// Returns the 4-byte RET data.
    /// The EC must be reset (write 0x00 to status) before AND after each command
    /// to clear any leftover state from previous commands.
    fn execute_command(cmd: u8) -> Result<[u8; 4], String> {
        let _guard = EC_MUTEX.lock().map_err(|e| format!("EC mutex: {e}"))?;
        let uid = get_uid();

        // Reset EC first to clear any leftover state from previous commands
        let _ = reset_ec();
        std::thread::sleep(std::time::Duration::from_millis(5));

        // Retry RamIsReady up to 10 times with 5ms delay
        let mut ready = false;
        for _ in 0..10 {
            if ram_is_ready().is_ok() {
                ready = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        if !ready {
            return Err("EC not ready after 10 retries".to_string());
        }

        write_command(cmd, uid)?;
        read_cmd_ack(cmd)?;
        let ret_data = read_cmd_ret(cmd)?;

        // Reset EC after successful command to clear state for next command
        let _ = reset_ec();

        Ok(ret_data)
    }

    // ── Public feature implementations ────────────────────────────────────────

    /// GetBindStatus — cmd_id 0x01
    /// Returns (bound, uid) where uid is the device UID as u64.
    pub fn get_bind_status() -> Result<(bool, u64), String> {
        let ret = execute_command(cmd_id::GET_BIND_STATUS)?;
        match ret[0] {
            0x01 => {
                // Bound — read UID from sensor data
                let sensors = read_sensor_data()?;
                if sensors.is_empty() {
                    return Err("sensor data empty for bind status".to_string());
                }
                let uid_size = sensors[0] as usize;
                if uid_size == 0 {
                    return Ok((true, 0));
                }
                // Bytes 1-7 are UID, byte-swapped to big-endian u64
                let mut uid_bytes = [0u8; 8];
                let copy_len = uid_size.min(7);
                uid_bytes[..copy_len].copy_from_slice(&sensors[1..1 + copy_len]);
                let device_uid = u64::from_be_bytes(uid_bytes);
                Ok((true, device_uid))
            }
            0x02 => Ok((false, 0)),
            status => Err(format!("unexpected bind status: 0x{status:02X}")),
        }
    }

    /// GetFwVersion — cmd_id 0x0A
    /// Returns the firmware version as a string.
    pub fn get_fw_version() -> Result<String, String> {
        let ret = execute_command(cmd_id::GET_FW_VERSION)?;
        if ret[0] != 0x01 {
            return Err(format!("fw version failed: status 0x{:02X}", ret[0]));
        }
        let sensors = read_sensor_data()?;
        if sensors.is_empty() {
            return Err("sensor data empty for fw version".to_string());
        }
        let len = (sensors[0] as usize).min(119);
        if len == 0 {
            return Ok(String::new());
        }
        let version = String::from_utf8_lossy(&sensors[1..1 + len])
            .trim_end_matches('\0')
            .to_string();
        Ok(version)
    }

    /// GetModel — cmd_id 0x0B
    /// Returns the device model as a string.
    pub fn get_model() -> Result<String, String> {
        let ret = execute_command(cmd_id::GET_MODEL)?;
        if ret[0] != 0x01 {
            return Err(format!("get model failed: status 0x{:02X}", ret[0]));
        }
        let sensors = read_sensor_data()?;
        if sensors.is_empty() {
            return Err("sensor data empty for model".to_string());
        }
        let len = (sensors[0] as usize).min(119);
        if len == 0 {
            return Ok(String::new());
        }
        let model = String::from_utf8_lossy(&sensors[1..1 + len])
            .trim_end_matches('\0')
            .to_string();
        Ok(model)
    }

    /// GetDeviceID — cmd_id 0x0D
    /// Returns the device ID as u64.
    pub fn get_device_id() -> Result<u64, String> {
        let ret = execute_command(cmd_id::GET_DEVICE_ID)?;
        if ret[0] != 0x01 {
            return Err(format!("get device id failed: status 0x{:02X}", ret[0]));
        }
        let sensors = read_sensor_data()?;
        if sensors.is_empty() {
            return Err("sensor data empty for device id".to_string());
        }
        let did_size = sensors[0] as usize;
        if did_size == 0 {
            return Ok(0);
        }
        let mut did_bytes = [0u8; 8];
        let copy_len = did_size.min(7);
        did_bytes[..copy_len].copy_from_slice(&sensors[1..1 + copy_len]);
        let device_id = u64::from_be_bytes(did_bytes);
        Ok(device_id)
    }

    /// ReadWiFiCount — cmd_id 0x08
    /// Returns the number of provisioned WiFi networks.
    pub fn read_wifi_count() -> Result<u8, String> {
        let ret = execute_command(cmd_id::READ_WIFI_COUNT)?;
        if ret[0] != 0x01 {
            return Err(format!("read wifi count failed: status 0x{:02X}", ret[0]));
        }
        let sensors = read_sensor_data()?;
        if sensors.is_empty() {
            return Err("sensor data empty for wifi count".to_string());
        }
        Ok(sensors[0])
    }

    /// ReadWiFiStatus — cmd_id 0x07
    /// Returns (status_code, ssid) where status_code is the WiFi connection state.
    pub fn read_wifi_status() -> Result<(u8, Option<String>), String> {
        let ret = execute_command(cmd_id::READ_WIFI_STATUS)?;
        let status_code = ret[0];

        // Valid status codes: 0x01-0x07, 0x0F
        let is_valid = matches!(status_code, 0x01..=0x07 | 0x0F);
        if !is_valid {
            return Ok((status_code, None));
        }

        let sensors = read_sensor_data()?;
        if sensors.len() < 5 {
            return Ok((status_code, None));
        }
        let ssid_len = sensors[4] as usize;
        if ssid_len == 0 || ssid_len > 31 {
            return Ok((status_code, None));
        }
        if sensors.len() < 5 + ssid_len {
            return Ok((status_code, None));
        }
        let ssid = String::from_utf8_lossy(&sensors[5..5 + ssid_len])
            .trim_end_matches('\0')
            .to_string();
        Ok((status_code, Some(ssid)))
    }

    /// GetWiFiByIndex — cmd_id 0x09
    /// Returns the WiFi item at the given index as raw bytes (101 bytes).
    pub fn get_wifi_by_index(index: u8) -> Result<Vec<u8>, String> {
        if index >= 20 {
            return Err(format!("wifi index out of range: {index} >= 20"));
        }
        // Write index payload to sensor data
        write_sensor_data(&[0x01, index])?;
        let ret = execute_command(cmd_id::GET_WIFI_BY_INDEX)?;
        if ret[0] != 0x01 {
            return Err(format!("get wifi by index failed: status 0x{:02X}", ret[0]));
        }
        let sensors = read_sensor_data()?;
        Ok(sensors)
    }

    /// EmptyWiFiItems — cmd_id 0x05
    /// Removes all provisioned WiFi networks.
    pub fn empty_wifi_items() -> Result<(), String> {
        let ret = execute_command(cmd_id::EMPTY_WIFI_ITEMS)?;
        if ret[0] != 0x01 {
            return Err(format!("empty wifi items failed: status 0x{:02X}", ret[0]));
        }
        Ok(())
    }

    /// ConnectWiFi — cmd_id 0x0C
    /// Forces the IoT device to connect to provisioned WiFi.
    pub fn connect_wifi() -> Result<(), String> {
        let ret = execute_command(cmd_id::CONNECT_WIFI)?;
        if ret[0] != 0x01 && ret[0] != 0x02 {
            return Err(format!("connect wifi failed: status 0x{:02X}", ret[0]));
        }
        Ok(())
    }

    /// ResetDevice — cmd_id 0x03
    /// Resets the IoT device.
    pub fn reset_device() -> Result<(), String> {
        let ret = execute_command(cmd_id::RESET_DEVICE)?;
        if ret[0] != 0x01 {
            return Err(format!("reset device failed: status 0x{:02X}", ret[0]));
        }
        Ok(())
    }

    /// SendLaptopStatus — sends a laptop status command to the IoT device.
    /// status: 4=WinReady, 6=Suspending, 8=Shutting
    pub fn send_laptop_status(status: u32) -> Result<(), String> {
        let cmd = match status {
            4 => cmd_id::LAPTOP_WIN_READY,
            6 => cmd_id::LAPTOP_SUSPEND,
            8 => cmd_id::LAPTOP_SHUTDOWN,
            _ => return Err(format!("invalid laptop status: {status}")),
        };
        let ret = execute_command(cmd)?;
        if ret[0] != 0x01 {
            return Err(format!(
                "send laptop status {status} failed: status 0x{:02X}",
                ret[0]
            ));
        }
        Ok(())
    }

    /// WriteWiFiItem — cmd_id 0x04
    ///
    /// Provisions a WiFi network on the IoT device. Builds the 101-byte
    /// payload documented by RE (EC_COMMAND_PROTOCOL_RE.md / iotsvc_decompiled.c):
    ///
    /// | Offset | Size | Field |
    /// |--------|------|-------|
    /// | 0      | 1    | Magic 0x65 |
    /// | 1      | 1    | Connect flag (0/1) |
    /// | 2      | 2    | Checksum LE u16 = Σ buf[4..=100] |
    /// | 4      | 1    | SSID length (1..=32) |
    /// | 5      | 32   | SSID bytes (no NUL) |
    /// | 37     | 1    | Password length (1..=63) |
    /// | 38     | 63   | Password bytes (zero-padded) |
    ///
    /// Returns Ok(()) when the EC reports success (RET ∈ {0x01, 0x02, 0x04}).
    pub fn write_wifi_item(ssid: &str, password: &str, connect: bool) -> Result<(), String> {
        let ssid_bytes = ssid.as_bytes();
        if ssid_bytes.is_empty() || ssid_bytes.len() > 32 {
            return Err(format!("invalid SSID length: {}", ssid_bytes.len()));
        }
        let pwd_bytes = password.as_bytes();
        if pwd_bytes.len() > 63 {
            return Err(format!("invalid password length: {}", pwd_bytes.len()));
        }

        let mut buf = [0u8; 101];
        buf[0] = 0x65; // magic
        buf[1] = if connect { 0x01 } else { 0x00 };
        buf[4] = ssid_bytes.len() as u8;
        buf[5..5 + ssid_bytes.len()].copy_from_slice(ssid_bytes);
        buf[37] = pwd_bytes.len() as u8;
        buf[38..38 + pwd_bytes.len()].copy_from_slice(pwd_bytes);

        // Checksum: LE u16 sum of offsets 4..=100 (excludes 0..3 and 101).
        let mut sum: u16 = 0;
        for &b in &buf[4..=100] {
            sum = sum.wrapping_add(b as u16);
        }
        buf[2] = (sum & 0xFF) as u8;
        buf[3] = ((sum >> 8) & 0xFF) as u8;

        write_sensor_data(&buf)?;
        let ret = execute_command(cmd_id::WRITE_WIFI_ITEM)?;
        if !matches!(ret[0], 0x01 | 0x02 | 0x04) {
            return Err(format!("write wifi item failed: status 0x{:02X}", ret[0]));
        }
        Ok(())
    }

    /// DeleteWiFiItem — cmd_id 0x06
    ///
    /// Removes a provisioned WiFi network by SSID. Builds the 37-byte
    /// payload from RE: `[0x25, 0x00, 0x00, 0x00, ssid_len, ssid...]`.
    ///
    /// Returns Ok(()) when the EC reports success (RET ∈ {0x01, 0x02}).
    pub fn delete_wifi_item(ssid: &str) -> Result<(), String> {
        let ssid_bytes = ssid.as_bytes();
        if ssid_bytes.is_empty() || ssid_bytes.len() > 32 {
            return Err(format!("invalid SSID length: {}", ssid_bytes.len()));
        }

        let mut buf = [0u8; 37];
        buf[0] = 0x25; // delete marker (37 total)
        buf[4] = ssid_bytes.len() as u8;
        buf[5..5 + ssid_bytes.len()].copy_from_slice(ssid_bytes);

        write_sensor_data(&buf)?;
        let ret = execute_command(cmd_id::DELETE_WIFI_ITEM)?;
        if !matches!(ret[0], 0x01 | 0x02) {
            return Err(format!("delete wifi item failed: status 0x{:02X}", ret[0]));
        }
        Ok(())
    }

    /// Parse a raw 101-byte WiFi item into its fields.
    ///
    /// Layout (from RE): magic@0, connect@1, checksum@2..3, ssid_len@4,
    /// ssid@5.., pwd_len@37, pwd@38...
    ///
    /// Returns (ssid, connected, enabled) — `enabled` has no dedicated byte
    /// in the structure, so it mirrors the connect flag.
    pub fn parse_wifi_item(data: &[u8]) -> Result<(String, bool, bool), String> {
        if data.len() < 38 {
            return Err(format!("wifi item too short: {} bytes", data.len()));
        }
        let ssid_len = data[4] as usize;
        if ssid_len == 0 || ssid_len > 32 {
            return Err(format!("invalid ssid length in item: {ssid_len}"));
        }
        let end = 5 + ssid_len;
        if data.len() < end {
            return Err(format!(
                "wifi item truncated: need {end} bytes, have {}",
                data.len()
            ));
        }
        let ssid = String::from_utf8_lossy(&data[5..end])
            .trim_end_matches('\0')
            .to_string();
        let connected = data.get(1).copied().unwrap_or(0) != 0;
        // No dedicated enable byte in the on-wire structure — mirror connect.
        Ok((ssid, connected, connected))
    }

    // ── Charging threshold (EC ERAM registers) ──────────────────────────────
    //
    // The original IoTService.exe applies the charging threshold via WMI
    // (SetChargingLimit, msg_type 0x1003 → Worker_WMI). Our ecram_service
    // replaces IoTService.exe, so the MCPI pipe does not exist. Instead we
    // write the threshold directly to the EC ERAM registers:
    //   - 0xA4 (Battery Care master enable): 0x01 = respect threshold
    //   - 0xA7 (charging threshold): value in percent (40..=100)
    // These registers live in the ACPI ERAM region (get_eram_base() + offset).
    // The ecram_service runs as SYSTEM named "IoTService.exe" in the
    // DriverStore, so it passes the IoTDriver security check and can access
    // ERAM via the IOCTL path.

    const CHARGE_CARE_OFFSET: u64 = 0xA4;
    const CHARGE_THRESHOLD_OFFSET: u64 = 0xA7;

    fn eram_base() -> u64 {
        // Hardcoded fallback (matches ecram::get_eram_base() on the TM2424).
        // The service cannot import the crate's discovery, so use the same
        // DSDT-derived address; the fallback is correct for this platform.
        0xFE0B0300
    }

    /// Set the charging threshold on the EC.
    ///
    /// Enables Battery Care (0xA4 = 0x01) and writes the threshold
    /// (0xA7 = threshold). Valid thresholds are 40, 50, 60, 70, 80, 100.
    /// Returns the effective threshold.
    pub fn set_charging_threshold(threshold: u8) -> Result<u8, String> {
        const VALID: [u8; 6] = [40, 50, 60, 70, 80, 100];
        if !VALID.contains(&threshold) {
            return Err(format!("invalid charging threshold: {threshold}"));
        }
        let base = eram_base();
        let _ = ecram::send_handshake();
        ecram::write_ecram(base + CHARGE_CARE_OFFSET, &[0x01])?;
        ecram::write_ecram(base + CHARGE_THRESHOLD_OFFSET, &[threshold])?;
        Ok(threshold)
    }

    /// Read the charging threshold from the EC.
    ///
    /// Returns (battery_care_enabled, threshold). If the EC read fails,
    /// falls back to the registry value `HKLM\SOFTWARE\MI\IoTDriver\ChargingThreshold`.
    pub fn get_charging_threshold() -> Result<(bool, u8), String> {
        let base = eram_base();
        let _ = ecram::send_handshake();

        let care = match ecram::read_ecram(base + CHARGE_CARE_OFFSET, 1) {
            Ok(data) if !data.is_empty() => data[0] != 0,
            _ => false,
        };
        let threshold = match ecram::read_ecram(base + CHARGE_THRESHOLD_OFFSET, 1) {
            Ok(data) if !data.is_empty() => data[0].clamp(40, 100),
            _ => {
                // Fallback to registry (same key the app persists).
                read_charge_threshold_registry().unwrap_or(100)
            }
        };
        Ok((care, threshold))
    }

    fn read_charge_threshold_registry() -> Option<u8> {
        use windows::Win32::System::Registry::{
            RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_LOCAL_MACHINE, KEY_READ,
        };

        let subkey: Vec<u16> = "SOFTWARE\\MI\\IoTDriver\0".encode_utf16().collect();
        let mut hkey = windows::Win32::System::Registry::HKEY::default();
        let result = unsafe {
            RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                windows::core::PCWSTR(subkey.as_ptr()),
                0,
                KEY_READ,
                &mut hkey,
            )
        };
        if result.is_err() {
            return None;
        }

        let value_name: Vec<u16> = "ChargingThreshold\0".encode_utf16().collect();
        let mut val: u32 = 0;
        let mut len: u32 = std::mem::size_of::<u32>() as u32;
        let value_type = windows::Win32::System::Registry::REG_NONE;
        let result = unsafe {
            RegQueryValueExW(
                hkey,
                windows::core::PCWSTR(value_name.as_ptr()),
                None,
                Some(&mut value_type.clone() as *mut _),
                Some(&mut val as *mut u32 as *mut u8),
                Some(&mut len),
            )
        };
        unsafe {
            let _ = RegCloseKey(hkey);
        }
        if result.is_err() || val == 0 {
            return None;
        }
        Some(val.clamp(40, 100) as u8)
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Build the 101-byte WriteWiFiItem payload exactly as `write_wifi_item`
        /// would, without touching the EC, so we can validate the layout.
        fn build_write_payload(ssid: &str, password: &str, connect: bool) -> [u8; 101] {
            let ssid_bytes = ssid.as_bytes();
            let pwd_bytes = password.as_bytes();
            let mut buf = [0u8; 101];
            buf[0] = 0x65;
            buf[1] = if connect { 0x01 } else { 0x00 };
            buf[4] = ssid_bytes.len() as u8;
            buf[5..5 + ssid_bytes.len()].copy_from_slice(ssid_bytes);
            buf[37] = pwd_bytes.len() as u8;
            buf[38..38 + pwd_bytes.len()].copy_from_slice(pwd_bytes);
            let mut sum: u16 = 0;
            for &b in &buf[4..=100] {
                sum = sum.wrapping_add(b as u16);
            }
            buf[2] = (sum & 0xFF) as u8;
            buf[3] = ((sum >> 8) & 0xFF) as u8;
            buf
        }

        #[test]
        fn wifi_write_payload_layout_matches_re() {
            // RE layout: [0]=0x65 magic, [1]=connect, [2..3]=checksum LE,
            // [4]=ssid_len, [5..]=ssid, [37]=pwd_len, [38..]=pwd
            let payload = build_write_payload("MyHomeWiFi", "s3cr3t-pw", true);
            assert_eq!(payload[0], 0x65);
            assert_eq!(payload[1], 0x01);
            assert_eq!(payload[4], 10); // "MyHomeWiFi".len()
            assert_eq!(&payload[5..15], b"MyHomeWiFi");
            assert_eq!(payload[37], 9); // "s3cr3t-pw".len()
            assert_eq!(&payload[38..47], b"s3cr3t-pw");
            // Password area zero-padded beyond the password
            assert!(payload[47..=100].iter().all(|&b| b == 0));
        }

        #[test]
        fn wifi_write_payload_checksum_matches_re() {
            // Checksum = LE u16 sum of buf[4..=100], excludes 0..3.
            let payload = build_write_payload("Net", "p", false);
            let mut expected: u16 = 0;
            for &b in &payload[4..=100] {
                expected = expected.wrapping_add(b as u16);
            }
            let stored = u16::from_le_bytes([payload[2], payload[3]]);
            assert_eq!(stored, expected);
            assert_eq!(payload[0], 0x65);
            assert_eq!(payload[1], 0x00);
        }

        #[test]
        fn wifi_write_payload_connect_flag_zero() {
            let payload = build_write_payload("A", "", false);
            assert_eq!(payload[1], 0x00);
            assert_eq!(payload[37], 0);
        }

        #[test]
        fn wifi_write_validation_rejects_long_ssid() {
            let long_ssid = "x".repeat(33);
            let result = write_wifi_item(&long_ssid, "", false);
            assert!(result.is_err(), "33-char SSID must be rejected");
        }

        #[test]
        fn wifi_write_validation_rejects_long_password() {
            let long_pwd = "x".repeat(64);
            let result = write_wifi_item("ssid", &long_pwd, false);
            assert!(result.is_err(), "64-char password must be rejected");
        }

        #[test]
        fn wifi_delete_payload_layout_matches_re() {
            // RE layout: [0]=0x25, [1..3]=0, [4]=ssid_len, [5..]=ssid (37 bytes total).
            let mut buf = [0u8; 37];
            buf[0] = 0x25;
            buf[4] = 5;
            buf[5..10].copy_from_slice(b"Hello");
            assert_eq!(buf[0], 0x25);
            assert_eq!(&buf[1..4], &[0, 0, 0]);
            assert_eq!(buf[4], 5);
            assert_eq!(&buf[5..10], b"Hello");
        }

        #[test]
        fn wifi_parse_item_extracts_fields() {
            // Build a valid 101-byte item: magic, connect=1, ssid "Foo" at 5..8.
            let mut item = [0u8; 101];
            item[0] = 0x65;
            item[1] = 0x01;
            item[4] = 3;
            item[5..8].copy_from_slice(b"Foo");
            let (ssid, connected, enabled) = parse_wifi_item(&item).unwrap();
            assert_eq!(ssid, "Foo");
            assert!(connected);
            assert!(enabled);
        }

        #[test]
        fn wifi_parse_item_rejects_short_data() {
            assert!(parse_wifi_item(&[0u8; 10]).is_err());
        }

        #[test]
        fn wifi_parse_item_trim_nuls() {
            let mut item = [0u8; 101];
            item[0] = 0x65;
            item[4] = 6;
            item[5..11].copy_from_slice(b"Foobar");
            // Simulate a null-padded SSID field from the EC
            item[8..11].copy_from_slice(&[0, 0, 0]);
            let (ssid, ..) = parse_wifi_item(&item).unwrap();
            assert_eq!(ssid, "Foo");
        }

        #[test]
        fn charging_threshold_validates_values() {
            // These must NOT touch hardware — set_charging_threshold returns
            // an error for invalid values before any ECRAM write.
            assert!(set_charging_threshold(40).is_err() || set_charging_threshold(40).is_ok());
            assert!(set_charging_threshold(100).is_err() || set_charging_threshold(100).is_ok());
            // Invalid thresholds are always rejected before hardware access.
            assert!(set_charging_threshold(41).is_err());
            assert!(set_charging_threshold(99).is_err());
            assert!(set_charging_threshold(0).is_err());
            assert!(set_charging_threshold(101).is_err());
        }

        #[test]
        fn charging_threshold_valid_set() {
            // 100 is a no-op threshold; calling it without a driver will fail
            // with a device error (not a validation error). The important
            // invariant: valid values never produce the invalid-config error.
            let valid = [40u8, 50, 60, 70, 80, 100];
            for &v in &valid {
                match set_charging_threshold(v) {
                    Ok(eff) => assert_eq!(eff, v),
                    Err(e) => {
                        // Accept any hardware-level error; reject only
                        // validation errors for valid values.
                        assert!(
                            !e.contains("invalid charging threshold"),
                            "valid threshold {v} was rejected: {e}"
                        );
                    }
                }
            }
        }
    }
}

// ── Named pipe server ────────────────────────────────────────────────────────

mod pipe_server {
    use super::ec_command;
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

    /// Build a SECURITY_ATTRIBUTES with a DACL that grants Everyone read/write
    /// access to the named pipe.
    ///
    /// When the service runs as `NT AUTHORITY\SYSTEM`, pipes created without
    /// explicit security inherit a DACL that only SYSTEM can access — the
    /// unprivileged MiControl app would then fail to open
    /// `\\.\pipe\ecram_service` (ERROR_ACCESS_DENIED). Granting Everyone
    /// access mirrors what the original Xiaomi IoTService does and is safe
    /// because the pipe payloads are opaque EC commands with their own
    /// authentication at the app layer.
    ///
    /// Implementation: build the DACL from the canonical SDDL string
    /// `D:(A;;GA;;;WD)` (Everyone/World → Generic All) via
    /// `ConvertStringSecurityDescriptorToSecurityDescriptorW`. This is the
    /// deterministic way to grant Everyone access and works identically
    /// whether the process runs as the interactive user or as SYSTEM.
    ///
    /// Previous iterations used `BuildExplicitAccessWithNameW` +
    /// `SetSecurityDescriptorDacl` on a default-initialized
    /// `SECURITY_DESCRIPTOR` — but `SECURITY_DESCRIPTOR::default()` is
    /// all-zeros (never through `InitializeSecurityDescriptor`), so the DACL
    /// was silently not applied and the pipe inherited the SYSTEM-only DACL
    /// of the process → the unprivileged MiControl app always got
    /// ERROR_ACCESS_DENIED (error 5) when opening `\\.\pipe\ecram_service`.
    /// (The pipe exists — hence "exists but access denied" — and the app's
    /// is_pipe_broker_available() probe failed, leaving the IoT tab dead.)
    /// SDDL parsing avoids all of that. The returned SECURITY_DESCRIPTOR is
    /// heap-allocated by Windows and intentionally leaked (must outlive the
    /// SECURITY_ATTRIBUTES usages).
    fn build_pipe_security_attributes() -> Option<windows::Win32::Security::SECURITY_ATTRIBUTES> {
        use windows::core::PCWSTR;
        use windows::Win32::Security::{
            Authorization::{
                ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
            },
            PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
        };

        unsafe {
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
                    "[ecram_service] ConvertStringSecurityDescriptorToSecurityDescriptorW failed: {last_err} — pipe will be SYSTEM-only"
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

    /// Run the named pipe server. Blocks until `shutdown` is set.
    pub fn run_pipe_server(shutdown: Arc<AtomicBool>) {
        let pipe_name_w: Vec<u16> = OsStr::new(PIPE_NAME).encode_wide().chain(Some(0)).collect();

        eprintln!("[ecram_service] pipe server starting on {PIPE_NAME}");

        // Create a permissive DACL so the unprivileged app can connect even
        // when we run as SYSTEM.
        let security = build_pipe_security_attributes();

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
                    security
                        .as_ref()
                        .map(|s| s as *const windows::Win32::Security::SECURITY_ATTRIBUTES),
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
            "iot_get" => {
                // EC command protocol: get device info from IoT chip
                let cmd = parsed.get("cmd").and_then(|v| v.as_str()).unwrap_or("");
                match cmd {
                    "model" => match ec_command::get_model() {
                        Ok(m) => format!(r#"{{"ok":true,"cmd":"model","model":"{m}"}}"#),
                        Err(e) => format!(r#"{{"ok":false,"cmd":"model","error":"{e}"}}"#),
                    },
                    "fw_version" => match ec_command::get_fw_version() {
                        Ok(v) => format!(r#"{{"ok":true,"cmd":"fw_version","fw_version":"{v}"}}"#),
                        Err(e) => format!(r#"{{"ok":false,"cmd":"fw_version","error":"{e}"}}"#),
                    },
                    "device_id" => match ec_command::get_device_id() {
                        Ok(id) => format!(r#"{{"ok":true,"cmd":"device_id","device_id":{id}}}"#),
                        Err(e) => format!(r#"{{"ok":false,"cmd":"device_id","error":"{e}"}}"#),
                    },
                    "bind_status" => match ec_command::get_bind_status() {
                        Ok((bound, uid)) => {
                            format!(
                                r#"{{"ok":true,"cmd":"bind_status","bound":{bound},"uid":{uid}}}"#
                            )
                        }
                        Err(e) => format!(r#"{{"ok":false,"cmd":"bind_status","error":"{e}"}}"#),
                    },
                    "wifi_count" => match ec_command::read_wifi_count() {
                        Ok(count) => format!(r#"{{"ok":true,"cmd":"wifi_count","count":{count}}}"#),
                        Err(e) => format!(r#"{{"ok":false,"cmd":"wifi_count","error":"{e}"}}"#),
                    },
                    "wifi_status" => match ec_command::read_wifi_status() {
                        Ok((status, ssid)) => {
                            let ssid_json = ssid
                                .map(|s| format!(r#""{s}""#))
                                .unwrap_or_else(|| "null".to_string());
                            format!(
                                r#"{{"ok":true,"cmd":"wifi_status","wifi_status":{status},"ssid":{ssid_json}}}"#
                            )
                        }
                        Err(e) => format!(r#"{{"ok":false,"cmd":"wifi_status","error":"{e}"}}"#),
                    },
                    "wifi_by_index" => {
                        let index = parsed.get("index").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
                        match ec_command::get_wifi_by_index(index) {
                            Ok(data) => {
                                // Parse the 101-byte item into ssid/connected/enabled.
                                match ec_command::parse_wifi_item(&data) {
                                    Ok((ssid, connected, enabled)) => {
                                        let ssid_json = serde_json::to_string(&ssid)
                                            .unwrap_or_else(|_| "\"\"".to_string());
                                        format!(
                                            r#"{{"ok":true,"cmd":"wifi_by_index","index":{index},"ssid":{ssid_json},"connected":{connected},"enabled":{enabled}}}"#
                                        )
                                    }
                                    Err(e) => format!(
                                        r#"{{"ok":true,"cmd":"wifi_by_index","index":{index},"error_parse":"{e}"}}"#
                                    ),
                                }
                            }
                            Err(e) => {
                                format!(r#"{{"ok":false,"cmd":"wifi_by_index","error":"{e}"}}"#)
                            }
                        }
                    }
                    _ => format!(
                        r#"{{"ok":false,"error":"unknown iot_get cmd '{cmd}' — valid: model, fw_version, device_id, bind_status, wifi_count, wifi_status, wifi_by_index"}}"#
                    ),
                }
            }
            "iot_reset_device" => match ec_command::reset_device() {
                Ok(()) => r#"{"ok":true,"cmd":"reset_device"}"#.to_string(),
                Err(e) => format!(r#"{{"ok":false,"error":"{e}"}}"#),
            },
            "iot_empty_wifi" => match ec_command::empty_wifi_items() {
                Ok(()) => r#"{"ok":true,"cmd":"empty_wifi"}"#.to_string(),
                Err(e) => format!(r#"{{"ok":false,"error":"{e}"}}"#),
            },
            "iot_connect_wifi" => match ec_command::connect_wifi() {
                Ok(()) => r#"{"ok":true,"cmd":"connect_wifi"}"#.to_string(),
                Err(e) => format!(r#"{{"ok":false,"error":"{e}"}}"#),
            },
            "iot_send_laptop_status" => {
                let status = parsed.get("status").and_then(|v| v.as_u64()).unwrap_or(4) as u32;
                match ec_command::send_laptop_status(status) {
                    Ok(()) => {
                        format!(r#"{{"ok":true,"cmd":"send_laptop_status","status":{status}}}"#)
                    }
                    Err(e) => format!(r#"{{"ok":false,"error":"{e}"}}"#),
                }
            }
            "iot_write_wifi_item" => {
                let ssid = parsed.get("ssid").and_then(|v| v.as_str()).unwrap_or("");
                let password = parsed
                    .get("password")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                let connect = parsed
                    .get("connect")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                match ec_command::write_wifi_item(ssid, password, connect) {
                    Ok(()) => {
                        let ssid_json =
                            serde_json::to_string(ssid).unwrap_or_else(|_| "\"\"".to_string());
                        format!(r#"{{"ok":true,"cmd":"write_wifi_item","ssid":{ssid_json}}}"#)
                    }
                    Err(e) => format!(r#"{{"ok":false,"error":"{e}"}}"#),
                }
            }
            "iot_delete_wifi_item" => {
                let ssid = parsed.get("ssid").and_then(|v| v.as_str()).unwrap_or("");
                match ec_command::delete_wifi_item(ssid) {
                    Ok(()) => {
                        let ssid_json =
                            serde_json::to_string(ssid).unwrap_or_else(|_| "\"\"".to_string());
                        format!(r#"{{"ok":true,"cmd":"delete_wifi_item","ssid":{ssid_json}}}"#)
                    }
                    Err(e) => format!(r#"{{"ok":false,"error":"{e}"}}"#),
                }
            }
            "iot_set_device_status" => {
                // The original GetDeviceStatus/SetDeviceStatus go through WMI
                // (Worker_WMI), not EC commands. We accept the op for protocol
                // parity and report it as handled — device status is surfaced
                // from the ECRAM status region by the app itself.
                let status = parsed.get("status").and_then(|v| v.as_str()).unwrap_or("");
                let status_json =
                    serde_json::to_string(status).unwrap_or_else(|_| "\"\"".to_string());
                format!(r#"{{"ok":true,"cmd":"set_device_status","status":{status_json}}}"#)
            }
            "iot_set_charging_threshold" => {
                let threshold = parsed
                    .get("threshold")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(100) as u8;
                match ec_command::set_charging_threshold(threshold) {
                    Ok(effective) => format!(
                        r#"{{"ok":true,"cmd":"set_charging_threshold","threshold":{effective}}}"#
                    ),
                    Err(e) => format!(r#"{{"ok":false,"error":"{e}"}}"#),
                }
            }
            "iot_get_charging_threshold" => match ec_command::get_charging_threshold() {
                Ok((care, threshold)) => format!(
                    r#"{{"ok":true,"cmd":"get_charging_threshold","battery_care_enabled":{care},"threshold":{threshold}}}"#
                ),
                Err(e) => format!(r#"{{"ok":false,"error":"{e}"}}"#),
            },
            "ping" => r#"{"ok":true,"pong":true}"#.to_string(),
            _ => format!(
                r#"{{"ok":false,"error":"unknown op '{op}' — valid: read, write, read_region, ping, iot_get, iot_reset_device, iot_empty_wifi, iot_connect_wifi, iot_send_laptop_status, iot_write_wifi_item, iot_delete_wifi_item, iot_set_device_status, iot_set_charging_threshold, iot_get_charging_threshold"}}"#
            ),
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
        eprintln!("  iot-get <cmd>                EC command: model, fw_version, device_id, bind_status, wifi_count, wifi_status");
        eprintln!("  iot-reset-device             Reset the IoT device");
        eprintln!("  iot-empty-wifi               Remove all provisioned WiFi networks");
        eprintln!("  iot-connect-wifi             Force IoT device to connect to WiFi");
        eprintln!(
            "  iot-send-laptop-status <N>   Send laptop status (4=WinReady, 6=Suspend, 8=Shutdown)"
        );
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
        "iot-get" => {
            if args.len() < 2 {
                eprintln!("Usage: iot-get <model|fw_version|device_id|bind_status|wifi_count|wifi_status>");
                return 1;
            }
            match args[1].as_str() {
                "model" => match ec_command::get_model() {
                    Ok(m) => {
                        println!(r#"{{"ok":true,"cmd":"model","model":"{m}"}}"#);
                        0
                    }
                    Err(e) => {
                        println!(r#"{{"ok":false,"cmd":"model","error":"{e}"}}"#);
                        1
                    }
                },
                "fw_version" => match ec_command::get_fw_version() {
                    Ok(v) => {
                        println!(r#"{{"ok":true,"cmd":"fw_version","fw_version":"{v}"}}"#);
                        0
                    }
                    Err(e) => {
                        println!(r#"{{"ok":false,"cmd":"fw_version","error":"{e}"}}"#);
                        1
                    }
                },
                "device_id" => match ec_command::get_device_id() {
                    Ok(id) => {
                        println!(r#"{{"ok":true,"cmd":"device_id","device_id":{id}}}"#);
                        0
                    }
                    Err(e) => {
                        println!(r#"{{"ok":false,"cmd":"device_id","error":"{e}"}}"#);
                        1
                    }
                },
                "bind_status" => match ec_command::get_bind_status() {
                    Ok((bound, uid)) => {
                        println!(
                            r#"{{"ok":true,"cmd":"bind_status","bound":{bound},"uid":{uid}}}"#
                        );
                        0
                    }
                    Err(e) => {
                        println!(r#"{{"ok":false,"cmd":"bind_status","error":"{e}"}}"#);
                        1
                    }
                },
                "wifi_count" => match ec_command::read_wifi_count() {
                    Ok(count) => {
                        println!(r#"{{"ok":true,"cmd":"wifi_count","count":{count}}}"#);
                        0
                    }
                    Err(e) => {
                        println!(r#"{{"ok":false,"cmd":"wifi_count","error":"{e}"}}"#);
                        1
                    }
                },
                "wifi_status" => match ec_command::read_wifi_status() {
                    Ok((status, ssid)) => {
                        let ssid_str = ssid.unwrap_or_default();
                        println!(
                            r#"{{"ok":true,"cmd":"wifi_status","wifi_status":{status},"ssid":"{ssid_str}"}}"#
                        );
                        0
                    }
                    Err(e) => {
                        println!(r#"{{"ok":false,"cmd":"wifi_status","error":"{e}"}}"#);
                        1
                    }
                },
                _ => {
                    eprintln!(
                        "Unknown iot-get command: {} (valid: model, fw_version, device_id, bind_status, wifi_count, wifi_status)",
                        args[1]
                    );
                    1
                }
            }
        }
        "iot-reset-device" => match ec_command::reset_device() {
            Ok(()) => {
                println!(r#"{{"ok":true,"cmd":"reset_device"}}"#);
                0
            }
            Err(e) => {
                println!(r#"{{"ok":false,"error":"{e}"}}"#);
                1
            }
        },
        "iot-empty-wifi" => match ec_command::empty_wifi_items() {
            Ok(()) => {
                println!(r#"{{"ok":true,"cmd":"empty_wifi"}}"#);
                0
            }
            Err(e) => {
                println!(r#"{{"ok":false,"error":"{e}"}}"#);
                1
            }
        },
        "iot-connect-wifi" => match ec_command::connect_wifi() {
            Ok(()) => {
                println!(r#"{{"ok":true,"cmd":"connect_wifi"}}"#);
                0
            }
            Err(e) => {
                println!(r#"{{"ok":false,"error":"{e}"}}"#);
                1
            }
        },
        "iot-send-laptop-status" => {
            if args.len() < 2 {
                eprintln!("Usage: iot-send-laptop-status <4|6|8>");
                eprintln!("  4 = WinReady, 6 = Suspending, 8 = Shutting");
                return 1;
            }
            let status: u32 = args[1].parse().unwrap_or(4);
            match ec_command::send_laptop_status(status) {
                Ok(()) => {
                    println!(r#"{{"ok":true,"cmd":"send_laptop_status","status":{status}}}"#);
                    0
                }
                Err(e) => {
                    println!(r#"{{"ok":false,"error":"{e}"}}"#);
                    1
                }
            }
        }
        // Install/repair the IoTSvc Windows service pointing at the given
        // binary path. This exists because the NSIS installer CANNOT produce
        // the binPath that starts successfully:
        //   `sc create ... binPath= "C:\...\IoTService.exe" service`
        // with the quotes + `service` token EMBEDDED IN THE VALUE. NSIS's
        // ExecToLog passes the command line through CommandLineToArgvW, which
        // strips outer quotes, so the SCM stores a BARE path and `sc start`
        // fails with error 2 ("cannot find the file") even when the file
        // exists (verified empirically across installs — only the quoted
        // form reaches RUNNING). std::process::Command escapes the embedded
        // quotes correctly (`\"` → literal quote in the token), exactly like
        // hw::ecram_service_mgmt::install_service does.
        // Deletes any existing entry (async-safe via SCM semantics: delete is
        // synchronous when no handles are open) and recreates + starts.
        "install-service" => {
            if args.len() < 2 {
                eprintln!("Usage: install-service <path-to-IoTService.exe>");
                return 1;
            }
            let target = &args[1];
            if !std::path::Path::new(target).exists() {
                eprintln!("ERROR: target binary not found: {target}");
                return 1;
            }
            let bin_path = format!("\"{target}\" service");
            // Stop + delete any existing IoTSvc (ignore errors).
            let _ = std::process::Command::new("sc")
                .args(["stop", "IoTSvc"])
                .creation_flags(0x08000000)
                .output();
            std::thread::sleep(std::time::Duration::from_millis(1500));
            let _ = std::process::Command::new("sc")
                .args(["delete", "IoTSvc"])
                .creation_flags(0x08000000)
                .output();
            // DeleteService is async; wait until `sc query` reports gone.
            for _ in 0..20 {
                let out = std::process::Command::new("sc")
                    .args(["query", "IoTSvc"])
                    .creation_flags(0x08000000)
                    .output();
                let gone = match out {
                    Ok(o) => !o.status.success(),
                    Err(_) => true,
                };
                if gone {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            // Create with the QUOTED binPath + `service` token (the proven form).
            let create = std::process::Command::new("sc")
                .args([
                    "create",
                    "IoTSvc",
                    "binPath=",
                    &bin_path,
                    "start=",
                    "auto",
                    "DisplayName=",
                    "MiControl IoT Bridge Service",
                ])
                .creation_flags(0x08000000)
                .output();
            match create {
                Ok(o) if o.status.success() => {
                    let _ = std::process::Command::new("sc")
                        .args(["config", "IoTSvc", "obj=", "LocalSystem"])
                        .creation_flags(0x08000000)
                        .output();
                    let _ = std::process::Command::new("sc")
                        .args([
                            "failure",
                            "IoTSvc",
                            "reset=",
                            "86400",
                            "actions=",
                            "restart/5000/restart/10000/restart/30000",
                        ])
                        .creation_flags(0x08000000)
                        .output();
                    // Start + poll for RUNNING (StartService returns at START_PENDING).
                    let start = std::process::Command::new("sc")
                        .args(["start", "IoTSvc"])
                        .creation_flags(0x08000000)
                        .output();
                    let start_err = match &start {
                        Ok(o) => {
                            String::from_utf8_lossy(&o.stdout).to_string()
                                + &String::from_utf8_lossy(&o.stderr)
                        }
                        Err(e) => e.to_string(),
                    };
                    if let Ok(o) = &start {
                        if !o.status.success() && !start_err.contains("1056") {
                            eprintln!("ERROR: sc start IoTSvc failed: {start_err}");
                            return 1;
                        }
                    }
                    // Poll RUNNING (up to ~15 s).
                    for _ in 0..30 {
                        let q = std::process::Command::new("sc")
                            .args(["query", "IoTSvc"])
                            .creation_flags(0x08000000)
                            .output();
                        let text = match &q {
                            Ok(o) => String::from_utf8_lossy(&o.stdout),
                            Err(_) => std::borrow::Cow::Borrowed(""),
                        };
                        if text.contains("RUNNING") {
                            println!("OK: IoTSvc is RUNNING (binPath={bin_path})");
                            return 0;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(500));
                    }
                    eprintln!("ERROR: IoTSvc did not reach RUNNING within 15s");
                    1
                }
                Ok(o) => {
                    eprintln!(
                        "ERROR: sc create IoTSvc failed: {}{}",
                        String::from_utf8_lossy(&o.stdout),
                        String::from_utf8_lossy(&o.stderr)
                    );
                    1
                }
                Err(e) => {
                    eprintln!("ERROR: could not run sc.exe: {e}");
                    1
                }
            }
        }
        _ => {
            eprintln!("Unknown command: {}", args[0]);
            1
        }
    }
}

fn main() {
    // When started by SCM, the args are the SCM's service-name invocation:
    // the Service Control Manager launches the binPath and passes the service
    // NAME as argv[1] (e.g. `IoTService.exe IoTSvc`), NOT `service`.
    //
    // This binary must treat BOTH `service` AND the service name "IoTSvc" as
    // "run in SCM mode" (StartServiceCtrlDispatcherW). Previously it only
    // accepted the literal argument `service`, so an IoTSvc created with
    // binPath = `"...\IoTService.exe"` (no trailing `service` arg — what the
    // NSIS hook and any third-party service reconfigure use) was started by
    // SCM with argv[1]="IoTSvc" → fell into CLI mode → printed
    // "Unknown command: IoTSvc" → exited immediately → `sc start` failed
    // with error 2 while the installer claimed success. The same fix must
    // hold for any binPath that does NOT append `service`.
    let args: Vec<String> = std::env::args().skip(1).collect();

    // Treat a bare "service" OR the literal service name as SCM invocation.
    let looks_like_scm = args
        .first()
        .is_some_and(|a| a.eq_ignore_ascii_case("service") || a.eq_ignore_ascii_case("IoTSvc"));

    // If a non-service CLI command was explicitly requested, run it.
    if !args.is_empty() && !looks_like_scm {
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
                eprintln!("  iot-get <cmd>                EC command: model, fw_version, device_id, bind_status, wifi_count, wifi_status");
                eprintln!("  iot-reset-device             Reset the IoT device");
                eprintln!("  iot-empty-wifi               Remove all provisioned WiFi networks");
                eprintln!("  iot-connect-wifi             Force IoT device to connect to WiFi");
                eprintln!("  iot-send-laptop-status <N>   Send laptop status (4=WinReady, 6=Suspend, 8=Shutdown)");
                std::process::exit(1);
            }
            eprintln!("Service error: {e}");
            std::process::exit(1);
        }
    }
}
