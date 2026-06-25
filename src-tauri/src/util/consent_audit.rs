//! Audit log for consent grant/revoke events (GDPR Art.30).
//!
//! Records timestamped entries when the user grants or revokes telemetry
//! consent, stored in `%LOCALAPPDATA%\MiControl\consent_audit.log`.

use keyring::Entry;
use std::io::Write;
use std::path::PathBuf;
use std::time::SystemTime;

const KEYRING_SERVICE: &str = "com.mipc.micontrol";
const TELEMETRY_CONSENT_KEY: &str = "telemetry_consent";

/// The current privacy policy version. Bump this when the privacy policy changes.
pub const POLICY_VERSION: u32 = 2;

/// Maximum size of the audit log file before rotation (1 MB).
const MAX_LOG_SIZE_BYTES: u64 = 1_048_576;

/// Maximum number of rotated log files to keep (excluding the active `.log`).
const MAX_LOG_FILES: u32 = 3;

/// Build the path to the audit log file (%LOCALAPPDATA%\MiControl\consent_audit.log).
fn audit_log_path() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| {
        let home = std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into());
        format!("{}\\AppData\\Local", home)
    });
    PathBuf::from(base)
        .join("MiControl")
        .join("consent_audit.log")
}

/// Format the current time as a Unix epoch timestamp (seconds).
fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Rotate the audit log if it exceeds [`MAX_LOG_SIZE_BYTES`].
///
/// Rotates: `.log` → `.log.1`, `.log.1` → `.log.2`, ..., `.log.{MAX_LOG_FILES}` is deleted.
/// Creates a new empty `.log` file.
fn rotate_if_needed() {
    let path = audit_log_path();

    let size = match std::fs::metadata(&path) {
        Ok(meta) => meta.len(),
        Err(_) => return, // File doesn't exist yet — nothing to rotate
    };

    if size <= MAX_LOG_SIZE_BYTES {
        return;
    }

    log::info!(
        "Consent audit log reached {} bytes (limit {}), rotating...",
        size,
        MAX_LOG_SIZE_BYTES
    );

    // Delete the oldest rotated file (.log.{MAX_LOG_FILES})
    let oldest = path.with_extension(format!("log.{MAX_LOG_FILES}"));
    if oldest.exists() {
        if let Err(e) = std::fs::remove_file(&oldest) {
            log::warn!(
                "Failed to delete old rotated audit log {}: {e}",
                oldest.display()
            );
        }
    }

    // Shift files: .log.{n} → .log.{n+1}, from highest to lowest
    for n in (1..MAX_LOG_FILES).rev() {
        let src = path.with_extension(format!("log.{n}"));
        let dst = path.with_extension(format!("log.{}", n + 1));
        if src.exists() {
            if let Err(e) = std::fs::rename(&src, &dst) {
                log::warn!(
                    "Failed to rotate audit log {} → {}: {e}",
                    src.display(),
                    dst.display()
                );
            }
        }
    }

    // Move the current .log → .log.1
    let rotated = path.with_extension("log.1");
    if let Err(e) = std::fs::rename(&path, &rotated) {
        log::warn!(
            "Failed to rotate audit log {} → {}: {e}",
            path.display(),
            rotated.display()
        );
        return; // Don't truncate if rename failed
    }

    // Create a new empty .log file
    if let Err(e) = std::fs::File::create(&path) {
        log::error!("Failed to create new audit log after rotation: {e}");
    }
}

/// Log a consent event to the audit log.
pub fn log_consent_event(event: &str, policy_version: u32) {
    let path = audit_log_path();

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // Rotate the log if it has grown too large
    rotate_if_needed();

    let ts = unix_timestamp();
    let entry = format!("{ts}\t{event}\tpolicy_version={policy_version}");

    // Compute HMAC for integrity protection using HKDF-derived sub-key (S19-17)
    let hmac_tag = match crate::util::auth::derive_subkey("audit_integrity") {
        Ok(key) => crate::util::auth::compute_hmac(&key, entry.as_bytes()).unwrap_or_else(|e| {
            log::error!("Failed to compute HMAC for audit log: {e}");
            String::new()
        }),
        Err(e) => {
            log::error!("Failed to derive audit HMAC key: {e}");
            // Write without HMAC if key is unavailable — better to log than to lose the entry
            String::new()
        }
    };

    let signed_entry = format!("{entry}\thmac={hmac_tag}\n");

    // Append to the audit log
    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        Ok(mut file) => {
            if let Err(e) = file.write_all(signed_entry.as_bytes()) {
                log::error!("Failed to write consent audit log: {e}");
            }
        }
        Err(e) => {
            log::error!("Failed to open consent audit log: {e}");
        }
    }
}

/// Log that consent was granted.
pub fn log_consent_granted(policy_version: u32) {
    log_consent_event("CONSENT_GRANTED", policy_version);
}

/// Log that consent was revoked.
pub fn log_consent_revoked(policy_version: u32) {
    log_consent_event("CONSENT_REVOKED", policy_version);
}

/// Read the audit log entries.
pub fn read_audit_log() -> Vec<String> {
    match std::fs::read_to_string(audit_log_path()) {
        Ok(content) => content.lines().map(|l| l.to_string()).collect(),
        Err(_) => Vec::new(),
    }
}

/// Delete the audit log file (used by data deletion — GDPR Art.17).
pub fn purge_audit_log() {
    let path = audit_log_path();
    if path.exists() {
        if let Err(e) = std::fs::remove_file(&path) {
            log::warn!("Failed to purge consent audit log: {e}");
        }
    }
}

/// Check whether the user has granted telemetry consent (via the keyring).
/// Returns `true` if consent is granted, `false` if denied or not set.
///
/// Used at startup to decide whether to initialise Sentry crash reporting.
pub fn check_sentry_consent() -> bool {
    let entry = match Entry::new(KEYRING_SERVICE, TELEMETRY_CONSENT_KEY) {
        Ok(e) => e,
        Err(_) => return false,
    };
    match entry.get_password() {
        Ok(val) => {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&val) {
                parsed["value"].as_str() == Some("granted")
            } else {
                false
            }
        }
        Err(_) => false,
    }
}

/// Verify the integrity of all audit log entries.
///
/// Returns `Ok(())` if all entries are valid, or `Err(message)` describing
/// the first tampered entry.
pub fn verify_audit_log() -> Result<(), String> {
    let path = audit_log_path();
    if !path.exists() {
        return Ok(());
    }

    let key = crate::util::auth::derive_subkey("audit_integrity")?;

    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("Cannot read audit log: {e}"))?;

    for (line_num, line) in content.lines().enumerate() {
        if line.is_empty() {
            continue;
        }

        // Parse: {ts}\t{event}\tpolicy_version={ver}\thmac={hmac}
        let parts: Vec<&str> = line.splitn(2, "\thmac=").collect();
        if parts.len() != 2 {
            return Err(format!("Line {}: missing HMAC tag", line_num + 1));
        }

        let entry = parts[0];
        let stored_hmac = parts[1];

        if stored_hmac.is_empty() {
            // Entry was written when HMAC key was unavailable — skip verification
            continue;
        }

        if !crate::util::auth::verify_hmac(&key, entry.as_bytes(), stored_hmac) {
            return Err(format!(
                "Line {}: HMAC verification failed — entry may be tampered",
                line_num + 1
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialize tests that modify LOCALAPPDATA to prevent parallel test pollution.
    static LOCALAPPDATA_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_log_consent_event_writes_hmac() {
        let _lock = LOCALAPPDATA_LOCK.lock().unwrap();

        // Use a temp directory for testing
        let orig = std::env::var("LOCALAPPDATA").ok();
        let tmp = std::env::temp_dir().join("micontrol_test_audit");
        std::env::set_var("LOCALAPPDATA", &tmp);

        log_consent_event("TEST_EVENT", 2);

        let log_path = tmp.join("MiControl").join("consent_audit.log");
        assert!(log_path.exists(), "Audit log file should exist");

        let content = std::fs::read_to_string(&log_path).unwrap();
        assert!(
            content.contains("TEST_EVENT"),
            "Log should contain the event"
        );
        assert!(content.contains("hmac="), "Log should contain HMAC tag");

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
        if let Some(orig_val) = orig {
            std::env::set_var("LOCALAPPDATA", orig_val);
        }
    }

    #[test]
    fn test_verify_audit_log_detects_tampering() {
        let _lock = LOCALAPPDATA_LOCK.lock().unwrap();

        let orig = std::env::var("LOCALAPPDATA").ok();
        let tmp = std::env::temp_dir().join("micontrol_test_audit_verify");
        std::env::set_var("LOCALAPPDATA", &tmp);

        // Write a valid entry
        log_consent_event("TEST_EVENT", 2);

        // Tamper with the log file
        let log_path = tmp.join("MiControl").join("consent_audit.log");
        let content = std::fs::read_to_string(&log_path).unwrap();
        let tampered = content.replace("TEST_EVENT", "HACKED_EVENT");
        std::fs::write(&log_path, tampered).unwrap();

        // Verification should fail
        let result = verify_audit_log();
        assert!(result.is_err(), "Tampered log should fail verification");

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp);
        if let Some(orig_val) = orig {
            std::env::set_var("LOCALAPPDATA", orig_val);
        }
    }

    #[test]
    fn test_rotate_if_needed_no_file() {
        let _lock = LOCALAPPDATA_LOCK.lock().unwrap();

        let orig = std::env::var("LOCALAPPDATA").ok();
        let tmp = std::env::temp_dir().join("micontrol_test_audit_rotate_none");
        std::env::set_var("LOCALAPPDATA", &tmp);

        // No log file exists — rotation should be a no-op
        rotate_if_needed();

        let log_path = tmp.join("MiControl").join("consent_audit.log");
        assert!(
            !log_path.exists(),
            "No log file should be created by rotation"
        );

        let _ = std::fs::remove_dir_all(&tmp);
        if let Some(orig_val) = orig {
            std::env::set_var("LOCALAPPDATA", orig_val);
        }
    }

    #[test]
    fn test_rotate_if_needed_small_file() {
        let _lock = LOCALAPPDATA_LOCK.lock().unwrap();

        let orig = std::env::var("LOCALAPPDATA").ok();
        let tmp = std::env::temp_dir().join("micontrol_test_audit_rotate_small");
        std::env::set_var("LOCALAPPDATA", &tmp);

        // Create a small log file
        let log_path = tmp.join("MiControl").join("consent_audit.log");
        std::fs::create_dir_all(log_path.parent().unwrap()).unwrap();
        std::fs::write(&log_path, "small content").unwrap();

        // Rotation should not happen
        rotate_if_needed();

        let content = std::fs::read_to_string(&log_path).unwrap();
        assert_eq!(content, "small content", "Small file should not be rotated");
        assert!(
            !log_path.with_extension("log.1").exists(),
            "No rotated file should exist"
        );

        let _ = std::fs::remove_dir_all(&tmp);
        if let Some(orig_val) = orig {
            std::env::set_var("LOCALAPPDATA", orig_val);
        }
    }

    #[test]
    fn test_rotate_if_needed_large_file() {
        let _lock = LOCALAPPDATA_LOCK.lock().unwrap();

        let orig = std::env::var("LOCALAPPDATA").ok();
        let tmp = std::env::temp_dir().join("micontrol_test_audit_rotate_large");
        std::env::set_var("LOCALAPPDATA", &tmp);

        // Create a log file exceeding MAX_LOG_SIZE_BYTES
        let log_path = tmp.join("MiControl").join("consent_audit.log");
        std::fs::create_dir_all(log_path.parent().unwrap()).unwrap();
        let large_content = "x".repeat((MAX_LOG_SIZE_BYTES + 1) as usize);
        std::fs::write(&log_path, &large_content).unwrap();

        // Rotation should happen
        rotate_if_needed();

        // The original file should now be empty (new file created)
        let new_content = std::fs::read_to_string(&log_path).unwrap();
        assert_eq!(
            new_content, "",
            "New log file should be empty after rotation"
        );

        // The rotated file (.log.1) should contain the old content
        let rotated_path = log_path.with_extension("log.1");
        let rotated_content = std::fs::read_to_string(&rotated_path).unwrap();
        assert_eq!(
            rotated_content, large_content,
            "Rotated file should contain old content"
        );

        let _ = std::fs::remove_dir_all(&tmp);
        if let Some(orig_val) = orig {
            std::env::set_var("LOCALAPPDATA", orig_val);
        }
    }

    #[test]
    fn test_rotate_if_needed_multiple_rotations() {
        let _lock = LOCALAPPDATA_LOCK.lock().unwrap();

        let orig = std::env::var("LOCALAPPDATA").ok();
        let tmp = std::env::temp_dir().join("micontrol_test_audit_rotate_multi");
        std::env::set_var("LOCALAPPDATA", &tmp);

        let log_path = tmp.join("MiControl").join("consent_audit.log");
        std::fs::create_dir_all(log_path.parent().unwrap()).unwrap();

        // Pre-create rotated files to verify shifting
        std::fs::write(&log_path, "current").unwrap();
        std::fs::write(log_path.with_extension("log.1"), "rotation1").unwrap();
        std::fs::write(log_path.with_extension("log.2"), "rotation2").unwrap();

        // Make the current file large enough to trigger rotation
        let large_content = "x".repeat((MAX_LOG_SIZE_BYTES + 1) as usize);
        std::fs::write(&log_path, &large_content).unwrap();

        rotate_if_needed();

        // .log should be empty (new file)
        assert_eq!(std::fs::read_to_string(&log_path).unwrap(), "");
        // .log.1 should have the old current content
        assert_eq!(
            std::fs::read_to_string(log_path.with_extension("log.1")).unwrap(),
            large_content
        );
        // .log.2 should have the old .log.1 content
        assert_eq!(
            std::fs::read_to_string(log_path.with_extension("log.2")).unwrap(),
            "rotation1"
        );
        // .log.3 should have the old .log.2 content
        assert_eq!(
            std::fs::read_to_string(log_path.with_extension("log.3")).unwrap(),
            "rotation2"
        );

        let _ = std::fs::remove_dir_all(&tmp);
        if let Some(orig_val) = orig {
            std::env::set_var("LOCALAPPDATA", orig_val);
        }
    }
}
