//! Audio effects — user preferences with honest capability reporting.
//!
//! ## What this module does
//!
//! Stores three user preferences (mic noise canceling, speaker noise canceling,
//! voice focus) in the **signed-in user's** hive and reports them back together
//! with what the machine can actually do.
//!
//! ## Why it is HKCU and in-process
//!
//! Older builds dispatched these setters through `elev_bridge`, whose first hop
//! is the `MiControlBridge` service running as `LocalSystem`. Every
//! `HKEY_CURRENT_USER` write therefore landed in `HKU\S-1-5-18\...` while
//! `get_audio_effects` ran in the (non-elevated) app process and read the user's
//! own `HKCU`. Result: the UI always showed all three toggles off. The writes
//! are now in-process and the values left behind by older builds are migrated
//! once, on read (`migrate_legacy_system_effects`).
//!
//! ## Why the toggles are labelled "saved preference", not "applied"
//!
//! `applies_to_audio` is `false`. There is no public, per-effect Win32 control
//! for endpoint noise suppression:
//!
//! * `PKEY_AudioEndpoint_Disable_SysFx` is **not** a noise-canceling switch; it
//!   disables *every* system effect on the endpoint (beamforming, AGC, the
//!   Realtek/Dolby sAPO chain). Writing it while labelling the switch "noise
//!   canceling on" would silently degrade capture quality, so it is
//!   deliberately not used here.
//! * `HKCU\...\CurrentVersion\AudioControls\NoiseSuppression` (written by older
//!   builds) is invented — no Windows component reads it. That write has been
//!   removed; the stale value is left untouched because deleting a value under
//!   a Windows-owned key is riskier than leaving an unread DWORD behind.
//! * Windows Voice Clarity / Studio Effects are toggled by the OS, per device,
//!   through Settings; MiControl can only *detect* them.
//!
//! The UI reads `applies_to_audio` and labels the card accordingly. When a real
//! implementation lands (endpoint APO selection + measurement), flip the
//! constant and the UI stops apologising.

use crate::hw::errors::{HardwareError, HardwareResult};
use serde::{Deserialize, Serialize};

/// Registry key (relative to the user's hive) that stores the toggles.
#[cfg(any(windows, test))]
const AUDIO_EFFECTS_REG_KEY: &str = r"SOFTWARE\MiControl\AudioEffects";

/// Legacy store left behind by pre-fix builds: the setters were elevated, so
/// the same relative path was created under the SYSTEM profile hive
/// (`HKU\S-1-5-18`). Read-only here — never written again.
#[cfg(any(windows, test))]
const LEGACY_SYSTEM_SUBKEY: &str = r"S-1-5-18\SOFTWARE\MiControl\AudioEffects";

/// Value name marking that the one-time SYSTEM-hive migration already ran.
#[cfg(any(windows, test))]
const MIGRATION_MARKER: &str = "MigratedFromSystemHive";

/// Registry value names.
const MIC_NC_VALUE: &str = "MicNoiseCanceling";
const SPK_NC_VALUE: &str = "SpeakerNoiseCanceling";
const VOICE_FOCUS_VALUE: &str = "VoiceFocus";

#[cfg(any(windows, test))]
const EFFECT_COUNT: usize = 3;

/// All persisted preference value names, in a fixed order so the read/write
/// helpers and the migration planner can index them consistently.
#[cfg(any(windows, test))]
const EFFECT_VALUES: [&str; EFFECT_COUNT] = [MIC_NC_VALUE, SPK_NC_VALUE, VOICE_FOCUS_VALUE];

/// First Windows 11 build with the Voice Clarity / AudioControls plumbing.
const MIN_VOICE_CLARITY_BUILD: u32 = 22621;

/// Device class exposed by the OS for neural processors ("Neural processors").
/// Matched by class *name* rather than by GUID so a changed class GUID cannot
/// produce a false negative, and so no unverifiable GUID is hardcoded.
const NPU_DEVICE_CLASS_NAME: &str = "ComputeAccelerator";

/// Whether MiControl applies these preferences to the audio pipeline.
///
/// `false` in this build — see the module docs for why. The frontend uses this
/// flag to label the card as stored preferences instead of applied effects.
pub const APPLIES_TO_AUDIO: bool = false;

// Compile-time guard: flipping `APPLIES_TO_AUDIO` requires editing this line
// too, so nobody can quietly relabel the toggles as "applied" without a real
// implementation behind them.
const _: () = assert!(!APPLIES_TO_AUDIO);

/// AI noise cancellation status.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AudioEffectsStatus {
    /// Mic noise canceling preference (saved, not applied).
    pub mic_noise_canceling: bool,
    /// Speaker noise canceling preference (saved, not applied).
    pub speaker_noise_canceling: bool,
    /// Voice focus preference (saved, not applied).
    pub voice_focus: bool,
    /// Whether the OS *and* the hardware look capable of Voice Clarity.
    pub voice_clarity_available: bool,
    /// Machine-readable reason behind `voice_clarity_available`:
    /// `detected` | `os-unsupported` | `no-npu` | `unknown-os`.
    pub voice_clarity_reason: String,
    /// Whether the preferences above reach the audio pipeline at all.
    pub applies_to_audio: bool,
}

// ── Voice Clarity capability ─────────────────────────────────────────────────

/// Outcome of the Voice Clarity capability probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VoiceClaritySupport {
    /// OS build supports it and a neural processor is present.
    Detected,
    /// Windows build predates the feature.
    OsUnsupported,
    /// OS is new enough but no ComputeAccelerator device is installed.
    NoNpu,
    /// The OS build could not be read.
    UnknownOs,
}

impl VoiceClaritySupport {
    fn available(self) -> bool {
        matches!(self, Self::Detected)
    }

    fn reason(self) -> &'static str {
        match self {
            Self::Detected => "detected",
            Self::OsUnsupported => "os-unsupported",
            Self::NoNpu => "no-npu",
            Self::UnknownOs => "unknown-os",
        }
    }
}

/// Pure decision table for Voice Clarity support.
///
/// Kept separate from the registry probes so it can be unit-tested without
/// depending on the machine it runs on.
fn evaluate_voice_clarity(build: Option<u32>, npu_present: bool) -> VoiceClaritySupport {
    match build {
        None => VoiceClaritySupport::UnknownOs,
        Some(build) if build < MIN_VOICE_CLARITY_BUILD => VoiceClaritySupport::OsUnsupported,
        Some(_) if !npu_present => VoiceClaritySupport::NoNpu,
        Some(_) => VoiceClaritySupport::Detected,
    }
}

/// Device-instance subkeys under a class key are named `0000`, `0001`, …;
/// helper subkeys such as `Properties` are not instances.
fn has_device_instance(subkeys: &[String]) -> bool {
    subkeys
        .iter()
        .any(|name| !name.is_empty() && name.chars().all(|c| c.is_ascii_digit()))
}

// ── Migration planning ───────────────────────────────────────────────────────

/// Decide which legacy values still need copying into the user's hive.
///
/// Rule: an existing user value always wins — a legacy value only fills a gap
/// the user has never written. That is what makes the migration safe to retry
/// on every launch: turning a toggle off writes an explicit `0` which a stale
/// SYSTEM `1` can never overwrite.
///
/// Returns the values to write (same order as [`EFFECT_VALUES`]) and how many
/// of them are actually present.
#[cfg(any(windows, test))]
fn plan_migration(
    existing: [Option<u32>; EFFECT_COUNT],
    legacy: [Option<u32>; EFFECT_COUNT],
) -> ([Option<u32>; EFFECT_COUNT], usize) {
    let mut copy = [None; EFFECT_COUNT];
    let mut count = 0;
    for index in 0..EFFECT_COUNT {
        if let (None, Some(value)) = (existing[index], legacy[index]) {
            copy[index] = Some(value);
            count += 1;
        }
    }
    (copy, count)
}

/// Interpret a persisted DWORD as an on/off preference.
#[cfg(any(windows, test))]
fn effect_enabled(value: Option<u32>) -> bool {
    value.is_some_and(|v| v != 0)
}

// ── Public read path ─────────────────────────────────────────────────────────

/// Get the current audio effects status (saved preferences + capability).
pub fn get_audio_effects() -> HardwareResult<AudioEffectsStatus> {
    #[cfg(windows)]
    {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_CURRENT_USER;

        // Idempotent, marker-guarded: recovers the state older builds stranded
        // in the SYSTEM hive before reporting it, so the first paint of the
        // card matches what the user configured historically.
        migrate_legacy_system_effects();

        let values = RegKeyGuard::open_read(HKEY_CURRENT_USER, AUDIO_EFFECTS_REG_KEY)
            .ok()
            .flatten()
            .map(|key| read_values(&key))
            .unwrap_or([None; EFFECT_COUNT]);

        let clarity = detect_voice_clarity();

        Ok(AudioEffectsStatus {
            mic_noise_canceling: effect_enabled(values[0]),
            speaker_noise_canceling: effect_enabled(values[1]),
            voice_focus: effect_enabled(values[2]),
            voice_clarity_available: clarity.available(),
            voice_clarity_reason: clarity.reason().to_string(),
            applies_to_audio: APPLIES_TO_AUDIO,
        })
    }
    #[cfg(not(windows))]
    {
        Ok(AudioEffectsStatus {
            mic_noise_canceling: false,
            speaker_noise_canceling: false,
            voice_focus: false,
            voice_clarity_available: false,
            voice_clarity_reason: "os-unsupported".to_string(),
            applies_to_audio: APPLIES_TO_AUDIO,
        })
    }
}

// ── Public write path ────────────────────────────────────────────────────────

/// Set the mic noise canceling preference.
///
/// Saved per user; not applied to the audio pipeline (see module docs).
pub fn set_mic_noise_canceling(enabled: bool) -> HardwareResult<()> {
    persist_audio_effect(MIC_NC_VALUE, enabled)?;
    log::info!(
        target: "hw::audio_effects",
        "Mic noise canceling preference saved as {} (not applied to the audio pipeline)",
        enabled
    );
    Ok(())
}

/// Set the speaker noise canceling preference.
///
/// Saved per user; not applied to the audio pipeline (see module docs).
pub fn set_speaker_noise_canceling(enabled: bool) -> HardwareResult<()> {
    persist_audio_effect(SPK_NC_VALUE, enabled)?;
    log::info!(
        target: "hw::audio_effects",
        "Speaker noise canceling preference saved as {} (not applied to the audio pipeline)",
        enabled
    );
    Ok(())
}

/// Set the voice focus preference.
///
/// Saved per user; not applied to the audio pipeline (see module docs).
pub fn set_voice_focus(enabled: bool) -> HardwareResult<()> {
    persist_audio_effect(VOICE_FOCUS_VALUE, enabled)?;
    log::info!(
        target: "hw::audio_effects",
        "Voice focus preference saved as {} (not applied to the audio pipeline)",
        enabled
    );
    Ok(())
}

/// Persist an audio effect setting to the **current user's** registry hive.
///
/// Must stay in-process (never routed through the elevated bridge): the bridge
/// runs as LocalSystem, so an elevated `HKEY_CURRENT_USER` write lands in
/// `HKU\S-1-5-18` and is invisible to [`get_audio_effects`].
#[cfg(windows)]
fn persist_audio_effect(name: &str, enabled: bool) -> HardwareResult<()> {
    use crate::util::registry::RegKeyGuard;
    use windows::Win32::System::Registry::HKEY_CURRENT_USER;

    let key = RegKeyGuard::create_write(HKEY_CURRENT_USER, AUDIO_EFFECTS_REG_KEY)
        .map_err(|e| HardwareError::Registry(format!("Create audio effects key: {e}")))?;

    key.write_u32(name, if enabled { 1 } else { 0 })
        .map_err(|e| HardwareError::Registry(format!("Write {name}: {e}")))?;

    Ok(())
}

#[cfg(not(windows))]
fn persist_audio_effect(_name: &str, _enabled: bool) -> HardwareResult<()> {
    Ok(())
}

// ── Windows helpers ──────────────────────────────────────────────────────────

#[cfg(windows)]
fn read_values(key: &crate::util::registry::RegKeyGuard) -> [Option<u32>; EFFECT_COUNT] {
    let mut values = [None; EFFECT_COUNT];
    for (index, name) in EFFECT_VALUES.iter().enumerate() {
        values[index] = key.read_u32(name).ok().flatten();
    }
    values
}

/// Copy the three preferences stranded in the SYSTEM hive into the user's hive.
///
/// * Runs at most once (marker value [`MIGRATION_MARKER`]).
/// * Never overwrites a value the user already has — see [`plan_migration`].
/// * A blocked/failed read of the legacy key leaves the marker unset so a later
///   launch retries; because only gaps are filled, a retry has no side effects.
#[cfg(windows)]
fn migrate_legacy_system_effects() {
    use crate::util::registry::RegKeyGuard;
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_USERS};

    let Ok(dst) = RegKeyGuard::create_write(HKEY_CURRENT_USER, AUDIO_EFFECTS_REG_KEY) else {
        log::warn!(
            target: "hw::audio_effects",
            "Migration skipped: cannot open the user's AudioEffects key"
        );
        return;
    };
    if dst.read_u32(MIGRATION_MARKER).ok().flatten().is_some() {
        return;
    }

    let Ok(Some(src)) = RegKeyGuard::open_read(HKEY_USERS, LEGACY_SYSTEM_SUBKEY) else {
        // Nothing to migrate, or the SYSTEM hive is not readable in this
        // context. The marker stays unset so a later launch can still recover
        // the values; re-running is harmless because only gaps are filled.
        return;
    };

    let (copy, count) = plan_migration(read_values(&dst), read_values(&src));
    for index in 0..EFFECT_COUNT {
        let Some(value) = copy[index] else { continue };
        if let Err(e) = dst.write_u32(EFFECT_VALUES[index], value) {
            log::warn!(
                target: "hw::audio_effects",
                "Migration of {} failed: {e}; will retry next launch",
                EFFECT_VALUES[index]
            );
            return;
        }
    }

    let _ = dst.write_u32(MIGRATION_MARKER, 1);
    if count > 0 {
        log::info!(
            target: "hw::audio_effects",
            "Migrated {count} audio effect preference(s) from the SYSTEM hive into HKCU"
        );
    } else {
        log::info!(
            target: "hw::audio_effects",
            "No audio effect preferences needed migrating from the SYSTEM hive"
        );
    }
}

/// Probe Voice Clarity support once per process.
#[cfg(windows)]
fn detect_voice_clarity() -> VoiceClaritySupport {
    use std::sync::OnceLock;

    static CACHE: OnceLock<VoiceClaritySupport> = OnceLock::new();
    *CACHE.get_or_init(|| {
        let support = evaluate_voice_clarity(windows_build_number(), npu_present());
        log::info!(
            target: "hw::audio_effects",
            "Voice Clarity capability probe: {}",
            support.reason()
        );
        support
    })
}

/// OS build number from `HKLM\...\CurrentVersion\CurrentBuildNumber`.
#[cfg(windows)]
fn windows_build_number() -> Option<u32> {
    use crate::util::registry::RegKeyGuard;
    use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;

    let key = RegKeyGuard::open_read(
        HKEY_LOCAL_MACHINE,
        r"SOFTWARE\Microsoft\Windows NT\CurrentVersion",
    )
    .ok()
    .flatten()?;

    key.read_string("CurrentBuildNumber")
        .ok()
        .flatten()?
        .trim()
        .parse()
        .ok()
}

/// Whether any device of the `ComputeAccelerator` class is installed.
///
/// Scans `HKLM\SYSTEM\CurrentControlSet\Control\Class` for the class key, then
/// requires at least one device instance (`0000`, `0001`, …) — an empty class
/// key only means the OS knows the class, not that an NPU is present.
#[cfg(windows)]
fn npu_present() -> bool {
    use crate::util::registry::RegKeyGuard;
    use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;

    const CLASS_ROOT: &str = r"SYSTEM\CurrentControlSet\Control\Class";

    let Ok(Some(classes)) = RegKeyGuard::open_read(HKEY_LOCAL_MACHINE, CLASS_ROOT) else {
        return false;
    };
    let Ok(class_keys) = classes.enum_subkeys() else {
        return false;
    };

    for class_key_name in class_keys {
        let path = format!(r"{CLASS_ROOT}\{class_key_name}");
        let Ok(Some(class_key)) = RegKeyGuard::open_read(HKEY_LOCAL_MACHINE, &path) else {
            continue;
        };
        let is_npu_class = class_key
            .read_string("Class")
            .ok()
            .flatten()
            .is_some_and(|class| class.eq_ignore_ascii_case(NPU_DEVICE_CLASS_NAME));
        if !is_npu_class {
            continue;
        }
        if class_key
            .enum_subkeys()
            .map(|subkeys| has_device_instance(&subkeys))
            .unwrap_or(false)
        {
            return true;
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Migration ────────────────────────────────────────────────────────────

    #[test]
    fn migration_fills_an_empty_user_store_from_the_system_hive() {
        let (copy, count) = plan_migration([None, None, None], [Some(1), Some(0), Some(1)]);

        assert_eq!(count, 3);
        assert_eq!(copy, [Some(1), Some(0), Some(1)]);
    }

    #[test]
    fn migration_never_overwrites_an_explicit_user_preference() {
        // The regression this guards: the user turns an effect OFF after the
        // fix; a stale SYSTEM 1 must not resurrect it on the next launch.
        let (copy, count) =
            plan_migration([Some(0), Some(0), Some(0)], [Some(1), Some(1), Some(1)]);

        assert_eq!(count, 0);
        assert_eq!(copy, [None, None, None]);
    }

    #[test]
    fn migration_only_fills_the_missing_slots() {
        let (copy, count) = plan_migration([Some(0), None, Some(1)], [Some(1), Some(1), Some(0)]);

        assert_eq!(count, 1);
        assert_eq!(copy, [None, Some(1), None]);
    }

    #[test]
    fn migration_is_a_no_op_without_a_legacy_store() {
        let (copy, count) = plan_migration([None, None, None], [None, None, None]);

        assert_eq!(count, 0);
        assert_eq!(copy, [None, None, None]);
    }

    #[test]
    fn effect_values_are_read_as_booleans() {
        assert!(!effect_enabled(None));
        assert!(!effect_enabled(Some(0)));
        assert!(effect_enabled(Some(1)));
        assert!(effect_enabled(Some(0xFFFF_FFFF)));
    }

    #[test]
    fn preferences_are_stored_in_the_user_hive_only() {
        // Guard against re-routing the setters through the elevated bridge:
        // the write path is `HKEY_CURRENT_USER` + this relative path, while the
        // SYSTEM hive appears only as the read-only legacy source.
        assert!(AUDIO_EFFECTS_REG_KEY.starts_with(r"SOFTWARE\MiControl\"));
        assert!(!AUDIO_EFFECTS_REG_KEY.contains("S-1-5-18"));
        assert!(LEGACY_SYSTEM_SUBKEY.starts_with(r"S-1-5-18\"));
    }

    // ── Voice Clarity capability ─────────────────────────────────────────────

    #[test]
    fn voice_clarity_requires_a_new_enough_os_and_an_npu() {
        assert_eq!(
            evaluate_voice_clarity(Some(26200), true),
            VoiceClaritySupport::Detected
        );
        assert_eq!(
            evaluate_voice_clarity(Some(26200), false),
            VoiceClaritySupport::NoNpu
        );
        assert_eq!(
            evaluate_voice_clarity(Some(19045), true),
            VoiceClaritySupport::OsUnsupported
        );
        assert_eq!(
            evaluate_voice_clarity(None, true),
            VoiceClaritySupport::UnknownOs
        );
    }

    #[test]
    fn voice_clarity_is_only_available_when_detected() {
        assert!(VoiceClaritySupport::Detected.available());
        assert!(!VoiceClaritySupport::NoNpu.available());
        assert!(!VoiceClaritySupport::OsUnsupported.available());
        assert!(!VoiceClaritySupport::UnknownOs.available());

        assert_eq!(VoiceClaritySupport::Detected.reason(), "detected");
        assert_eq!(VoiceClaritySupport::NoNpu.reason(), "no-npu");
    }

    // ── NPU probe ────────────────────────────────────────────────────────────

    #[test]
    fn device_instances_are_numeric_subkeys_only() {
        assert!(has_device_instance(&[
            "0000".to_string(),
            "Properties".to_string()
        ]));
        assert!(has_device_instance(&["0001".to_string()]));
        assert!(!has_device_instance(&["Properties".to_string()]));
        assert!(!has_device_instance(&[]));
        assert!(!has_device_instance(&["0a00".to_string()]));
    }

    // ── Live registry probes (read-only) ─────────────────────────────────────

    /// Proves the `CurrentBuildNumber` read path works against a real hive —
    /// the previous detection looked up a key that never existed.
    #[cfg(windows)]
    #[test]
    fn os_build_probe_reads_a_plausible_build_number() {
        let build = windows_build_number().expect("CurrentBuildNumber must be readable");
        assert!(build >= 10240, "implausible Windows build number: {build}");
    }
}
