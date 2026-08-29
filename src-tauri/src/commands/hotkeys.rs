//! Tauri commands for keyboard hotkey configuration.
//!
//! Exposes hotkey config get/set and key detection to the frontend.

use crate::hw::hotkeys::{
    read_in_memory, run_copilot_apply_if_needed, save_config, update_in_memory, HotkeyMap,
};

#[tauri::command]
pub async fn get_hotkey_config() -> Result<HotkeyMap, String> {
    Ok(read_in_memory())
}

/// Persist the hotkey config AND apply hardware-level changes.
///
/// This saves the config file and updates the in-memory map, then applies the
/// Copilot-key fix (Scancode Map + Windows interception policies) through the
/// elevated bridge. The apply is hardware/OS-level work that can take a few
/// seconds over the bridge, so this command returns only after it completes;
/// the frontend shows a loading state until then.
///
/// Returns a descriptor of what was applied so the UI can decide whether a
/// reboot hint is warranted.
#[tauri::command]
pub async fn set_hotkey_config(config: HotkeyMap) -> Result<HotkeyApplyResult, String> {
    // S27-005: Wrap in run_blocking — save_config() does sync filesystem I/O.
    let config_for_save = config.clone();
    crate::util::blocking::run_blocking(move || save_config(&config_for_save))
        .await
        .map_err(|e| e.to_string())?;
    update_in_memory(config);

    let applied = run_copilot_apply_if_needed().await;
    Ok(applied.into())
}

/// Poll the state of the last Copilot-key hardware apply.
///
/// Useful when a previous `set_hotkey_config` returned `reboot_required` (the
/// Scancode Map only takes effect on reboot) and the UI wants to reflect the
/// current apply state without re-running the elevated apply.
#[tauri::command]
pub async fn get_remap_apply_state() -> Result<HotkeyApplyResult, String> {
    Ok(crate::hw::hotkeys::copilot_apply_state().into())
}

/// Serialized description of what the Copilot apply did.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct HotkeyApplyResult {
    /// Whether an apply actually ran (write to Scancode Map + policies).
    pub applied: bool,
    /// Whether the change only takes effect after a reboot.
    pub reboot_required: bool,
    /// Human-readable note (may be empty).
    pub note: String,
}

impl From<crate::hw::hotkeys::CopilotApplyOutcome> for HotkeyApplyResult {
    fn from(o: crate::hw::hotkeys::CopilotApplyOutcome) -> Self {
        Self {
            applied: o.applied,
            reboot_required: o.reboot_required,
            note: o.note,
        }
    }
}

/// Start a 10-second window where the hook captures and logs all key presses.
/// Call `get_detected_key` to poll for the result.
#[tauri::command]
pub async fn start_key_detect() {
    crate::hw::hotkeys::start_detect_mode();
}

/// Return the VK code of the last key captured in detect mode, or 0 if none yet.
#[tauri::command]
pub async fn get_detected_key() -> u32 {
    crate::hw::hotkeys::get_detected_vk()
}

/// Return whether the WH_KEYBOARD_LL hook is currently installed.
#[tauri::command]
pub async fn is_hook_active() -> bool {
    crate::hw::hotkeys::is_hook_active()
}

/// Grant "Always Allow" consent for a script hotkey action (S29-001).
///
/// Called by the frontend when the user clicks "Always Allow" in the
/// consent dialog. This writes `true` into `hotkey_consent.json` for the
/// given script hash, allowing future executions without re-prompting.
#[tauri::command]
pub async fn grant_script_consent(
    interpreter: String,
    path: String,
    args: Vec<String>,
) -> Result<(), String> {
    crate::util::blocking::run_blocking(move || {
        crate::hw::hotkeys::grant_consent(&interpreter, &path, &args).map_err(|e| {
            crate::hw::errors::HardwareError::Other(format!("Failed to grant script consent: {e}"))
        })
    })
    .await
    .map_err(|e| e.to_string())
}
