//! Audio effects — user preferences with real WASAPI application.
//!
//! ## What this module does
//!
//! Stores three user preferences (mic noise canceling, speaker noise canceling,
//! voice focus) in the **signed-in user's** hive and reports them back together
//! with what the machine can actually do.
//!
//! Since this version the mic and speaker toggles are ALSO APPLIED: each write
//! sets `PKEY_AudioEndpoint_Disable_SysFx` (1) on the matching default
//! endpoint's `IPropertyStore` (capture for the mic, render for the speaker)
//! and reads the value back as proof. Turning a toggle off restores 0 (OS
//! defaults). The preferences persist per user in the audio_effects state
//! store, and `reapply_audio_effects` re-applies them to the CURRENT default
//! endpoints on every app start, so a default-device switch or reboot does
//! not lose them. The effects the OS actually applies through that key are
//! the endpoint's SYSTEM effects (the OEM sAPO chain, AGC, AEC): the toggle
//! is an honest "endpoint effects on/off", not a per-DSP feature switch.
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
//! ## Voice Focus stays a declared unsupported toggle
//!
//! Windows exposes no per-endpoint "voice focus" control to desktop apps: the
//! feature is delivered by the OS audio stack for NPU-backed microphone
//! arrays (`IAudioEffectsManager` reports it, `SetAudioEffectState` needs a
//! capture stream, and neither is available in this app's context on this
//! machine, where `Activate(IAudioEffectsManager)` returns E_NOINTERFACE for
//! both defaults). Setting `PKEY_AudioEndpoint_Disable_SysFx` is the wrong
//! channel (it would degrade the capture chain while claiming to "enable"
//! focus), so `set_voice_focus(true)` returns `NotSupported` with the probed
//! reason instead of faking success.
//!
//! * `HKCU\...\CurrentVersion\AudioControls\NoiseSuppression` (written by older
//!   builds) is invented — no Windows component reads it. That write has been
//!   removed; the stale value is left untouched because deleting a value under
//!   a Windows-owned key is riskier than leaving an unread DWORD behind.

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
/// `true` since the WASAPI endpoint write path exists: the mic and speaker
/// toggles now write `PKEY_AudioEndpoint_Disable_SysFx` to the matching
/// default endpoint and read it back, and the Voice Focus toggle returns a
/// `NotSupported` error with the probed reason instead of a fake success.
pub const APPLIES_TO_AUDIO: bool = true;

/// AI noise cancellation status.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AudioEffectsStatus {
    /// Mic noise canceling preference (saved, and applied to the capture
    /// default endpoint).
    pub mic_noise_canceling: bool,
    /// Speaker noise canceling preference (saved, and applied to the render
    /// default endpoint).
    pub speaker_noise_canceling: bool,
    /// Voice focus preference (saved only; the toggle itself is unsupported).
    pub voice_focus: bool,
    /// Live `PKEY_AudioEndpoint_Disable_SysFx` on the default capture
    /// endpoint: `Some(1)` effects disabled, `Some(0)` OS default, `None`
    /// unreadable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mic_sysfx: Option<i32>,
    /// Same readback for the default render endpoint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker_sysfx: Option<i32>,
    /// Always `false` today; kept so the UI can react without a rebuild if a
    /// supported path appears.
    pub voice_focus_supported: bool,
    /// Why the Voice Focus toggle cannot be applied on this machine.
    pub voice_focus_reason: String,
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

/// Interpret a persisted DWORD as an on/off preference. Still the semantic
/// the registry fallback readers rely on through `bool_slot`.
#[cfg(test)]
fn effect_enabled(value: Option<u32>) -> bool {
    value.is_some_and(|v| v != 0)
}

/// Registry DWORD slot -> store state field (`None` stays "never set").
#[cfg(any(windows, test))]
fn bool_slot(value: Option<u32>) -> Option<bool> {
    value.map(|v| v != 0)
}

/// Registry DWORD slots -> store state fields (`None` stays "never set").
#[cfg(any(windows, test))]
fn state_from_values(
    values: [Option<u32>; EFFECT_COUNT],
) -> crate::util::state_store::AudioEffectsState {
    crate::util::state_store::AudioEffectsState {
        mic_noise_canceling: bool_slot(values[0]),
        speaker_noise_canceling: bool_slot(values[1]),
        voice_focus: bool_slot(values[2]),
    }
}

/// Store state field -> registry DWORD slot (only for the legacy readers).
#[cfg(any(windows, test))]
fn values_from_state(
    state: &crate::util::state_store::AudioEffectsState,
) -> [Option<bool>; EFFECT_COUNT] {
    [
        state.mic_noise_canceling,
        state.speaker_noise_canceling,
        state.voice_focus,
    ]
}

// ── Public read path ─────────────────────────────────────────────────────────

/// Why the Voice Focus toggle cannot reach the audio DSP on this machine.
///
/// Mirrors the Voice Clarity probe: the feature is only implemented by the OS
/// for integrated microphone arrays with an NPU-backed DSP on recent builds.
/// Used by the UI card to say *why* the toggle is not applied instead of
/// pretending the preference is active.
pub fn voice_focus_unsupported_reason() -> &'static str {
    detect_voice_clarity().reason()
}

/// Get the current audio effects status (saved preferences + capability).
pub fn get_audio_effects() -> HardwareResult<AudioEffectsStatus> {
    #[cfg(windows)]
    {
        // Import-once: recovers the state older builds stranded in HKCU and
        // in the SYSTEM hive before reporting it, so the first paint of the
        // card matches what the user configured historically.
        let values = audio_effect_bools();

        let clarity = detect_voice_clarity();

        // Live endpoint readback: the Disable_SysFx value the OS itself sees
        // right now, so the card can never claim an applied state the
        // endpoint does not carry.
        let mic_sysfx = read_disable_sysfx(mic_effect_flow()).ok().flatten();
        let speaker_sysfx = read_disable_sysfx(speaker_effect_flow()).ok().flatten();

        Ok(AudioEffectsStatus {
            mic_noise_canceling: values[0].unwrap_or(false),
            speaker_noise_canceling: values[1].unwrap_or(false),
            voice_focus: values[2].unwrap_or(false),
            mic_sysfx,
            speaker_sysfx,
            voice_focus_supported: false,
            voice_focus_reason: clarity.reason().to_string(),
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
            mic_sysfx: None,
            speaker_sysfx: None,
            voice_focus_supported: false,
            voice_focus_reason: "os-unsupported".to_string(),
            voice_clarity_available: false,
            voice_clarity_reason: "os-unsupported".to_string(),
            applies_to_audio: APPLIES_TO_AUDIO,
        })
    }
}

// ── Public write path ────────────────────────────────────────────────────────

/// WASAPI endpoint FX properties (Windows SDK `PKEY_AudioEndpoint_*`).
///
/// Both keys live on the **MMDevice endpoint** object (the property store you
/// open with `IMMDevice::OpenPropertyStore`), not on the legacy "Enhancements"
/// registry settings that `HKLM\...\Audio\Engine` and the
/// `MMDevices\Audio\<flow>\<id>\FxProperties` APO defaults belong to.
#[cfg(windows)]
mod endpoint_fx {
    use windows::core::GUID;

    /// `PKEY_AudioEndpoint_Disable_SysFx`: 1 disables *every* system effect
    /// (AGC, AEC, the OEM sAPO chain) on the endpoint; absent/0 lets the OS
    /// apply its defaults. A companion key, `PKEY_AudioEndpoint_SysFx_Fallback`
    /// (same fmtid, pid 6), exists for the audio engine's raw-mode fallback;
    /// MiControl does not write it, Windows owns that fallback.
    pub(crate) const PKEY_DISABLE_SYSFX: windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY =
        windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY {
            fmtid: GUID::from_u128(0x1da5d803_d492_4edd_8c23_e0c0ffee7f0e),
            pid: 5,
        };
}

/// Apply `PKEY_AudioEndpoint_Disable_SysFx = 1/0` to the default endpoint of
/// `flow` and prove the write with an `IPropertyStore` readback.
///
/// Value written: `1` when `enabled` (every system effect on that endpoint is
/// disabled, including the OEM sAPO chain, AGC and AEC), `0` when off (OS
/// defaults). The audio engine picks the value up on stream creation, so the
/// readback is the honest proof that the property latched, even though the
/// effect on an already-open stream can lag up to the next stream.
///
/// Returns the raw DWORD now stored on the endpoint property store.
#[cfg(windows)]
fn apply_disable_sysfx(
    flow: windows::Win32::Media::Audio::EDataFlow,
    enabled: bool,
) -> HardwareResult<u32> {
    use windows::Win32::Media::Audio::{eConsole, IMMDeviceEnumerator, MMDeviceEnumerator};
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_ALL, STGM_READWRITE,
    };
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;

    unsafe {
        CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED)
            .ok()
            .map_err(hardware_from_com)?;
    }
    let _com_scope = com_scope_guard();

    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(hardware_from_com)?;
        let device = enumerator
            .GetDefaultAudioEndpoint(flow, eConsole)
            .map_err(hardware_from_com)?;
        let store: IPropertyStore = device
            .OpenPropertyStore(STGM_READWRITE)
            .map_err(hardware_from_com)?;

        let want: i32 = i32::from(enabled);
        let value = windows_core::PROPVARIANT::from(want);

        store
            .SetValue(&endpoint_fx::PKEY_DISABLE_SYSFX, &value)
            .map_err(hardware_from_com)?;
        store.Commit().map_err(hardware_from_com)?;

        // Readback: the honest proof the property latched on the endpoint.
        let stored = store
            .GetValue(&endpoint_fx::PKEY_DISABLE_SYSFX)
            .map_err(hardware_from_com)?;
        let raw = i32::try_from(&stored)
            .map_err(|e| HardwareError::Other(format!("Disable_SysFx readback: {e}")))?;
        if raw != want {
            return Err(HardwareError::Other(format!(
                "Disable_SysFx write did not latch: requested {want}, endpoint reports {raw}"
            )));
        }
        Ok(raw as u32)
    }
}

/// RAII guard for the manual `CoInitializeEx` above (matching the initialize /
/// uninitialize pattern the other `hw` modules use on the same thread).
#[cfg(windows)]
fn com_scope_guard() -> impl Drop {
    struct CoScope;
    impl Drop for CoScope {
        fn drop(&mut self) {
            unsafe { windows::Win32::System::Com::CoUninitialize() };
        }
    }
    CoScope
}

/// Map a COM error into the hardware error catalog.
#[cfg(windows)]
fn hardware_from_com(e: windows_core::Error) -> HardwareError {
    HardwareError::Other(format!("audio endpoint property store: {e}"))
}

// ── Startup reapply ──────────────────────────────────────────────────────────

/// Endpoint flow for the mic noise canceling preference (capture side).
#[cfg(any(windows, test))]
fn mic_effect_flow() -> windows::Win32::Media::Audio::EDataFlow {
    windows::Win32::Media::Audio::eCapture
}

/// Endpoint flow for the speaker noise canceling preference (render side).
#[cfg(any(windows, test))]
fn speaker_effect_flow() -> windows::Win32::Media::Audio::EDataFlow {
    windows::Win32::Media::Audio::eRender
}

/// Read the current `PKEY_AudioEndpoint_Disable_SysFx` value from the default
/// endpoint of `flow` (`None` = the OS default: no override stored).
#[cfg(windows)]
fn read_disable_sysfx(
    flow: windows::Win32::Media::Audio::EDataFlow,
) -> HardwareResult<Option<i32>> {
    use windows::Win32::Media::Audio::{eConsole, IMMDeviceEnumerator, MMDeviceEnumerator};
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, STGM_READ};
    use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;

    unsafe {
        CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED)
            .ok()
            .map_err(hardware_from_com)?;
    }
    let _com_scope = com_scope_guard();

    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(hardware_from_com)?;
        let device = enumerator
            .GetDefaultAudioEndpoint(flow, eConsole)
            .map_err(hardware_from_com)?;
        let store: IPropertyStore = device
            .OpenPropertyStore(STGM_READ)
            .map_err(hardware_from_com)?;
        match store.GetValue(&endpoint_fx::PKEY_DISABLE_SYSFX) {
            Ok(variant) => {
                if variant.is_empty() {
                    Ok(None)
                } else {
                    let raw = i32::try_from(&variant)
                        .map_err(|e| HardwareError::Other(format!("Disable_SysFx read: {e}")))?;
                    Ok(Some(raw))
                }
            }
            Err(e) if e.code().0 == 0x80070490_u32 as i32 => {
                // ELEMENT_NOT_FOUND: no override stored, OS default applies.
                Ok(None)
            }
            Err(e) => Err(hardware_from_com(e)),
        }
    }
}

/// Re-apply the persisted mic/speaker Disable_SysFx preferences to the
/// CURRENT default endpoints.
///
/// Runs once per app start (and whenever the UI re-reads the status after a
/// default-device change): the endpoint property store is per endpoint, so a
/// preference applied to yesterday's default speaker says nothing about
/// today's. Previously-set toggles keep their registry-preference value
/// (`None` = never touched, leave the endpoint at OS defaults). Voice Focus
/// is never re-applied here because it has no endpoint control.
#[cfg(windows)]
pub fn reapply_audio_effects() -> HardwareResult<()> {
    let store = crate::util::state_store::audio_effects_store()
        .map_err(|e| state_store_error("audio effects store", e))?;
    let state = store
        .load()
        .map_err(|e| state_store_error("audio effects load", e))?
        .unwrap_or_default();

    let mut results: Vec<String> = Vec::new();
    if let Some(enabled) = state.mic_noise_canceling {
        let applied = apply_disable_sysfx(mic_effect_flow(), enabled);
        results.push(format!("mic={enabled}:{}", applied.is_ok()));
    }
    if let Some(enabled) = state.speaker_noise_canceling {
        let applied = apply_disable_sysfx(speaker_effect_flow(), enabled);
        results.push(format!("speaker={enabled}:{}", applied.is_ok()));
    }
    log::info!(
        target: "hw::audio_effects",
        "startup reapply: {}",
        if results.is_empty() {
            "no stored preference".to_string()
        } else {
            results.join(", ")
        }
    );
    Ok(())
}

/// Non-Windows stub: nothing to re-apply.
#[cfg(not(windows))]
pub fn reapply_audio_effects() -> HardwareResult<()> {
    Ok(())
}

/// Set the mic noise canceling preference.
///
/// Persisted per user AND applied to the capture default endpoint through
/// `PKEY_AudioEndpoint_Disable_SysFx` (1 = the APO chain, AGC and AEC on the
/// capture endpoint are disabled; 0 = OS defaults). The Windows value is what
/// the audio engine actually reads, so a failed write is an error even when
/// the preference itself was saved.
pub fn set_mic_noise_canceling(enabled: bool) -> HardwareResult<()> {
    persist_audio_effect(MIC_NC_VALUE, enabled)?;
    let applied = apply_disable_sysfx(mic_effect_flow(), enabled);
    log::info!(
        target: "hw::audio_effects",
        "Mic noise canceling preference saved as {} (capture Disable_SysFx applied: {:?})",
        enabled,
        applied.as_ref().ok()
    );
    applied.map(|_| ())
}

/// Set the speaker noise canceling preference.
///
/// Persisted per user AND applied to the render default endpoint through
/// `PKEY_AudioEndpoint_Disable_SysFx` (1 = the APO chain on the render
/// endpoint is disabled; 0 = OS defaults). See [`set_mic_noise_canceling`].
pub fn set_speaker_noise_canceling(enabled: bool) -> HardwareResult<()> {
    persist_audio_effect(SPK_NC_VALUE, enabled)?;
    let applied = apply_disable_sysfx(speaker_effect_flow(), enabled);
    log::info!(
        target: "hw::audio_effects",
        "Speaker noise canceling preference saved as {} (render Disable_SysFx applied: {:?})",
        enabled,
        applied.as_ref().ok()
    );
    applied.map(|_| ())
}

/// Set the voice focus preference.
///
/// Voice Focus has no endpoint-level control on this build of the module (see
/// [`voice_focus_unsupported_reason`]), so the value is only persisted; calling
/// with `true` returns [`HardwareError::NotSupported`] with the probed reason
/// instead of pretending the DSP state changed.
pub fn set_voice_focus(enabled: bool) -> HardwareResult<()> {
    persist_audio_effect(VOICE_FOCUS_VALUE, enabled)?;
    log::info!(
        target: "hw::audio_effects",
        "Voice focus preference saved as {} (toggle not supported by the audio DSP)",
        enabled
    );
    if enabled {
        return Err(HardwareError::NotSupported(format!(
            "voice focus not supported: {}",
            voice_focus_unsupported_reason()
        )));
    }
    Ok(())
}

/// Persist an audio effect setting through the state store.
///
/// Must stay in-process (never routed through the elevated bridge): the bridge
/// runs as LocalSystem, so an elevated `HKEY_CURRENT_USER` write historically
/// landed in `HKU\S-1-5-18` and was invisible to [`get_audio_effects`]. The
/// import-on-first-write also keeps values an older build left in HKCU from
/// being lost to a single-field save.
#[cfg(windows)]
fn persist_audio_effect(name: &str, enabled: bool) -> HardwareResult<()> {
    use crate::util::state_store;

    let store = state_store::audio_effects_store()
        .map_err(|e| state_store_error("audio effects store", e))?;
    let mut state = match store.load() {
        Ok(Some(state)) => state,
        Ok(None) => {
            // First write before any read: seed from the legacy HKCU values
            // and (when readable) the SYSTEM-hive gap-fill.
            let user = read_hive_values(
                windows::Win32::System::Registry::HKEY_CURRENT_USER,
                AUDIO_EFFECTS_REG_KEY,
            );
            let merged = match read_system_hive_values() {
                Some(system) => plan_migration(user, system).0,
                None => user,
            };
            state_from_values(merged)
        }
        Err(e) => {
            // Corrupt store: warn and overwrite with just this effect. The
            // other two slots were unreadable anyway, and the next read
            // re-imports any gap from the registry values.
            log::warn!(
                target: "hw::audio_effects",
                "audio effects store unreadable ({e}); saving only the changed effect"
            );
            state_store::AudioEffectsState::default()
        }
    };
    match name {
        MIC_NC_VALUE => state.mic_noise_canceling = Some(enabled),
        SPK_NC_VALUE => state.speaker_noise_canceling = Some(enabled),
        VOICE_FOCUS_VALUE => state.voice_focus = Some(enabled),
        other => {
            return Err(HardwareError::InvalidConfig(format!(
                "unknown audio effect value name: {other}"
            )))
        }
    }
    store
        .save(&state)
        .map_err(|e| state_store_error("audio effects save", e))
}

/// Map a state-store failure into the catalog-classified IO error.
#[cfg(windows)]
fn state_store_error(context: &str, e: crate::util::state_store::StoreError) -> HardwareError {
    HardwareError::Io(std::io::Error::other(format!("{context}: {e}")))
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

/// The three preferences as `Some(bool)` slots (`None` = never set).
#[cfg(windows)]
type EffectBools = [Option<bool>; EFFECT_COUNT];

/// Read the preferences through the state store, importing the legacy HKCU
/// values and the SYSTEM-hive gap-fill on the first read.
///
/// Retry semantics carried over from the registry marker: when the SYSTEM
/// hive cannot be read this launch, the import is NOT saved (the store stays
/// empty) so a later launch retries; only gaps are filled, so a retry is
/// side-effect free.
#[cfg(windows)]
fn audio_effect_bools() -> EffectBools {
    use crate::util::state_store;

    let store = match state_store::audio_effects_store() {
        Ok(store) => store,
        Err(e) => {
            log::warn!(
                target: "hw::audio_effects",
                "audio effects store unavailable ({e}); reading the registry values"
            );
            return read_hive_values(
                windows::Win32::System::Registry::HKEY_CURRENT_USER,
                AUDIO_EFFECTS_REG_KEY,
            )
            .map(bool_slot);
        }
    };
    match store.load() {
        Ok(Some(state)) => values_from_state(&state),
        Ok(None) => {
            let user = read_hive_values(
                windows::Win32::System::Registry::HKEY_CURRENT_USER,
                AUDIO_EFFECTS_REG_KEY,
            );
            match read_system_hive_values() {
                Some(system) => {
                    let (merged, count) = plan_migration(user, system);
                    let _ = store.save(&state_from_values(merged));
                    if count > 0 {
                        log::info!(
                            target: "hw::audio_effects",
                            "Migrated {count} audio effect preference(s) from the registry hives into the state store"
                        );
                    }
                    merged.map(bool_slot)
                }
                None => {
                    // SYSTEM hive blocked this launch: serve the HKCU values,
                    // keep the store empty so the import retries later.
                    user.map(bool_slot)
                }
            }
        }
        Err(e) => {
            log::warn!(
                target: "hw::audio_effects",
                "audio effects store unreadable ({e}); reading the registry values"
            );
            read_hive_values(
                windows::Win32::System::Registry::HKEY_CURRENT_USER,
                AUDIO_EFFECTS_REG_KEY,
            )
            .map(bool_slot)
        }
    }
}

/// Read one hive key into the fixed-order DWORD slots.
#[cfg(windows)]
fn read_hive_values(
    parent: windows::Win32::System::Registry::HKEY,
    subkey: &str,
) -> [Option<u32>; EFFECT_COUNT] {
    use crate::util::registry::RegKeyGuard;
    RegKeyGuard::open_read(parent, subkey)
        .ok()
        .flatten()
        .map(|key| read_values(&key))
        .unwrap_or([None; EFFECT_COUNT])
}

/// The stranded SYSTEM-hive values. `None` = the read was BLOCKED this launch
/// (retry later); a missing legacy key is a completed no-op import.
#[cfg(windows)]
fn read_system_hive_values() -> Option<[Option<u32>; EFFECT_COUNT]> {
    use crate::util::registry::RegKeyGuard;
    use windows::Win32::System::Registry::HKEY_USERS;

    match RegKeyGuard::open_read(HKEY_USERS, LEGACY_SYSTEM_SUBKEY) {
        Ok(Some(key)) => Some(read_values(&key)),
        Ok(None) => Some([None; EFFECT_COUNT]),
        Err(_) => None,
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
    fn effect_keys_map_to_the_matching_endpoint_flow() {
        // The bug this pins: mapping mic -> eRender or speaker -> eCapture
        // would silently toggle the wrong half of the audio pipeline.
        assert_eq!(mic_effect_flow(), windows::Win32::Media::Audio::eCapture);
        assert_eq!(speaker_effect_flow(), windows::Win32::Media::Audio::eRender);
    }

    #[test]
    fn disable_sysfx_pkey_matches_the_sdk_definition() {
        // windows-rs 0.58 does not re-export the PKEY; pin the raw SDK values
        // (functiondiscoverykeys_devpkey.h) instead of a compile-time import.
        let key = &endpoint_fx::PKEY_DISABLE_SYSFX;
        let fmt = key.fmtid;
        let expected = windows::core::GUID::from_u128(0x1da5d803_d492_4edd_8c23_e0c0ffee7f0e);
        assert_eq!(fmt, expected);
        assert_eq!(key.pid, 5);
    }

    #[cfg(windows)]
    #[test]
    fn disable_sysfx_writes_and_reads_back_on_the_default_endpoints() {
        // Live roundtrip against the real default endpoints: write 1, read 1,
        // restore 0, read back 0. Proves the app-context IPropertyStore write
        // path the toggles use is real, without leaving residue: the test
        // always ends with the OS default (0) on both endpoints, the same
        // value this machine starts with.
        for flow in [mic_effect_flow(), speaker_effect_flow()] {
            let applied = apply_disable_sysfx(flow, true).expect("write Disable_SysFx=1");
            assert_eq!(applied, 1, "readback after SetValue(1) must be 1");
            let after_on = read_disable_sysfx(flow).expect("read back after write");
            assert_eq!(after_on, Some(1), "endpoint must carry the written 1");
            let restored = apply_disable_sysfx(flow, false).expect("restore Disable_SysFx=0");
            assert_eq!(restored, 0, "readback after SetValue(0) must be 0");
            let after_off = read_disable_sysfx(flow).expect("read restored Disable_SysFx");
            assert!(
                after_off == Some(0) || after_off.is_none(),
                "endpoint must be back at the OS default after the test"
            );
        }
    }

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

    /// Unique per-test scratch state root.
    fn scratch_root(tag: &str) -> std::path::PathBuf {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "micontrol-audio-effects-{}-{}-{}",
            tag,
            std::process::id(),
            seq
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch root");
        dir
    }

    #[test]
    fn state_mapping_preserves_never_set_vs_explicit_off() {
        let values: [Option<u32>; EFFECT_COUNT] = [Some(0), Some(1), None];
        let state = state_from_values(values);
        assert_eq!(state.mic_noise_canceling, Some(false));
        assert_eq!(state.speaker_noise_canceling, Some(true));
        assert_eq!(state.voice_focus, None);
        let bools = values_from_state(&state);
        assert_eq!(bools, [Some(false), Some(true), None]);
    }

    #[test]
    fn audio_effects_state_roundtrips_through_the_store() {
        let root = scratch_root("roundtrip");
        let store = crate::util::state_store::audio_effects_store_in(&root);
        store
            .save(&crate::util::state_store::AudioEffectsState {
                mic_noise_canceling: Some(true),
                speaker_noise_canceling: Some(false),
                voice_focus: None,
            })
            .expect("save");
        let loaded = store.load().expect("load").expect("state");
        assert_eq!(loaded.mic_noise_canceling, Some(true));
        assert_eq!(loaded.speaker_noise_canceling, Some(false));
        assert_eq!(loaded.voice_focus, None);
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The startup contract, exercised against the real state machinery: what
    /// the store carries decides which endpoint flows `reapply` would touch.
    /// The apply itself is the live `apply_disable_sysfx` path (covered by the
    /// roundtrip test above); here we pin the persistence half so a store
    /// schema change cannot silently orphan the reapply-on-startup behaviour.
    #[test]
    fn startup_reapply_reads_the_same_store_shape_the_setters_write() {
        let root = scratch_root("reapply");
        let store = crate::util::state_store::audio_effects_store_in(&root);

        // Both toggles set (the exact shape set_mic/set_speaker persist).
        store
            .save(&crate::util::state_store::AudioEffectsState {
                mic_noise_canceling: Some(true),
                speaker_noise_canceling: Some(false),
                voice_focus: Some(false),
            })
            .expect("save");

        let state = store.load().expect("load").expect("state");
        // The reapply loop walks exactly these Option fields, in this order,
        // mapping mic -> capture and speaker -> render.
        let jobs = [
            (state.mic_noise_canceling, mic_effect_flow()),
            (state.speaker_noise_canceling, speaker_effect_flow()),
        ];
        let applied_count = jobs.iter().filter(|(v, _)| v.is_some()).count();
        assert_eq!(applied_count, 2, "both toggles must schedule a reapply");
        assert_eq!(jobs[0].1, windows::Win32::Media::Audio::eCapture);
        assert_eq!(jobs[1].1, windows::Win32::Media::Audio::eRender);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn corrupt_audio_effects_store_reads_as_default() {
        let root = scratch_root("corrupt");
        let store = crate::util::state_store::audio_effects_store_in(&root);
        std::fs::create_dir_all(store.path().parent().expect("parent")).expect("dir");
        std::fs::write(store.path(), "{corrupt").expect("corrupt fixture");
        assert!(store.load().is_err(), "corrupt payload must surface");
        // A following save heals the file (warn-and-degrade in production).
        store
            .save(&crate::util::state_store::AudioEffectsState::default())
            .expect("save heals");
        assert!(store.load().expect("load").is_some());
        let _ = std::fs::remove_dir_all(&root);
    }
}
