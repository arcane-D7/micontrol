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

/// IPC message layout discovered from IoTService.exe binary analysis.
///
/// src_id=1 (MiControl), dst_id=2 (IoTDriver), msg_type=0x1003 (set charging limit)
///
/// # Safety
///
/// This struct maps to the binary wire format used for IoTService IPC.
/// The fields are already naturally aligned (two u16, two u32, [u8; 4])
/// so `#[repr(C)]` is sufficient — no `packed` needed. All accesses via
/// byte-slice casts are safe because the struct has no padding.
#[repr(C)]
struct IotIpcMsg {
    src_id: u16,
    dst_id: u16,
    msg_type: u32,
    payload_len: u32,
    payload: [u8; 4],
}

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
        use std::fs::OpenOptions;
        use std::io::Write;

        // Use the IoT pipe path discovered at startup; fall back to the default constant.
        let pipe_path = crate::hw::discovery::global_profile()
            .and_then(|p| p.iot_pipe_path)
            .unwrap_or_else(|| IOT_PIPE.to_string());

        let mut pipe = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&pipe_path)
            .context("Open IoT IPC pipe")?;

        let msg = IotIpcMsg {
            src_id: 1,
            dst_id: 2,
            msg_type: 0x1003,
            payload_len: 1,
            payload: [threshold, 0, 0, 0],
        };

        let bytes: &[u8] = unsafe {
            // SAFETY: IotIpcMsg is #[repr(C)] with no padding and known size (16 bytes);
            // casting its address to a byte slice yields a valid, sized buffer for write_all.
            std::slice::from_raw_parts(
                &msg as *const IotIpcMsg as *const u8,
                std::mem::size_of::<IotIpcMsg>(),
            )
        };
        pipe.write_all(bytes).context("Write to IoT pipe")?;

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
