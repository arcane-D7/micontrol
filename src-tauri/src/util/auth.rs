//! HMAC-SHA256 authentication for the elevated bridge protocol.
//!
//! Both the main process and the elevated helper share a secret key stored in
//! `%LOCALAPPDATA%\MiControl\elev_key.bin`.  Every command and response message
//! includes an `hmac` field computed over the JSON body (excluding the `hmac`
//! field itself).  The elevated helper rejects any command whose HMAC does not
//! verify, preventing an attacker from injecting commands via file swapping.

use fs2::FileExt;
use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;
use std::os::windows::ffi::OsStrExt;
use std::path::PathBuf;

type HmacSha256 = Hmac<Sha256>;

/// Maximum age of a command in milliseconds (30 seconds).
pub const MAX_COMMAND_AGE_MS: u64 = 30_000;

/// Returns the path to the shared HMAC key file.
/// `%LOCALAPPDATA%\MiControl\elev_key.bin`
fn key_path() -> PathBuf {
    crate::elevated::elev_dir().join("elev_key.bin")
}

/// Get the shared HMAC key, creating it if it does not exist.
///
/// The key is 32 random bytes generated on first call and persisted to disk.
/// Both the main process and the elevated helper call this to obtain the key.
pub fn get_or_create_key() -> Result<Vec<u8>, String> {
    let path = key_path();

    // Ensure the parent directory exists
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot create elev_key directory: {e}"))?;
    }

    // Open or create the key file, then acquire an exclusive lock.
    // This prevents the main process and elevated helper from generating
    // different keys simultaneously on first startup.
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| format!("Cannot open HMAC key file: {e}"))?;

    // Acquire exclusive lock with retry (up to 5 seconds)
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match file.try_lock_exclusive() {
            Ok(()) => break,
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if std::time::Instant::now() > deadline {
                    return Err("Timeout acquiring HMAC key file lock (5s)".to_string());
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => {
                return Err(format!("Cannot lock HMAC key file: {e}"));
            }
        }
    }

    // Read existing key if present and valid
    use std::io::Read;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)
        .map_err(|e| format!("Cannot read HMAC key file: {e}"))?;

    if buf.len() == 32 {
        // Key already exists and is valid — return it
        let _ = file.unlock();
        return Ok(buf);
    }

    // Generate a new 32-byte key
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);

    // Write the new key (truncate + write)
    use std::io::Write;
    let mut file = file;
    file.set_len(0)
        .map_err(|e| format!("Cannot truncate HMAC key file: {e}"))?;
    file.write_all(&key)
        .map_err(|e| format!("Cannot write HMAC key file: {e}"))?;
    file.sync_all()
        .map_err(|e| format!("Cannot sync HMAC key file: {e}"))?;

    let _ = file.unlock();

    // Restrict ACL on the key file — fail if we can't lock it down
    restrict_file_acl(&path).map_err(|e| {
        // If ACL restriction fails, delete the key file and return error
        let _ = std::fs::remove_file(&path);
        format!("Failed to restrict ACL on key file: {e}")
    })?;

    Ok(key.to_vec())
}

/// Read the shared HMAC key (fail-closed if it does not exist).
///
/// Used by the elevated helper: if the key file is missing or unreadable,
/// all commands are rejected.
pub fn read_key() -> Result<Vec<u8>, String> {
    let path = key_path();
    let bytes = std::fs::read(&path).map_err(|e| format!("Cannot read HMAC key file: {e}"))?;
    if bytes.len() != 32 {
        return Err("HMAC key file is corrupt (wrong length)".to_string());
    }
    Ok(bytes)
}

/// Compute the HMAC-SHA256 tag for the given data, returned as a hex string.
///
/// Returns `Err` if the HMAC key derivation fails (e.g. invalid key length).
pub fn compute_hmac(key: &[u8], data: &[u8]) -> Result<String, String> {
    let mut mac =
        HmacSha256::new_from_slice(key).map_err(|e| format!("HMAC key derivation failed: {e}"))?;
    mac.update(data);
    Ok(mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect())
}

/// Verify that the expected HMAC matches the data.
///
/// Returns `false` (fail-closed) if HMAC computation fails.
pub fn verify_hmac(key: &[u8], data: &[u8], expected_hex: &str) -> bool {
    let actual = match compute_hmac(key, data) {
        Ok(h) => h,
        Err(e) => {
            log::error!("HMAC computation failed during verification: {e}");
            return false; // Fail-closed
        }
    };
    // Constant-time comparison
    if actual.len() != expected_hex.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (a, b) in actual.bytes().zip(expected_hex.bytes()) {
        diff |= a ^ b;
    }
    diff == 0
}

/// Generate a random 16-byte nonce as a hex string.
pub fn generate_nonce() -> String {
    let mut buf = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut buf);
    buf.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Check if a timestamp (in milliseconds since Unix epoch) is within the
/// acceptable freshness window.
pub fn is_timestamp_fresh(timestamp_ms: u64) -> bool {
    let now = now_ms();
    // Allow 30 seconds of clock skew in either direction
    now >= timestamp_ms.saturating_sub(MAX_COMMAND_AGE_MS)
        && now <= timestamp_ms.saturating_add(MAX_COMMAND_AGE_MS)
}

/// Current time in milliseconds since Unix epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Sign a JSON payload by adding an `hmac` field.
///
/// The HMAC is computed over the serialized JSON of the payload **without**
/// the `hmac` field.  The payload is modified in place to include the `hmac`.
/// If HMAC computation fails, the `hmac` field is left unset and the error is
/// logged — `verify_payload` will reject the unsigned payload.
pub fn sign_payload(payload: &mut serde_json::Value, key: &[u8]) {
    // Serialize without hmac to get the canonical body
    let body = payload.to_string();
    match compute_hmac(key, body.as_bytes()) {
        Ok(hmac) => {
            payload["hmac"] = serde_json::json!(hmac);
        }
        Err(e) => {
            log::error!("Failed to sign payload: {e}");
        }
    }
}

/// Verify a signed JSON payload.
///
/// Returns `Ok(())` if the HMAC is valid and the timestamp is fresh.
/// Returns `Err(message)` if verification fails.
pub fn verify_payload(payload: &mut serde_json::Value, key: &[u8]) -> Result<(), String> {
    // Extract and remove the hmac field
    let expected_hmac = payload
        .get("hmac")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing hmac field".to_string())?
        .to_string();

    payload
        .as_object_mut()
        .ok_or_else(|| "Payload is not an object".to_string())?
        .remove("hmac");

    let body = payload.to_string();

    // S24-003: Try current key first, then old key for grace period.
    if verify_hmac(key, body.as_bytes(), &expected_hmac) {
        // Current key verified — continue to timestamp check.
    } else if let Ok(old_key) = read_old_key() {
        if verify_hmac(&old_key, body.as_bytes(), &expected_hmac) {
            log::warn!("Payload verified with old key — key rotation in progress");
        } else {
            return Err("HMAC verification failed".to_string());
        }
    } else {
        return Err("HMAC verification failed".to_string());
    }

    // Check timestamp freshness — fail-closed if the field is absent.
    let ts = payload
        .get("created_at_ms")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| "Missing required created_at_ms field".to_string())?;
    if !is_timestamp_fresh(ts) {
        return Err(format!(
            "Command timestamp {ts} is stale (older than {MAX_COMMAND_AGE_MS} ms)"
        ));
    }

    Ok(())
}

/// Restrict the ACL on a file so only the current user (and SYSTEM) have access.
///
/// Uses `SetNamedSecurityInfoW` Win32 API instead of shelling out to `icacls.exe`.
/// Returns an error if the restriction fails — callers MUST NOT use the key if this fails.
#[cfg(windows)]
pub fn restrict_file_acl(path: &std::path::Path) -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{LocalFree, ERROR_SUCCESS, HLOCAL};
    use windows::Win32::Security::Authorization::{
        BuildExplicitAccessWithNameW, SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W,
        SET_ACCESS, SE_FILE_OBJECT,
    };
    use windows::Win32::Security::{
        ACL, DACL_SECURITY_INFORMATION, NO_INHERITANCE, PROTECTED_DACL_SECURITY_INFORMATION,
    };
    use windows::Win32::Storage::FileSystem::{
        DELETE, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
    };

    // Convert path to wide string
    let path_w: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // Get the current username
    let username = std::env::var("USERNAME").map_err(|e| format!("Cannot get USERNAME: {e}"))?;

    // Build wide strings that live long enough
    let username_w: Vec<u16> = username.encode_utf16().chain(std::iter::once(0)).collect();
    let system_w: Vec<u16> = "SYSTEM".encode_utf16().chain(std::iter::once(0)).collect();

    let access_mask =
        FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0 | FILE_GENERIC_EXECUTE.0 | DELETE.0;

    // Build explicit access entry for the current user
    let mut user_ea = EXPLICIT_ACCESS_W::default();
    // SAFETY: BuildExplicitAccessWithNameW copies the trustee name internally
    unsafe {
        BuildExplicitAccessWithNameW(
            &mut user_ea,
            PCWSTR(username_w.as_ptr()),
            access_mask,
            SET_ACCESS,
            NO_INHERITANCE,
        );
    }

    // Build explicit access entry for SYSTEM
    let mut system_ea = EXPLICIT_ACCESS_W::default();
    // SAFETY: same as above
    unsafe {
        BuildExplicitAccessWithNameW(
            &mut system_ea,
            PCWSTR(system_w.as_ptr()),
            access_mask,
            SET_ACCESS,
            NO_INHERITANCE,
        );
    }

    let entries = [user_ea, system_ea];

    // Create a new ACL from the entries
    let mut new_acl: *mut ACL = std::ptr::null_mut();
    // SAFETY: SetEntriesInAclW allocates a new ACL; we free it with LocalFree below
    let _ = unsafe { SetEntriesInAclW(Some(&entries), None, &mut new_acl) };
    // Check if SetEntriesInAclW succeeded
    if new_acl.is_null() {
        return Err("SetEntriesInAclW returned null ACL".to_string());
    }

    // Set the security descriptor on the file (replace DACL, remove inheritance)
    // SAFETY: path_w is a valid null-terminated wide string, new_acl is a valid ACL
    let result = unsafe {
        SetNamedSecurityInfoW(
            PCWSTR(path_w.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(new_acl.cast()),
            None,
        )
    };

    // Free the ACL memory allocated by SetEntriesInAclW
    // SAFETY: new_acl was allocated by SetEntriesInAclW and must be freed with LocalFree
    unsafe {
        let _ = LocalFree(HLOCAL(new_acl as _));
    }

    if result != ERROR_SUCCESS {
        return Err(format!(
            "SetNamedSecurityInfoW failed with error code {}",
            result.0
        ));
    }

    Ok(())
}

#[cfg(not(windows))]
pub fn restrict_file_acl(_path: &std::path::Path) -> Result<(), String> {
    Ok(())
}

/// Check if the HMAC key needs rotation (default: 30 days).
///
/// Returns true if the key file is older than the rotation period.
pub fn key_needs_rotation() -> bool {
    let path = key_path();
    match std::fs::metadata(&path) {
        Ok(meta) => {
            if let Ok(modified) = meta.modified() {
                let age = std::time::SystemTime::now()
                    .duration_since(modified)
                    .unwrap_or_default();
                age.as_secs() > 30 * 24 * 60 * 60 // 30 days
            } else {
                false
            }
        }
        Err(_) => false, // No key file yet — will be created by get_or_create_key
    }
}

/// Rotate the HMAC key — generates a new key and stores it.
///
/// The old key is accepted for a grace period (7 days) by keeping a backup
/// file `elev_key.bin.old`.
pub fn rotate_key() -> Result<(), String> {
    let path = key_path();
    let old_path = key_path().with_extension("bin.old");

    // Backup the old key
    if path.exists() {
        std::fs::copy(&path, &old_path)
            .map_err(|e| format!("Failed to backup old HMAC key: {e}"))?;
    }

    // Generate a new key
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);

    // Write the new key
    std::fs::write(&path, key).map_err(|e| format!("Failed to write new HMAC key: {e}"))?;

    // Restrict ACL on the new key
    restrict_file_acl(&path)?;

    log::info!("HMAC key rotated successfully");
    Ok(())
}

/// Read the old (backup) key for grace period verification.
///
/// Returns `Ok(key)` if the backup exists and is within the grace period (7 days).
/// Returns `Err` if no backup exists or it's expired.
pub fn read_old_key() -> Result<Vec<u8>, String> {
    let old_path = key_path().with_extension("bin.old");

    // Check if backup exists
    if !old_path.exists() {
        return Err("No backup key file".to_string());
    }

    // Check if backup is within grace period (7 days)
    if let Ok(meta) = std::fs::metadata(&old_path) {
        if let Ok(modified) = meta.modified() {
            let age = std::time::SystemTime::now()
                .duration_since(modified)
                .unwrap_or_default();
            if age.as_secs() > 7 * 24 * 60 * 60 {
                // Grace period expired — delete the backup
                let _ = std::fs::remove_file(&old_path);
                return Err("Backup key grace period expired".to_string());
            }
        }
    }

    // Read the backup key
    let bytes =
        std::fs::read(&old_path).map_err(|e| format!("Cannot read backup key file: {e}"))?;
    if bytes.len() != 32 {
        return Err("Backup key file is corrupt".to_string());
    }
    Ok(bytes)
}

// ── HKDF sub-key derivation (S19-17) ─────────────────────────────────────────

/// Derive a purpose-specific sub-key from the master HMAC key using HKDF-SHA256.
///
/// Uses the existing HMAC key as input key material (IKM) and `purpose` as
/// the info parameter to derive a 32-byte sub-key.
///
/// # Purposes
/// - `"hmac_signing"` — for HMAC payload signing
/// - `"audit_integrity"` — for consent audit log HMAC
/// - `"wifi_encryption"` — for AES-256-GCM WiFi password encryption
pub fn derive_subkey(purpose: &str) -> Result<[u8; 32], String> {
    let ikm = get_or_create_key()?;
    derive_subkey_from_key(&ikm, purpose)
}

/// Derive a sub-key from an explicit key using HKDF-SHA256.
///
/// This is the testable core of [`derive_subkey`] that doesn't touch disk.
/// Uses the provided key as input key material (IKM) with no salt and
/// `purpose` as the info parameter.
pub fn derive_subkey_from_key(key: &[u8], purpose: &str) -> Result<[u8; 32], String> {
    use hkdf::Hkdf;
    use sha2::Sha256;

    let hk = Hkdf::<Sha256>::new(None, key);
    let mut okm = [0u8; 32];
    hk.expand(purpose.as_bytes(), &mut okm)
        .map_err(|e| format!("HKDF expansion failed: {e}"))?;
    Ok(okm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hmac_roundtrip() {
        let key = b"test-key-32-bytes-long-1234567890";
        let data = b"hello world";
        let tag = compute_hmac(key, data).expect("HMAC computation should succeed");
        assert!(verify_hmac(key, data, &tag));
    }

    #[test]
    fn test_hmac_wrong_key_fails() {
        let key1 = b"test-key-32-bytes-long-1234567890";
        let key2 = b"different-key-32-bytes-long-123456";
        let data = b"hello world";
        let tag = compute_hmac(key1, data).expect("HMAC computation should succeed");
        assert!(!verify_hmac(key2, data, &tag));
    }

    #[test]
    fn test_hmac_tampered_data_fails() {
        let key = b"test-key-32-bytes-long-1234567890";
        let data = b"hello world";
        let tag = compute_hmac(key, data).expect("HMAC computation should succeed");
        assert!(!verify_hmac(key, b"hello worle", &tag));
    }

    #[test]
    fn test_hmac_missing_tag_fails() {
        let key = b"test-key-32-bytes-long-1234567890";
        let data = b"hello world";
        assert!(!verify_hmac(key, data, ""));
    }

    #[test]
    fn test_nonce_uniqueness() {
        let n1 = generate_nonce();
        let n2 = generate_nonce();
        assert_ne!(n1, n2);
        assert_eq!(n1.len(), 32); // 16 bytes = 32 hex chars
    }

    #[test]
    fn test_timestamp_fresh_now() {
        let now = now_ms();
        assert!(is_timestamp_fresh(now));
    }

    #[test]
    fn test_timestamp_stale_rejected() {
        let now = now_ms();
        let old = now - MAX_COMMAND_AGE_MS - 1000; // 31 seconds old
        assert!(!is_timestamp_fresh(old));
    }

    #[test]
    fn test_timestamp_future_rejected() {
        let now = now_ms();
        let future = now + MAX_COMMAND_AGE_MS + 1000; // 31 seconds in future
        assert!(!is_timestamp_fresh(future));
    }

    #[test]
    fn test_sign_and_verify_payload() {
        let key = b"test-key-32-bytes-long-1234567890";
        let mut payload = serde_json::json!({
            "cmd": "set_brightness",
            "args": {"level": 80},
            "created_at_ms": now_ms(),
            "nonce": generate_nonce(),
        });
        sign_payload(&mut payload, key);
        assert!(payload.get("hmac").is_some());

        let mut payload2 = payload.clone();
        assert!(verify_payload(&mut payload2, key).is_ok());
    }

    #[test]
    fn test_verify_payload_missing_hmac() {
        let key = b"test-key-32-bytes-long-1234567890";
        let mut payload = serde_json::json!({
            "cmd": "set_brightness",
            "created_at_ms": now_ms(),
        });
        assert!(verify_payload(&mut payload, key).is_err());
    }

    #[test]
    fn test_verify_payload_tampered() {
        let key = b"test-key-32-bytes-long-1234567890";
        let mut payload = serde_json::json!({
            "cmd": "set_brightness",
            "args": {"level": 80},
            "created_at_ms": now_ms(),
            "nonce": generate_nonce(),
        });
        sign_payload(&mut payload, key);

        // Tamper with the command after signing
        payload["cmd"] = serde_json::json!("set_charging_threshold");
        assert!(verify_payload(&mut payload, key).is_err());
    }

    #[test]
    fn test_verify_payload_stale_timestamp() {
        let key = b"test-key-32-bytes-long-1234567890";
        let mut payload = serde_json::json!({
            "cmd": "set_brightness",
            "created_at_ms": now_ms() - 60_000, // 60 seconds old
            "nonce": generate_nonce(),
        });
        sign_payload(&mut payload, key);
        assert!(verify_payload(&mut payload, key).is_err());
    }

    #[test]
    fn test_verify_payload_wrong_key() {
        let key1 = b"test-key-32-bytes-long-1234567890";
        let key2 = b"different-key-32-bytes-long-123456";
        let mut payload = serde_json::json!({
            "cmd": "set_brightness",
            "created_at_ms": now_ms(),
            "nonce": generate_nonce(),
        });
        sign_payload(&mut payload, key1);
        assert!(verify_payload(&mut payload, key2).is_err());
    }

    #[test]
    fn test_key_needs_rotation_no_file() {
        // When no key file exists, rotation is not needed (will be created)
        // This test just verifies the function doesn't panic
        let _ = key_needs_rotation();
    }

    // ── HKDF sub-key derivation tests (S19-17) ──────────────────────────────

    #[test]
    fn test_derive_subkey_produces_32_bytes() {
        let key = b"test-key-32-bytes-long-1234567890";
        let subkey =
            derive_subkey_from_key(key, "test_purpose").expect("HKDF derivation should succeed");
        assert_eq!(subkey.len(), 32);
    }

    #[test]
    fn test_derive_subkey_different_purposes_produce_different_keys() {
        let key = b"test-key-32-bytes-long-1234567890";
        let k1 = derive_subkey_from_key(key, "hmac_signing").unwrap();
        let k2 = derive_subkey_from_key(key, "audit_integrity").unwrap();
        let k3 = derive_subkey_from_key(key, "wifi_encryption").unwrap();
        assert_ne!(k1, k2, "Different purposes should produce different keys");
        assert_ne!(k1, k3, "Different purposes should produce different keys");
        assert_ne!(k2, k3, "Different purposes should produce different keys");
    }

    #[test]
    fn test_derive_subkey_is_deterministic() {
        let key = b"test-key-32-bytes-long-1234567890";
        let k1 = derive_subkey_from_key(key, "test_purpose").unwrap();
        let k2 = derive_subkey_from_key(key, "test_purpose").unwrap();
        assert_eq!(k1, k2, "Same input should produce same output");
    }

    #[test]
    fn test_derive_subkey_different_keys_produce_different_output() {
        let key1 = b"test-key-32-bytes-long-1234567890";
        let key2 = b"other-key-32-bytes-long-1234567890";
        let k1 = derive_subkey_from_key(key1, "test_purpose").unwrap();
        let k2 = derive_subkey_from_key(key2, "test_purpose").unwrap();
        assert_ne!(k1, k2, "Different keys should produce different output");
    }

    #[test]
    fn test_derive_subkey_empty_purpose() {
        let key = b"test-key-32-bytes-long-1234567890";
        let subkey =
            derive_subkey_from_key(key, "").expect("HKDF with empty purpose should succeed");
        assert_eq!(subkey.len(), 32);
    }
}
