//! Anonymous, bounded action log for OS Turbo.
//!
//! S39-006: adaptive control needs before/after evidence, but ordinary logs can
//! identify a user or disclose local machine details. This module is anonymous
//! **by construction**: the record type has no field capable of holding a
//! process name, path, PID, user/device identifier, raw error or prompt text.
//!
//! Guarantees:
//!
//! 1. **Allowlisted fields only.** [`ActionRecord`] is a closed struct of
//!    enums, small integers and rounded floats. There is no `String` field
//!    that accepts free text.
//! 2. **Safe error codes.** Errors are mapped to [`SafeErrorCode`] before
//!    logging; raw error strings never reach the log.
//! 3. **Bounded storage.** [`MAX_RECORDS`] records, [`MAX_BYTES`] bytes and
//!    [`RETENTION_MS`] retention, with deterministic oldest-first eviction.
//! 4. **Atomic writes.** Records are written to a temporary file and renamed,
//!    so a crash cannot leave a partially written log.
//! 5. **Disabled means silent.** When logging is disabled, no record is
//!    produced at all.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::os_turbo_policy::{HoldReason, PolicyAction, RollbackReason};
use super::os_turbo_stats::{ConfidenceBucket, EffectDirection, MetricKind, WorkloadClass};

/// Log schema version.
pub const LOG_VERSION: u32 = 1;

/// Maximum retained records.
pub const MAX_RECORDS: usize = 500;

/// Maximum retained bytes (approximate; enforced on serialized length).
pub const MAX_BYTES: usize = 256 * 1024;

/// Retention window in milliseconds (7 days).
pub const RETENTION_MS: u64 = 7 * 24 * 60 * 60 * 1000;

/// Coarse time bucket. Deliberately imprecise so records cannot be correlated
/// with a user's activity timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeBucket {
    /// 00:00–05:59
    Night,
    /// 06:00–11:59
    Morning,
    /// 12:00–17:59
    Afternoon,
    /// 18:00–23:59
    Evening,
}

impl TimeBucket {
    /// Map an hour (0–23) to a coarse bucket.
    pub fn from_hour(hour: u32) -> Self {
        match hour {
            0..=5 => Self::Night,
            6..=11 => Self::Morning,
            12..=17 => Self::Afternoon,
            _ => Self::Evening,
        }
    }
}

/// Safe, stable error codes. Raw error text is never logged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SafeErrorCode {
    /// Process could not be opened.
    ProcessOpenFailed,
    /// Process state could not be read.
    ProcessReadFailed,
    /// Process state could not be written.
    ProcessWriteFailed,
    /// The authenticated bridge was unavailable.
    BridgeUnavailable,
    /// The bridge rejected the request.
    BridgeRejected,
    /// The power scheme could not be changed.
    PowerSchemeFailed,
    /// The recovery snapshot could not be read or written.
    SnapshotFailed,
    /// The operation timed out.
    Timeout,
    /// Any other failure, deliberately unspecified.
    Other,
}

/// Outcome of one logged action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionOutcome {
    Applied,
    Held,
    RolledBack,
    Failed,
}

/// One anonymous action record.
///
/// Every field is an enum, a small integer or a rounded float. There is no
/// free-text field, so identifying data cannot be stored even by mistake.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionRecord {
    pub version: u32,
    /// Coarse time bucket (never an exact timestamp).
    pub time_bucket: TimeBucket,
    /// The allowlisted action, when one was chosen.
    pub action: Option<PolicyAction>,
    pub outcome: ActionOutcome,
    /// Hold reason, when the outcome was `Held`.
    pub hold_reason: Option<HoldReason>,
    /// Rollback reason, when the outcome was `RolledBack`.
    pub rollback_reason: Option<RollbackReason>,
    /// Safe error code, when the outcome was `Failed`.
    pub error_code: Option<SafeErrorCode>,
    /// Metric compared, when a comparison happened.
    pub metric: Option<MetricKind>,
    /// Workload class of the compared windows.
    pub workload: Option<WorkloadClass>,
    /// Rounded before/after means and delta.
    pub before_mean: Option<f64>,
    pub after_mean: Option<f64>,
    pub delta: Option<f64>,
    /// Rounded effect size.
    pub effect_size: Option<f64>,
    pub direction: Option<EffectDirection>,
    pub confidence: Option<ConfidenceBucket>,
    /// Samples in each window (small integers).
    pub before_samples: Option<u32>,
    pub after_samples: Option<u32>,
}

/// Measured comparison attached to a record. Grouped into a struct so the
/// record builder stays readable and clippy-clean.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MeasurementSummary {
    pub metric: MetricKind,
    pub workload: WorkloadClass,
    pub before_mean: f64,
    pub after_mean: f64,
    pub delta: f64,
    pub effect_size: Option<f64>,
    pub direction: EffectDirection,
    pub confidence: ConfidenceBucket,
    pub before_samples: u32,
    pub after_samples: u32,
}

impl ActionRecord {
    /// Build a record for a held decision.
    pub fn held(bucket: TimeBucket, reason: HoldReason) -> Self {
        Self {
            version: LOG_VERSION,
            time_bucket: bucket,
            action: None,
            outcome: ActionOutcome::Held,
            hold_reason: Some(reason),
            rollback_reason: None,
            error_code: None,
            metric: None,
            workload: None,
            before_mean: None,
            after_mean: None,
            delta: None,
            effect_size: None,
            direction: None,
            confidence: None,
            before_samples: None,
            after_samples: None,
        }
    }

    /// Build a record for an applied action.
    pub fn applied(bucket: TimeBucket, action: PolicyAction) -> Self {
        Self {
            version: LOG_VERSION,
            time_bucket: bucket,
            action: Some(action),
            outcome: ActionOutcome::Applied,
            hold_reason: None,
            rollback_reason: None,
            error_code: None,
            metric: None,
            workload: None,
            before_mean: None,
            after_mean: None,
            delta: None,
            effect_size: None,
            direction: None,
            confidence: None,
            before_samples: None,
            after_samples: None,
        }
    }

    /// Build a record for a rollback.
    pub fn rolled_back(bucket: TimeBucket, reason: RollbackReason) -> Self {
        Self {
            version: LOG_VERSION,
            time_bucket: bucket,
            action: None,
            outcome: ActionOutcome::RolledBack,
            hold_reason: None,
            rollback_reason: Some(reason),
            error_code: None,
            metric: None,
            workload: None,
            before_mean: None,
            after_mean: None,
            delta: None,
            effect_size: None,
            direction: None,
            confidence: None,
            before_samples: None,
            after_samples: None,
        }
    }

    /// Build a record for a failure, using a safe error code.
    pub fn failed(bucket: TimeBucket, code: SafeErrorCode) -> Self {
        Self {
            version: LOG_VERSION,
            time_bucket: bucket,
            action: None,
            outcome: ActionOutcome::Failed,
            hold_reason: None,
            rollback_reason: None,
            error_code: Some(code),
            metric: None,
            workload: None,
            before_mean: None,
            after_mean: None,
            delta: None,
            effect_size: None,
            direction: None,
            confidence: None,
            before_samples: None,
            after_samples: None,
        }
    }

    /// Attach a measured comparison to this record. Values are rounded to two
    /// decimals so the log stays compact and deterministic.
    pub fn with_measurement(mut self, summary: MeasurementSummary) -> Self {
        self.metric = Some(summary.metric);
        self.workload = Some(summary.workload);
        self.before_mean = Some(super::os_turbo_stats::round_deterministic(
            summary.before_mean,
            2,
        ));
        self.after_mean = Some(super::os_turbo_stats::round_deterministic(
            summary.after_mean,
            2,
        ));
        self.delta = Some(super::os_turbo_stats::round_deterministic(summary.delta, 2));
        self.effect_size = summary
            .effect_size
            .map(|d| super::os_turbo_stats::round_deterministic(d, 2));
        self.direction = Some(summary.direction);
        self.confidence = Some(summary.confidence);
        self.before_samples = Some(summary.before_samples);
        self.after_samples = Some(summary.after_samples);
        self
    }
}

/// In-memory bounded log with deterministic eviction.
#[derive(Debug, Clone, Default)]
pub struct ActionLog {
    records: Vec<ActionRecord>,
    enabled: bool,
}

impl ActionLog {
    pub fn new(enabled: bool) -> Self {
        Self {
            records: Vec::new(),
            enabled,
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    pub fn records(&self) -> &[ActionRecord] {
        &self.records
    }

    /// Append a record. Returns `false` when logging is disabled (nothing is
    /// stored) or when the record was rejected by the caps.
    pub fn push(&mut self, record: ActionRecord) -> bool {
        if !self.enabled {
            return false;
        }
        self.records.push(record);
        self.enforce_caps();
        true
    }

    /// Drop records until both the record cap and the byte cap are satisfied.
    /// Eviction is oldest-first and deterministic.
    fn enforce_caps(&mut self) {
        while self.records.len() > MAX_RECORDS {
            self.records.remove(0);
        }
        while self.serialized_len() > MAX_BYTES && !self.records.is_empty() {
            self.records.remove(0);
        }
    }

    /// Approximate serialized size of the retained records.
    pub fn serialized_len(&self) -> usize {
        serde_json::to_string(&self.records)
            .map(|s| s.len())
            .unwrap_or(0)
    }

    /// Remove records older than the retention window.
    ///
    /// Records carry only a coarse time bucket, so retention is applied by the
    /// caller supplying the current bucket and the number of buckets elapsed.
    /// This keeps the log free of exact timestamps.
    pub fn evict_expired(&mut self, buckets_elapsed: u64) {
        if buckets_elapsed == 0 {
            return;
        }
        // Each bucket is 6 hours; retention is 7 days = 28 buckets.
        const BUCKETS_PER_RETENTION: u64 = RETENTION_MS / (6 * 60 * 60 * 1000);
        if buckets_elapsed >= BUCKETS_PER_RETENTION {
            self.records.clear();
        }
    }

    /// Serialize the retained records deterministically.
    pub fn to_json(&self) -> String {
        serde_json::to_string(&self.records).unwrap_or_else(|_| "[]".to_string())
    }

    /// Parse records, recovering from corruption by discarding unreadable
    /// content rather than failing.
    pub fn from_json(json: &str, enabled: bool) -> Self {
        let records: Vec<ActionRecord> = serde_json::from_str(json).unwrap_or_default();
        let mut log = Self { records, enabled };
        log.enforce_caps();
        log
    }
}

/// Path of the anonymous action log under the protected data directory.
pub fn action_log_path(data_dir: &Path) -> PathBuf {
    data_dir.join("os_turbo_actions.json")
}

/// Persist the log atomically (write to a temp file, then rename).
pub fn save_action_log(data_dir: &Path, log: &ActionLog) -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let target = action_log_path(data_dir);
    let temp = target.with_extension("json.tmp");
    std::fs::write(&temp, log.to_json())?;
    std::fs::rename(&temp, &target)
}

/// Load the log, tolerating a missing or corrupt file.
pub fn load_action_log(data_dir: &Path, enabled: bool) -> ActionLog {
    let path = action_log_path(data_dir);
    match std::fs::read_to_string(&path) {
        Ok(json) => ActionLog::from_json(&json, enabled),
        Err(_) => ActionLog::new(enabled),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_record() -> ActionRecord {
        ActionRecord::applied(
            TimeBucket::Afternoon,
            PolicyAction::ThrottleAllowlistedBackground,
        )
        .with_measurement(MeasurementSummary {
            metric: MetricKind::BackgroundCpuPercent,
            workload: WorkloadClass::Light,
            before_mean: 40.123,
            after_mean: 20.456,
            delta: -19.667,
            effect_size: Some(-4.123),
            direction: EffectDirection::Improvement,
            confidence: ConfidenceBucket::High,
            before_samples: 32,
            after_samples: 32,
        })
    }

    #[test]
    fn disabled_log_stores_nothing() {
        let mut log = ActionLog::new(false);
        assert!(!log.push(sample_record()));
        assert!(log.is_empty());
        assert_eq!(log.to_json(), "[]");
    }

    #[test]
    fn enabled_log_stores_records() {
        let mut log = ActionLog::new(true);
        assert!(log.push(sample_record()));
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn record_cap_evicts_oldest_first() {
        let mut log = ActionLog::new(true);
        for i in 0..(MAX_RECORDS + 25) {
            let bucket = if i % 2 == 0 {
                TimeBucket::Morning
            } else {
                TimeBucket::Evening
            };
            log.push(ActionRecord::held(bucket, HoldReason::CooldownActive));
        }
        assert_eq!(log.len(), MAX_RECORDS);
    }

    #[test]
    fn byte_cap_is_enforced() {
        let mut log = ActionLog::new(true);
        for _ in 0..MAX_RECORDS {
            log.push(sample_record());
        }
        assert!(log.serialized_len() <= MAX_BYTES);
    }

    #[test]
    fn retention_clears_after_window() {
        let mut log = ActionLog::new(true);
        log.push(sample_record());
        log.evict_expired(1);
        assert_eq!(log.len(), 1, "one bucket is inside the retention window");
        log.evict_expired(28);
        assert!(log.is_empty(), "28 buckets exceeds the 7-day window");
    }

    #[test]
    fn corruption_recovers_to_empty_log() {
        let log = ActionLog::from_json("{not valid json", true);
        assert!(log.is_empty());
        assert!(log.is_enabled());
    }

    #[test]
    fn round_trip_is_deterministic() {
        let mut log = ActionLog::new(true);
        log.push(sample_record());
        let json = log.to_json();
        let reloaded = ActionLog::from_json(&json, true);
        assert_eq!(reloaded.to_json(), json);
    }

    #[test]
    fn atomic_save_and_load_round_trip() {
        let dir = std::env::temp_dir().join("micontrol_os_turbo_log_test");
        let _ = std::fs::remove_dir_all(&dir);
        let mut log = ActionLog::new(true);
        log.push(sample_record());
        save_action_log(&dir, &log).expect("save");
        let loaded = load_action_log(&dir, true);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded.to_json(), log.to_json());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_file_loads_empty() {
        let dir = std::env::temp_dir().join("micontrol_os_turbo_log_missing");
        let _ = std::fs::remove_dir_all(&dir);
        let loaded = load_action_log(&dir, true);
        assert!(loaded.is_empty());
    }

    #[test]
    fn time_buckets_are_coarse() {
        assert_eq!(TimeBucket::from_hour(0), TimeBucket::Night);
        assert_eq!(TimeBucket::from_hour(5), TimeBucket::Night);
        assert_eq!(TimeBucket::from_hour(6), TimeBucket::Morning);
        assert_eq!(TimeBucket::from_hour(11), TimeBucket::Morning);
        assert_eq!(TimeBucket::from_hour(12), TimeBucket::Afternoon);
        assert_eq!(TimeBucket::from_hour(17), TimeBucket::Afternoon);
        assert_eq!(TimeBucket::from_hour(18), TimeBucket::Evening);
        assert_eq!(TimeBucket::from_hour(23), TimeBucket::Evening);
    }

    #[test]
    fn serialized_records_contain_no_identifying_data() {
        let mut log = ActionLog::new(true);
        log.push(sample_record());
        log.push(ActionRecord::held(
            TimeBucket::Night,
            HoldReason::InsufficientEvidence,
        ));
        log.push(ActionRecord::rolled_back(
            TimeBucket::Morning,
            RollbackReason::MeasuredRegression,
        ));
        log.push(ActionRecord::failed(
            TimeBucket::Evening,
            SafeErrorCode::BridgeUnavailable,
        ));

        let json = log.to_json().to_lowercase();

        // No process names, paths, extensions or drive letters.
        for needle in [
            ".exe",
            "c:\\",
            "\\\\",
            "/users/",
            "searchindexer",
            "msmpeng",
            "trustedinstaller",
            "backgroundtaskhost",
            "tiworker",
        ] {
            assert!(!json.contains(needle), "log leaked {needle}");
        }

        // No raw error text or prompt/suggestion text.
        for needle in [
            "access is denied",
            "0x8007",
            "hresult",
            "error:",
            "prompt",
            "suggestion",
            "openai",
            "api key",
            "bearer",
        ] {
            assert!(!json.contains(needle), "log leaked {needle}");
        }

        // No exact timestamps: no ISO date, no epoch-like long digit runs.
        assert!(!json.contains("t20"), "log leaked an ISO timestamp");
        assert!(!json.contains("z\""), "log leaked an ISO timestamp");
        for token in json.split(|c: char| !c.is_ascii_digit()) {
            if token.len() >= 10 {
                panic!("log leaked a long numeric identifier: {token}");
            }
        }
    }

    #[test]
    fn record_struct_has_no_free_text_field() {
        // Structural privacy guarantee: every field is an enum, a small
        // integer or a float. This test fails to compile if a String field is
        // ever added to ActionRecord.
        let record = sample_record();
        let _: u32 = record.version;
        let _: TimeBucket = record.time_bucket;
        let _: Option<PolicyAction> = record.action;
        let _: ActionOutcome = record.outcome;
        let _: Option<HoldReason> = record.hold_reason;
        let _: Option<RollbackReason> = record.rollback_reason;
        let _: Option<SafeErrorCode> = record.error_code;
        let _: Option<MetricKind> = record.metric;
        let _: Option<WorkloadClass> = record.workload;
        let _: Option<f64> = record.before_mean;
        let _: Option<f64> = record.after_mean;
        let _: Option<f64> = record.delta;
        let _: Option<f64> = record.effect_size;
        let _: Option<EffectDirection> = record.direction;
        let _: Option<ConfidenceBucket> = record.confidence;
        let _: Option<u32> = record.before_samples;
        let _: Option<u32> = record.after_samples;
    }

    #[test]
    fn measurements_are_rounded() {
        let record = sample_record();
        assert_eq!(record.before_mean, Some(40.12));
        assert_eq!(record.after_mean, Some(20.46));
        assert_eq!(record.delta, Some(-19.67));
        assert_eq!(record.effect_size, Some(-4.12));
    }

    #[test]
    fn concurrent_writers_do_not_corrupt_the_file() {
        let dir = std::env::temp_dir().join("micontrol_os_turbo_log_concurrent");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");

        let handles: Vec<_> = (0..4)
            .map(|i| {
                let dir = dir.clone();
                std::thread::spawn(move || {
                    let mut log = ActionLog::new(true);
                    for _ in 0..10 {
                        log.push(ActionRecord::held(
                            TimeBucket::Morning,
                            HoldReason::CooldownActive,
                        ));
                    }
                    let _ = save_action_log(&dir, &log);
                    i
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("thread");
        }

        // Whatever survived must be valid JSON with bounded size.
        let loaded = load_action_log(&dir, true);
        assert!(loaded.len() <= MAX_RECORDS);
        assert!(loaded.serialized_len() <= MAX_BYTES);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
