//! Tauri commands for secure credential storage.
//!
//! Uses the OS keyring (via `keyring` crate) to store and retrieve
//! secrets like API keys, never exposing them to the frontend.

use keyring::Entry;

const SERVICE_NAME: &str = "com.mipc.micontrol";

#[tauri::command]
pub fn set_secret(key: String, value: String) -> Result<(), String> {
    let entry = Entry::new(SERVICE_NAME, &key).map_err(|e| e.to_string())?;
    entry.set_password(&value).map_err(|e| e.to_string())?;

    // Audit log for telemetry consent grant/revoke
    if key == "telemetry_consent" && (value == "granted" || value.contains("\"granted\"")) {
        crate::util::consent_audit::log_consent_granted(crate::util::consent_audit::POLICY_VERSION);
    }

    Ok(())
}

/// S27-004: Allowlist of keyring keys that the frontend may read.
const ALLOWED_SECRET_KEYS: &[&str] = &["openai_api_key", "telemetry_consent"];

#[tauri::command]
pub fn get_secret(key: String) -> Result<Option<String>, String> {
    // S27-004: Reject keys not in the allowlist to prevent secret exfiltration.
    if !ALLOWED_SECRET_KEYS.contains(&key.as_str()) {
        return Err(format!(
            "Access denied: key '{key}' is not in the allowlist"
        ));
    }
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
        Ok(_) => {
            // Audit log for telemetry consent revocation
            if key == "telemetry_consent" {
                crate::util::consent_audit::log_consent_revoked(
                    crate::util::consent_audit::POLICY_VERSION,
                );
            }
            Ok(())
        }
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}
