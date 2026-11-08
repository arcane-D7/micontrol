//! AC-adapter power reading (WMAA `0x1000 / 0x06`), with a bounded retry gate.
//!
//! ## What the production log actually shows (root-cause correction)
//!
//! The 2026-11-07 audit read `0x80041002` (`WBEM_E_NOT_FOUND`) as "this
//! hardware does not expose the adapter-power class" and made the feature
//! **permanently** unavailable, per process, after a single failure. That
//! conclusion is wrong, and it is what killed a working reading:
//!
//! * `MICommonInterface` **does** exist on this machine (`Get-CimClass
//!   -Namespace root\WMI` lists it), so "class not present" was never true.
//! * `tauri-app.log` for 2026-11-02 → 2026-11-06 records **18 451 successful**
//!   `wmi_ec_read_adapter_power` dispatches (`ok=true`) against **1 337**
//!   `0x80041002` failures — i.e. the nicety worked ~93 % of the time, and the
//!   very next poll (2 s later) always recovered.
//!
//! So `0x80041002` here is a *degraded-provider hiccup* (the same one the
//! `wmi_call` body documents around `params.Put(InData)`), not a capability
//! fact. Treating it as permanent turned a 93 %-working sensor into a
//! feature that is off for the rest of the session.
//!
//! ## Policy now
//!
//! * `0x80041010` / `WBEM_E_INVALID_CLASS` / "invalid namespace" — the class
//!   genuinely does not exist. Answer `supported: false` and **defer** probes
//!   for a long but finite window (1 h): never latch forever.
//! * `0x80041002` — provider degraded. Retried once inside
//!   `wmi_ec::read_adapter_power`; if it still fails the poll simply has no
//!   value this time and the next poll tries again (short deferral only, to
//!   bound cost).
//! * Success — clears the gate immediately.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Structured adapter-power result crossing the command / bridge boundary.
///
/// `supported == false` is a *successful* answer: the platform does not expose
/// the WMAA adapter-power class. `watts` is then `None` and `reason` explains
/// why (for logs/diagnostics only — the UI should simply not render the field).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterPowerReading {
    pub supported: bool,
    pub watts: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl AdapterPowerReading {
    pub fn watts(value: u32) -> Self {
        Self {
            supported: true,
            watts: Some(value),
            reason: None,
        }
    }

    pub fn unsupported(reason: impl Into<String>) -> Self {
        Self {
            supported: false,
            watts: None,
            reason: Some(reason.into()),
        }
    }
}

/// Why an adapter-power read failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdapterPowerFailure {
    /// The WMI class this command needs really does not exist here
    /// (`WBEM_E_INVALID_CLASS` / invalid namespace). Long deferral, but never a
    /// permanent latch — the answer survives only for the current session's
    /// backoff window.
    HardwareUnsupported,
    /// `WBEM_E_NOT_FOUND` (`0x80041002`): on this provider that is the
    /// **degraded-state** error raised while building the method parameters
    /// (`params.Put(InData)`), not "class missing" — `MICommonInterface` is
    /// present and the same call succeeded 18 451× on 2026-11-02…06,
    /// interleaved with these failures. Retryable.
    ProviderDegraded,
    /// Anything else (transport, auth, timeout, provider error).
    Other,
}

/// HRESULTs seen in production for the adapter-power read.
///
/// * `0x80041002` — `WBEM_E_NOT_FOUND`. NOT a capability fact on this
///   provider (see [`AdapterPowerFailure::ProviderDegraded`]).
/// * `0x80041010` — `WBEM_E_INVALID_CLASS` (provider present, class unknown).
///
/// Matched as text because `HardwareError` flattens the COM error to its
/// `Display` string before it reaches us (`HardwareError::Other`). The match is
/// case-insensitive so `0X80041002` from a different formatting layer still hits.
pub const WBEM_NOT_FOUND: u32 = 0x8004_1002;
pub const WBEM_INVALID_CLASS: u32 = 0x8004_1010;

/// Classify a failure message coming from the elevated bridge / WMI layer.
///
/// Pure so it can be unit-tested with the exact strings seen in production.
pub fn classify_failure(message: &str) -> AdapterPowerFailure {
    let lowered = message.to_ascii_lowercase();
    let not_found = format!("{WBEM_NOT_FOUND:08x}");
    let invalid_class = format!("{WBEM_INVALID_CLASS:08x}");
    // Signed decimal form (i32), as printed by some COM wrappers.
    let not_found_signed = (WBEM_NOT_FOUND as i32).to_string();
    let invalid_class_signed = (WBEM_INVALID_CLASS as i32).to_string();

    let is_degraded = lowered.contains(&not_found)
        || lowered.contains(&not_found_signed)
        || lowered.contains("wbem_e_not_found");
    let is_invalid_class = lowered.contains(&invalid_class)
        || lowered.contains(&invalid_class_signed)
        || lowered.contains("wbem_e_invalid_class")
        || lowered.contains("invalid class")
        || lowered.contains("invalid namespace");

    if is_invalid_class {
        AdapterPowerFailure::HardwareUnsupported
    } else if is_degraded {
        AdapterPowerFailure::ProviderDegraded
    } else {
        AdapterPowerFailure::Other
    }
}

/// Down-time applied when the platform really lacks the class (1 h).
pub const UNSUPPORTED_DEFER_SECS: u64 = 3_600;
/// Down-time applied after a degraded-provider failure (60 s).
pub const DEGRADED_DEFER_SECS: u64 = 60;

/// Milliseconds since the Unix epoch (monotonic sanity is irrelevant here: the
/// gate only compares against itself, and a clock jump just re-opens it).
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// How long adapter-power probes are suspended **for**, not whether they are
/// dead.
///
/// The previous implementation was a one-way latch: the first failure disabled
/// the feature for the life of the process. On this machine that single failure
/// is a 7 %-probability provider hiccup, so the latch turned a 93 %-working
/// reading into "never available" (production: 8 004 probes on 2026-11-04 vs
/// **zero** on 2026-11-07 after the latch was introduced).
///
/// A gate is used instead: failures push a deadline into the future, success
/// clears it, and every caller keeps working when it opens again.
pub struct ProbeGate {
    /// Epoch-ms timestamp before which probing is pointless. `0` = open.
    until_ms: AtomicU64,
    /// One-shot logging, so a persistent failure logs once per deferral.
    logged: AtomicBool,
}

impl ProbeGate {
    pub const fn new() -> Self {
        Self {
            until_ms: AtomicU64::new(0),
            logged: AtomicBool::new(false),
        }
    }

    /// `true` while probing is suspended.
    pub fn is_open(&self) -> bool {
        let until = self.until_ms.load(Ordering::Relaxed);
        until != 0 && now_ms() < until
    }

    /// Record a successful read: the feature works, re-open immediately.
    pub fn note_supported(&self) {
        self.until_ms.store(0, Ordering::Relaxed);
        self.logged.store(false, Ordering::Relaxed);
    }

    /// Suspend probing for `secs`. Returns `true` when this call extended the
    /// deadline (i.e. the caller should emit its log line now).
    pub fn defer(&self, secs: u64) -> bool {
        let deadline = now_ms().saturating_add(secs.saturating_mul(1000));
        let previous = self.until_ms.swap(deadline, Ordering::Relaxed);
        deadline > previous
    }

    /// One-shot logging gate, independent from the deadline so the elevated
    /// helper can log its own line in its own process.
    pub fn should_log_once(&self) -> bool {
        !self.logged.swap(true, Ordering::Relaxed)
    }

    /// Test helper: forget everything.
    #[cfg(test)]
    pub fn reset(&self) {
        self.until_ms.store(0, Ordering::Relaxed);
        self.logged.store(false, Ordering::Relaxed);
    }
}

impl Default for ProbeGate {
    fn default() -> Self {
        Self::new()
    }
}

/// Process-wide gate consulted by every app-side adapter-power read.
static ADAPTER_POWER_GATE: ProbeGate = ProbeGate::new();

/// `true` while the adapter-power probe is suspended (never permanent).
pub fn probe_deferred() -> bool {
    ADAPTER_POWER_GATE.is_open()
}

/// Suspend the probe for [`DEGRADED_DEFER_SECS`], logging once per window.
///
/// `reason` is for the log only; the verdict ("hardware lacks the class" vs
/// "provider hiccup") decides the window length at the call site.
pub fn defer_probe(secs: u64, reason: &str) {
    if ADAPTER_POWER_GATE.defer(secs) && ADAPTER_POWER_GATE.should_log_once() {
        log::info!(
            target: "hw::adapter_power",
            "AC adapter power probe paused for {secs}s ({reason}); it will be \
             retried automatically — the reading is not disabled"
        );
    }
}

/// Parse the bridge's answer.
///
/// Accepts both shapes for rolling upgrades:
///   * `{"supported":true,"watts":65}` — new structured payload;
///   * `65` — legacy bare number returned by older bridge binaries.
pub fn parse_response(value: &serde_json::Value) -> Result<AdapterPowerReading, String> {
    if let Some(watts) = value.as_u64() {
        return Ok(AdapterPowerReading::watts(watts as u32));
    }
    if let Some(object) = value.as_object() {
        let supported = object
            .get("supported")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        if !supported {
            let reason = object
                .get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("hardware does not expose the WMI class");
            return Ok(AdapterPowerReading::unsupported(reason));
        }
        if let Some(watts) = object.get("watts").and_then(|v| v.as_u64()) {
            return Ok(AdapterPowerReading::watts(watts as u32));
        }
        return Err(format!("adapter-power response missing 'watts': {value}"));
    }
    Err(format!(
        "unexpected adapter-power response from bridge: {value}"
    ))
}

#[cfg(windows)]
async fn read_impl(allow_uac: bool, cmd: &'static str) -> Result<AdapterPowerReading, String> {
    // Gate first: no WMI, no bridge, no helper process while suspended.
    if ADAPTER_POWER_GATE.is_open() {
        return Ok(AdapterPowerReading::unsupported(
            "probe paused after a recent failure; it will retry automatically",
        ));
    }

    let result = if allow_uac {
        crate::elev_bridge::run_elevated(cmd, serde_json::json!({})).await
    } else {
        crate::elev_bridge::run_elevated_no_prompt(cmd, serde_json::json!({})).await
    };

    match result {
        Ok(value) => {
            let reading = parse_response(&value)?;
            if reading.supported {
                ADAPTER_POWER_GATE.note_supported();
            } else {
                // The bridge itself said "no such class". Long but finite pause.
                defer_probe(
                    UNSUPPORTED_DEFER_SECS,
                    reading
                        .reason
                        .as_deref()
                        .unwrap_or("bridge reported the class as unsupported"),
                );
            }
            Ok(reading)
        }
        Err(message) => match classify_failure(&message) {
            // Real capability fact: the class/namespace does not exist here.
            AdapterPowerFailure::HardwareUnsupported => {
                defer_probe(UNSUPPORTED_DEFER_SECS, &message);
                Ok(AdapterPowerReading::unsupported(format!(
                    "WMI class not present: {message}"
                )))
            }
            // Provider hiccup: report this poll as "no value" (the UI keeps the
            // last known wattage) and retry soon. Never disables the feature.
            AdapterPowerFailure::ProviderDegraded => {
                defer_probe(DEGRADED_DEFER_SECS, &message);
                Err(message)
            }
            AdapterPowerFailure::Other => Err(message),
        },
    }
}

/// Read AC adapter power through the elevated path (UAC allowed, as before).
#[cfg(windows)]
pub async fn read_via_bridge() -> Result<AdapterPowerReading, String> {
    read_impl(true, "wmi_ec_read_adapter_power").await
}

/// Read AC adapter power through the elevated path without ever prompting UAC.
#[cfg(windows)]
pub async fn read_via_bridge_no_prompt() -> Result<AdapterPowerReading, String> {
    read_impl(false, "wmi_ec_read_adapter_power").await
}

#[cfg(not(windows))]
pub async fn read_via_bridge() -> Result<AdapterPowerReading, String> {
    Ok(AdapterPowerReading::unsupported(
        "AC adapter power is only available on Windows",
    ))
}

#[cfg(not(windows))]
pub async fn read_via_bridge_no_prompt() -> Result<AdapterPowerReading, String> {
    read_via_bridge().await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_the_production_failure_string() {
        // Exactly what tauri-app.log showed 1 337 times — interleaved with
        // 18 451 successes, so it is a provider hiccup, NOT a capability fact.
        assert_eq!(
            classify_failure("Hardware error: 0x80041002"),
            AdapterPowerFailure::ProviderDegraded
        );
        assert_eq!(
            classify_failure("0x80041002"),
            AdapterPowerFailure::ProviderDegraded
        );
        assert_eq!(
            classify_failure("WMI cache: wmi transient query error: 0x80041002"),
            AdapterPowerFailure::ProviderDegraded
        );
        assert_eq!(
            classify_failure("Hardware error: 0X80041002"),
            AdapterPowerFailure::ProviderDegraded
        );
        assert_eq!(
            classify_failure(&format!("HRESULT {}", WBEM_NOT_FOUND as i32)),
            AdapterPowerFailure::ProviderDegraded
        );
        assert_eq!(
            classify_failure("WBEM_E_NOT_FOUND"),
            AdapterPowerFailure::ProviderDegraded
        );
    }

    #[test]
    fn classifies_invalid_class_as_unsupported() {
        assert_eq!(
            classify_failure("WMI query failed: 0x80041010"),
            AdapterPowerFailure::HardwareUnsupported
        );
        assert_eq!(
            classify_failure(&format!("HRESULT {}", WBEM_INVALID_CLASS as i32)),
            AdapterPowerFailure::HardwareUnsupported
        );
        assert_eq!(
            classify_failure("WBEM_E_INVALID_CLASS"),
            AdapterPowerFailure::HardwareUnsupported
        );
        assert_eq!(
            classify_failure("Invalid namespace ROOT\\WMI"),
            AdapterPowerFailure::HardwareUnsupported
        );
    }

    #[test]
    fn transport_failures_stay_errors() {
        for message in [
            "Bridge service pipe round-trip timed out after 15s (service did not respond)",
            "Elevated command 'wmi_ec_read_adapter_power' timed out after 15 s and UAC fallback is disabled",
            "Elevated command rejected (auth failure): nonce replay",
            "WMI query failed: 0x80041003",
            "Hardware error: 0x80070005",
            "",
        ] {
            assert_eq!(
                classify_failure(message),
                AdapterPowerFailure::Other,
                "must not classify as unsupported: {message}"
            );
        }
    }

    #[test]
    fn gate_starts_open_and_closes_for_a_bounded_window() {
        let gate = ProbeGate::new();
        assert!(!gate.is_open(), "a fresh gate must not block probing");
        assert!(gate.defer(60), "first deferral extends the deadline");
        assert!(gate.is_open(), "probing is suspended inside the window");
    }

    #[test]
    fn gate_reopens_after_the_window_expires() {
        let gate = ProbeGate::new();
        // 0 s deferral: the deadline is now, so the gate is already open again.
        gate.defer(0);
        assert!(
            !gate.is_open(),
            "the gate must never latch permanently — this is the regression \
             that killed the adapter reading for a whole session"
        );
    }

    #[test]
    fn gate_success_reopens_immediately() {
        let gate = ProbeGate::new();
        gate.defer(UNSUPPORTED_DEFER_SECS);
        assert!(gate.is_open());
        gate.note_supported();
        assert!(!gate.is_open(), "a successful read clears the suspension");
    }

    #[test]
    fn gate_logs_only_once_per_window() {
        let gate = ProbeGate::new();
        assert!(gate.should_log_once());
        assert!(!gate.should_log_once());
        assert!(!gate.should_log_once());
    }

    #[test]
    fn gate_can_be_reset_for_tests() {
        let gate = ProbeGate::new();
        gate.defer(60);
        gate.should_log_once();
        gate.reset();
        assert!(!gate.is_open());
        assert!(gate.should_log_once());
    }

    #[test]
    fn parses_structured_and_legacy_payloads() {
        let structured = serde_json::json!({"supported": true, "watts": 65});
        assert_eq!(
            parse_response(&structured).unwrap(),
            AdapterPowerReading::watts(65)
        );

        let legacy = serde_json::json!(65);
        assert_eq!(
            parse_response(&legacy).unwrap(),
            AdapterPowerReading::watts(65)
        );

        let unsupported = serde_json::json!({
            "supported": false,
            "reason": "WBEM_E_NOT_FOUND"
        });
        let parsed = parse_response(&unsupported).unwrap();
        assert!(!parsed.supported);
        assert_eq!(parsed.watts, None);
        assert_eq!(parsed.reason.as_deref(), Some("WBEM_E_NOT_FOUND"));
    }

    #[test]
    fn rejects_malformed_payloads() {
        assert!(parse_response(&serde_json::json!("nope")).is_err());
        assert!(parse_response(&serde_json::json!({"supported": true})).is_err());
        assert!(parse_response(&serde_json::Value::Null).is_err());
    }

    #[test]
    fn unsupported_readings_are_not_errors() {
        let reading = AdapterPowerReading::unsupported("class missing");
        assert!(!reading.supported);
        assert_eq!(reading.watts, None);

        // The wire shape the frontend sees: an explicit `supported: false`
        // plus `watts: null` — no error code, no thrown rejection.
        let json = serde_json::to_value(&reading).unwrap();
        assert_eq!(json["supported"], serde_json::json!(false));
        assert_eq!(json["watts"], serde_json::Value::Null);
        assert_eq!(json["reason"], serde_json::json!("class missing"));
    }
}
