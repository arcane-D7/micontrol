//! Tauri commands for keyboard hotkey configuration.
//!
//! Exposes hotkey config get/set and key detection to the frontend.

use crate::hw::hotkeys::{save_config, update_in_memory, HotkeyMap};

#[tauri::command]
pub async fn get_hotkey_config() -> Result<HotkeyMap, String> {
    Ok(crate::hw::hotkeys::read_in_memory())
}

#[tauri::command]
pub async fn set_hotkey_config(config: HotkeyMap) -> Result<(), String> {
    // S27-005: Wrap in run_blocking — save_config() does sync filesystem I/O.
    let config_for_save = config.clone();
    crate::util::blocking::run_blocking(move || save_config(&config_for_save))
        .await
        .map_err(|e| e.to_string())?;
    update_in_memory(config);
    Ok(())
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
