//! Unit tests for the central error catalog.
//!
//! Pins the catalog invariants the JS gate cannot check from the Rust side:
//! enum/catalog parity through `spec()`, unique ids, id/i18n-key formats,
//! the full `HardwareError` mapping and the `AppError` Display contract.

use super::catalog::*;
use crate::hw::errors::{HardwareError, RegionBlockKind};

/// Ids must be `domain.snake_case`: exactly one dot, both halves non-empty
/// lowercase snake_case without leading/trailing/double underscores.
fn assert_valid_id(id: &str) {
    let parts: Vec<&str> = id.split('.').collect();
    assert_eq!(parts.len(), 2, "id must be domain.snake, got: {id}");
    for part in parts {
        assert!(!part.is_empty(), "empty segment in id: {id}");
        assert!(
            part.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "id segment must be lowercase snake_case, got: {id}"
        );
        assert!(
            !part.starts_with('_') && !part.ends_with('_') && !part.contains("__"),
            "id segment must not have stray underscores: {id}"
        );
    }
}

#[test]
fn catalog_ids_are_unique() {
    let mut seen = std::collections::HashSet::new();
    for spec in CATALOG {
        assert!(seen.insert(spec.id), "duplicate catalog id: {}", spec.id);
    }
}

#[test]
fn catalog_id_and_i18n_key_format() {
    for spec in CATALOG {
        assert_valid_id(spec.id);
        assert_eq!(
            spec.i18n_key,
            format!("errors.{}", spec.id),
            "i18n_key must be errors.<id> for {}",
            spec.id
        );
        assert!(spec.i18n_key.starts_with("errors."));
    }
}

#[test]
fn every_code_resolves_to_its_catalog_entry() {
    for spec in CATALOG {
        let resolved = spec.code.spec();
        assert_eq!(resolved.code, spec.code, "spec() resolved the wrong code");
        assert_eq!(resolved.id, spec.id);
        assert_eq!(resolved.i18n_key, spec.i18n_key);
    }
}

#[test]
fn catalog_holds_one_entry_per_code() {
    let mut seen = std::collections::HashSet::new();
    for spec in CATALOG {
        assert!(
            seen.insert(spec.code),
            "code listed twice in CATALOG: {:?}",
            spec.code
        );
    }
    // No magic number: two branches adding codes at once made a pinned size
    // fail after a clean union. The variant/row parity is enforced from the
    // source text by `scripts/check-error-catalog.mjs` (it parses the enum and
    // the CATALOG and fails when they disagree); this test only proves the
    // no-duplicates invariant above.
    assert!(!CATALOG.is_empty(), "CATALOG must not be empty");
}

/// One sample per `HardwareError` variant. `HardwareError::code()` is an
/// exhaustive match, so a new variant fails compilation until it is mapped;
/// the `samples.len()` assertion then forces the mapping test to be updated.
fn samples() -> Vec<HardwareError> {
    vec![
        HardwareError::WmiQuery {
            query: "SELECT 1".into(),
            source: "boom".into(),
        },
        HardwareError::WmiConnection("namespace".into()),
        HardwareError::Io(std::io::Error::other("device gone")),
        HardwareError::Hid("bad report".into()),
        HardwareError::InvalidConfig("brightness=999".into()),
        HardwareError::Timeout("ecram poll".into()),
        HardwareError::NotSupported("no ambient sensor".into()),
        HardwareError::Ecram("offset out of bounds".into()),
        HardwareError::EcramBlocked {
            region: "ERAM".into(),
            kind: RegionBlockKind::DriverBlocked,
            detail: "open IoT driver device".into(),
        },
        HardwareError::EcramBlocked {
            region: "ERAM".into(),
            kind: RegionBlockKind::ServiceUnavailable,
            detail: "pipe not answering".into(),
        },
        HardwareError::EcramBlocked {
            region: "SMA2".into(),
            kind: RegionBlockKind::Unknown,
            detail: "short answer".into(),
        },
        HardwareError::Wifi("invalid ssid".into()),
        HardwareError::Cast("no displays".into()),
        HardwareError::Display("igcl init".into()),
        HardwareError::Touchpad("gesture parse".into()),
        HardwareError::Battery("status read".into()),
        HardwareError::Hotkey("register failed".into()),
        HardwareError::Registry("access denied".into()),
        HardwareError::ElevatedBridge("dispatch refused".into()),
        HardwareError::AiRequest("http 500".into()),
        HardwareError::HotkeyConfig("serialize failed".into()),
        HardwareError::TaskJoin("cancelled".into()),
        HardwareError::Wmi("method call".into()),
        HardwareError::Other("uncategorized".into()),
    ]
}

#[test]
fn hardware_error_maps_every_variant_to_a_distinct_code() {
    let samples = samples();
    assert_eq!(
        samples.len(),
        24,
        "HardwareError variant count changed; update this mapping test"
    );
    let codes: Vec<ErrorCode> = samples.iter().map(|e| e.code()).collect();
    let mut unique = codes.clone();
    unique.sort_by_key(|c| c.as_str());
    unique.dedup();
    assert_eq!(
        unique.len(),
        codes.len(),
        "two variants share one ErrorCode"
    );
}

#[test]
fn ecram_blocked_dispatches_on_region_block_kind() {
    let blocked = |kind| HardwareError::EcramBlocked {
        region: "ERAM".into(),
        kind,
        detail: "detail".into(),
    };
    assert_eq!(
        blocked(RegionBlockKind::DriverBlocked).code(),
        ErrorCode::EcramDriverBlocked
    );
    assert_eq!(
        blocked(RegionBlockKind::ServiceUnavailable).code(),
        ErrorCode::EcramServiceUnavailable
    );
    assert_eq!(
        blocked(RegionBlockKind::Unknown).code(),
        ErrorCode::EcramUnknown
    );
}

#[test]
fn legacy_wire_codes_are_preserved() {
    // The frontend switches on these strings; they must never drift.
    assert_eq!(ErrorCode::WmiQuery.as_str(), "wmi_query");
    assert_eq!(
        ErrorCode::EcramDriverBlocked.as_str(),
        "ecram_driver_blocked"
    );
    assert_eq!(
        ErrorCode::EcramServiceUnavailable.as_str(),
        "ecram_service_unavailable"
    );
    assert_eq!(ErrorCode::Other.as_str(), "other");
    assert_eq!(ErrorCode::SpawnTimeout.as_str(), "spawn_timeout");
    assert_eq!(format!("{}", ErrorCode::Timeout), "timeout");
}

#[test]
fn app_error_display_contains_id_and_context() {
    let err = AppError::new(ErrorCode::EcramServiceUnavailable, "pipe connect failed");
    let display = err.to_string();
    assert!(
        display.contains("ecram.service_unavailable"),
        "Display must contain the catalog id, got: {display}"
    );
    assert!(
        display.contains("pipe connect failed"),
        "Display must contain the context, got: {display}"
    );
    assert!(std::error::Error::source(&err).is_none());

    let with_src = AppError::with_source(
        ErrorCode::Io,
        "read os turbo recovery",
        Box::new(std::io::Error::other("denied")),
    );
    let display = with_src.to_string();
    assert!(display.contains("io.failure"), "got: {display}");
    assert!(display.contains("read os turbo recovery"), "got: {display}");
    assert!(
        display.contains("denied"),
        "Display should include the source, got: {display}"
    );
    assert!(std::error::Error::source(&with_src).is_some());
}

#[test]
fn from_hardware_error_carries_code_and_context() {
    let app: AppError = HardwareError::Timeout("elevated bridge".into()).into();
    assert_eq!(app.code, ErrorCode::Timeout);
    assert!(
        app.context.contains("elevated bridge"),
        "context keeps the message, got: {app}"
    );

    let expected = HardwareError::Ecram("short read".into());
    let expected_msg = expected.to_string();
    let app: AppError = expected.into();
    assert_eq!(app.code, ErrorCode::Ecram);
    assert_eq!(app.context, expected_msg);
}

#[test]
fn app_error_new_uses_the_catalog() {
    // Construction resolves the code through the catalog: the spec lookup
    // must agree with the catalog row for a handful of codes across the
    // enum (a missing row would panic here).
    for code in [
        ErrorCode::WmiQuery,
        ErrorCode::EcramDriverBlocked,
        ErrorCode::EcramServiceUnavailable,
        ErrorCode::SpawnTimeout,
        ErrorCode::StoreCorrupt,
        ErrorCode::PolicyInvalid,
        ErrorCode::WatchdogRejected,
        ErrorCode::Other,
    ] {
        let err = AppError::new(code, "smoke");
        assert_eq!(err.code.spec().code, code);
        assert!(err.code.spec().id.contains('.'));
        assert!(err.code.spec().i18n_key.starts_with("errors."));
    }
}
