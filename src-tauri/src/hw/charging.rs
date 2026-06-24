use anyhow::{Context, Result};
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

pub fn set_charging_threshold(threshold: u8) -> Result<ChargingResult> {
    if !VALID_THRESHOLDS.contains(&threshold) {
        anyhow::bail!("Invalid threshold {threshold}. Must be one of: 40,50,60,70,80,100");
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

pub fn get_charging_threshold() -> Result<u8> {
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
/// src_id=1 (MiControl), dst_id=2 (IoTDriver), msg_type=0x1003 (set charging limit)
#[repr(C, packed)]
struct IotIpcMsg {
    src_id: u16,
    dst_id: u16,
    msg_type: u32,
    payload_len: u32,
    payload: [u8; 4],
}

fn send_via_pipe(threshold: u8) -> Result<()> {
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
            // SAFETY: IotIpcMsg is #[repr(C, packed)] with known size; casting its address to a
            // byte slice yields a valid, sized buffer for write_all to the pipe.
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

fn persist_threshold_registry(threshold: u8) -> Result<()> {
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegOpenKeyExW, RegSetValueExW, HKEY_LOCAL_MACHINE, KEY_WRITE, REG_DWORD,
        };

        unsafe {
            // SAFETY: The wide strings are null-terminated; hkey is initialized only after
            // RegOpenKeyExW returns OK. The pointers reference valid stack-local data with
            // correct alignment for REG_DWORD.
            let key_w: Vec<u16> = OsStr::new(CHARGE_REG_KEY)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let mut hkey = std::mem::MaybeUninit::uninit();
            // If key doesn't exist, create it
            let res = RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(key_w.as_ptr()),
                0,
                KEY_WRITE,
                hkey.as_mut_ptr(),
            );
            if res.is_err() {
                return Ok(()); // Registry key doesn't exist — IoT driver may be absent
            }
            let hkey = hkey.assume_init();
            let val_w: Vec<u16> = OsStr::new(CHARGE_REG_VALUE)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let val = threshold as u32;
            RegSetValueExW(
                hkey,
                PCWSTR(val_w.as_ptr()),
                0,
                REG_DWORD,
                Some(&val.to_le_bytes()),
            )
            .ok()
            .context("Write charging threshold to registry")?;
            let _ = RegCloseKey(hkey).ok();
        }
    }
    Ok(())
}

#[cfg(windows)]
fn read_threshold_registry() -> Option<Result<u8>> {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_LOCAL_MACHINE, REG_VALUE_TYPE,
    };

    unsafe {
        // SAFETY: Same pattern as persist_threshold_registry — wide strings are null-terminated,
        // hkey is only assume_init'd after RegOpenKeyExW succeeds, and the cast pointer to the
        // 4-byte stack buffer is valid and aligned for RegQueryValueExW.
        let key_w: Vec<u16> = OsStr::new(CHARGE_REG_KEY)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let mut hkey = std::mem::MaybeUninit::uninit();
        let res = RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(key_w.as_ptr()),
            0,
            windows::Win32::System::Registry::KEY_READ,
            hkey.as_mut_ptr(),
        );
        if res.is_err() {
            return None;
        }
        let hkey = hkey.assume_init();
        let val_w: Vec<u16> = OsStr::new(CHARGE_REG_VALUE)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let mut data: u32 = 0;
        let mut data_size = 4u32;
        let mut ty = REG_VALUE_TYPE::default();
        let res = RegQueryValueExW(
            hkey,
            PCWSTR(val_w.as_ptr()),
            None,
            Some(&mut ty),
            Some((&mut data as *mut u32).cast()),
            Some(&mut data_size),
        );
        let _ = RegCloseKey(hkey).ok();
        if res.is_err() {
            return None;
        }
        Some(Ok(data.clamp(40, 100) as u8))
    }
}
