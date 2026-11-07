//! AC-adapter power reading with an explicit "unsupported hardware" result.
//!
//! Audit 2026-11-07, P2: `wmi_ec_read_adapter_power` was the **only** elevated
//! command that failed, 115 times, always with `0x80041002`
//! (`WBEM_E_NOT_FOUND`). The WMAA adapter-power class simply does not exist on
//! this machine, but the app treated every poll as a generic error: the
//! elevated helper logged `ok=false`, the UI surfaced an error, and the next
//! poll repeated the whole WMI probe 20 s later.
//!
//! Two changes:
//!   1. `WBEM_E_NOT_FOUND` (and `WBEM_E_INVALID_CLASS`) are classified as
//!      **hardware not supported**, not as an error: callers get
//!      `Ok(AdapterPowerReading { supported: false, .. })` and can hide the
//!      field instead of showing a red error.
//!   2. A per-process in-memory circuit remembers that decision: once the
//!      platform has been proven not to expose the class, later reads return
//!      the cached "unsupported" answer without touching WMI or the bridge
//!      again. The first (and only) log line is emitted at trip time.
//!
//! Note on process lifetime: the elevated helper is a short-lived process
//! (one per scheduled-task dispatch), so its own circuit dies with it. The
//! circuit that actually stops the retry spam therefore lives in the **app**
//! process — this module — and is consulted before every dispatch.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

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
    /// The WMI class/instance this command needs does not exist on this
    /// hardware — permanent for the session (and for the machine).
    HardwareUnsupported,
    /// Anything else (transport, auth, timeout, provider error).
    Other,
}

/// HRESULTs that mean "this hardware does not expose that WMI class".
///
/// * `0x80041002` — `WBEM_E_NOT_FOUND` (class or instance missing; the value
///   observed 115× in production).
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

    if lowered.contains(&not_found)
        || lowered.contains(&invalid_class)
        || lowered.contains(&not_found_signed)
        || lowered.contains(&invalid_class_signed)
        || lowered.contains("wbem_e_not_found")
        || lowered.contains("wbem_e_invalid_class")
    {
        AdapterPowerFailure::HardwareUnsupported
    } else {
        AdapterPowerFailure::Other
    }
}

const CIRCUIT_UNKNOWN: u8 = 0;
const CIRCUIT_SUPPORTED: u8 = 1;
const CIRCUIT_UNSUPPORTED: u8 = 2;

/// Per-process latch for "this machine does not support adapter-power reads".
///
/// Once tripped it stays tripped for the life of the process: the topology of
/// a laptop does not change at runtime, so re-probing every 20 s is pure churn.
pub struct UnsupportedCircuit {
    state: AtomicU8,
    logged: AtomicBool,
}

impl UnsupportedCircuit {
    pub const fn new() -> Self {
        Self {
            state: AtomicU8::new(CIRCUIT_UNKNOWN),
            logged: AtomicBool::new(false),
        }
    }

    /// `true` once the platform has been proven not to support the feature.
    pub fn is_open(&self) -> bool {
        self.state.load(Ordering::Relaxed) == CIRCUIT_UNSUPPORTED
    }

    /// Record a successful read: the feature works, keep the circuit closed.
    pub fn note_supported(&self) {
        self.state.store(CIRCUIT_SUPPORTED, Ordering::Relaxed);
    }

    /// Record "hardware unsupported". Returns `true` only on the transition,
    /// i.e. the caller should emit its **single** log line now.
    pub fn trip(&self) -> bool {
        let previous = self.state.swap(CIRCUIT_UNSUPPORTED, Ordering::Relaxed);
        previous != CIRCUIT_UNSUPPORTED
    }

    /// One-shot logging gate, independent from the state transition so the
    /// elevated helper can log its own single line in its own process.
    pub fn should_log_once(&self) -> bool {
        !self.logged.swap(true, Ordering::Relaxed)
    }

    /// Test helper: forget everything.
    #[cfg(test)]
    pub fn reset(&self) {
        self.state.store(CIRCUIT_UNKNOWN, Ordering::Relaxed);
        self.logged.store(false, Ordering::Relaxed);
    }
}

impl Default for UnsupportedCircuit {
    fn default() -> Self {
        Self::new()
    }
}

/// Process-wide circuit consulted by every app-side adapter-power read.
static ADAPTER_POWER_CIRCUIT: UnsupportedCircuit = UnsupportedCircuit::new();

/// `true` when a previous read already proved the class is missing.
pub fn is_known_unsupported() -> bool {
    ADAPTER_POWER_CIRCUIT.is_open()
}

/// Mark the feature unsupported, logging at most once per process.
pub fn note_unsupported(reason: &str) {
    let first = ADAPTER_POWER_CIRCUIT.trip();
    if first && ADAPTER_POWER_CIRCUIT.should_log_once() {
        log::info!(
            target: "hw::adapter_power",
            "AC adapter power not supported by this hardware ({reason}) — \
             feature disabled for this session, no further probes will be made"
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
    // Circuit first: no WMI, no bridge, no helper process.
    if ADAPTER_POWER_CIRCUIT.is_open() {
        return Ok(AdapterPowerReading::unsupported(
            "cached: WMI class missing on this hardware",
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
                ADAPTER_POWER_CIRCUIT.note_supported();
            } else {
                note_unsupported(
                    reading
                        .reason
                        .as_deref()
                        .unwrap_or("bridge reported unsupported"),
                );
            }
            Ok(reading)
        }
        Err(message) => match classify_failure(&message) {
            AdapterPowerFailure::HardwareUnsupported => {
                note_unsupported("WBEM_E_NOT_FOUND (0x80041002)");
                // Not an error: the answer is "this machine has no such class".
                Ok(AdapterPowerReading::unsupported(
                    "WBEM_E_NOT_FOUND (0x80041002): WMI class not present",
                ))
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
        // Exactly what tauri-app.log showed 115 times.
        assert_eq!(
            classify_failure("Hardware error: 0x80041002"),
            AdapterPowerFailure::HardwareUnsupported
        );
        assert_eq!(
            classify_failure("0x80041002"),
            AdapterPowerFailure::HardwareUnsupported
        );
        assert_eq!(
            classify_failure("WMI cache: wmi transient query error: 0x80041002"),
            AdapterPowerFailure::HardwareUnsupported
        );
        assert_eq!(
            classify_failure("Hardware error: 0X80041002"),
            AdapterPowerFailure::HardwareUnsupported
        );
    }

    #[test]
    fn classifies_invalid_class_and_signed_decimal_forms() {
        assert_eq!(
            classify_failure("WMI query failed: 0x80041010"),
            AdapterPowerFailure::HardwareUnsupported
        );
        assert_eq!(
            classify_failure(&format!("HRESULT {}", WBEM_NOT_FOUND as i32)),
            AdapterPowerFailure::HardwareUnsupported
        );
        assert_eq!(
            classify_failure("WBEM_E_NOT_FOUND"),
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
    fn circuit_trips_once_then_stays_open() {
        let circuit = UnsupportedCircuit::new();
        assert!(!circuit.is_open());
        assert!(circuit.trip(), "first trip is the transition");
        assert!(circuit.is_open());
        assert!(!circuit.trip(), "second trip is not a transition");
        assert!(circuit.is_open());
    }

    #[test]
    fn circuit_logs_only_once() {
        let circuit = UnsupportedCircuit::new();
        assert!(circuit.should_log_once());
        assert!(!circuit.should_log_once());
        assert!(!circuit.should_log_once());
    }

    #[test]
    fn circuit_can_be_reset_for_tests() {
        let circuit = UnsupportedCircuit::new();
        circuit.trip();
        circuit.should_log_once();
        circuit.reset();
        assert!(!circuit.is_open());
        assert!(circuit.should_log_once());
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
