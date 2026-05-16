use tauri::State;
use crate::state::{AppState, PerformanceMode};
use crate::hw::performance::{get_performance_mode as hw_get_perf, PerformanceResult};
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
