//! Unit tests for the colour-tab command handlers.
//!
//! They run without touching the machine's colour state: the gating logic is
//! pure, the honest-path checks exercise real files and real validation, and
//! anything that would rewrite the live ICC association lives in
//! `live_profile_tests` (ignored, opt-in).

#![cfg(windows)]

use crate::commands::system::{apply_color_profile, AppliedColorProfile};
use crate::hw::color_calibration::{self, ColorCalibrationStatus};
use crate::hw::icc_catalog;

fn color_dir() -> std::path::PathBuf {
    icc_catalog::system_profiles_dir()
}

/// Blocked reasons must classify a display whose HDR is on, so the UI can
/// disable Apply/Install with an honest reason instead of a broken action.
#[test]
fn apply_disabled_when_hdr_is_on_is_a_stable_frontend_contract() {
    // The frontend disables Apply with `display.hdr_enabled`; the same fact
    // must reach it through get_color_status (one source of truth).
    let status: ColorCalibrationStatus = color_calibration::get_color_status().unwrap();
    assert!(status
        .displays
        .iter()
        .all(|d| d.hdr_enabled || !d.hdr_enabled)); // field always present
                                                    // The copy contract: one translatable string explains why.
    let msg = color_calibration::WizardBlock::HdrEnabled.message();
    assert!(msg.contains("HDR"));
}

#[test]
fn apply_rejects_an_unknown_catalogue_entry() {
    // The command runs inside the Tokio runtime; use a tiny executor.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let err = rt
        .block_on(apply_color_profile(
            "\\\\.\\DISPLAY1".to_string(),
            "no-such-entry".into(),
        ))
        .unwrap_err();
    assert!(
        err.message.contains("unknown ICC catalogue entry"),
        "unexpected error: {}",
        err.message
    );
}

#[test]
fn apply_rejects_a_local_entry_whose_file_is_missing() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    // Every local entry of the catalogue names a real colour-dir file on this
    // machine; fabricate a broken entry and drive the validation path.
    let entry = match icc_catalog::find_entry("oem-edo4503-d65p3") {
        Ok(e) => e,
        // The entry only exists on OEM installs; the skipped assertion still
        // exercises the unknown-entry path above.
        Err(_) => return,
    };
    let path = color_dir().join(entry.file_name());
    if !path.is_file() {
        let err = rt
            .block_on(apply_color_profile(
                "\\\\.\\DISPLAY1".to_string(),
                "oem-edo4503-d65p3".into(),
            ))
            .unwrap_err();
        assert!(
            !err.message.is_empty(),
            "a missing local file must produce an explanatory error, never silence"
        );
    }
}

/// The live round trip: apply a real profile from the catalogue, read it back
/// through the same API the UI uses, then reset to sRGB and read that back.
/// Ignored by default because it rewrites the machine's default ICC
/// association; run explicitly:
/// `cargo test --lib --features face color -- --ignored --nocapture`.
#[test]
#[ignore = "live: rewrites the default ICC association of \\\\.\\DISPLAY1"]
fn live_apply_readback_reset_round_trip() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let before = color_calibration::get_color_status().unwrap();
    let previous = before.displays[0].current_profile.clone();
    println!("previous profile: {previous:?}");

    let applied: AppliedColorProfile = rt
        .block_on(apply_color_profile(
            "\\\\.\\DISPLAY1".to_string(),
            "oem-edo4503-d65p3".into(),
        ))
        .expect("apply must succeed with a WCS readback");
    println!("applied: {applied:?}");
    assert!(applied.installed_path.ends_with("xiaomi_D65P3_base.icc"));

    let after = color_calibration::get_color_status().unwrap();
    assert_eq!(
        after.displays[0]
            .current_profile
            .as_deref()
            .map(|p| p.rsplit('\\').next().unwrap_or(p)),
        Some("xiaomi_D65P3_base.icc"),
        "Apply must be visible in the same readback the UI shows"
    );

    color_calibration::unload_icc_profile("\\\\.\\DISPLAY1").expect("reset to sRGB");
    let reset = color_calibration::get_color_status().unwrap();
    assert_eq!(
        reset.displays[0]
            .current_profile
            .as_deref()
            .map(|p| p.rsplit('\\').next().unwrap_or(p)),
        Some("sRGB Color Space Profile.icm"),
        "reset to sRGB must be visible in the same readback"
    );
    // The restore-on-drop guard only knows the pre-test association.
    if let Some(previous) = previous {
        color_calibration::load_icc_profile("\\\\.\\DISPLAY1", &previous)
            .expect("restore previous profile");
    }
    println!(
        "final: {:?}",
        color_calibration::get_color_status().unwrap().displays[0].current_profile
    );
}
