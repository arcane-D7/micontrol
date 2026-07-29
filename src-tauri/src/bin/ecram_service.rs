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
                                let hex: String = data.iter().map(|b| format!("{b:02X}")).collect();
                                format!(
                                    r#"{{"ok":true,"cmd":"wifi_by_index","index":{index},"data":"{hex}"}}"#
                                )
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
            "ping" => r#"{"ok":true,"pong":true}"#.to_string(),
            _ => format!(
                r#"{{"ok":false,"error":"unknown op '{op}' — valid: read, write, read_region, ping, iot_get, iot_reset_device, iot_empty_wifi, iot_connect_wifi, iot_send_laptop_status"}}"#
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
