//! HMAC-SHA256 authentication for the elevated bridge protocol.
//!
//! Both the main process and the elevated helper share a secret key stored in
//! `%LOCALAPPDATA%\MiControl\elev_key.bin`.  Every command and response message
//! includes an `hmac` field computed over the JSON body (excluding the `hmac`
//! field itself).  The elevated helper rejects any command whose HMAC does not
//! verify, preventing an attacker from injecting commands via file swapping.

use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;
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
    if let Ok(bytes) = std::fs::read(&path) {
        if bytes.len() == 32 {
            return Ok(bytes);
        }
    }
    // Generate a new 32-byte key
    let mut key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut key);
    std::fs::write(&path, key).map_err(|e| format!("Cannot write HMAC key file: {e}"))?;
    restrict_file_acl(&path);
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
pub fn compute_hmac(key: &[u8], data: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    mac.update(data);
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect()
}

/// Verify that the expected HMAC matches the data.
pub fn verify_hmac(key: &[u8], data: &[u8], expected_hex: &str) -> bool {
    let actual = compute_hmac(key, data);
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
pub fn sign_payload(payload: &mut serde_json::Value, key: &[u8]) {
    // Serialize without hmac to get the canonical body
    let body = payload.to_string();
    let hmac = compute_hmac(key, body.as_bytes());
    payload["hmac"] = serde_json::json!(hmac);
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
    if !verify_hmac(key, body.as_bytes(), &expected_hmac) {
        return Err("HMAC verification failed".to_string());
    }

    // Check timestamp freshness (if present)
    if let Some(ts) = payload.get("created_at_ms").and_then(|v| v.as_u64()) {
        if !is_timestamp_fresh(ts) {
            return Err(format!(
                "Command timestamp {ts} is stale (older than {MAX_COMMAND_AGE_MS} ms)"
            ));
        }
    }

    Ok(())
}

/// Best-effort: restrict the ACL on a file to the current user and SYSTEM only.
///
/// Uses `icacls` to remove inherited permissions and grant full control only to
/// the current user and SYSTEM.  Failures are logged but do not propagate — the
/// default ACL on `%LOCALAPPDATA%` is already user-only.
#[cfg(windows)]
pub fn restrict_file_acl(path: &std::path::Path) {
    let username = std::env::var("USERNAME").unwrap_or_default();
    let path_str = path.to_string_lossy();
    let output = std::process::Command::new("icacls")
        .arg(&*path_str)
        .args(["/inheritance:r"])
        .args(["/grant", &format!("{username}:F")])
        .args(["/grant", "SYSTEM:F"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    if let Err(e) = output {
        log::warn!("Failed to restrict ACL on {}: {e}", path.display());
    }
}

#[cfg(not(windows))]
pub fn restrict_file_acl(_path: &std::path::Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hmac_roundtrip() {
        let key = b"test-key-32-bytes-long-1234567890";
        let data = b"hello world";
        let tag = compute_hmac(key, data);
        assert!(verify_hmac(key, data, &tag));
    }

    #[test]
    fn test_hmac_wrong_key_fails() {
        let key1 = b"test-key-32-bytes-long-1234567890";
        let key2 = b"different-key-32-bytes-long-123456";
        let data = b"hello world";
        let tag = compute_hmac(key1, data);
        assert!(!verify_hmac(key2, data, &tag));
    }

    #[test]
    fn test_hmac_tampered_data_fails() {
        let key = b"test-key-32-bytes-long-1234567890";
        let data = b"hello world";
        let tag = compute_hmac(key, data);
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
}
