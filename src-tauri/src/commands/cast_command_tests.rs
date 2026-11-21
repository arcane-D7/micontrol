//! Unit tests for the cast-tab command handlers.
//!
//! They exercise the real handlers through the Tokio runtime without
//! assuming any projection hardware: on this machine the honest answers are
//! empty endpoint lists and refused programmatic casts, and the contract is
//! that those answers arrive STRUCTURED (never a fake success).

#![cfg(windows)]

use crate::commands::hardware::{get_cast_devices, get_cast_state, probe_cast_capabilities};
use crate::hw::screen_cast::{
    casting_error_status_name, merge_devices, probe_conclusion, state_from_devices, CastDevice,
};

fn row(id: &str, connected: Option<bool>) -> CastDevice {
    CastDevice {
        name: id.to_string(),
        id: id.to_string(),
        device_type: "miracast".into(),
        connected,
    }
}

/// `get_cast_devices` must succeed and carry the connected field on every
/// row: the frontend chip renders straight from it and a missing field would
/// silently render "unknown" everywhere.
#[tokio::test]
async fn get_cast_devices_rows_carry_the_connected_field() {
    let devices = get_cast_devices().await.expect("enumeration succeeds");
    let json = serde_json::to_value(&devices).expect("rows serialise");
    for row in json.as_array().expect("array") {
        assert!(
            row.get("connected").is_some(),
            "every row must carry connected: {row}"
        );
    }
}

/// The state summary and the list must agree on the same endpoint source:
/// active=true only when some row reports connected=true.
#[tokio::test]
async fn get_cast_state_agrees_with_the_device_list() {
    let devices = get_cast_devices().await.expect("enumeration succeeds");
    let state = get_cast_state().await.expect("state succeeds");
    let any_connected = devices.iter().any(|d| d.connected == Some(true));
    match state.active {
        Some(true) => assert!(
            any_connected,
            "state says active but no endpoint reports connected"
        ),
        Some(false) => assert!(
            !devices.is_empty() && !any_connected,
            "state says inactive but the list disagrees"
        ),
        None => assert!(devices.is_empty(), "state says none but endpoints exist"),
    }
}

/// The capability probe is the honesty contract: it must run, carry per-API
/// evidence, and (without a media source path) deny screen mirroring with
/// the Win+K fallback named.
#[tokio::test]
async fn probe_structured_and_honest() {
    let report = probe_cast_capabilities()
        .await
        .expect("probe runs on this build");
    assert!(
        !report.probes.is_empty(),
        "the probe must carry per-API evidence"
    );
    // Every evidence line is non-empty and typed.
    for p in &report.probes {
        assert!(!p.api.is_empty());
        assert!(!p.status.is_empty());
        assert!(["ok", "failed", "refused", "skipped"].contains(&p.status.as_str()));
    }
    if !report.screen_mirroring_by_api {
        assert!(
            report.conclusion.contains("Win+K"),
            "the denied conclusion must name the fallback: {}",
            report.conclusion
        );
    }
}

/// State mapping: the three UI states are distinct and none collapses into a
/// fake "no session" when there is simply nothing to read.
#[test]
fn state_mapping_is_exact() {
    let none = state_from_devices(&[], None);
    assert_eq!(none.active, None);

    let inactive = state_from_devices(&[row("a", Some(false))], None);
    assert_eq!(inactive.active, Some(false));

    let active = state_from_devices(&[row("a", Some(true))], None);
    assert_eq!(active.active, Some(true));
    assert_eq!(active.device_name.as_deref(), Some("a"));
}

/// Merge dedupes by id and never lets a state-less row overwrite a read.
#[test]
fn merge_keeps_first_state_and_dedupes() {
    let merged = merge_devices(vec![
        vec![row("x", None)],
        vec![row("x", Some(true)), row("y", Some(false))],
    ]);
    assert_eq!(merged.len(), 2);
    assert_eq!(
        merged.iter().find(|d| d.id == "x").unwrap().connected,
        Some(true)
    );
}

/// The refusal vocabulary stays stable: the UI and the probe report string-
/// match these names.
#[test]
fn error_status_vocabulary() {
    assert_eq!(casting_error_status_name(0), "Succeeded");
    assert_eq!(casting_error_status_name(5), "InvalidCastingSource");
    assert_eq!(casting_error_status_name(99), "Unknown");
}

/// The conclusion flips ONLY on a real Succeeded answer from Windows.
#[test]
fn conclusion_requires_windows_consent() {
    assert!(!probe_conclusion(Some("InvalidCastingSource")).0);
    assert!(!probe_conclusion(None).0);
    assert!(probe_conclusion(Some("Succeeded")).0);
}
