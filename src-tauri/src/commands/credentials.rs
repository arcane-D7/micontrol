use keyring::Entry;

const SERVICE_NAME: &str = "com.mipc.micontrol";

#[tauri::command]
pub fn set_secret(key: String, value: String) -> Result<(), String> {
    let entry = Entry::new(SERVICE_NAME, &key).map_err(|e| e.to_string())?;
    entry.set_password(&value).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_secret(key: String) -> Result<Option<String>, String> {
    let entry = Entry::new(SERVICE_NAME, &key).map_err(|e| e.to_string())?;
    match entry.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub fn delete_secret(key: String) -> Result<(), String> {
    let entry = Entry::new(SERVICE_NAME, &key).map_err(|e| e.to_string())?;
    match entry.delete_credential() {
        Ok(_) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}
