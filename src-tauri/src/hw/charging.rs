//! Charging threshold control via IoTService IPC and registry fallback.
//!
//! Reads and sets the battery charging limit through the Xiaomi IoT
//! service named pipe, with a Windows Registry fallback path.

use crate::hw::errors::{HardwareError, HardwareResult};
use anyhow::Context;
use serde::{Deserialize, Serialize};

/// Named pipe path to the IoTService IPC broker.
const IOT_PIPE: &str = r"\\.\pipe\LOCAL\IoTService_IPC_Broker";

/// Registry fallback for charging threshold.
#[cfg(windows)]
const CHARGE_REG_KEY: &str = r"SOFTWARE\MI\IoTDriver";
#[cfg(windows)]
const CHARGE_REG_VALUE: &str = "ChargingThreshold";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ChargingResult {
    pub success: bool,
    pub method: String,
    pub threshold: u8,
}

/// Valid charging threshold levels (percent). 100 = no limit (charge to full).
const VALID_THRESHOLDS: [u8; 6] = [40, 50, 60, 70, 80, 100];

pub fn set_charging_threshold(threshold: u8) -> HardwareResult<ChargingResult> {
    if !VALID_THRESHOLDS.contains(&threshold) {
        return Err(HardwareError::InvalidConfig(format!(
            "Invalid threshold {threshold}. Must be one of: 40,50,60,70,80,100"
        )));
    }

    // Try named pipe first
    match send_via_pipe(threshold) {
        Ok(()) => {
            persist_threshold_registry(threshold).ok();
            return Ok(ChargingResult {
                success: true,
                method: "iot_pipe".to_string(),
                threshold,
            });
        }
        Err(e) => log::warn!("IoT pipe send failed: {e}, falling back to registry"),
    }

    // Registry fallback
    persist_threshold_registry(threshold).context("Registry fallback for charging threshold")?;
    Ok(ChargingResult {
        success: true,
        method: "registry".to_string(),
        threshold,
    })
}

pub fn get_charging_threshold() -> HardwareResult<u8> {
    #[cfg(windows)]
    {
        read_threshold_registry().unwrap_or(Ok(80))
    }
    #[cfg(not(windows))]
    {
        Ok(80)
    }
}

// ── Private helpers ──────────────────────────────────────────────────────────

/// IPC message layout discovered from IoTService.exe binary analysis (v25.0.0.9).
///
/// The IoTService validates the first 4 bytes against the magic "MCPI"
/// (0x4950434D in little-endian) and rejects messages without it.
///
/// Header layout (16 bytes, from Ghidra decompilation of FUN_140043ac0):
///   - magic: u32 (offset 0)  — MCPI magic (0x4950434D)
///   - src_id: u16 (offset 4)
///   - dst_id: u16 (offset 6)
///   - type_lo: u16 (offset 8)  — low 16 bits of msg_type
///   - routing: i16 (offset 10) — 0=normal unicast
///   - field: u16 (offset 12)  — sub-type, 0 for normal
///   - payload_len: u16 (offset 14) — total message size (header + payload)
///
/// src_id=1 (MiControl), dst_id=2 (IoTDriver), type_lo=0x1003 (set charging limit)
///
/// # Safety
///
/// This struct maps to the binary wire format used for IoTService IPC.
/// The fields are naturally aligned with no padding.
#[repr(C)]
struct IotIpcMsg {
    magic: u32,
    src_id: u16,
    dst_id: u16,
    type_lo: u16,
    routing: i16,
    field: u16,
    payload_len: u16,
}

/// MCPI magic value: bytes `4D 43 50 49` = "MCPI" in ASCII.
const MCPI_MAGIC: u32 = 0x4950434D;

/// Compile-time assertion: IotIpcMsg must be exactly 16 bytes.
const _: () = assert!(std::mem::size_of::<IotIpcMsg>() == 16);

/// Send a charging threshold command to the IoTService IPC pipe.
///
/// This is intentionally fire-and-forget: the IoTService pipe protocol
/// does not return a response for charging threshold commands (msg_type
/// `0x1003`). The command is validated before sending, and the registry
/// is updated separately. If the pipe send fails, the registry value
/// still reflects the user's intent and will be applied on the next
/// IoTService restart.
fn send_via_pipe(threshold: u8) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        use std::io::Write;
        use std::os::windows::ffi::OsStrExt;
        use std::os::windows::io::FromRawHandle;
        use windows::Win32::Foundation::{
            GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
        };
        use windows::Win32::Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
        };
        use windows::Win32::System::Pipes::{SetNamedPipeHandleState, PIPE_READMODE_MESSAGE};

        // Use the IoT pipe path discovered at startup; fall back to the default constant.
        let pipe_path = crate::hw::discovery::global_profile()
            .and_then(|p| p.iot_pipe_path)
            .unwrap_or_else(|| IOT_PIPE.to_string());

        // Open the pipe using CreateFileW so we can set MESSAGE mode.
        let path_w: Vec<u16> = std::ffi::OsStr::new(&pipe_path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        // SAFETY: CreateFileW with a valid null-terminated wide string path.
        let raw_handle = unsafe {
            CreateFileW(
                windows::core::PCWSTR(path_w.as_ptr()),
                (GENERIC_READ | GENERIC_WRITE).0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                HANDLE::default(),
            )
        };

        let raw_handle = raw_handle.context("Open IoT IPC pipe")?;

        if raw_handle == INVALID_HANDLE_VALUE {
            return Err(anyhow::anyhow!(
                "Open IoT IPC pipe returned INVALID_HANDLE_VALUE: {}",
                pipe_path
            )
            .into());
        }

        // Set pipe to MESSAGE read mode (required by IoTService pipe server)
        let mode = PIPE_READMODE_MESSAGE;
        // SAFETY: raw_handle is a valid pipe handle from CreateFileW.
        let _ = unsafe { SetNamedPipeHandleState(raw_handle, Some(&mode as *const _), None, None) };

        // Wrap the raw handle in a std::fs::File for Write trait
        // SAFETY: raw_handle is a valid file handle owned by us; std::fs::File will close it on drop.
        let mut pipe = unsafe {
            std::fs::File::from_raw_handle(raw_handle.0 as std::os::windows::io::RawHandle)
        };

        // Build the 16-byte MCPI header with 1-byte payload (threshold)
        // payload_len = header(16) + payload(1) = 17
        let msg = IotIpcMsg {
            magic: MCPI_MAGIC,
            src_id: 1,
            dst_id: 2,
            type_lo: 0x1003,
            routing: 0,
            field: 0,
            payload_len: 17,
        };

        let bytes: &[u8] = unsafe {
            // SAFETY: IotIpcMsg is #[repr(C)] with no padding and known size (16 bytes);
            // casting its address to a byte slice yields a valid, sized buffer for write_all.
            std::slice::from_raw_parts(
                &msg as *const IotIpcMsg as *const u8,
                std::mem::size_of::<IotIpcMsg>(),
            )
        };
        pipe.write_all(bytes)
            .context("Write MCPI header to IoT pipe")?;
        // Write the 1-byte payload (threshold value)
        pipe.write_all(&[threshold])
            .context("Write threshold payload to IoT pipe")?;

        // Do NOT block on a read here — IoTService does not send an
        // acknowledgment for 0x1003 (set-charging-limit) messages.
        // This is a fire-and-forget command per the IoT protocol:
        // the service accepts the threshold and applies it internally
        // without returning a response payload. A blocking pipe.read()
        // would hang the elevated helper indefinitely.
    }
    #[cfg(not(windows))]
    {
        let _ = threshold;
    }
    Ok(())
}

fn persist_threshold_registry(threshold: u8) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
        // S25-006: Use RegKeyGuard instead of raw RegOpenKeyExW/RegCloseKey.
        // create_write opens with KEY_ALL_ACCESS which includes KEY_WRITE.
        match RegKeyGuard::create_write(HKEY_LOCAL_MACHINE, CHARGE_REG_KEY) {
            Ok(key) => {
                key.write_u32(CHARGE_REG_VALUE, threshold as u32)
                    .map_err(|e| {
                        HardwareError::Registry(format!(
                            "Write charging threshold to registry: {e}"
                        ))
                    })?;
            }
            Err(e) => {
                // Key doesn't exist or can't be opened — IoT driver may be absent
                log::debug!("Cannot open charging registry key for write: {e}");
                return Ok(());
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn read_threshold_registry() -> Option<HardwareResult<u8>> {
    use crate::util::registry::RegKeyGuard;
    use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
    // S25-006: Use RegKeyGuard instead of raw RegOpenKeyExW/RegCloseKey.
    let key = RegKeyGuard::open_read(HKEY_LOCAL_MACHINE, CHARGE_REG_KEY).ok()??;
    match key.read_u32(CHARGE_REG_VALUE) {
        Ok(Some(data)) => Some(Ok(data.clamp(40, 100) as u8)),
        Ok(None) => None,
        Err(e) => Some(Err(HardwareError::Registry(format!(
            "Read charging threshold: {e}"
        )))),
    }
}
