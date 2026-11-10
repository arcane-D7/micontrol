//! Fan control and temperature monitoring.
//!
//! Reads fan speed, CPU/GPU temperature, and power from WMI
//! and Intel ESIF/DPTF, with support for setting fan modes.

use crate::hw::errors::{HardwareError, HardwareResult};
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::sync::{Mutex as StdMutex, OnceLock};
use std::time::{Duration as StdDuration, Instant as StdInstant};

/// Where a temperature reading came from. Lets the UI distinguish "sensor not
/// available on this platform" from "sensor exists but read failed".
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum TempSource {
    /// Intel ESIF/DPTF (`EsifDeviceInformation`) — real CPU/GPU die sensors.
    Esif,
    /// A non-ambient ACPI thermal zone. Never `TZ00` (ambient on this platform).
    Acpi,
    /// EC RAM block via the `ecram_service` pipe.
    EcRam,
    /// No source produced a plausible reading.
    #[default]
    Unavailable,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FanInfo {
    pub mode: FanMode,
    /// Measured fan RPM — `None` when the platform exposes **no** RPM sensor.
    ///
    /// This is the normal state here: `Win32_Fan.DesiredSpeed` comes back empty
    /// and every EC RAM read is denied (`Access denied` / IOCTL failure, even
    /// as SYSTEM). `None` is deliberately not `0`: "0 RPM" reads as "the fan is
    /// stopped", a claim we cannot make.
    pub speed_rpm: Option<u32>,
    /// Measured fan speed as a percentage of full scale, derived from
    /// [`FanInfo::speed_rpm`]. `None` whenever `speed_rpm` is `None` — the
    /// registry value is a *request*, not a measurement, and must never be
    /// drawn as a gauge.
    pub speed_percent: Option<u8>,
    /// Percentage last requested by the user (`HKLM\SOFTWARE\MI\FanControl`,
    /// value `FixedSpeed`). Always present. This is what the slider shows; it is
    /// not evidence of the current fan speed.
    pub requested_speed_percent: u8,
    /// GPU temperature in Celsius. None when no sensor is available
    /// (ESIF/DPTF driver absent and ACPI thermal zone unavailable).
    pub gpu_temp_celsius: Option<f32>,
    /// CPU temperature in Celsius. None when no sensor is available.
    ///
    /// The ACPI ambient zone is **never** used as a CPU temperature: `TZ00`
    /// read 27.85 °C while ESIF reported 57 °C on the CPU hotspot, so falling
    /// back to it would show the room as the processor.
    pub cpu_temp_celsius: Option<f32>,
    /// Ambient/chassis temperature (`TZ00`), when ACPI exposes it. This is the
    /// only thing `TZ00` is reported as.
    pub ambient_temp_celsius: Option<f32>,
    /// CPU package power from Intel ESIF/DPTF (EsifDeviceInformation._0 Power
    /// field), in watts. The raw WMI value is in deciwatts (×0.1 W). None when
    /// the DPTF driver is absent or reports zero.
    pub tdp_watts: Option<f32>,
    /// Which source produced [`FanInfo::cpu_temp_celsius`].
    #[serde(default)]
    pub temp_source: TempSource,
    /// The EC's **active** performance mode, read back from the firmware
    /// (`WMAA 0x0800 / 0`) — the truth, not the stored preference.
    ///
    /// `None` when the EC read failed (elevated bridge unavailable). Never
    /// substitute the registry value here: on this machine the registry said
    /// "Balance" for hours while the EC sat at QFAN=9.
    #[serde(default)]
    pub ec_active: Option<crate::hw::wmi_ec::EcActiveMode>,
    /// Raw `FRD0` byte behind [`FanInfo::ec_active`]. `0` means "the EC is in a
    /// mode this ACPI channel cannot name", not "Silence".
    #[serde(default)]
    pub ec_active_raw: Option<u16>,
    /// OS-reported lid state at the time of the reading.
    ///
    /// `Some(false)` = lid closed. The EC switches to a **clamshell fan table**
    /// on this platform (the fan ramps within ~1 s of the lid closing while
    /// CPU/GPU power and temperatures stay flat), and that table is not
    /// reachable from software: no WMAA function writes fan duty, IGCL
    /// enumerates no fans and the EC performance-mode writes are no-ops. This
    /// field exists so the UI can *explain* the fan instead of hiding it, and
    /// so logs can attribute a fan report to lid state. `None` = the platform
    /// did not report the lid switch.
    #[serde(default)]
    pub lid_open: Option<bool>,
}

/// What `set_fan_mode` actually did — as opposed to what the mode is called.
///
/// The old API returned `Ok(())` unconditionally, so the UI showed "Fan mode
/// set" even when every hardware path had failed. This carries the real
/// outcome, without changing which hardware writes are attempted.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct FanModeResult {
    /// The mode was persisted and at least one hardware path accepted it.
    /// `true` does **not** mean the fan speed changed — see `rpm_control`.
    pub applied: bool,
    /// What actually ran, e.g. `registry+ec_profile`, `registry+ec_profile+igcl`,
    /// or `registry` when no hardware path accepted the mode.
    pub method: String,
    /// `true` only when an IGCL fan handle was programmed with a fixed speed —
    /// i.e. RPM were really pinned. Always `false` on this platform (IGCL
    /// enumerates no fans), which is why "Fixed Speed" cannot fix RPM here.
    pub rpm_control: bool,
    /// Honest caveat to show next to the result, when there is one.
    pub warning: Option<String>,
    /// EC mode register (`QFAN`) read back *before* the write. `None` = the
    /// readback was unavailable, so nothing can be claimed either way.
    #[serde(default)]
    pub ec_mode_before: Option<u16>,
    /// EC mode register read back *after* the write.
    #[serde(default)]
    pub ec_mode_after: Option<u16>,
    /// The write was **proven** to latch the EC register (see
    /// [`crate::hw::wmi_ec::qfan_write_verified`]). This is the only field the
    /// UI may use to say "applied" about the EC.
    #[serde(default)]
    pub ec_verified: bool,
}

impl FanModeResult {
    /// Build the result from the outcomes of the individual hardware paths.
    pub fn new(
        mode: &FanMode,
        ec_profile: &str,
        ec_applied: bool,
        rpm_control: bool,
        ec_mode_before: Option<u16>,
        ec_mode_after: Option<u16>,
    ) -> Self {
        let mut method = String::from("registry");
        if ec_applied {
            method.push_str("+ec_profile");
        }
        if rpm_control {
            method.push_str("+igcl");
        }
        let readback = match (ec_mode_before, ec_mode_after) {
            (Some(b), Some(a)) => format!("the EC reported {b} before the write and {a} after"),
            (_, Some(a)) => format!("the EC reports {a}"),
            _ => "the EC did not answer the readback".to_string(),
        };
        let warning = match mode {
            FanMode::Fixed if !rpm_control => Some(format!(
                "Fixed speed requested, but this platform exposes no RPM control \
                 (IGCL enumerates no fans): the EC fan profile '{ec_profile}' was \
                 applied instead. Fan RPM are not pinned."
            )),
            FanMode::Fixed => None,
            FanMode::Off => Some(
                "This platform cannot stop the fan: the EC was set to Super Quiet \
                 (minimum profile). The fan keeps running."
                    .to_string(),
            ),
            FanMode::Auto if !ec_applied => Some(format!(
                "The requested EC profile '{ec_profile}' did not latch: {readback}. \
                 Only the stored setting changed — the before/after values above \
                 come from the firmware's own mode register, not from the setting."
            )),
            FanMode::Auto => None,
        };
        Self {
            applied: ec_applied || rpm_control,
            method,
            rpm_control,
            warning,
            ec_mode_before,
            ec_mode_after,
            ec_verified: ec_applied,
        }
    }

    /// Parse the elevated helper's reply.
    ///
    /// An older helper (pre-0.2.21) answers `null` for `set_fan_mode`. Rather
    /// than turning that into a hard UI error, report it plainly: the registry
    /// write is what the old code guaranteed, and there is no readback.
    pub fn from_bridge(value: &serde_json::Value) -> Self {
        match serde_json::from_value::<Self>(value.clone()) {
            Ok(result) => result,
            Err(_) => Self {
                applied: true,
                method: "registry".to_string(),
                rpm_control: false,
                warning: Some(
                    "The elevated helper did not report the fan result (older build); \
                                 only the stored setting is confirmed."
                        .to_string(),
                ),
                ec_mode_before: None,
                ec_mode_after: None,
                ec_verified: false,
            },
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FanMode {
    Auto,
    Fixed,
    Off,
}

const FAN_REG_KEY: &str = r"SOFTWARE\MI\FanControl";
const FAN_REG_MODE: &str = "FanMode";
const FAN_REG_SPEED: &str = "FixedSpeed";

// ── ESIF thermal readings ─────────────────────────────────────────────────
//
// EsifDeviceInformation (ROOT\WMI) is populated by the Intel DPTF/ESIF driver.
// Participants _0/_1/_2 track the CPU hotspot; _10 tracks the GPU/secondary SoC
// domain. The Power field is in deciwatts (×0.1 W); Temperature is Celsius.
// One WMI query returns all participants, so we read temps and TDP together.

#[derive(Clone, Debug)]
pub struct EsifReadings {
    pub cpu_temp: Option<f32>,
    pub gpu_temp: Option<f32>,
    pub tdp_watts: Option<f32>,
    /// Chassis/ambient temperature from the ACPI `TZ00` zone, when available.
    /// Kept separate on purpose: it is real data, but it is not the CPU.
    pub ambient_temp: Option<f32>,
    /// Which source produced `cpu_temp`.
    pub cpu_source: TempSource,
}

fn get_esif_readings() -> HardwareResult<EsifReadings> {
    #[cfg(windows)]
    {
        use crate::hw::wmi_cache;
        use crate::util::wmi_extract;
        use std::collections::HashMap;
        use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
        use std::sync::OnceLock;
        use std::time::{SystemTime, UNIX_EPOCH};

        // S37-005: Failure cache. The unprivileged MiControl process is DENIED
        // access to the EsifDeviceInformation WMI class (HRESULT 0x80041003),
        // so this query is *expected* to fail on every poll. Before the cache,
        // each 2 s fan poll issued a doomed WMI round-trip AND logged a WARN —
        // flooding the log and wasting a WMI connection. We remember a failure
        // and short-circuit afterwards.
        //
        // P11: ACCESS_DENIED is not a bug — it simply means the elevated path
        // is not usable yet (or the process is unprivileged by design). It is
        // logged ONCE at INFO and the retry window grows 60 → 120 → … → 900 s
        // instead of hammering WMI every minute forever. Any other error still
        // logs a single WARN.
        static LAST_FAILURE: OnceLock<AtomicU64> = OnceLock::new();
        static DENIED_LOGGED: OnceLock<std::sync::atomic::AtomicBool> = OnceLock::new();
        static OTHER_FAILURE_LOGGED: OnceLock<std::sync::atomic::AtomicBool> = OnceLock::new();
        /// Current retry window; doubles on every consecutive failure.
        static BACKOFF_SECS: OnceLock<AtomicU64> = OnceLock::new();
        const BACKOFF_START_SECS: u64 = 60;
        const BACKOFF_MAX_SECS: u64 = 900;
        let last_fail = LAST_FAILURE.get_or_init(|| AtomicU64::new(0));
        let backoff = BACKOFF_SECS.get_or_init(|| AtomicU64::new(BACKOFF_START_SECS));
        let now_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        // Skip the doomed query while we're inside the failure window.
        let window = backoff.load(AtomicOrdering::Relaxed);
        let failed_at = last_fail.load(AtomicOrdering::Relaxed);
        if failed_at.saturating_add(window) > now_epoch {
            log::debug!(
                target: "hw::fan",
                "ESIF WMI query in failure window — skipping (retry in {}s)",
                failed_at.saturating_add(window).saturating_sub(now_epoch)
            );
            return Err(crate::hw::errors::HardwareError::Other(
                "ESIF WMI denied (cached)".into(),
            ));
        }

        let access_denied = std::cell::Cell::new(false);
        let query_result = wmi_cache::with_wmi(|wmi| {
            match wmi
                .raw_query("SELECT InstanceName, Temperature, Power FROM EsifDeviceInformation")
            {
                Ok(r) => Ok(r),
                Err(e) => {
                    // Classify the raw WMI error here, where the HRESULT is
                    // still available (P11).
                    if wmi_cache::is_access_denied(&e) {
                        access_denied.set(true);
                    }
                    // Only debug-log here — the single log is emitted on the
                    // outer error path (below).
                    last_fail.store(now_epoch, AtomicOrdering::Relaxed);
                    Err(anyhow::Error::from(e))
                }
            }
        });

        let results: Vec<HashMap<String, wmi::Variant>> = match query_result {
            Ok(r) => {
                // Success — reset the retry window to its starting value.
                backoff.store(BACKOFF_START_SECS, AtomicOrdering::Relaxed);
                r
            }
            Err(e) => {
                last_fail.store(now_epoch, AtomicOrdering::Relaxed);
                let next_window =
                    wmi_cache::grow_failure_backoff(window, BACKOFF_START_SECS, BACKOFF_MAX_SECS);
                backoff.store(next_window, AtomicOrdering::Relaxed);
                if access_denied.get() {
                    // Expected while unprivileged: INFO once per process, then
                    // debug. Thermal data comes from the elevated bridge.
                    let first = DENIED_LOGGED
                        .get_or_init(|| std::sync::atomic::AtomicBool::new(false))
                        .swap(true, AtomicOrdering::Relaxed);
                    if !first {
                        log::info!(
                            target: "hw::fan",
                            "ESIF WMI access denied (0x80041003) — expected while \
                             unprivileged (thermal data comes from the elevated bridge); \
                             retry window grows to {next_window}s"
                        );
                    } else {
                        log::debug!(target: "hw::fan", "ESIF WMI access denied (retry in {next_window}s): {e}");
                    }
                } else {
                    let first = OTHER_FAILURE_LOGGED
                        .get_or_init(|| std::sync::atomic::AtomicBool::new(false))
                        .swap(true, AtomicOrdering::Relaxed);
                    if !first {
                        log::warn!(target: "hw::fan", "ESIF WMI query failed (next retry in {next_window}s): {e}");
                    } else {
                        log::debug!(target: "hw::fan", "ESIF WMI query failed (retry in {next_window}s): {e}");
                    }
                }
                return Err(e);
            }
        };

        log::debug!(
            target: "hw::fan",
            "ESIF query returned {} participants",
            results.len()
        );

        let extract_u32_temp = |row: &HashMap<String, wmi::Variant>, key: &str| -> Option<f32> {
            wmi_extract::extract_u32(row, key).map(|v| v as f32)
        };

        let instance_suffix = |row: &HashMap<String, wmi::Variant>, suffix: &str| -> bool {
            wmi_extract::extract_string(row, "InstanceName").is_some_and(|s| s.ends_with(suffix))
        };

        // CPU temp: max Temperature across participants (hotspot).
        // Zero is a valid reading (0°C idle), so we do NOT filter it out.
        let cpu_temp = results
            .iter()
            .filter_map(|r| extract_u32_temp(r, "Temperature"))
            .fold(f32::NEG_INFINITY, |acc, v| acc.max(v));
        let cpu_temp = if cpu_temp.is_finite() {
            Some(cpu_temp.clamp(0.0, 120.0))
        } else {
            None // No ESIF data — do NOT fabricate a value
        };

        // GPU temp: prefer participant _10 (GPU/secondary SoC domain on Panther Lake)
        let gpu_temp = results
            .iter()
            .find(|r| instance_suffix(r, "_10"))
            .and_then(|r| extract_u32_temp(r, "Temperature"))
            .map(|v| v.clamp(0.0, 120.0))
            .or_else(|| {
                // Fallback: package maximum (same die, valid under GPU load)
                let m = results
                    .iter()
                    .filter_map(|r| extract_u32_temp(r, "Temperature"))
                    .fold(f32::NEG_INFINITY, |acc, v| acc.max(v));
                if m.is_finite() {
                    Some(m.clamp(0.0, 120.0))
                } else {
                    None
                }
            });

        // TDP: participant _0 is the highest-level DPTF power domain
        let tdp_watts = results
            .iter()
            .find(|r| instance_suffix(r, "_0"))
            .and_then(|r| extract_u32_temp(r, "Power"))
            .map(|v| (v / 10.0).clamp(0.0, 150.0));

        Ok(EsifReadings {
            cpu_temp,
            gpu_temp,
            tdp_watts,
            ambient_temp: None,
            cpu_source: if cpu_temp.is_some() {
                TempSource::Esif
            } else {
                TempSource::Unavailable
            },
        })
    }
    #[cfg(not(windows))]
    {
        Ok(EsifReadings {
            cpu_temp: None,
            gpu_temp: None,
            tdp_watts: None,
            ambient_temp: None,
            cpu_source: TempSource::Unavailable,
        })
    }
}

/// Read ESIF/ACPI thermal readings through the elevated bridge service.
///
/// The unprivileged MiControl process is DENIED access to the
/// `EsifDeviceInformation` and `MSAcpi_ThermalZoneTemperature` WMI classes
/// (HRESULT 0x80041003 "Access to a CIM resource was not available to the
/// client"). The bridge service runs as `NT AUTHORITY\SYSTEM`, so it CAN
/// read those classes. This is the fallback used when the direct (in-process)
/// WMI query returns no usable data.
///
/// # Failure backoff (circuit breaker)
///
/// An elevated round-trip is comparatively expensive (service pipe or a
/// scheduled-task launch). When it fails repeatedly — e.g. the bridge service
/// is not installed or its pipe denies access — retrying on every 2s/15s poll
/// wasted resources and, in the worst case, spawned a crashing helper every
/// cycle. We therefore back off after consecutive failures:
/// [`MAX_CONSECUTIVE_FAILURES`] failures in a row open the circuit and skip
/// elevated attempts for [`BACKOFF_SECS`] (60s).
// ── Elevated thermal circuit-breaker state ──────────────────────────────────
// Module-level so it can be reset on power-resume (sleep/wake) — see
// `reset_elevated_thermal_circuit`. Without the reset, a sleep cycle that
// kills the bridge service leaves the circuit open for 60 s, repeating on
// every poll and permanently suppressing temperatures (S37-006).
/// Consecutive failures that open the elevated-thermal circuit.
const MAX_CONSECUTIVE_FAILURES: u32 = 3;
/// How long the circuit stays open once the failure threshold is reached.
const BACKOFF_SECS: u64 = 60;

/// Decision returned by [`ThermalCircuit::begin`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CircuitDecision {
    /// Circuit closed and no attempt in flight — do the elevated round-trip.
    Proceed,
    /// Circuit open; skip without any elevated attempt until `resumes_in`.
    SkipOpen { resumes_in: StdDuration },
    /// Another caller already has an elevated read in flight — coalesce.
    SkipInFlight,
}

/// Circuit breaker for the elevated thermal read.
///
/// P15: the previous implementation kept a bare failure counter plus a
/// "backoff until" instant that was only consulted at the top of
/// `get_elevated_thermal_readings`. Every caller that was already in flight
/// when the circuit opened still incremented the counter and re-emitted
/// "opening circuit for 60s" (production: 6 WARN in 18 s, N=3→8) because the
/// counter was never reset, and concurrent 2 s polls kept entering before the
/// backoff was stored. This type makes the state machine explicit:
///
/// * `begin()` is the single gate and runs BEFORE the elevated attempt; it
///   also marks the attempt in flight so parallel callers coalesce instead of
///   stampeding the bridge.
/// * `failure()` resets the counter when it opens the circuit, so the WARN is
///   emitted exactly once per open window.
/// * `cooldown` is constructor-injected so tests can use a short window and
///   pass explicit `now` values (no sleeps, no flakes).
struct ThermalCircuit {
    failures: u32,
    open_until: Option<StdInstant>,
    in_flight: bool,
    max_failures: u32,
    cooldown: StdDuration,
}

impl ThermalCircuit {
    fn new(max_failures: u32, cooldown: StdDuration) -> Self {
        Self {
            failures: 0,
            open_until: None,
            in_flight: false,
            max_failures,
            cooldown,
        }
    }

    /// Gate an attempt: `Proceed` only when the circuit is closed and nothing
    /// else is in flight.
    fn begin(&mut self, now: StdInstant) -> CircuitDecision {
        if let Some(until) = self.open_until {
            if now < until {
                return CircuitDecision::SkipOpen {
                    resumes_in: until.saturating_duration_since(now),
                };
            }
            // Cooldown elapsed — close the circuit and start a fresh window.
            self.open_until = None;
            self.failures = 0;
        }
        if self.in_flight {
            return CircuitDecision::SkipInFlight;
        }
        self.in_flight = true;
        CircuitDecision::Proceed
    }

    /// Record a successful attempt. Closes the circuit.
    fn success(&mut self) {
        self.in_flight = false;
        self.failures = 0;
        self.open_until = None;
    }

    /// Record a failed attempt. Returns `true` when this failure opened the
    /// circuit (so the caller logs the transition exactly once).
    fn failure(&mut self, now: StdInstant) -> bool {
        self.in_flight = false;
        self.failures = self.failures.saturating_add(1);
        if self.failures >= self.max_failures {
            // Reset so the next open window needs another full run of
            // failures; otherwise every later failure would re-log "opening".
            self.failures = 0;
            self.open_until = Some(now + self.cooldown);
            true
        } else {
            false
        }
    }

    /// Release the in-flight marker without recording an outcome (the caller
    /// returned early between `begin` and success/failure).
    fn abandon(&mut self) {
        self.in_flight = false;
    }

    /// Force-close the circuit (sleep/wake: the bridge may be alive again).
    fn reset(&mut self) {
        let max_failures = self.max_failures;
        let cooldown = self.cooldown;
        *self = Self::new(max_failures, cooldown);
    }

    fn was_active(&self) -> bool {
        self.open_until.is_some() || self.failures > 0
    }
}

static THERMAL_CIRCUIT: OnceLock<StdMutex<ThermalCircuit>> = OnceLock::new();

fn thermal_circuit() -> &'static StdMutex<ThermalCircuit> {
    THERMAL_CIRCUIT.get_or_init(|| {
        StdMutex::new(ThermalCircuit::new(
            MAX_CONSECUTIVE_FAILURES,
            StdDuration::from_secs(BACKOFF_SECS),
        ))
    })
}

/// RAII token for an in-flight elevated read. Dropping it without calling
/// [`ThermalAttempt::success`]/[`ThermalAttempt::failure`] releases the
/// in-flight marker — a stuck marker would coalesce every later read into a
/// permanent skip.
struct ThermalAttempt {
    finished: bool,
}

impl ThermalAttempt {
    fn success(mut self) {
        self.finished = true;
        thermal_circuit()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .success();
    }

    /// Returns `true` when this failure opened the circuit.
    fn failure(mut self) -> bool {
        self.finished = true;
        thermal_circuit()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .failure(StdInstant::now())
    }
}

impl Drop for ThermalAttempt {
    fn drop(&mut self) {
        if !self.finished {
            thermal_circuit()
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .abandon();
        }
    }
}

/// Outcome of trying to start an elevated read.
enum ThermalGate {
    /// Perform the elevated round-trip with this token.
    Proceed(ThermalAttempt),
    /// Circuit open; skip until the cooldown elapses.
    SkipOpen(StdDuration),
    /// Another caller is already reading — coalesce.
    SkipInFlight,
}

fn begin_thermal_attempt() -> ThermalGate {
    let decision = thermal_circuit()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .begin(StdInstant::now());
    match decision {
        CircuitDecision::Proceed => ThermalGate::Proceed(ThermalAttempt { finished: false }),
        CircuitDecision::SkipOpen { resumes_in } => ThermalGate::SkipOpen(resumes_in),
        CircuitDecision::SkipInFlight => ThermalGate::SkipInFlight,
    }
}

/// Set when the bridge service is detected as unavailable after a thermal
/// failure. On the next backoff expiry, `get_elevated_thermal_readings`
/// attempts `ensure_bridge_service()` once to recover (S37-006).
static HEAL_BRIDGE_NEEDED: AtomicBool = AtomicBool::new(false);
/// TTL cache of the last successful elevated thermal read (15 s). Only one
/// elevated round-trip per cache window is made — the 2s fan poll reuses it.
static THERMAL_CACHE: OnceLock<StdMutex<Option<(StdInstant, EsifReadings)>>> = OnceLock::new();
const THERMAL_CACHE_SECS: u64 = 15;

impl EsifReadings {
    /// No reading available (circuit open / attempt coalesced / query failed).
    fn unavailable() -> Self {
        Self {
            cpu_temp: None,
            gpu_temp: None,
            tdp_watts: None,
            ambient_temp: None,
            cpu_source: TempSource::Unavailable,
        }
    }
}

/// Reset the elevated-thermal circuit-breaker (called on power resume).
///
/// After sleep/resume, the MiControlBridge service may be momentarily gone;
/// previously-open circuits now retry immediately instead of waiting out the
/// 60 s backoff — so temperatures recover right after wake.
pub fn reset_elevated_thermal_circuit() {
    HEAL_BRIDGE_NEEDED.store(false, AtomicOrdering::Relaxed);
    let was_active = {
        let mut circuit = thermal_circuit().lock().unwrap_or_else(|p| p.into_inner());
        let active = circuit.was_active();
        circuit.reset();
        active
    };
    if was_active {
        log::info!(target: "hw::fan", "Reset thermal circuit-breaker (sleep/wake)");
    }
    // Also drop the TTL cache so the first post-resume read is fresh.
    if let Some(c) = THERMAL_CACHE.get() {
        if let Ok(mut guard) = c.lock() {
            *guard = None;
        }
    }
}

pub async fn get_elevated_thermal_readings() -> EsifReadings {
    // (TTL cache is module-level: THERMAL_CACHE / THERMAL_CACHE_SECS.)

    let cache = THERMAL_CACHE.get_or_init(|| StdMutex::new(None));
    if let Ok(mut guard) = cache.lock() {
        if let Some((at, ref readings)) = *guard {
            if at.elapsed() < StdDuration::from_secs(THERMAL_CACHE_SECS) {
                log::debug!(
                    target: "hw::fan",
                    "Elevated thermal read served from cache ({} ms old)",
                    at.elapsed().as_millis()
                );
                return readings.clone();
            }
        }
        *guard = None; // Expired — re-fetch.
    }

    // P15: the ONLY entry gate. Checked BEFORE the elevated round-trip, so an
    // open circuit really means "no elevated attempt for BACKOFF_SECS" —
    // and an in-flight attempt makes parallel polls coalesce instead of
    // stampeding the bridge.
    let attempt = match begin_thermal_attempt() {
        ThermalGate::Proceed(attempt) => attempt,
        ThermalGate::SkipOpen(resumes_in) => {
            log::debug!(
                target: "hw::fan",
                "Elevated thermal read in backoff — skipping (circuit resumes in {resumes_in:?})"
            );
            return EsifReadings::unavailable();
        }
        ThermalGate::SkipInFlight => {
            log::debug!(
                target: "hw::fan",
                "Elevated thermal read already in flight — coalescing, skipping this poll"
            );
            return EsifReadings::unavailable();
        }
    };

    // S37-006: Heal the bridge service once the backoff expires.
    if HEAL_BRIDGE_NEEDED.swap(false, AtomicOrdering::Relaxed) {
        log::info!(target: "hw::fan", "Attempting bridge service heal (backoff expired)");
        if let Ok(v) = crate::elev_bridge::ensure_bridge_service().await {
            log::info!(
                target: "hw::fan",
                "Bridge service heal result: {}",
                v.get("status").and_then(|s| s.as_str()).unwrap_or("ok")
            );
        } else {
            log::warn!(target: "hw::fan", "Bridge service heal failed — will retry at next backoff expiry");
            HEAL_BRIDGE_NEEDED.store(true, AtomicOrdering::Relaxed);
        }
    }

    let raw = match crate::elev_bridge::run_elevated_no_prompt(
        "read_thermal_readings",
        serde_json::json!({}),
    )
    .await
    {
        Ok(v) => v,
        Err(e) => {
            // Failure path — count towards the circuit breaker. `failure()`
            // returns true exactly once per open window (the counter is reset
            // when the circuit opens), so this WARN cannot repeat for every
            // in-flight attempt (P15: 6 WARN in 18 s, N=3→8 in production).
            let opened = attempt.failure();
            if opened {
                log::warn!(
                    target: "hw::fan",
                    "Elevated thermal read failed {MAX_CONSECUTIVE_FAILURES} consecutive times — \
                     opening circuit for {BACKOFF_SECS}s (no elevated attempts until then)"
                );
                // S37-006: The service may have died (overnight idle / sleep).
                // Opening the circuit is fine, but don't leave temps suppressed
                // forever — if the bridge is gone, schedule a reinstall attempt
                // when the backoff expires.
                if !crate::elev_bridge::is_bridge_service_available() {
                    HEAL_BRIDGE_NEEDED.store(true, AtomicOrdering::Relaxed);
                    log::info!(
                        target: "hw::fan",
                        "Bridge service unavailable after thermal failure — will attempt heal"
                    );
                }
            } else {
                log::debug!(
                    target: "hw::fan",
                    "Elevated thermal read unavailable: {e}"
                );
            }
            return EsifReadings::unavailable();
        }
    };

    // Success — close the circuit and reset the failure counter.
    attempt.success();
    let mut cpu_temp: Option<f32> = None;
    let mut cpu_source = TempSource::Unavailable;
    let mut gpu_temp: Option<f32> = None;
    let mut tdp_watts: Option<f32> = None;

    // ESIF participants: max Temperature = CPU hotspot; _10 = GPU domain.
    if let Some(esif) = raw
        .get("esif")
        .and_then(|v| v.get("participants"))
        .and_then(|a| a.as_array())
    {
        let max_temp = esif
            .iter()
            .filter_map(|p| p.get("temp_c").and_then(|t| t.as_u64()).map(|t| t as f32))
            .fold(f32::NEG_INFINITY, f32::max);
        if max_temp.is_finite() {
            cpu_temp = Some(max_temp.clamp(0.0, 120.0));
            cpu_source = TempSource::Esif;
        }
        gpu_temp = esif
            .iter()
            .find(|p| {
                p.get("instance")
                    .and_then(|i| i.as_str())
                    .is_some_and(|s| s.ends_with("_10"))
            })
            .and_then(|p| p.get("temp_c").and_then(|t| t.as_u64()))
            .map(|t| (t as f32).clamp(0.0, 120.0));
        tdp_watts = esif
            .iter()
            .find(|p| {
                p.get("instance")
                    .and_then(|i| i.as_str())
                    .is_some_and(|s| s.ends_with("_0"))
            })
            .and_then(|p| p.get("power_dw").and_then(|t| t.as_u64()))
            .map(|dw| ((dw as f32) / 10.0).clamp(0.0, 150.0));
    }

    // ACPI thermal zone fallback (CPU-side) — **ambient zone excluded**.
    // On this platform the only ACPI zone is TZ00, which is the chassis sensor
    // (27.85 °C in the same sample where ESIF read 57 °C on the CPU hotspot).
    // Using it as the CPU temperature would report the room as the processor,
    // so an ambient-only zone list leaves `cpu_temp` at `None` (P2-8).
    let mut ambient_temp: Option<f32> = None;
    let acpi_zones = raw
        .get("acpi")
        .and_then(|v| v.get("zones"))
        .and_then(|a| a.as_array());
    if let Some(zones) = acpi_zones {
        let temp_of = |z: &serde_json::Value| -> Option<f32> {
            z.get("temp_c")
                .and_then(|t| t.as_f64())
                .filter(|t| t.is_finite() && *t > 0.0)
                .map(|t| (t as f32).clamp(0.0, 120.0))
        };
        if cpu_temp.is_none() {
            cpu_temp = zones
                .iter()
                .filter(|z| {
                    z.get("instance")
                        .and_then(|i| i.as_str())
                        .is_some_and(|name| !crate::hw::thermal::is_ambient_zone(name))
                })
                .find_map(temp_of);
            if cpu_temp.is_some() {
                cpu_source = TempSource::Acpi;
            }
        }
        ambient_temp = zones
            .iter()
            .filter(|z| {
                z.get("instance")
                    .and_then(|i| i.as_str())
                    .is_some_and(crate::hw::thermal::is_ambient_zone)
            })
            .find_map(temp_of);
    }

    let readings = EsifReadings {
        cpu_temp,
        gpu_temp,
        tdp_watts,
        ambient_temp,
        cpu_source,
    };

    // ── S37-004: Store the successful reading in the TTL cache ──────────────
    if let Ok(mut guard) = cache.lock() {
        *guard = Some((StdInstant::now(), readings.clone()));
    }

    readings
}

/// Compute `FanInfo` using only the local (in-process) WMI reads.
///
/// Preferred entry point for synchronous contexts (e.g. the hardware-state
/// batch job). For the interactive `get_fan_info` command, use
/// [`get_fan_info_seeded`] with readings fetched via the elevated bridge,
/// because the thermal WMI classes deny unprivileged callers.
pub fn get_fan_info() -> HardwareResult<FanInfo> {
    get_fan_info_seeded(EsifReadings::unavailable(), None)
}

// ── EC active mode via the elevated bridge ──────────────────────────────────

/// TTL cache for the elevated EC-mode read. The UI polls `get_fan_info` every
/// 2 s; the EC mode changes only when somebody writes it, so a short TTL keeps
/// the round-trips down without showing a stale mode after a change.
type EcModeReading = (u16, crate::hw::wmi_ec::EcActiveMode);
type EcModeCache = OnceLock<StdMutex<Option<(StdInstant, Option<EcModeReading>)>>>;
static EC_MODE_CACHE: EcModeCache = OnceLock::new();
const EC_MODE_CACHE_SECS: u64 = 3;

/// Invalidate the cached EC mode (call after any mode write).
pub fn invalidate_ec_mode_cache() {
    if let Some(cache) = EC_MODE_CACHE.get() {
        if let Ok(mut guard) = cache.lock() {
            *guard = None;
        }
    }
}

/// Read the EC's active performance mode through the elevated bridge.
///
/// `MICommonInterface` denies user-mode CIM access on this platform, so the
/// read has to run in the elevated dispatcher; the whitelisted `wmi_ec_read`
/// command with `fun2=0x0800, fun3=0` is exactly that read. `None` means the
/// read did not answer — never a fabricated mode.
pub async fn get_ec_active_mode() -> Option<EcModeReading> {
    let cache = EC_MODE_CACHE.get_or_init(|| StdMutex::new(None));
    if let Ok(guard) = cache.lock() {
        if let Some((at, value)) = *guard {
            if at.elapsed() < StdDuration::from_secs(EC_MODE_CACHE_SECS) {
                return value;
            }
        }
    }

    let raw = crate::elev_bridge::run_elevated_no_prompt(
        "wmi_ec_read",
        serde_json::json!({ "fun2": 0x0800, "fun3": 0x0000 }),
    )
    .await;

    let value = match raw {
        Ok(v) => {
            let sger = v.get("sger").and_then(|s| s.as_u64()).unwrap_or(0);
            let frd0 = v.get("frd0").and_then(|s| s.as_u64()).map(|f| f as u16);
            match (sger, frd0) {
                (0x8000, Some(raw_mode)) => Some((
                    raw_mode,
                    crate::hw::wmi_ec::EcActiveMode::from_raw(raw_mode),
                )),
                _ => {
                    log::debug!(
                        target: "hw::fan",
                        "EC mode read via bridge returned no usable value: {v}"
                    );
                    None
                }
            }
        }
        Err(e) => {
            log::debug!(target: "hw::fan", "EC mode read via bridge failed: {e}");
            None
        }
    };

    if let Ok(mut guard) = cache.lock() {
        *guard = Some((StdInstant::now(), value));
    }
    value
}

/// EC RAM reading via the `ecram_service` pipe.
struct EcRamReading {
    cpu_temp_celsius: f32,
    rpm: Option<u32>,
}

/// Read the ACPI ERAM block (0xFE0B0300, 256 bytes) via the pipe.
///
/// CPU temp is at offset 0x03, fan RPM at 0x04-0x05, GPU temp is not available
/// here. The block is **known to read back as all zeros on this platform**
/// while still reporting success (the IOCTL is denied even to SYSTEM), so a
/// zero temperature or zero RPM is treated as "no reading" — never as data.
fn read_ecram_reading() -> Option<EcRamReading> {
    let eram = crate::hw::ecram::read_ecram_via_pipe(crate::hw::ecram::get_eram_base(), 0x10)?;
    if eram.len() < 0x08 {
        return None;
    }
    let cpu_t = eram[0x03] as f32;
    let fan_rpm = u16::from_le_bytes([eram[0x04], eram[0x05]]) as u32;
    log::debug!(
        target: "hw::fan",
        "EC RAM via pipe: cpu_temp={cpu_t}°C, fan_rpm={fan_rpm}"
    );
    Some(EcRamReading {
        cpu_temp_celsius: cpu_t,
        rpm: (fan_rpm > 0).then_some(fan_rpm),
    })
}

/// Assemble `FanInfo` from already-gathered parts. Pure — unit-testable.
fn build_fan_info(
    mode: FanMode,
    requested_speed_percent: u8,
    speed_rpm: Option<u32>,
    thermal: ThermalTotals,
    ec: Option<EcModeReading>,
) -> FanInfo {
    FanInfo {
        mode,
        // A percentage of full scale is only meaningful next to a measured RPM.
        // Without one we report "unavailable", never the requested value.
        speed_percent: speed_rpm.map(|rpm| ((rpm as f32 / 5000.0) * 100.0).clamp(0.0, 100.0) as u8),
        speed_rpm,
        requested_speed_percent,
        gpu_temp_celsius: thermal.gpu_temp,
        cpu_temp_celsius: thermal.cpu_temp,
        ambient_temp_celsius: thermal.ambient_temp,
        tdp_watts: thermal.tdp_watts,
        temp_source: thermal.source,
        ec_active: ec.map(|(_, m)| m),
        ec_active_raw: ec.map(|(raw, _)| raw),
        lid_open: os_lid_state(),
    }
}

/// OS-reported lid state, or `None` when the platform has no lid notification
/// (the Windows listener lives behind `cfg(windows)`).
fn os_lid_state() -> Option<bool> {
    #[cfg(windows)]
    {
        crate::hw::power_listener::lid_state()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Temperature half of [`build_fan_info`].
struct ThermalTotals {
    cpu_temp: Option<f32>,
    gpu_temp: Option<f32>,
    ambient_temp: Option<f32>,
    tdp_watts: Option<f32>,
    source: TempSource,
}

/// Compute `FanInfo`, seeding the ESIF readings with values fetched from the
/// elevated bridge when the direct (in-process) WMI query is denied.
///
/// `ec` is the EC's active mode `(raw, decoded)`. It can only be read from an
/// elevated process on this platform (`MICommonInterface` denies user-mode
/// CIM access with `Access denied`), which is why it is passed in instead of
/// being fetched here — see [`get_ec_active_mode`].
pub fn get_fan_info_seeded(
    elevated: EsifReadings,
    ec: Option<EcModeReading>,
) -> HardwareResult<FanInfo> {
    // ── RPM: only a real sensor reading is reported ──────────────────────────
    // Win32_Fan.DesiredSpeed is empty on this platform and every EC RAM path is
    // denied, so this is normally `None` — not `0`.
    let wmi_rpm = get_fan_rpm_wmi().unwrap_or(None);

    let esif = get_esif_readings().unwrap_or_else(|_| EsifReadings::unavailable());

    // If the direct (unprivileged) WMI read returned nothing — which is
    // expected on this platform because the thermal classes deny non-elevated
    // callers — use the readings fetched via the elevated bridge (SYSTEM).
    let esif = if esif.cpu_temp.is_none() && esif.gpu_temp.is_none() {
        elevated
    } else {
        esif
    };

    // The EC RAM pipe is only worth a round-trip when WMI gave neither an RPM
    // nor a CPU temperature. On this platform it always is (both are empty).
    let ecram = if wmi_rpm.is_none() || esif.cpu_temp.is_none() {
        read_ecram_reading()
    } else {
        None
    };

    // ── Temperature fallback chain ──────────────────────────────────────────
    // 1. Intel ESIF/DPTF (Wmi EsifDeviceInformation) — real die sensors
    // 2. EC RAM via ecram_service pipe (CPU temp at 0x03)
    // 3. A non-ambient ACPI zone — never TZ00 (ambient here: 27.85 °C vs 57 °C)
    // 4. Nothing → `None` + TempSource::Unavailable
    let (cpu_temp, cpu_source) = if let Some(t) = esif.cpu_temp {
        // `esif.cpu_source` distinguishes a real ESIF die sensor from a
        // non-ambient ACPI zone picked up by the elevated read.
        (Some(t), esif.cpu_source)
    } else if let Some(t) = ecram
        .as_ref()
        .map(|e| e.cpu_temp_celsius)
        .filter(|t| t.is_finite() && *t > 0.0 && *t <= 120.0)
    {
        (Some(t), TempSource::EcRam)
    } else {
        match crate::hw::thermal::cpu_temp_from_acpi() {
            Ok(Some(t)) => (Some(t), TempSource::Acpi),
            Ok(None) => {
                log::debug!(
                    target: "hw::fan",
                    "no CPU sensor: ESIF unavailable, EC RAM unusable, ACPI only has the \
                     ambient zone — reporting unavailable instead of the room temperature"
                );
                (None, TempSource::Unavailable)
            }
            Err(e) => {
                log::debug!(target: "hw::fan", "ACPI thermal zone unavailable: {e}");
                (None, TempSource::Unavailable)
            }
        }
    };

    // GPU temp: ESIF only. The ACPI zone is the ambient sensor here — using it
    // as the GPU temperature would repeat the P2-8 mistake on the other metric.
    let gpu_temp = esif.gpu_temp;

    let ambient_temp = esif
        .ambient_temp
        .or_else(|| crate::hw::thermal::ambient_temp_from_acpi().ok().flatten());

    let tdp_watts = esif.tdp_watts;

    // Fan RPM: WMI first, then EC RAM via pipe. `None` means "no sensor".
    let speed_rpm = wmi_rpm.or_else(|| ecram.as_ref().and_then(|e| e.rpm));

    let (mode, requested_speed_percent) = get_fan_mode_registry().unwrap_or((FanMode::Auto, 50));

    // WORKING FORM — DO NOT MODIFY: EC performance mode is read via WMI
    // MiInterface (ACPI WMAA method), NOT via IoTDriver or ECRAM.
    // wmi_ec::get_performance_mode() calls wmi_read(FUN2_EC_FUNC, 0) which
    // goes through the MICommonInterface WMI class in root\WMI.
    //
    // PITFALL (verified 2026-11-10): that class DENIES user-mode access
    // (`Access denied`), so this in-process read only succeeds when the whole
    // app runs elevated — the normal path is the elevated bridge, which is why
    // the decoded mode is passed in by the command layer instead.
    let ec = match ec {
        Some(v) => Some(v),
        None => crate::hw::wmi_ec::read_active_mode().ok(),
    };
    if let Some((raw, mode)) = ec {
        log::debug!(
            target: "hw::fan",
            "EC active mode via WMAA: raw={raw} ({mode:?})"
        );
    }

    Ok(build_fan_info(
        mode,
        requested_speed_percent,
        speed_rpm,
        ThermalTotals {
            cpu_temp,
            gpu_temp,
            ambient_temp,
            tdp_watts,
            source: cpu_source,
        },
        ec,
    ))
}

/// Apply a fan mode and report what actually happened.
///
/// The hardware writes are unchanged: for every mode the registry is
/// persisted, the EC performance profile is written over WMAA, and IGCL is
/// asked to take over the fan (which is a no-op on this platform, where IGCL
/// enumerates no fans). What changed is the **return value**: instead of a
/// bare `Ok(())`, the caller gets [`FanModeResult`], which says whether a
/// hardware path accepted the mode and whether RPM were really pinned.
///
/// `Fixed Speed` cannot pin RPM here — there is no controllable fan handle and
/// no RPM readback — so it degrades to "the EC profile closest to the requested
/// band" and says so in `warning`. `Off` cannot stop the fan: it maps to the
/// SuperQuiet profile and says so too.
pub fn set_fan_mode(mode: FanMode, speed_percent: u8) -> HardwareResult<FanModeResult> {
    persist_fan_registry(&mode, speed_percent)?;

    // WORKING FORM — DO NOT MODIFY: EC performance mode is set via WMI
    // MiInterface (ACPI WMAA method), NOT via IoTDriver or ECRAM.
    // wmi_ec::set_performance_mode() calls wmi_write(FUN2_EC_FUNC, mode, 0)
    // which goes through the MICommonInterface WMI class in root\WMI.
    //
    // PITFALL (verified live 2026-11-10): WMAA FUN3 5..8 write SMMT/SMMD + a
    // DPTF variable and never touch QFAN, the register the read path reports.
    // "Success" is not evidence — the register is read back before and after,
    // and only `qfan_write_verified` may set `ec_verified`.
    let ec_mode = match mode {
        FanMode::Auto | FanMode::Fixed => {
            if speed_percent >= 80 {
                crate::hw::wmi_ec::EcPerformanceMode::UltraPerformance
            } else if speed_percent >= 50 {
                crate::hw::wmi_ec::EcPerformanceMode::Balanced
            } else {
                crate::hw::wmi_ec::EcPerformanceMode::Quiet
            }
        }
        FanMode::Off => crate::hw::wmi_ec::EcPerformanceMode::SuperQuiet,
    };
    let ec_profile = ec_profile_label(ec_mode);

    // Runs inside the elevated helper, so the in-process WMAA read works here.
    let before = crate::hw::wmi_ec::read_active_mode()
        .ok()
        .map(|(raw, _)| raw);
    let write_result = crate::hw::wmi_ec::set_performance_mode(ec_mode);
    let after = crate::hw::wmi_ec::read_active_mode()
        .ok()
        .map(|(raw, _)| raw);

    let mut ec_applied = match (&write_result, before, after) {
        (Err(e), _, _) => {
            log::warn!("WMI EC set_performance_mode: {e}");
            false
        }
        (Ok(()), Some(b), Some(a)) => {
            let ok = crate::hw::wmi_ec::qfan_write_verified(ec_mode as u16, b, a);
            if !ok {
                log::warn!(
                    target: "hw::fan",
                    "EC profile write did not latch: requested FUN3={} ({ec_profile}), QFAN {b} -> {a}",
                    ec_mode as u16
                );
            }
            ok
        }
        (Ok(()), _, _) => {
            log::warn!(
                target: "hw::fan",
                "EC profile write could not be verified (no mode readback)"
            );
            false
        }
    };
    let mut ec_mode_after = after;

    // ── Fallback: the channel that actually moves QFAN ───────────────────────
    // The WMAA profiles 5..8 (the ones this tab requests for Balance/Quiet/Off)
    // never write QFAN on this firmware. When the readback proves the write did
    // not latch, ask the same EC through the original Xiaomi channel
    // (HQWmiCommonInterface.SetPerformanceMode → `ECWT(value, QFAN)`), then
    // verify again. The requested fan *band* is preserved: >=80% → Turbo,
    // >=50% → Balance, below/off → Silence.
    let mut used_qfan_channel = false;
    if !ec_applied {
        let target = ec_qfan_for_fan_mode(&mode, speed_percent);
        match crate::hw::performance::set_ec_qfan(u32::from(target)) {
            Ok(()) => {
                let rb = crate::hw::wmi_ec::read_active_mode()
                    .ok()
                    .map(|(raw, _)| raw);
                let baseline = before.or(ec_mode_after);
                if let (Some(b), Some(a)) = (baseline, rb) {
                    if crate::hw::wmi_ec::qfan_write_verified(target, b, a) {
                        ec_applied = true;
                        used_qfan_channel = true;
                    }
                    ec_mode_after = Some(a);
                }
                if !ec_applied {
                    log::warn!(
                        target: "hw::fan",
                        "EC QFAN write via HQ WMI did not latch either: requested {target}, readback {rb:?}"
                    );
                }
            }
            Err(e) => log::warn!(target: "hw::fan", "EC QFAN write via HQ WMI failed: {e}"),
        }
        invalidate_ec_mode_cache();
    }

    // `true` only when an IGCL fan handle was actually programmed — i.e. the
    // fixed speed was really applied to a fan. Always false here.
    let rpm_control = match mode {
        FanMode::Auto => set_fan_auto_igcl().unwrap_or_else(|e| {
            log::warn!("IGCL fan auto: {e}");
            false
        }),
        FanMode::Fixed => set_fan_fixed_igcl(speed_percent).unwrap_or_else(|e| {
            log::warn!("IGCL fan fixed: {e}");
            false
        }),
        FanMode::Off => {
            log::warn!("Fan off mode not directly supported — using minimum speed");
            false
        }
    };

    let mut result = FanModeResult::new(
        &mode,
        ec_profile,
        ec_applied,
        rpm_control,
        before,
        ec_mode_after,
    );
    // Record the real before/after register values (the honest evidence).
    result.ec_mode_before = before;
    result.ec_mode_after = ec_mode_after;
    if used_qfan_channel {
        result.method.push_str("+ec_qfan");
    }

    // The lid state explains a fan that ignores the request: with the lid
    // closed the EC runs its own clamshell fan table, so neither the EC profile
    // write nor an IGCL handle can be observed in the fan. Say so instead of
    // reporting a mode that the hardware is not following.
    if crate::hw::power_listener::is_lid_closed() {
        let note = "The lid is closed: this platform's EC switches to a clamshell fan table \
                    while the lid is shut, so the requested profile may not be what the fan \
                    follows. Reopen the lid to apply or verify it."
            .to_string();
        result.warning = Some(match result.warning.take() {
            Some(existing) => format!("{existing} {note}"),
            None => note,
        });
    }

    log::info!(
        target: "hw::fan",
        "set_fan_mode {mode:?} {speed_percent}% → applied={} method={} rpm_control={} profile={ec_profile} qfan={:?}->{:?}",
        result.applied,
        result.method,
        result.rpm_control,
        result.ec_mode_before,
        result.ec_mode_after
    );
    Ok(result)
}

/// The EC mode register value (`QFAN`) that matches a requested fan band.
///
/// Uses the EC's own numbering (the one `HQWmiCommonInterface.SetPerformanceMode`
/// writes): 2 = Turbo, 1 = Balance, 0 = Silence. These are the three profiles
/// with distinct fan tables; "Off" has no hardware equivalent and maps to
/// Silence, which the result message says out loud.
fn ec_qfan_for_fan_mode(mode: &FanMode, speed_percent: u8) -> u16 {
    match mode {
        FanMode::Off => 0,
        FanMode::Auto | FanMode::Fixed => {
            if speed_percent >= 80 {
                2 // Turbo
            } else if speed_percent >= 50 {
                1 // Balance
            } else {
                0 // Silence
            }
        }
    }
}

/// Human-readable name of the EC fan profile written by `set_fan_mode`.
///
/// The WMAA values map to the EC's fan profile bands; the names mirror
/// [`crate::hw::wmi_ec::EcPerformanceMode`] and are what the honest result
/// message quotes back to the user.
fn ec_profile_label(mode: crate::hw::wmi_ec::EcPerformanceMode) -> &'static str {
    use crate::hw::wmi_ec::EcPerformanceMode as Ec;
    match mode {
        Ec::Performance => "Performance",
        Ec::Balanced => "Balanced",
        Ec::Quiet => "Quiet",
        Ec::SuperQuiet => "SuperQuiet",
        Ec::UltraPerformance => "UltraPerformance",
        Ec::Extreme => "Extreme",
    }
}

// ── WMI helpers ─────────────────────────────────────────────────────────────

/// Read fan RPM from `Win32_Fan`.
///
/// Returns `None` when the platform exposes no RPM sensor — which is the case
/// here: the class exists but `DesiredSpeed` comes back empty, and no other
/// property carries a tachometer reading. Callers must not turn `None` into
/// `0`: "0 RPM" claims the fan has stopped.
fn get_fan_rpm_wmi() -> HardwareResult<Option<u32>> {
    #[cfg(windows)]
    {
        use crate::hw::wmi_cache;
        use crate::util::wmi_extract;
        use std::collections::HashMap;

        // Win32_Fan does not have a CurrentReading property on most Xiaomi Book platforms.
        // It does have DesiredSpeed (UInt64) and VariableSpeed (Boolean), but these are
        // typically empty. The ESIF/DPTF driver provides temperature data separately via
        // get_esif_readings().
        let query_result = wmi_cache::with_cimv2(|wmi| {
            match wmi.raw_query("SELECT DesiredSpeed, VariableSpeed FROM Win32_Fan") {
                Ok(r) => Ok(r),
                Err(e) => {
                    log::debug!(target: "hw::fan", "Win32_Fan raw_query error: {e}");
                    Err(anyhow::Error::from(e))
                }
            }
        });
        let results: Vec<HashMap<String, wmi::Variant>> = match query_result {
            Ok(r) => r,
            Err(e) => {
                log::debug!(target: "hw::fan", "Win32_Fan query failed: {e}");
                return Ok(None); // No fan data — don't fail the whole call
            }
        };
        if let Some(row) = results.first() {
            // DesiredSpeed is UInt64 and may be null on some platforms. A zero
            // here means "not populated", not "fan stopped": no Xiaomi Book Pro
            // reports 0 through this property while the fan is spinning.
            if let Some(rpm) = wmi_extract::extract_u64(row, "DesiredSpeed") {
                if rpm > 0 {
                    return Ok(Some(rpm as u32));
                }
            }
        }
        Ok(None)
    }
    #[cfg(not(windows))]
    {
        Ok(None)
    }
}

// ── Registry persistence ─────────────────────────────────────────────────────

fn persist_fan_registry(mode: &FanMode, speed_percent: u8) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegCreateKeyExW, RegSetValueExW, HKEY_LOCAL_MACHINE, KEY_WRITE, REG_DWORD,
            REG_OPTION_NON_VOLATILE,
        };
        unsafe {
            // SAFETY: Null-terminated wide strings; MaybeUninit<HKEY> written by
            // RegCreateKeyExW before assume_init. Stack-local DWORD values have valid alignment.
            let key_w: Vec<u16> = OsStr::new(FAN_REG_KEY)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let mut hkey = std::mem::MaybeUninit::uninit();
            RegCreateKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(key_w.as_ptr()),
                0,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                hkey.as_mut_ptr(),
                None,
            )
            .ok()
            .context("Create fan reg key")?;
            let hkey = hkey.assume_init();

            let mode_val: u32 = match mode {
                FanMode::Auto => 0,
                FanMode::Fixed => 1,
                FanMode::Off => 2,
            };
            let mode_w: Vec<u16> = OsStr::new(FAN_REG_MODE)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let _ = RegSetValueExW(
                hkey,
                PCWSTR(mode_w.as_ptr()),
                0,
                REG_DWORD,
                Some(&mode_val.to_le_bytes()),
            )
            .ok();

            let speed_val = speed_percent as u32;
            let speed_w: Vec<u16> = OsStr::new(FAN_REG_SPEED)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let _ = RegSetValueExW(
                hkey,
                PCWSTR(speed_w.as_ptr()),
                0,
                REG_DWORD,
                Some(&speed_val.to_le_bytes()),
            )
            .ok();

            let _ = RegCloseKey(hkey).ok();
        }
    }
    Ok(())
}

fn get_fan_mode_registry() -> HardwareResult<(FanMode, u8)> {
    #[cfg(windows)]
    {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::System::Registry::{
            RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY_LOCAL_MACHINE, REG_VALUE_TYPE,
        };
        unsafe {
            // SAFETY: Null-terminated wide strings; hkey is assume_init only after
            // RegOpenKeyExW succeeds. The u32 pointer cast is valid for DWORD-sized stack buffer.
            let key_w: Vec<u16> = OsStr::new(FAN_REG_KEY)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let mut hkey = std::mem::MaybeUninit::uninit();
            if RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(key_w.as_ptr()),
                0,
                windows::Win32::System::Registry::KEY_READ,
                hkey.as_mut_ptr(),
            )
            .is_err()
            {
                return Ok((FanMode::Auto, 50));
            }
            let hkey = hkey.assume_init();

            let mut mode_val: u32 = 0;
            let mut size = 4u32;
            let mut ty = REG_VALUE_TYPE::default();
            let mode_w: Vec<u16> = OsStr::new(FAN_REG_MODE)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let _ = RegQueryValueExW(
                hkey,
                PCWSTR(mode_w.as_ptr()),
                None,
                Some(&mut ty),
                Some((&mut mode_val as *mut u32).cast()),
                Some(&mut size),
            );

            let mut speed_val: u32 = 50;
            let speed_w: Vec<u16> = OsStr::new(FAN_REG_SPEED)
                .encode_wide()
                .chain(Some(0))
                .collect();
            let _ = RegQueryValueExW(
                hkey,
                PCWSTR(speed_w.as_ptr()),
                None,
                Some(&mut ty),
                Some((&mut speed_val as *mut u32).cast()),
                Some(&mut size),
            );

            let _ = RegCloseKey(hkey).ok();
            let mode = match mode_val {
                1 => FanMode::Fixed,
                2 => FanMode::Off,
                _ => FanMode::Auto,
            };
            Ok((mode, speed_val.clamp(20, 100) as u8))
        }
    }
    #[cfg(not(windows))]
    {
        Ok((FanMode::Auto, 50))
    }
}

// ── IGCL fan control ─────────────────────────────────────────────────────────
//
// Intel IGCL (ControlLib.dll) exposes fan handles via ctlEnumFans.
// On laptops with only integrated graphics (no dGPU), ctlEnumFans typically
// returns 0 handles — the laptop fan is controlled by the EC/firmware and
// responds to performance mode changes, not IGCL.  The code below is real and
// will work on any machine where IGCL reports ≥1 fan handle.
//
// C layouts:
//   ctl_fan_speed_units_t: 0 = PERCENT, 1 = RPM
//   ctl_fan_speed_t { size:u32, version:u8, [3-pad], units:u32, value:i32 }

#[cfg(windows)]
mod igcl_fan {
    use std::ffi::c_void;

    pub type CtlDeviceHandle = *mut c_void;
    pub type CtlFanHandle = *mut c_void;
    pub type CtlResult = u32;

    #[repr(C)]
    pub struct CtlFanSpeed {
        pub size: u32,
        pub version: u8,
        pub _pad: [u8; 3],
        pub units: u32, // 0 = PERCENT, 1 = RPM
        pub value: i32,
    }

    pub type FnCtlEnumFans =
        unsafe extern "C" fn(CtlDeviceHandle, *mut u32, *mut CtlFanHandle) -> CtlResult;
    pub type FnCtlFanSetDefaultMode = unsafe extern "C" fn(CtlFanHandle) -> CtlResult;
    pub type FnCtlFanSetFixedSpeedMode =
        unsafe extern "C" fn(CtlFanHandle, *const CtlFanSpeed) -> CtlResult;
}

/// Run `f` for every IGCL fan handle on the first enumerated device.
/// Returns `Ok(0)` (no fans found, nothing done) if the device has no IGCL-
/// accessible fans — expected on integrated-only platforms where the EC
/// firmware manages the fan as a function of the active performance mode.
#[cfg(windows)]
fn with_igcl_fans<F>(f: F) -> HardwareResult<usize>
where
    F: Fn(igcl_fan::CtlFanHandle, &libloading::Library) -> HardwareResult<()>,
{
    use igcl_fan::*;

    let count = crate::hw::display::with_igcl_device_pub(|device, lib| unsafe {
        // SAFETY: device is a valid IGCL device handle from ctlEnumerateDevices. The
        // ctlEnumFans function returns fan handles owned by IGCL; we iterate them immediately
        // and do not retain handles after the closure ends.
        let ctl_enum_fans: libloading::Symbol<FnCtlEnumFans> =
            lib.get(b"ctlEnumFans\0").context("ctlEnumFans")?;

        let mut count: u32 = 0;
        ctl_enum_fans(device, &mut count, std::ptr::null_mut());
        if count == 0 {
            log::debug!("[fan] ctlEnumFans: no IGCL fan handles (EC-managed fan — OK)");
            return Ok(0usize);
        }

        let mut handles = vec![std::ptr::null_mut::<std::ffi::c_void>(); count as usize];
        let rc = ctl_enum_fans(device, &mut count, handles.as_mut_ptr());
        if rc != 0 {
            return Err(HardwareError::Display(format!(
                "ctlEnumFans failed: {rc:#x}"
            )));
        }

        for &fan in &handles[..count as usize] {
            f(fan, lib)?;
        }
        Ok(count as usize)
    })?;
    Ok(count)
}

#[cfg(not(windows))]
fn with_igcl_fans<F>(_f: F) -> HardwareResult<usize>
where
    F: Fn(*mut std::ffi::c_void, &libloading::Library) -> HardwareResult<()>,
{
    Ok(0)
}

/// Hand the fan back to the EC's automatic curve.
///
/// Returns `true` only when at least one IGCL fan handle was programmed. On
/// this platform IGCL enumerates no fans, so it returns `false` — the EC keeps
/// managing the fan, which is the same end state, but it is not something we
/// can claim to have done.
fn set_fan_auto_igcl() -> HardwareResult<bool> {
    #[cfg(windows)]
    {
        use igcl_fan::*;
        let n = with_igcl_fans(|fan, lib| unsafe {
            // SAFETY: fan is a valid IGCL fan handle from ctlEnumFans. The ctlFanSetDefaultMode
            // function pointer is loaded from the IGCL DLL which is still borrowed by the outer
            // with_igcl_device_pub closure.
            let set_default: libloading::Symbol<FnCtlFanSetDefaultMode> = lib
                .get(b"ctlFanSetDefaultMode\0")
                .context("ctlFanSetDefaultMode")?;
            let rc = set_default(fan);
            if rc != 0 {
                return Err(HardwareError::Display(format!(
                    "ctlFanSetDefaultMode: {rc:#x}"
                )));
            }
            log::info!("[fan] IGCL ctlFanSetDefaultMode OK");
            Ok(())
        })?;
        if n == 0 {
            log::debug!("[fan] Auto mode: no IGCL fans — performance mode controls EC fan");
        }
        Ok(n > 0)
    }
    #[cfg(not(windows))]
    {
        Ok(false)
    }
}

/// Pin the fan to a fixed percentage through IGCL.
///
/// Returns `true` only when a fan handle was actually programmed — i.e. the
/// speed really was pinned. `false` means "nothing to control": the EC profile
/// written by the caller is the only thing that changed.
fn set_fan_fixed_igcl(speed_percent: u8) -> HardwareResult<bool> {
    #[cfg(windows)]
    {
        use igcl_fan::*;
        let clamped = speed_percent.clamp(20, 100) as i32;
        let n = with_igcl_fans(|fan, lib| unsafe {
            // SAFETY: fan is a valid IGCL fan handle. CtlFanSpeed is a POD struct with correct
            // layout (size, version, pad, units, value). The function pointer is loaded from
            // the IGCL DLL which is still borrowed by the outer with_igcl_device_pub closure.
            let set_fixed: libloading::Symbol<FnCtlFanSetFixedSpeedMode> = lib
                .get(b"ctlFanSetFixedSpeedMode\0")
                .context("ctlFanSetFixedSpeedMode")?;
            let speed = CtlFanSpeed {
                size: std::mem::size_of::<CtlFanSpeed>() as u32,
                version: 0,
                _pad: [0; 3],
                units: 0, // PERCENT
                value: clamped,
            };
            let rc = set_fixed(fan, &speed);
            if rc != 0 {
                return Err(HardwareError::Display(format!(
                    "ctlFanSetFixedSpeedMode {clamped}%: {rc:#x}"
                )));
            }
            log::info!("[fan] IGCL ctlFanSetFixedSpeedMode {clamped}% OK");
            Ok(())
        })?;
        if n == 0 {
            log::debug!("[fan] Fixed {clamped}%: no IGCL fans — only perf mode affects EC fan");
        }
        Ok(n > 0)
    }
    #[cfg(not(windows))]
    {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P15: with an injected short cooldown the circuit must actually block
    /// attempts for the whole window — the production bug was "opening circuit
    /// for 60s" followed by a retry 2.2 s later.
    #[test]
    fn circuit_opens_after_threshold_and_blocks_for_the_full_cooldown() {
        let cooldown = StdDuration::from_millis(50);
        let mut circuit = ThermalCircuit::new(3, cooldown);
        let t0 = StdInstant::now();

        for i in 0..3 {
            assert_eq!(
                circuit.begin(t0 + StdDuration::from_millis(i)),
                CircuitDecision::Proceed,
                "failure {i} should be allowed"
            );
            let opened = circuit.failure(t0 + StdDuration::from_millis(i));
            assert_eq!(opened, i == 2, "only the third failure opens the circuit");
        }

        // Inside the window: no elevated attempt, and the remaining time is
        // reported from the injected clock (no sleeps involved).
        // The third failure happened at t0+2 ms, so the circuit is open until
        // t0+52 ms (2 ms + 50 ms cooldown).
        match circuit.begin(t0 + StdDuration::from_millis(10)) {
            CircuitDecision::SkipOpen { resumes_in } => {
                assert_eq!(resumes_in, StdDuration::from_millis(42));
            }
            other => panic!("expected SkipOpen, got {other:?}"),
        }
        // Just before expiry.
        assert!(matches!(
            circuit.begin(t0 + StdDuration::from_millis(51)),
            CircuitDecision::SkipOpen { .. }
        ));
        // At expiry the circuit closes and the attempt proceeds.
        assert_eq!(
            circuit.begin(t0 + cooldown + StdDuration::from_millis(3)),
            CircuitDecision::Proceed
        );
    }

    /// P15: the failure counter is reset when the circuit opens, so a later
    /// window needs another full run of failures before logging again.
    #[test]
    fn failure_counter_resets_when_circuit_opens() {
        let cooldown = StdDuration::from_secs(60);
        let mut circuit = ThermalCircuit::new(3, cooldown);
        let t0 = StdInstant::now();

        for _ in 0..3 {
            assert_eq!(circuit.begin(t0), CircuitDecision::Proceed);
            circuit.failure(t0);
        }
        let after_open = t0 + cooldown + StdDuration::from_millis(1);

        // First and second failure of the new window must NOT re-open (this is
        // exactly the N=4..8 spam seen in production).
        assert_eq!(circuit.begin(after_open), CircuitDecision::Proceed);
        assert!(!circuit.failure(after_open));
        assert_eq!(circuit.begin(after_open), CircuitDecision::Proceed);
        assert!(!circuit.failure(after_open));
        // The third one opens again.
        assert_eq!(circuit.begin(after_open), CircuitDecision::Proceed);
        assert!(circuit.failure(after_open));
    }

    /// P15: concurrent callers (2 s fan poll + performance monitor + UI
    /// command) must coalesce — only one elevated round-trip at a time.
    #[test]
    fn concurrent_attempts_are_coalesced() {
        let mut circuit = ThermalCircuit::new(3, StdDuration::from_secs(60));
        let now = StdInstant::now();

        assert_eq!(circuit.begin(now), CircuitDecision::Proceed);
        assert_eq!(circuit.begin(now), CircuitDecision::SkipInFlight);
        // A failure clears the in-flight marker (and counts once).
        assert!(!circuit.failure(now));
        assert_eq!(circuit.begin(now), CircuitDecision::Proceed);
    }

    /// A dropped/abandoned attempt must not leave the circuit permanently
    /// "in flight" (that would silently disable elevated reads forever).
    #[test]
    fn abandon_releases_in_flight_without_counting_a_failure() {
        let mut circuit = ThermalCircuit::new(3, StdDuration::from_secs(60));
        let now = StdInstant::now();

        assert_eq!(circuit.begin(now), CircuitDecision::Proceed);
        circuit.abandon();
        // Still closed, still no failures recorded: three failures are needed.
        assert_eq!(circuit.begin(now), CircuitDecision::Proceed);
        assert!(!circuit.failure(now));
        assert_eq!(circuit.begin(now), CircuitDecision::Proceed);
        assert!(!circuit.failure(now));
        assert_eq!(circuit.begin(now), CircuitDecision::Proceed);
        assert!(circuit.failure(now), "third failure after abandon opens");
    }

    /// A success closes an open circuit and clears the failure count.
    #[test]
    fn success_closes_the_circuit_and_resets_failures() {
        let cooldown = StdDuration::from_millis(10);
        let mut circuit = ThermalCircuit::new(2, cooldown);
        let t0 = StdInstant::now();
        circuit.begin(t0);
        circuit.failure(t0);
        circuit.begin(t0);
        assert!(circuit.failure(t0));

        let after = t0 + cooldown + StdDuration::from_millis(1);
        assert_eq!(circuit.begin(after), CircuitDecision::Proceed);
        circuit.success();
        // Closed with zero failures: a single failure must not open it.
        assert_eq!(circuit.begin(after), CircuitDecision::Proceed);
        assert!(!circuit.failure(after));
    }

    /// Sleep/wake reset must close the circuit and drop the failure history.
    #[test]
    fn reset_force_closes_the_circuit() {
        let mut circuit = ThermalCircuit::new(1, StdDuration::from_secs(60));
        let now = StdInstant::now();
        circuit.begin(now);
        assert!(circuit.failure(now));
        assert!(circuit.was_active());

        circuit.reset();
        assert!(!circuit.was_active());
        assert_eq!(circuit.begin(now), CircuitDecision::Proceed);
    }

    // ── P2-1: no RPM sensor must read as "unavailable", never 0 RPM + 50 % ──

    fn thermal(cpu: Option<f32>) -> ThermalTotals {
        ThermalTotals {
            cpu_temp: cpu,
            gpu_temp: None,
            ambient_temp: Some(27.85),
            tdp_watts: None,
            source: if cpu.is_some() {
                TempSource::Esif
            } else {
                TempSource::Unavailable
            },
        }
    }

    /// The production state: no tachometer, registry `FixedSpeed = 50`. The old
    /// code reported `rpm = 0` *and* a 50 % gauge at the same time.
    #[test]
    fn no_rpm_sensor_reports_unavailable_not_zero() {
        let info = build_fan_info(FanMode::Fixed, 50, None, thermal(Some(57.0)), None);
        assert_eq!(info.speed_rpm, None, "no sensor must not be reported as 0");
        assert_eq!(info.speed_percent, None, "gauge must not be synthesised");
        assert_eq!(
            info.requested_speed_percent, 50,
            "the requested value is still carried, but separately"
        );
        assert_eq!(info.cpu_temp_celsius, Some(57.0));
    }

    /// With a real tachometer reading the gauge is derived from it — never from
    /// the registry.
    #[test]
    fn measured_rpm_derives_the_percentage() {
        let info = build_fan_info(FanMode::Fixed, 50, Some(3000), thermal(Some(57.0)), None);
        assert_eq!(info.speed_rpm, Some(3000));
        assert_eq!(info.speed_percent, Some(60), "3000 / 5000 = 60 %");
    }

    #[test]
    fn gauge_is_capped_at_full_scale() {
        let info = build_fan_info(FanMode::Auto, 100, Some(9000), thermal(None), None);
        assert_eq!(info.speed_percent, Some(100));
    }

    /// The lid state rides along with the fan reading so a fan report can be
    /// attributed to the clamshell table; unknown stays `null`, never "closed".
    #[test]
    fn fan_info_carries_the_os_lid_state() {
        let mut info = build_fan_info(FanMode::Auto, 50, None, thermal(None), None);
        info.lid_open = Some(false);
        let open = serde_json::to_value(&info).expect("FanInfo serialises");
        assert_eq!(open["lid_open"], serde_json::json!(false));

        info.lid_open = None;
        let unknown = serde_json::to_value(&info).expect("FanInfo serialises");
        assert!(
            unknown["lid_open"].is_null(),
            "an unreported lid must not read as closed"
        );
    }

    /// An unavailable CPU sensor stays unavailable — no ambient value sneaks in.
    #[test]
    fn unavailable_cpu_temperature_stays_none() {
        let info = build_fan_info(FanMode::Auto, 50, None, thermal(None), None);
        assert_eq!(info.cpu_temp_celsius, None);
        assert_eq!(info.temp_source, TempSource::Unavailable);
        assert_eq!(
            info.ambient_temp_celsius,
            Some(27.85),
            "TZ00 is ambient only"
        );
    }

    // ── P2-2: set_fan_mode reports the real effect ──────────────────────────

    /// "Fixed Speed" on a platform with no IGCL fan: the EC profile is the only
    /// thing that happened, and the result says so instead of claiming success.
    #[test]
    fn fixed_without_igcl_does_not_claim_rpm_control() {
        let result = FanModeResult::new(&FanMode::Fixed, "Balanced", true, false, Some(9), Some(9));
        assert!(result.applied, "the EC profile was applied");
        assert!(!result.rpm_control, "RPM were not pinned");
        assert_eq!(result.method, "registry+ec_profile");
        let warning = result.warning.expect("a caveat is required");
        assert!(warning.contains("Balanced"), "names the profile applied");
        assert!(warning.contains("not pinned"), "does not promise RPM");
    }

    #[test]
    fn fixed_with_igcl_reports_rpm_control_and_no_warning() {
        let result = FanModeResult::new(&FanMode::Fixed, "Balanced", true, true, Some(9), Some(9));
        assert!(result.applied && result.rpm_control);
        assert_eq!(result.method, "registry+ec_profile+igcl");
        assert_eq!(result.warning, None);
    }

    #[test]
    fn off_mode_says_the_fan_keeps_running() {
        let result = FanModeResult::new(&FanMode::Off, "SuperQuiet", true, false, Some(9), Some(0));
        assert!(result.applied);
        let warning = result.warning.expect("a caveat is required");
        assert!(warning.contains("cannot stop the fan"));
        assert!(warning.contains("Super Quiet"));
    }

    /// No hardware path accepted the mode: `applied` must be false so the UI
    /// does not show a success toast.
    #[test]
    fn auto_with_no_hardware_path_is_not_applied() {
        let result = FanModeResult::new(&FanMode::Auto, "Balanced", false, false, Some(9), Some(9));
        assert!(!result.applied);
        assert_eq!(result.method, "registry");
        assert!(result.warning.is_some());
    }

    /// An older elevated helper answers `null`; that must not become a hard
    /// failure in the UI, but it must not be presented as a verified result
    /// either.
    #[test]
    fn legacy_null_bridge_reply_is_reported_as_unverified() {
        let result = FanModeResult::from_bridge(&serde_json::Value::Null);
        assert!(result.applied);
        assert!(!result.rpm_control);
        assert_eq!(result.method, "registry");
        assert!(result
            .warning
            .as_deref()
            .is_some_and(|w| w.contains("did not report")));
    }

    /// A new helper's payload round-trips through JSON unchanged.
    #[test]
    fn bridge_reply_round_trips() {
        let original = FanModeResult::new(&FanMode::Fixed, "Quiet", true, false, Some(2), Some(0));
        let value = serde_json::to_value(&original).expect("serialisable");
        assert_eq!(FanModeResult::from_bridge(&value), original);
    }

    /// The QFAN fallback must preserve the requested band, in the EC's own
    /// numbering (2=Turbo, 1=Balance, 0=Silence).
    #[test]
    fn qfan_fallback_keeps_the_requested_band() {
        assert_eq!(ec_qfan_for_fan_mode(&FanMode::Fixed, 100), 2);
        assert_eq!(ec_qfan_for_fan_mode(&FanMode::Fixed, 80), 2);
        assert_eq!(ec_qfan_for_fan_mode(&FanMode::Auto, 79), 1);
        assert_eq!(ec_qfan_for_fan_mode(&FanMode::Auto, 50), 1);
        assert_eq!(ec_qfan_for_fan_mode(&FanMode::Fixed, 49), 0);
        assert_eq!(ec_qfan_for_fan_mode(&FanMode::Off, 100), 0);
    }

    /// A result that could not be verified must say so, and must carry the raw
    /// register values instead of a friendly story.
    #[test]
    fn unverified_write_reports_the_real_readback() {
        // Requested the WMAA "Balanced" profile (FUN3=6); the EC stayed at 9.
        let result = FanModeResult::new(&FanMode::Auto, "Balanced", false, false, Some(9), Some(9));
        assert!(!result.applied, "an unlatched write is not applied");
        assert!(!result.ec_verified);
        assert_eq!(result.ec_mode_before, Some(9));
        assert_eq!(result.ec_mode_after, Some(9));
        let warning = result.warning.expect("a caveat is required");
        assert!(warning.contains("did not latch"), "says it did not latch");
        assert!(warning.contains("reported 9"), "quotes the real readback");
    }
}
