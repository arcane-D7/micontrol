//! Minimal reader for the encrypted Face Unlock settings store.

use serde::Deserialize;
use windows::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};

const DATA_PATH: &str = r"C:\ProgramData\MiControl\face\faces.dat";
const STORE_MAGIC: &[u8] = b"MICONTROL_FACE1\n";
const DPAPI_ENTROPY: &[u8] = b"micontrol_face_v1";

#[derive(Debug, Deserialize)]
#[serde(default)]
struct StoredSettings {
    face_unlock_enabled: bool,
    face_unlock_logon_enabled: bool,
    face_unlock_workstation_enabled: bool,
}

impl Default for StoredSettings {
    fn default() -> Self {
        Self {
            face_unlock_enabled: true,
            face_unlock_logon_enabled: true,
            face_unlock_workstation_enabled: true,
        }
    }
}

/// Return whether the provider should expose a tile for a usage scenario.
/// Missing or invalid state fails closed so an old registration cannot leave a
/// tile visible after the Face Unlock data was removed.
pub fn enabled_for_scenario(
    scenario: windows::Win32::UI::Shell::CREDENTIAL_PROVIDER_USAGE_SCENARIO,
) -> bool {
    let Ok(settings) = load_settings() else {
        return false;
    };
    if !settings.face_unlock_enabled {
        return false;
    }

    use windows::Win32::UI::Shell::{CPUS_LOGON, CPUS_UNLOCK_WORKSTATION};
    match scenario {
        CPUS_LOGON => settings.face_unlock_logon_enabled,
        CPUS_UNLOCK_WORKSTATION => settings.face_unlock_workstation_enabled,
        _ => false,
    }
}

fn load_settings() -> Result<StoredSettings, String> {
    let bytes = std::fs::read(DATA_PATH).map_err(|e| format!("read settings: {e}"))?;
    if bytes.len() <= STORE_MAGIC.len() || !bytes.starts_with(STORE_MAGIC) {
        return Err("invalid face store".into());
    }

    let encrypted = &bytes[STORE_MAGIC.len()..];
    let input = CRYPT_INTEGER_BLOB {
        cbData: encrypted.len() as u32,
        pbData: encrypted.as_ptr() as *mut u8,
    };
    let entropy = CRYPT_INTEGER_BLOB {
        cbData: DPAPI_ENTROPY.len() as u32,
        pbData: DPAPI_ENTROPY.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    unsafe {
        CryptUnprotectData(&input, None, Some(&entropy), None, None, 0, &mut output)
            .map_err(|e| format!("decrypt settings: {e}"))?;
    }
    if output.pbData.is_null() || output.cbData == 0 {
        return Err("empty decrypted settings".into());
    }

    let plaintext = unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) };
    let store: serde_json::Value =
        serde_json::from_slice(plaintext).map_err(|e| format!("parse face store: {e}"))?;
    let settings = store
        .get("settings")
        .cloned()
        .ok_or_else(|| "face settings missing".to_string())?;
    serde_json::from_value(settings).map_err(|e| format!("parse face settings: {e}"))
}
