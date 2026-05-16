use crate::hw::hotkeys::{save_config, update_in_memory, HotkeyMap};

#[tauri::command]
pub async fn get_hotkey_config() -> Result<HotkeyMap, String> {
    Ok(crate::hw::hotkeys::read_in_memory())
}

#[tauri::command]
pub async fn set_hotkey_config(config: HotkeyMap) -> Result<(), String> {
    save_config(&config).map_err(|e| e.to_string())?;
    update_in_memory(config);
    Ok(())
}
