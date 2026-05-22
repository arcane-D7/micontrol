use tauri::State;
use crate::state::{AppState, PerformanceMode};
use crate::hw::performance::{get_performance_mode as hw_get_perf, PerformanceResult, PerfDebugInfo, get_perf_debug as hw_perf_debug};
use crate::hw::charging::{get_charging_threshold as hw_get_charge, ChargingResult};
use crate::elev_bridge;

#[tauri::command]
pub async fn get_performance_mode(_state: State<'_, AppState>) -> Result<PerformanceMode, String> {
    hw_get_perf().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn set_performance_mode(
    mode: PerformanceMode,
    state: State<'_, AppState>,
) -> Result<PerformanceResult, String> {
    let raw = elev_bridge::run_elevated(
        "set_performance_mode",
        serde_json::json!({ "mode": mode }),
    )
    .await?;
    let result: PerformanceResult = serde_json::from_value(raw)
        .map_err(|e| format!("Unexpected elevated result: {e}"))?;
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
    let result: ChargingResult = serde_json::from_value(raw)
        .map_err(|e| format!("Unexpected elevated result: {e}"))?;
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
