//! Audit log for consent grant/revoke events (GDPR Art.30).
//!
//! Records timestamped entries when the user grants or revokes telemetry
//! consent, stored in `%LOCALAPPDATA%\MiControl\consent_audit.log`.
//!
//! ## Integrity across key rotation
//!
//! Each entry is HMAC-SHA256 signed with the `audit_integrity` HKDF sub-key
//! derived from the master HMAC key (`%ProgramData%\MiControl\elev_key.bin`).
//! Rotating the master key used to invalidate the whole history: the audit
//! found all four entries of 2026-09-05 failing verification after the
//! 2026-10-07 rotation, with no previous key kept anywhere.
//!
//! The fix keeps the audit sub-key *itself* on disk, next to the log:
//! - `consent_audit.key`     — sub-key that signs new entries;
//! - `consent_audit.key.old` — sub-key that signed entries before the last
//!   rotation (written by [`pin_key_before_rotation`], called by
//!   `auth::rotate_key` while the old master key is still on disk).
//!
//! Verification tries the current sub-key, then `.old`. An entry that no
//! surviving key can verify is reported as **unverifiable (pre-rotation)** when
//! it is older than the oldest key material we hold — it is not tampering, the
//! signing key is simply gone. Entries newer than that are still reported as
//! tampered, so the log keeps its purpose.

use keyring::Entry;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const KEYRING_SERVICE: &str = "com.mipc.micontrol";
const TELEMETRY_CONSENT_KEY: &str = "telemetry_consent";

/// The current privacy policy version. Bump this when the privacy policy changes.
pub const POLICY_VERSION: u32 = 2;

/// Maximum size of the audit log file before rotation (1 MB).
const MAX_LOG_SIZE_BYTES: u64 = 1_048_576;

/// Maximum number of rotated log files to keep (excluding the active `.log`).
const MAX_LOG_FILES: u32 = 3;

/// HKDF purpose for the audit-log sub-key.
const AUDIT_KEY_PURPOSE: &str = "audit_integrity";

/// Allowance for clock skew between the entry timestamp and the key file mtime
/// before an entry is considered "older than the key material we hold".
const KEY_AGE_SKEW_SECS: u64 = 300;

/// Directory holding the user-scoped state (log + audit keys).
fn local_state_dir() -> PathBuf {
    let base = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| {
        let home = std::env::var("USERPROFILE").unwrap_or_else(|_| ".".into());
        format!("{home}\\AppData\\Local")
    });
    PathBuf::from(base).join("MiControl")
}

/// Build the path to the audit log file (%LOCALAPPDATA%\MiControl\consent_audit.log).
fn audit_log_path() -> PathBuf {
    local_state_dir().join("consent_audit.log")
}

/// Audit sub-key currently signing new entries.
fn audit_key_path() -> PathBuf {
    local_state_dir().join("consent_audit.key")
}

/// Audit sub-key that signed entries before the last rotation.
fn audit_key_old_path() -> PathBuf {
    local_state_dir().join("consent_audit.key.old")
}

/// Read a 32-byte key file, returning `None` when absent or malformed.
fn read_key_file(path: &Path) -> Option<[u8; 32]> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() != 32 {
        log::warn!(
            "[consent_audit] key file {} has {} bytes (expected 32) — ignoring",
            path.display(),
            bytes.len()
        );
        return None;
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Some(key)
}

/// Persist a 32-byte key file with the same ACL restriction as the master key.
fn write_key_file(path: &Path, key: &[u8; 32]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot create {}: {e}", parent.display()))?;
    }
    std::fs::write(path, key).map_err(|e| format!("Cannot write {}: {e}", path.display()))?;
    // Best effort: the log itself lives in the same user-scoped directory.
    if let Err(e) = crate::util::auth::restrict_file_acl(path) {
        log::warn!(
            "[consent_audit] failed to restrict ACL on {}: {e}",
            path.display()
        );
    }
    Ok(())
}

/// Derive the current audit sub-key from the master HMAC key.
fn derive_audit_key() -> Result<[u8; 32], String> {
    crate::util::auth::derive_subkey(AUDIT_KEY_PURPOSE)
}

/// Ensure `consent_audit.key` holds the sub-key derived from the current master
/// key, preserving the previous value in `.old` if the master rotated without
/// going through `auth::rotate_key` (manual deletion, reinstall, corruption).
fn ensure_audit_key() -> Result<[u8; 32], String> {
    let current = derive_audit_key()?;
    match read_key_file(&audit_key_path()) {
        Some(stored) if stored == current => Ok(current),
        stored => {
            if let Some(prev) = stored {
                // Master changed outside rotate_key(): keep the previous audit
                // key so entries signed with it stay verifiable.
                if let Err(e) = write_key_file(&audit_key_old_path(), &prev) {
                    log::warn!("[consent_audit] cannot preserve previous audit key: {e}");
                } else {
                    log::warn!(
                        "[consent_audit] master key changed outside rotate_key() — \
                         previous audit sub-key preserved as {}",
                        audit_key_old_path().display()
                    );
                }
            }
            write_key_file(&audit_key_path(), &current)?;
            Ok(current)
        }
    }
}

/// Core of the pre-rotation pin.
///
/// The key that signed existing entries wins: the value already in
/// `consent_audit.key` when it differs from `current` (a rotation happened
/// outside `rotate_key`), otherwise `current` itself (derived from the master
/// that is still on disk). The pinned value goes to `.old`; `current` is then
/// written as the active key.
fn pin_key_files(current: &[u8; 32]) -> Result<(), String> {
    let pinned = match read_key_file(&audit_key_path()) {
        Some(stored) if &stored != current => stored,
        _ => *current,
    };
    write_key_file(&audit_key_old_path(), &pinned)?;
    write_key_file(&audit_key_path(), current)
}

/// Pin the audit sub-key that signed all entries so far into
/// `consent_audit.key.old`.
///
/// Must be called **before** the master key is replaced (`auth::rotate_key`).
/// Deriving the sub-key needs the old master, which is exactly why the pin has
/// to happen first — otherwise the history becomes permanently unverifiable,
/// as happened with the 2026-09-05 entries.
pub fn pin_key_before_rotation() {
    match derive_audit_key() {
        Ok(key) => match pin_key_files(&key) {
            Ok(()) => log::info!(
                "[consent_audit] pre-rotation audit key preserved in {}",
                audit_key_old_path().display()
            ),
            Err(e) => log::warn!("[consent_audit] failed to pin audit key before rotation: {e}"),
        },
        Err(e) => log::warn!("[consent_audit] cannot derive audit key before rotation: {e}"),
    }
}

/// Record the audit sub-key derived from the **new** master key, so entries
/// written after the rotation keep verifying against `consent_audit.key`.
///
/// Called by `auth::rotate_key` once the new master key is on disk.
pub fn activate_key_after_rotation() {
    match derive_audit_key() {
        Ok(key) => {
            if let Err(e) = write_key_file(&audit_key_path(), &key) {
                log::warn!("[consent_audit] failed to activate new audit key: {e}");
            }
        }
        Err(e) => log::warn!("[consent_audit] cannot derive audit key after rotation: {e}"),
    }
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

    // Compute HMAC for integrity protection using the HKDF-derived audit
    // sub-key (S19-17), materialising `consent_audit.key`/`.old` on disk so a
    // later master-key rotation cannot orphan this entry's signature.
    let hmac_tag = match ensure_audit_key() {
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

    // S26-003: Restrict ACL on consent_audit.log when first created.
    let needs_acl = !path.exists();

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
            // S26-003: Restrict ACL if the file was just created.
            if needs_acl {
                if let Err(e) = crate::util::auth::restrict_file_acl(&path) {
                    log::warn!("Failed to restrict ACL on consent audit log: {e}");
                }
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
            // Handle plain string format (current — stored by setTelemetryConsent)
            if val == "granted" {
                return true;
            }
            if val == "denied" {
                return false;
            }
            // Handle legacy JSON format (backwards compatibility)
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&val) {
                return parsed["value"].as_str() == Some("granted");
            }
            false
        }
        Err(_) => false,
    }
}

/// Result of verifying the consent audit log.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AuditVerification {
    /// Number of non-empty lines examined.
    pub entries: usize,
    /// Entries whose HMAC verified against a key we hold.
    pub verified: usize,
    /// Entries that cannot be verified because they were signed by a key that
    /// no longer exists (pre-rotation). Reported, not treated as tampering.
    pub unverifiable_pre_rotation: usize,
    /// Entries written without an HMAC tag (key unavailable at write time).
    pub unsigned: usize,
    /// Entries that fail verification but are *not* pre-rotation — tampering or
    /// on-disk corruption. One `Line N: …` message per failure.
    pub failures: Vec<String>,
}

impl AuditVerification {
    /// True when no entry shows evidence of tampering.
    pub fn is_tamper_free(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Oldest audit-key material on disk, as Unix epoch seconds.
///
/// An entry older than this cannot have been signed by any key we still hold,
/// so a verification failure is attributed to rotation rather than tampering.
fn oldest_key_epoch() -> Option<u64> {
    let mut oldest: Option<u64> = None;
    for path in [audit_key_path(), audit_key_old_path()] {
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        let Ok(modified) = meta.modified() else {
            continue;
        };
        let Ok(epoch) = modified.duration_since(SystemTime::UNIX_EPOCH) else {
            continue;
        };
        let secs = epoch.as_secs();
        oldest = Some(oldest.map_or(secs, |cur: u64| cur.min(secs)));
    }
    oldest
}

/// Verify the entries of `content` against the given candidate keys.
///
/// Kept separate from [`verify_audit_log`] so tests can exercise the
/// current-key / `.old`-key / pre-rotation classification without touching the
/// real master key or `%LOCALAPPDATA%`.
fn verify_entries(
    content: &str,
    candidates: &[[u8; 32]],
    oldest_key_epoch: Option<u64>,
) -> AuditVerification {
    let mut report = AuditVerification::default();

    for (line_num, line) in content.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        report.entries += 1;

        // Parse: {ts}\t{event}\tpolicy_version={ver}\thmac={hmac}
        let parts: Vec<&str> = line.splitn(2, "\thmac=").collect();
        if parts.len() != 2 {
            report
                .failures
                .push(format!("Line {}: missing HMAC tag", line_num + 1));
            continue;
        }
        let entry = parts[0];
        let stored_hmac = parts[1];

        if stored_hmac.is_empty() {
            // Entry was written when the HMAC key was unavailable — it carries
            // no signature, so there is nothing to verify or to distrust.
            report.unsigned += 1;
            continue;
        }

        if candidates
            .iter()
            .any(|key| crate::util::auth::verify_hmac(key, entry.as_bytes(), stored_hmac))
        {
            report.verified += 1;
            continue;
        }

        // Neither the current nor the previous key verifies. If the entry is
        // older than every key we still hold, its signing key was rotated away
        // before we started preserving it — unverifiable, not tampered.
        let entry_ts = entry
            .split('\t')
            .next()
            .and_then(|ts| ts.parse::<u64>().ok());
        let is_pre_rotation = match (entry_ts, oldest_key_epoch) {
            (Some(ts), Some(cutoff)) => ts.saturating_add(KEY_AGE_SKEW_SECS) < cutoff,
            _ => false,
        };
        if is_pre_rotation {
            report.unverifiable_pre_rotation += 1;
            log::warn!(
                "[consent_audit] line {} unverifiable (pre-rotation): signed before the \
                 oldest preserved audit key",
                line_num + 1
            );
        } else {
            report.failures.push(format!(
                "Line {}: HMAC verification failed — entry may be tampered",
                line_num + 1
            ));
        }
    }

    report
}

/// Verify the integrity of all audit log entries.
///
/// Returns `Ok(())` when nothing shows evidence of tampering. Entries that are
/// merely unverifiable because they predate the preserved audit key are
/// accepted (and logged) — see [`AuditVerification`].
pub fn verify_audit_log() -> Result<(), String> {
    let report = verify_audit_log_report()?;
    match report.failures.first() {
        Some(first) => Err(first.clone()),
        None => Ok(()),
    }
}

/// Full verification report, including the pre-rotation entries.
///
/// Tries the sub-key derived from the current master key, the sub-key stored in
/// `consent_audit.key`, and the pre-rotation `consent_audit.key.old`. Returns
/// `Err` only when no key material is available at all.
pub fn verify_audit_log_report() -> Result<AuditVerification, String> {
    let path = audit_log_path();
    if !path.exists() {
        return Ok(AuditVerification::default());
    }

    let mut candidates: Vec<[u8; 32]> = Vec::new();
    if let Ok(derived) = derive_audit_key() {
        candidates.push(derived);
    }
    for key in [
        read_key_file(&audit_key_path()),
        read_key_file(&audit_key_old_path()),
    ]
    .into_iter()
    .flatten()
    {
        if !candidates.contains(&key) {
            candidates.push(key);
        }
    }
    if candidates.is_empty() {
        return Err(
            "No audit key available (master key unreadable and no stored audit key)".into(),
        );
    }

    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("Cannot read audit log: {e}"))?;
    let report = verify_entries(&content, &candidates, oldest_key_epoch());

    if report.unverifiable_pre_rotation > 0 {
        log::warn!(
            "[consent_audit] {} of {} entries unverifiable (pre-rotation) — signed by a key \
             that was rotated away before audit keys were preserved",
            report.unverifiable_pre_rotation,
            report.entries
        );
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialize tests that modify LOCALAPPDATA to prevent parallel test pollution.
    static LOCALAPPDATA_LOCK: Mutex<()> = Mutex::new(());

    /// Build a correctly signed audit-log line for `key`.
    fn signed_line(ts: u64, event: &str, key: &[u8; 32]) -> String {
        let entry = format!("{ts}\t{event}\tpolicy_version=2");
        let tag = crate::util::auth::compute_hmac(key, entry.as_bytes()).unwrap();
        format!("{entry}\thmac={tag}")
    }

    fn test_key(seed: u8) -> [u8; 32] {
        [seed; 32]
    }

    /// Entries signed before a rotation must verify with the preserved `.old`
    /// key — the exact failure in the audit (4/4 entries invalid after the
    /// 2026-10-07 rotation).
    #[test]
    fn entries_verify_against_current_or_previous_key() {
        let key_a = test_key(0xAA); // pre-rotation
        let key_b = test_key(0xBB); // current
        let content = format!(
            "{}\n{}\n",
            signed_line(1_700_000_000, "CONSENT_GRANTED", &key_a),
            signed_line(1_750_000_000, "CONSENT_REVOKED", &key_b),
        );

        // Current key first, then `.old`.
        let report = verify_entries(&content, &[key_b, key_a], None);
        assert_eq!(report.entries, 2);
        assert_eq!(report.verified, 2, "both keys must be tried");
        assert!(report.is_tamper_free());
        assert_eq!(report.unverifiable_pre_rotation, 0);
    }

    /// An entry older than every key we hold is reported as unverifiable
    /// (pre-rotation), not as tampering.
    #[test]
    fn pre_rotation_entry_is_unverifiable_not_tampered() {
        let key_a = test_key(0xAA);
        let key_b = test_key(0xBB);
        // Signed by a key that no longer exists (key_c), dated before the
        // oldest preserved key.
        let content = format!(
            "{}\n",
            signed_line(1_600_000_000, "CONSENT_GRANTED", &test_key(0xCC))
        );

        let report = verify_entries(&content, &[key_b, key_a], Some(1_700_000_000));
        assert_eq!(report.entries, 1);
        assert_eq!(report.verified, 0);
        assert_eq!(report.unverifiable_pre_rotation, 1);
        assert!(
            report.is_tamper_free(),
            "a lost signing key is not evidence of tampering"
        );
    }

    /// A bad signature *inside* the coverage window is still tampering.
    #[test]
    fn tampered_entry_newer_than_key_material_fails() {
        let key_b = test_key(0xBB);
        let cutoff = 1_700_000_000u64;
        let mut line = signed_line(cutoff + 10_000, "CONSENT_REVOKED", &test_key(0xCC));
        // Keep the line well-formed, change the event.
        line = line.replace("CONSENT_REVOKED", "CONSENT_GRANTED");

        let report = verify_entries(&format!("{line}\n"), &[key_b], Some(cutoff));
        assert_eq!(report.verified, 0);
        assert_eq!(report.unverifiable_pre_rotation, 0);
        assert_eq!(report.failures.len(), 1);
        assert!(report.failures[0].contains("may be tampered"));
    }

    /// Missing / empty tags are classified, never mistaken for valid entries.
    #[test]
    fn missing_and_empty_hmac_tags_are_reported() {
        let key = test_key(0xBB);
        let content = format!(
            "1700000000\tCONSENT_GRANTED\tpolicy_version=2\thmac=\n\
             1700000001\tCONSENT_REVOKED\tpolicy_version=2\n\
             {}\n",
            signed_line(1_700_000_002, "CONSENT_GRANTED", &key)
        );

        let report = verify_entries(&content, &[key], None);
        assert_eq!(report.entries, 3);
        assert_eq!(report.unsigned, 1, "entry written without a key");
        assert_eq!(
            report.failures.len(),
            1,
            "a line without a tag is a format failure"
        );
        assert!(report.failures[0].contains("missing HMAC tag"));
        assert_eq!(report.verified, 1);
    }

    /// The pre-rotation pin keeps the key that signed the existing entries.
    #[test]
    fn pin_key_files_preserves_previous_key() {
        let _lock = LOCALAPPDATA_LOCK.lock().unwrap();

        let orig = std::env::var("LOCALAPPDATA").ok();
        let tmp = std::env::temp_dir().join("micontrol_test_audit_pin");
        let _ = std::fs::remove_dir_all(&tmp);
        std::env::set_var("LOCALAPPDATA", &tmp);

        let old = test_key(0xA1);
        let new = test_key(0xB2);
        // The file records the key that actually signed entries on disk.
        write_key_file(&audit_key_path(), &old).unwrap();

        pin_key_files(&new).unwrap();

        assert_eq!(
            read_key_file(&audit_key_old_path()).unwrap(),
            old,
            "the key that signed existing entries must land in .old"
        );
        assert_eq!(read_key_file(&audit_key_path()).unwrap(), new);

        let _ = std::fs::remove_dir_all(&tmp);
        if let Some(v) = orig {
            std::env::set_var("LOCALAPPDATA", v);
        }
    }

    /// With no materialised key yet, the pin seeds both files from the master —
    /// this is the case that was missing entirely in production.
    #[test]
    fn pin_key_files_seeds_old_when_no_current_key_exists() {
        let _lock = LOCALAPPDATA_LOCK.lock().unwrap();

        let orig = std::env::var("LOCALAPPDATA").ok();
        let tmp = std::env::temp_dir().join("micontrol_test_audit_pin_seed");
        let _ = std::fs::remove_dir_all(&tmp);
        std::env::set_var("LOCALAPPDATA", &tmp);

        let key = test_key(0xC3);
        pin_key_files(&key).unwrap();

        assert_eq!(read_key_file(&audit_key_old_path()).unwrap(), key);
        assert_eq!(read_key_file(&audit_key_path()).unwrap(), key);

        let _ = std::fs::remove_dir_all(&tmp);
        if let Some(v) = orig {
            std::env::set_var("LOCALAPPDATA", v);
        }
    }

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

        // The audit sub-key must be materialised next to the log, and the
        // entry must verify against it — otherwise a later master-key rotation
        // orphans the signature (the production failure).
        let key_path = tmp.join("MiControl").join("consent_audit.key");
        let stored = read_key_file(&key_path).expect("consent_audit.key must be written");
        let report = verify_entries(content.trim_end(), &[stored], None);
        assert_eq!(
            report.verified, 1,
            "entry must verify against the stored key"
        );
        assert!(report.is_tamper_free());

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
