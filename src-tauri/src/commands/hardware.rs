use crate::elev_bridge;
use crate::hw::charging::{get_charging_threshold as hw_get_charge, ChargingResult};
use crate::hw::performance::{
    get_perf_debug as hw_perf_debug, get_performance_mode as hw_get_perf, PerfDebugInfo,
    PerformanceResult,
};
use crate::state::{AppState, PerformanceMode};
use tauri::State;

const RAW_ECRAM_WRITE_ENABLE_ENV: &str = "MICONTROL_ENABLE_RAW_ECRAM_WRITE";
const RAW_ECRAM_WRITE_MAX_BYTES: usize = 32;

#[tauri::command]
pub async fn get_performance_mode(_state: State<'_, AppState>) -> Result<PerformanceMode, String> {
    hw_get_perf().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_performance_mode(
    mode: PerformanceMode,
    state: State<'_, AppState>,
) -> Result<PerformanceResult, String> {
    let raw =
        elev_bridge::run_elevated("set_performance_mode", serde_json::json!({ "mode": mode }))
            .await?;
    let result: PerformanceResult =
        serde_json::from_value(raw).map_err(|e| format!("Unexpected elevated result: {e}"))?;
    *state.performance_mode.lock().unwrap() = result.mode;
    Ok(result)
}

#[tauri::command]
pub async fn get_charging_threshold(_state: State<'_, AppState>) -> Result<u8, String> {
    hw_get_charge().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_charging_threshold(
    threshold: u8,
    state: State<'_, AppState>,
) -> Result<ChargingResult, String> {
    let raw = elev_bridge::run_elevated(
        "set_charging_threshold",
        serde_json::json!({ "threshold": threshold }),
    )
    .await?;
    let result: ChargingResult =
        serde_json::from_value(raw).map_err(|e| format!("Unexpected elevated result: {e}"))?;
    *state.charging_threshold.lock().unwrap() = result.threshold;
    Ok(result)
}

/// Returns diagnostic information about the performance mode control channel:
/// - which WMI instance was found
/// - whether a live SetPerformanceMode call succeeds
/// - current registry and overlay mode
/// - VHF device path if discovered
/// This runs in the main (non-elevated) process since it's read-only.
#[tauri::command]
pub async fn get_perf_debug() -> Result<PerfDebugInfo, String> {
    Ok(hw_perf_debug())
}

/// Read all ACPI ERAM fields via IoTDriver (direct or shim path).
///
/// On first call the shim (`ecram_shim.exe`) is deployed to the IoTDriver
/// DriverStore directory using SeRestorePrivilege.  Subsequent calls skip
/// deployment if the binary is already current.
///
/// Returns the decoded `EramMap` with all known register fields.
#[tauri::command]
pub async fn get_ecram_map() -> Result<crate::hw::ecram::EramMap, String> {
    tokio::task::spawn_blocking(crate::hw::ecram::read_eram_map)
        .await
        .map_err(|e| format!("blocking task panicked: {e}"))?
        .map_err(|e| e.to_string())
}

/// Read a named IoT region through the DriverStore shim and return it as hex.
///
/// Supported values: `ERAM`, `SMA2`, `IOT_STATUS`, `IOT_SENSORS`.
#[tauri::command]
pub async fn get_iot_region_hex(region: String) -> Result<String, String> {
    tokio::task::spawn_blocking(move || crate::hw::ecram::read_named_region_via_shim(&region))
        .await
        .map_err(|e| format!("blocking task panicked: {e}"))?
        .map(|bytes| bytes.iter().map(|b| format!("{b:02x}")).collect())
        .map_err(|e| e.to_string())
}

/// Write raw hex bytes into EC RAM through the DriverStore shim.
#[tauri::command]
pub async fn write_iot_hex(address: String, hex_data: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let addr = u64::from_str_radix(address.trim_start_matches("0x"), 16)
            .map_err(|e| anyhow::anyhow!("invalid address: {e}"))?;

        let normalized: String = hex_data
            .chars()
            .filter(|c| !c.is_ascii_whitespace() && *c != ',' && *c != '-')
            .collect();

        anyhow::ensure!(
            !normalized.is_empty() && normalized.len() % 2 == 0,
            "hex_data must contain an even number of hex digits"
        );

        let bytes = (0..normalized.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&normalized[i..i + 2], 16).map_err(Into::into))
            .collect::<anyhow::Result<Vec<u8>>>()?;

        let is_known_safe = is_known_safe_single_byte_write(addr, bytes.as_slice());
        if !is_known_safe {
            anyhow::ensure!(
                raw_ecram_write_enabled(),
                "Raw ECRAM writes are disabled. Set {}=1 to enable advanced writes.",
                RAW_ECRAM_WRITE_ENABLE_ENV
            );
            anyhow::ensure!(
                bytes.len() <= RAW_ECRAM_WRITE_MAX_BYTES,
                "Raw write too large: {} bytes (max {})",
                bytes.len(),
                RAW_ECRAM_WRITE_MAX_BYTES
            );
            anyhow::ensure!(
                is_eram_range(addr, bytes.len()),
                "Raw write denied: address range must stay inside ERAM (0x{:#X}..0x{:#X})",
                crate::hw::ecram::ERAM_BASE,
                crate::hw::ecram::ERAM_BASE + crate::hw::ecram::ERAM_SIZE as u64
            );
        }

        crate::hw::ecram::write_ecram_via_shim(addr, &bytes)
    })
    .await
    .map_err(|e| format!("blocking task panicked: {e}"))?
    .map_err(|e| e.to_string())
}

/// Read `count` bytes (1–256) from ECRAM at `address` via the DriverStore shim.
///
/// Returns the bytes as a lowercase hex string.  Requires the process to be
/// running elevated (administrator).
#[tauri::command]
pub async fn read_ecram_raw(address: String, count: u32) -> Result<String, String> {
    tokio::task::spawn_blocking(move || {
        let addr = u64::from_str_radix(address.trim_start_matches("0x"), 16)
            .map_err(|e| anyhow::anyhow!("invalid address: {e}"))?;

        anyhow::ensure!(count >= 1 && count <= 256, "count must be 1–256");

        let bytes = crate::hw::ecram::read_ecram_via_shim(addr, count as usize)?;
        Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
    })
    .await
    .map_err(|e| format!("blocking task panicked: {e}"))?
    .map_err(|e: anyhow::Error| e.to_string())
}

/// Returns whether the current process is running with an elevated (Administrator) token.
#[tauri::command]
pub fn is_elevated() -> bool {
    crate::hw::ecram::is_process_elevated()
}

/// Re-launch the application as administrator (UAC prompt) and exit the current instance.
///
/// This triggers the standard Windows UAC prompt.  If the user approves, a new
/// elevated instance of the app starts and this instance exits.
#[tauri::command]
pub async fn relaunch_as_admin(app: tauri::AppHandle) -> Result<(), String> {
    #[cfg(windows)]
    {
        crate::elev_bridge::relaunch_self_as_admin()?;
        app.exit(0);
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        return Err("re-launch as admin is only supported on Windows".to_string());
    }
    #[allow(unreachable_code)]
    Ok(())
}

fn raw_ecram_write_enabled() -> bool {
    std::env::var(RAW_ECRAM_WRITE_ENABLE_ENV)
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            v == "1" || v == "true" || v == "yes" || v == "on"
        })
        .unwrap_or(false)
}

fn is_eram_range(addr: u64, len: usize) -> bool {
    if len == 0 {
        return false;
    }
    let start = crate::hw::ecram::ERAM_BASE;
    let end = start + crate::hw::ecram::ERAM_SIZE as u64;
    let write_end = addr.saturating_add(len as u64);
    addr >= start && write_end <= end
}

fn is_known_safe_single_byte_write(addr: u64, data: &[u8]) -> bool {
    if data.len() != 1 || !is_eram_range(addr, 1) {
        return false;
    }
    let offset = (addr - crate::hw::ecram::ERAM_BASE) as usize;
    matches!(
        offset,
        0x1B | 0x40 | 0x42 | 0x4A | 0x4B | 0x68 | 0x96 | 0xAE | 0xB2
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_safe_offsets_are_allowed() {
        let addr = crate::hw::ecram::ERAM_BASE + 0x96;
        assert!(is_known_safe_single_byte_write(addr, &[80]));
    }

    #[test]
    fn non_whitelisted_offset_is_not_known_safe() {
        let addr = crate::hw::ecram::ERAM_BASE + 0x10;
        assert!(!is_known_safe_single_byte_write(addr, &[1]));
    }

    #[test]
    fn eram_range_check_rejects_outside() {
        let addr = crate::hw::ecram::ERAM_BASE + crate::hw::ecram::ERAM_SIZE as u64 + 1;
        assert!(!is_eram_range(addr, 1));
    }
}
