//! Charging limit ("Battery Care") — firmware-authoritative path.
//!
//! # What actually controls the limit on this platform
//!
//! The limit lives in two EC registers: `ERAM 0xA4` (`LONL`, bit0 = care on)
//! and `ERAM 0xA7` (`HBDA`, the limit in percent). Both sit inside the
//! `SystemMemory 0xFE0B0300` region that `IoTDriver.sys` **refuses** to expose
//! through its ECRAM IOCTLs (`STATUS_ACCESS_DENIED` / `0x80070005`, verified
//! live even for the SYSTEM `IoTService.exe`). Every ERAM write path this file
//! used to attempt failed and silently fell back to the registry, so the UI
//! showed a limit the hardware never received.
//!
//! The firmware exposes the same two registers through the ACPI `WMAA` method
//! (`MICommonInterface.MiInterface`, `\_SB.PC00.WMID`, `_HID` `PNP0C14`,
//! `_UID` `"MIFS"`). Decoded from `ssdt24.dsl`, Method `WMAA`, `FUN1=0xFB00`,
//! `FUN2=0x1000`, `FUN3=0x02`:
//!
//! | FUN4 | effect                                        |
//! |------|-----------------------------------------------|
//! | 0    | `LONL &= ~1`, `HBDA = 0x64` → care OFF (100%) |
//! | 1    | `LONL |= 1`,  `HBDA = 0x50` → 80%             |
//! | 4    | `LONL |= 1`,  `HBDA = 0x5A` → 90%             |
//! | 5    | `LONL |= 1`,  `HBDA = 0x46` → 70%             |
//! | 6    | `LONL |= 1`,  `HBDA = 0x3C` → 60%             |
//! | 7    | `LONL |= 1`,  `HBDA = 0x32` → 50%             |
//! | 8    | `LONL |= 1`,  `HBDA = 0x28` → 40%             |
//!
//! The read branch (`FUN1=0xFA00`, same FUN2/FUN3) returns the same code in
//! `FRD1`: 0 = care off, otherwise the active level. The code set is the OEM
//! enum the original PC Manager UI sends (`gp.One=1 … gp.Eight=8`), i.e. the
//! payload is a **level code, not a percentage**.
//!
//! This module therefore writes and verifies through WMAA and never claims
//! success without a read-back. The registry (`HKLM\SOFTWARE\MI\IoTDriver`
//! `ChargingThreshold`) is only the user's *requested* value and for
//! re-assertion; it is never reported as applied hardware state.

use crate::hw::errors::{HardwareError, HardwareResult};
use serde::{Deserialize, Serialize};

/// Registry key shared with the original Xiaomi IoT software.
#[cfg(windows)]
const CHARGE_REG_KEY: &str = r"SOFTWARE\MI\IoTDriver";
#[cfg(windows)]
const CHARGE_REG_VALUE: &str = "ChargingThreshold";

/// WMAA sub-command group used by the charging-protection method.
pub const FUN2_CHARGE: u16 = 0x1000;
/// WMAA action id inside `FUN2_CHARGE` (read and write share it).
pub const FUN3_CHARGE: u16 = 0x02;
/// `SGER` value the ACPI method returns on success.
pub const SGER_SUCCESS: u16 = 0x8000;

/// Percentages the firmware can express. 100 means "no limit" (care off).
pub const VALID_THRESHOLDS: [u8; 7] = [40, 50, 60, 70, 80, 90, 100];

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct ChargingResult {
    /// True only when the firmware accepted **and** read back the new value.
    pub success: bool,
    /// `wmi_wmaa` when the value is on the hardware, `unverified` otherwise.
    pub method: String,
    /// The value the caller asked for.
    pub threshold: u8,
    /// The value the firmware reported after the write (`None` = unreadable).
    pub hardware_threshold: Option<u8>,
    /// Human-readable reason when `success` is false.
    pub detail: Option<String>,
}

/// The truth about the charging limit: what the user asked for, what the
/// firmware actually holds, and whether they agree.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct ChargeLimitState {
    /// Last requested value (registry). 100 = no limit.
    pub requested_threshold: u8,
    /// Value read back from the firmware; `None` when the read failed.
    pub hardware_threshold: Option<u8>,
    /// Raw firmware level code (0,1,4,5,6,7,8); `None` when unreadable.
    pub hardware_code: Option<u8>,
    /// True only when the firmware read succeeded and equals the request.
    pub applied: bool,
    /// `firmware` | `registry` (read failed) — how much of this is real.
    pub source: String,
    /// Why the hardware value is unknown, when it is.
    pub detail: Option<String>,
}

/// Map a user-facing percentage to the firmware level code.
///
/// Returns `None` for values the firmware cannot express.
pub fn percent_to_code(percent: u8) -> Option<u8> {
    match percent {
        40 => Some(8),
        50 => Some(7),
        60 => Some(6),
        70 => Some(5),
        80 => Some(1),
        90 => Some(4),
        100 => Some(0),
        _ => None,
    }
}

/// Map a firmware level code back to a percentage.
pub fn code_to_percent(code: u8) -> Option<u8> {
    match code {
        8 => Some(40),
        7 => Some(50),
        6 => Some(60),
        5 => Some(70),
        1 => Some(80),
        4 => Some(90),
        0 => Some(100),
        _ => None,
    }
}

/// Parse the `FRD1` value of a `FUN2=0x1000 / FUN3=0x02` read into a code.
///
/// `FRD1` is a DWORD; anything outside the OEM code set is a firmware
/// surprise and is rejected instead of being coerced into a percentage.
pub fn parse_level_code(frd1: u32) -> Option<u8> {
    let code = u8::try_from(frd1).ok()?;
    code_to_percent(code).map(|_| code)
}

fn validate(threshold: u8) -> HardwareResult<u8> {
    percent_to_code(threshold).ok_or_else(|| {
        HardwareError::InvalidConfig(format!(
            "Invalid threshold {threshold}. Must be one of: 40,50,60,70,80,90,100"
        ))
    })
}

/// Read the limit straight from the firmware.
///
/// Returns `Err` when the WMAA read fails (no admin, provider stall, WMI
/// blocked). Callers must not translate that error into a value.
pub fn read_hardware_threshold() -> HardwareResult<u8> {
    #[cfg(windows)]
    {
        let resp = crate::hw::wmi_ec::wmi_read(FUN2_CHARGE, FUN3_CHARGE)?;
        if resp.sger != SGER_SUCCESS {
            return Err(HardwareError::Wmi(format!(
                "WMAA 0x1000/0x02 read rejected: SGER=0x{:04X}",
                resp.sger
            )));
        }
        let code = parse_level_code(resp.frd1).ok_or_else(|| {
            HardwareError::Wmi(format!(
                "WMAA 0x1000/0x02 returned unknown level code {:#X}",
                resp.frd1
            ))
        })?;
        Ok(code_to_percent(code).expect("parse_level_code validated the code"))
    }
    #[cfg(not(windows))]
    {
        Err(HardwareError::NotSupported(
            "WMI only available on Windows".into(),
        ))
    }
}

/// Set the charging limit in the firmware and verify it by reading back.
///
/// There is deliberately **no silent fallback**: a failed write is an error,
/// because reporting success while the EC still charges to 100% is exactly
/// the bug this function exists to fix.
pub fn set_charging_threshold(threshold: u8) -> HardwareResult<ChargingResult> {
    let code = validate(threshold)?;

    #[cfg(windows)]
    {
        let resp = crate::hw::wmi_ec::wmi_write(FUN2_CHARGE, FUN3_CHARGE, code as u32)?;
        if resp.sger != SGER_SUCCESS {
            return Err(HardwareError::Wmi(format!(
                "WMAA 0x1000/0x02 write rejected: SGER=0x{:04X}",
                resp.sger
            )));
        }

        // The ACPI method sets SGER before touching the EC and never reports
        // a failure of the ECWT itself, so SGER alone is not proof. Read back.
        match read_hardware_threshold() {
            Ok(actual) if actual == threshold => {
                persist_threshold_registry(threshold).ok();
                log::info!(
                    target: "hw::charging",
                    "charging limit {threshold}% applied on firmware (WMAA 0x1000/0x02, code {code})"
                );
                Ok(ChargingResult {
                    success: true,
                    method: "wmi_wmaa".to_string(),
                    threshold,
                    hardware_threshold: Some(actual),
                    detail: None,
                })
            }
            Ok(actual) => {
                log::warn!(
                    target: "hw::charging",
                    "charging limit write acknowledged but firmware reads {actual}% (wanted {threshold}%)"
                );
                Err(HardwareError::Wmi(format!(
                    "firmware accepted the write but still reports {actual}% (wanted {threshold}%)"
                )))
            }
            Err(e) => {
                log::warn!(
                    target: "hw::charging",
                    "charging limit written but read-back failed: {e}"
                );
                Err(HardwareError::Wmi(format!(
                    "limit written to firmware but the read-back failed: {e}"
                )))
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = code;
        Err(HardwareError::NotSupported(
            "WMI only available on Windows".into(),
        ))
    }
}

/// Combined view: requested value (registry) + firmware value.
///
/// Never fails: an unreadable firmware is reported as
/// `hardware_threshold: None, applied: false, detail: Some(..)` so the UI can
/// say "not confirmed on the hardware" instead of inventing a value.
pub fn get_charge_limit_state() -> ChargeLimitState {
    let requested = get_requested_threshold();
    match read_hardware_threshold() {
        Ok(hardware) => ChargeLimitState {
            requested_threshold: requested,
            hardware_threshold: Some(hardware),
            hardware_code: percent_to_code(hardware),
            applied: hardware == requested,
            source: "firmware".to_string(),
            detail: None,
        },
        Err(e) => ChargeLimitState {
            requested_threshold: requested,
            hardware_threshold: None,
            hardware_code: None,
            applied: false,
            source: "registry".to_string(),
            detail: Some(e.to_string()),
        },
    }
}

/// The user's requested limit (registry), 100 when unset.
///
/// This is **not** hardware state — call it what it is.
pub fn get_requested_threshold() -> u8 {
    #[cfg(windows)]
    {
        read_threshold_registry()
            .and_then(|r| r.ok())
            .unwrap_or(100)
    }
    #[cfg(not(windows))]
    {
        100
    }
}

/// Backwards-compatible accessor: the requested limit.
///
/// Prefer [`get_charge_limit_state`] when the UI needs to tell the truth about
/// whether the hardware accepted it.
pub fn get_charging_threshold() -> HardwareResult<u8> {
    Ok(get_requested_threshold())
}

// ── Reconciliation (P1-2, recheck 2026-11-11) ───────────────────────────────
//
// The EC forgets the limit across an S3 cycle and across `ecram_service`
// restarts. The 2026-11-11 recheck found the hardware at 100 % (code 0) while
// the registry still said 80 %: the limit applied at 20:17 was gone after the
// 02:23 → 10:55 resume, and nothing ever re-asserted it. The read-back was
// honest (`applied=false`) but the machine charged to 100 % for 8 h.
//
// The reconciler re-asserts the *requested* value, but only through a gate:
// one write per cooldown window, exponential backoff on failure. A firmware
// that refuses the write can therefore never turn into a write loop.

/// Minimum spacing between two reconciliation writes.
pub const RECONCILE_MIN_INTERVAL_MS: u64 = 5 * 60 * 1000;
/// Ceiling for the failure backoff.
pub const RECONCILE_MAX_BACKOFF_MS: u64 = 60 * 60 * 1000;
/// Floor between two *forced* re-asserts (resume / AC plug).
pub const FORCE_MIN_INTERVAL_MS: u64 = 30 * 1000;
/// How long a firmware read-back stays usable for the hold state.
pub const FIRMWARE_CACHE_TTL_MS: u64 = 15 * 60 * 1000;
/// Hold supervisor tick. State machine only — it never writes on a timer.
pub const HOLD_TICK_MS: u64 = 45 * 1000;
/// Width of the band below the limit that still counts as "held".
pub const HOLD_RESUME_MARGIN: u8 = 2;

/// What the reconciler should do with the current (requested, hardware) pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconcileAction {
    /// The firmware already holds the requested value.
    InSync,
    /// Care is off (requested 100 %). Turning care *off* is a user action, not
    /// a drift to auto-correct: writing 100 % would silently disable battery
    /// care, so the reconciler leaves it alone.
    CareOff,
    /// The firmware value could not be read — never write blind.
    Unreadable,
    /// Write the requested value now.
    Apply,
    /// A write is due but the gate is closed; retry after `retry_in_ms`.
    Deferred { retry_in_ms: u64 },
}

impl ReconcileAction {
    /// Short label for the log line.
    pub fn describe(self) -> &'static str {
        match self {
            Self::InSync => "in sync",
            Self::CareOff => "care off",
            Self::Unreadable => "hardware unreadable",
            Self::Apply => "apply",
            Self::Deferred { .. } => "deferred",
        }
    }
}

/// Write gate for the reconciler. Pure and clock-injected so every transition
/// is unit-testable without touching the firmware.
#[derive(Debug, Default, Clone, Copy)]
pub struct ReconcileGate {
    last_attempt_ms: Option<u64>,
    consecutive_failures: u32,
}

impl ReconcileGate {
    pub const fn new() -> Self {
        Self {
            last_attempt_ms: None,
            consecutive_failures: 0,
        }
    }

    /// How long the gate stays closed after the last attempt.
    ///
    /// 5 min after a success; 10, 20, 40, 60 min after consecutive failures.
    pub fn cooldown_ms(&self) -> u64 {
        if self.consecutive_failures == 0 {
            return RECONCILE_MIN_INTERVAL_MS;
        }
        let factor = 1u64 << self.consecutive_failures.min(4);
        RECONCILE_MIN_INTERVAL_MS
            .saturating_mul(factor)
            .min(RECONCILE_MAX_BACKOFF_MS)
    }

    /// Decide what to do at `now_ms`.
    pub fn decide(&self, now_ms: u64, requested: u8, hardware: Option<u8>) -> ReconcileAction {
        if requested >= 100 {
            return ReconcileAction::CareOff;
        }
        let Some(hardware) = hardware else {
            return ReconcileAction::Unreadable;
        };
        if hardware == requested {
            return ReconcileAction::InSync;
        }
        match self.last_attempt_ms {
            None => ReconcileAction::Apply,
            Some(last) => {
                let elapsed = now_ms.saturating_sub(last);
                let cooldown = self.cooldown_ms();
                if elapsed >= cooldown {
                    ReconcileAction::Apply
                } else {
                    ReconcileAction::Deferred {
                        retry_in_ms: cooldown - elapsed,
                    }
                }
            }
        }
    }

    /// Same decision, but a *power event* (resume, AC plug) may shorten the
    /// wait to [`FORCE_MIN_INTERVAL_MS`].
    ///
    /// The 5 min window exists to stop a UI poll from becoming a write loop.
    /// A resume or an AC plug is not a poll: the EC has just dropped the
    /// setting and every minute spent waiting is charge the user did not ask
    /// for. The failure backoff is still honoured — a firmware that refuses
    /// the write must never be hammered, whatever triggered the attempt.
    pub fn decide_forced(
        &self,
        now_ms: u64,
        requested: u8,
        hardware: Option<u8>,
    ) -> ReconcileAction {
        let base = self.decide(now_ms, requested, hardware);
        if !matches!(base, ReconcileAction::Deferred { .. }) {
            return base;
        }
        if self.consecutive_failures > 0 {
            return base;
        }
        match self.last_attempt_ms {
            Some(last) if now_ms.saturating_sub(last) >= FORCE_MIN_INTERVAL_MS => {
                ReconcileAction::Apply
            }
            _ => base,
        }
    }

    /// Record that a write is being attempted now.
    pub fn note_attempt(&mut self, now_ms: u64) {
        self.last_attempt_ms = Some(now_ms);
    }

    /// The firmware accepted the write — clear the backoff.
    pub fn note_success(&mut self) {
        self.consecutive_failures = 0;
    }

    /// The write failed — widen the next window.
    pub fn note_failure(&mut self) {
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);
    }

    /// Consecutive failures so far (diagnostics).
    pub fn consecutive_failures(&self) -> u32 {
        self.consecutive_failures
    }
}

static RECONCILE_GATE: std::sync::Mutex<ReconcileGate> =
    std::sync::Mutex::new(ReconcileGate::new());

fn reconcile_gate() -> std::sync::MutexGuard<'static, ReconcileGate> {
    match RECONCILE_GATE.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

// ── Active hold (F5, 2026-11-11) ────────────────────────────────────────────
//
// What this firmware can and cannot do — measured, see
// `audit-20261107/CHARGE-HOLD-DEBUG.md`:
//
//  * The limit is a **ceiling**, not a setpoint. The EC stops charging at
//    `HBDA` and lets the pack sit there. There is no discharge command
//    anywhere: the WMAA write branch of `FUN2=0x1000` has exactly one case
//    (`FUN3=0x02` → `LONL`/`HBDA`), `IoTDriver.sys` exposes exactly two
//    IOCTLs over three physical regions (none of them ERAM), and the original
//    `IoTService.exe` has no charging command at all. A pack above the limit
//    therefore stays there — the app must say so, not promise a discharge.
//  * The EC drops `LONL`/`HBDA` on its own across power-source changes and S3
//    cycles: on 2026-11-11 the read-back returned code 0 while the last write
//    was 80 and no software wrote 0. That is why the original PC Manager
//    re-applies on every AC/DC transition
//    (`PerformanceModesModule::OnAcDcPowerSource` → `resume_charging_protect`).
//
// So "hold at 80 %" means: keep the firmware value at 80 and verify it. The
// supervisor below is the loop that does exactly that, and reports the phase
// the machine is really in.

/// Which part of the hold cycle the machine is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldPhase {
    /// No limit requested (100 %): the pack charges to full.
    Off,
    /// The firmware holds the requested value and the pack sits in the band.
    Holding,
    /// Above the limit. This firmware cannot discharge — it will hold on the
    /// next charge cycle instead.
    AboveLimit,
    /// Plugged, below the band, charging toward the limit.
    Charging,
    /// Plugged, below the band, not charging right now (thermal pause, etc.).
    Idle,
    /// On battery, below the limit — the ceiling applies at the next plug-in.
    OnBattery,
    /// The firmware reports a different value than the request.
    Drifted,
    /// The firmware value could not be read (or is stale).
    Unknown,
}

impl HoldPhase {
    /// Short label for logs and diagnostics.
    pub fn describe(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Holding => "holding",
            Self::AboveLimit => "above limit",
            Self::Charging => "charging",
            Self::Idle => "idle",
            Self::OnBattery => "on battery",
            Self::Drifted => "drifted",
            Self::Unknown => "unknown",
        }
    }
}

/// One snapshot of the hold state, as the UI needs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChargeHoldState {
    pub phase: HoldPhase,
    /// Battery level in percent; `None` when no snapshot is available.
    pub level: Option<u8>,
    /// Requested limit (registry). 100 = no limit.
    pub limit: u8,
    /// Lowest level that still counts as held (`limit - HOLD_RESUME_MARGIN`).
    pub band_low: u8,
    /// The limit itself.
    pub band_high: u8,
    /// Value the firmware reported at the last read-back.
    pub firmware_threshold: Option<u8>,
    /// True only when the firmware reported the requested value.
    pub applied: bool,
    pub charging: Option<bool>,
    pub plugged: Option<bool>,
    /// Always false on this firmware: there is no discharge channel. Exposed
    /// so the UI can state the limit without promising a discharge.
    pub can_discharge: bool,
    /// Why the phase is what it is, when that needs saying.
    pub detail: Option<String>,
    /// When the firmware value was last verified (ms since epoch).
    pub verified_ms: Option<u64>,
}

/// Decide the phase from four raw facts. Pure — every branch is unit-tested.
///
/// `firmware` is the last read-back of `WMAA 0x1000/0x02`; `None` means the
/// value is unknown (never read, or older than the cache TTL) and the app must
/// not claim the limit is applied.
pub fn decide_hold(
    level: Option<u8>,
    limit: u8,
    firmware: Option<u8>,
    charging: Option<bool>,
    plugged: Option<bool>,
) -> HoldPhase {
    if limit >= 100 {
        return HoldPhase::Off;
    }
    let Some(level) = level else {
        return HoldPhase::Unknown;
    };
    let Some(hardware) = firmware else {
        return HoldPhase::Unknown;
    };
    if hardware != limit {
        return HoldPhase::Drifted;
    }
    if level > limit {
        return HoldPhase::AboveLimit;
    }
    if level.saturating_add(HOLD_RESUME_MARGIN) >= limit {
        return HoldPhase::Holding;
    }
    match (plugged, charging) {
        (Some(false), _) => HoldPhase::OnBattery,
        (Some(true), Some(false)) => HoldPhase::Idle,
        _ => HoldPhase::Charging,
    }
}

/// Build the full state (phase + the numbers the UI renders).
pub fn compute_hold_state(
    level: Option<u8>,
    limit: u8,
    firmware: Option<u8>,
    charging: Option<bool>,
    plugged: Option<bool>,
    verified_ms: Option<u64>,
) -> ChargeHoldState {
    let phase = decide_hold(level, limit, firmware, charging, plugged);
    let detail = match phase {
        HoldPhase::Unknown if level.is_none() => Some("battery level unavailable".to_string()),
        HoldPhase::Unknown => Some("charge limit not verified yet".to_string()),
        HoldPhase::Drifted => Some(format!(
            "firmware reports {}%, requested {limit}%",
            firmware
                .map(|v| v.to_string())
                .unwrap_or_else(|| "an unknown value".into())
        )),
        HoldPhase::AboveLimit => Some(
            "above the limit: this firmware has no discharge channel, so the pack stays here \
             until it is used"
                .to_string(),
        ),
        _ => None,
    };
    ChargeHoldState {
        phase,
        level,
        limit,
        band_low: limit.saturating_sub(HOLD_RESUME_MARGIN),
        band_high: limit,
        firmware_threshold: firmware,
        applied: firmware == Some(limit),
        charging,
        plugged,
        can_discharge: false,
        detail,
        verified_ms,
    }
}

#[cfg(windows)]
static LAST_FIRMWARE: std::sync::Mutex<Option<(u8, u64)>> = std::sync::Mutex::new(None);

#[cfg(windows)]
fn firmware_cache() -> std::sync::MutexGuard<'static, Option<(u8, u64)>> {
    crate::util::panic::lock_or_recover(&LAST_FIRMWARE)
}

/// Record the firmware truth returned by a read-back.
///
/// The WMAA read runs in the *elevated* process, so the app process cannot
/// observe it directly; every path that receives a [`ChargeLimitState`] from
/// the bridge must call this, otherwise the hold state falls back to `Unknown`.
pub fn note_firmware_state(state: &ChargeLimitState) {
    #[cfg(windows)]
    {
        if let Some(value) = state.hardware_threshold {
            *firmware_cache() = Some((value, crate::util::auth::now_ms()));
        }
    }
    #[cfg(not(windows))]
    let _ = state;
}

#[cfg(windows)]
fn note_firmware_value(value: u8) {
    *firmware_cache() = Some((value, crate::util::auth::now_ms()));
}

/// Last verified firmware value and when it was seen, if still fresh.
#[cfg(windows)]
pub fn last_firmware(now_ms: u64) -> Option<(u8, u64)> {
    match *firmware_cache() {
        Some((value, at)) if now_ms.saturating_sub(at) <= FIRMWARE_CACHE_TTL_MS => {
            Some((value, at))
        }
        _ => None,
    }
}

/// Current hold state, computed from the cached battery snapshot.
///
/// Cheap by design: no WMI query and no elevated round-trip, so the UI can
/// poll it as often as it likes. The firmware half comes from the last
/// read-back; when it is older than [`FIRMWARE_CACHE_TTL_MS`] the phase is
/// `Unknown` and the supervisor re-verifies.
#[cfg(windows)]
pub fn current_hold_state() -> ChargeHoldState {
    let now = crate::util::auth::now_ms();
    let snapshot = crate::hw::battery::cached_battery_info();
    let (level, charging, plugged) = match &snapshot {
        Some(b) => (
            Some(b.level.clamp(0, 100)),
            Some(b.is_charging),
            Some(b.is_plugged),
        ),
        None => (None, None, None),
    };
    let (firmware, verified_ms) = match last_firmware(now) {
        Some((value, at)) => (Some(value), Some(at)),
        None => (None, None),
    };
    compute_hold_state(
        level,
        get_requested_threshold(),
        firmware,
        charging,
        plugged,
        verified_ms,
    )
}

#[cfg(not(windows))]
pub fn current_hold_state() -> ChargeHoldState {
    compute_hold_state(None, get_requested_threshold(), None, None, None, None)
}

/// Re-assert the requested charging limit when the firmware has drifted.
///
/// Called at app start, on resume from sleep, and whenever a read-back reports
/// `applied=false` with care on. Every path goes through [`ReconcileGate`], so
/// the worst case is one write per 5 min (widening to 60 min while the
/// firmware keeps refusing).
///
/// Runs in the **app** process and dispatches through the elevated bridge: the
/// MICommonInterface instance query is denied to non-admin callers, which is
/// why the read and the write are elevated commands in the first place.
#[cfg(windows)]
pub async fn reconcile_charge_limit(trigger: &str) -> ReconcileAction {
    reconcile_charge_limit_with(trigger, false).await
}

/// Reconcile, optionally treating the trigger as a power event.
///
/// `force = true` is for resume and AC-plug: the EC has just dropped the
/// setting, so waiting out the 5 min poll window means charging above the
/// limit. The failure backoff still applies (see [`ReconcileGate::decide_forced`]).
#[cfg(windows)]
pub async fn reconcile_charge_limit_with(trigger: &str, force: bool) -> ReconcileAction {
    use crate::elev_bridge;

    // Read the truth. A failed read is not a reason to write.
    let state: ChargeLimitState =
        match elev_bridge::run_elevated_no_prompt("get_charge_limit_state", serde_json::json!({}))
            .await
        {
            Ok(raw) => match serde_json::from_value(raw) {
                Ok(state) => state,
                Err(e) => {
                    log::debug!(
                        target: "hw::charging",
                        "[charge_limit] reconcile ({trigger}): unreadable state ({e})"
                    );
                    return ReconcileAction::Unreadable;
                }
            },
            Err(e) => {
                log::debug!(
                    target: "hw::charging",
                    "[charge_limit] reconcile ({trigger}): state read failed ({e})"
                );
                return ReconcileAction::Unreadable;
            }
        };

    // Every read refreshes the cache the hold state renders from.
    note_firmware_state(&state);

    let now = crate::util::auth::now_ms();
    let action = {
        let gate = reconcile_gate();
        if force {
            gate.decide_forced(now, state.requested_threshold, state.hardware_threshold)
        } else {
            gate.decide(now, state.requested_threshold, state.hardware_threshold)
        }
    };

    match action {
        ReconcileAction::Apply => {
            {
                let mut gate = reconcile_gate();
                gate.note_attempt(now);
            }
            let requested = state.requested_threshold;
            let result = elev_bridge::run_elevated_no_prompt(
                "set_charging_threshold",
                serde_json::json!({ "threshold": requested }),
            )
            .await;

            match result {
                Ok(raw) => {
                    let applied = serde_json::from_value::<ChargingResult>(raw)
                        .ok()
                        .filter(|r| r.success);
                    match applied {
                        Some(r) => {
                            reconcile_gate().note_success();
                            if let Some(hw) = r.hardware_threshold {
                                note_firmware_value(hw);
                            }
                            log::info!(
                                target: "hw::charging",
                                "[charge_limit] re-applied {requested}% on {trigger} \
                                 (was {:?}%, read-back {:?}%)",
                                state.hardware_threshold,
                                r.hardware_threshold
                            );
                        }
                        None => {
                            let mut gate = reconcile_gate();
                            gate.note_failure();
                            let next = gate.cooldown_ms() / 1000;
                            log::warn!(
                                target: "hw::charging",
                                "[charge_limit] re-apply {requested}% on {trigger} was not \
                                 confirmed by the firmware (was {:?}%) — next attempt in {next} s",
                                state.hardware_threshold
                            );
                        }
                    }
                }
                Err(e) => {
                    let mut gate = reconcile_gate();
                    gate.note_failure();
                    let next = gate.cooldown_ms() / 1000;
                    log::warn!(
                        target: "hw::charging",
                        "[charge_limit] re-apply {requested}% on {trigger} failed: {e} — \
                         next attempt in {next} s"
                    );
                }
            }
        }
        ReconcileAction::Deferred { retry_in_ms } => log::debug!(
            target: "hw::charging",
            "[charge_limit] reconcile ({trigger}): drift {:?}% → {}% deferred, retry in {} s",
            state.hardware_threshold,
            state.requested_threshold,
            retry_in_ms / 1000
        ),
        ReconcileAction::InSync | ReconcileAction::CareOff | ReconcileAction::Unreadable => {
            log::debug!(
                target: "hw::charging",
                "[charge_limit] reconcile ({trigger}): {} (requested {}%, hardware {:?}%)",
                action.describe(),
                state.requested_threshold,
                state.hardware_threshold
            );
        }
    }

    action
}

/// Periodic hold supervisor (45 s tick, 5 min verify).
///
/// The UI only reads the limit when the Battery tab opens, so a drift that
/// happens while the app sits in the tray would otherwise persist until the
/// user looked. Two different cadences, on purpose:
///
///  * **45 s** — recompute the phase from the cached battery snapshot. No WMI
///    query and no elevated call, so it costs nothing; it is what makes the
///    log show the real state ("holding at 80", "above limit", "charging") and
///    what the UI polls.
///  * **5 min** — verify the firmware value through the elevated bridge and
///    re-assert on drift. An elevated round-trip every 45 s would spawn a
///    scheduled task 80 times an hour; the reconcile gate already caps writes
///    to one per window, and the events that actually drop the setting
///    (resume, AC plug) take the forced path instead of waiting for this tick.
#[cfg(windows)]
pub fn start_charge_hold_supervisor() {
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    tauri::async_runtime::spawn(async {
        // Let the bridge service and the elevated path settle after boot.
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        let mut last_verify_ms: u64 = 0;
        let mut last_phase: Option<HoldPhase> = None;
        loop {
            let now = crate::util::auth::now_ms();
            let state = current_hold_state();

            if last_phase != Some(state.phase) {
                log::info!(
                    target: "hw::charging",
                    "[charge_hold] phase={} ({}%) limit={}% band=[{}..{}] firmware={:?}% charging={:?} plugged={:?}",
                    state.phase.describe(),
                    state
                        .level
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "?".into()),
                    state.limit,
                    state.band_low,
                    state.band_high,
                    state.firmware_threshold,
                    state.charging,
                    state.plugged
                );
                last_phase = Some(state.phase);
            }

            let verify_due = now.saturating_sub(last_verify_ms) >= RECONCILE_MIN_INTERVAL_MS;
            // A drifted/unknown firmware read is worth re-checking sooner than
            // the 5 min window — but never more than once a minute.
            let drift_due = matches!(state.phase, HoldPhase::Drifted | HoldPhase::Unknown)
                && now.saturating_sub(last_verify_ms) >= 60_000;
            if verify_due || drift_due {
                last_verify_ms = now;
                reconcile_charge_limit("hold").await;
                // The reconcile just refreshed the firmware cache; recompute so
                // the next tick's transition compares like with like.
                let refreshed = current_hold_state();
                if last_phase != Some(refreshed.phase) {
                    log::info!(
                        target: "hw::charging",
                        "[charge_hold] phase={} after verify (firmware={:?}%)",
                        refreshed.phase.describe(),
                        refreshed.firmware_threshold
                    );
                    last_phase = Some(refreshed.phase);
                }
            }

            tokio::time::sleep(std::time::Duration::from_millis(HOLD_TICK_MS)).await;
        }
    });
}

// ── Battery Care toggle ─────────────────────────────────────────────────────
//
// "Care" and "limit" are the same firmware field: code 0 means care off
// (charges to 100%), any other code means care on at that level. The toggle is
// therefore expressed through the same WMAA write.

/// Is charging protection currently active on the firmware?
pub fn get_battery_care() -> HardwareResult<bool> {
    match read_hardware_threshold() {
        Ok(percent) => Ok(percent != 100),
        Err(e) => {
            log::warn!(
                target: "hw::charging",
                "battery care state unreadable from firmware: {e}"
            );
            Err(e)
        }
    }
}

/// Turn charging protection on (at `limit`) or off (charges to 100%).
pub fn set_battery_care(enabled: bool, limit: u8) -> HardwareResult<ChargingResult> {
    let target = if enabled {
        if limit >= 100 || percent_to_code(limit).is_none() {
            80
        } else {
            limit
        }
    } else {
        100
    };
    set_charging_threshold(target)
}

// ── Registry persistence (requested value only) ─────────────────────────────

fn persist_threshold_registry(threshold: u8) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
        match RegKeyGuard::create_write(HKEY_LOCAL_MACHINE, CHARGE_REG_KEY) {
            Ok(key) => {
                key.write_u32(CHARGE_REG_VALUE, threshold as u32)
                    .map_err(|e| {
                        HardwareError::Registry(format!(
                            "Write charging threshold to registry: {e}"
                        ))
                    })?;
            }
            Err(e) => {
                log::debug!("Cannot open charging registry key for write: {e}");
            }
        }
    }
    Ok(())
}

#[cfg(windows)]
fn read_threshold_registry() -> Option<HardwareResult<u8>> {
    use crate::util::registry::RegKeyGuard;
    use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;
    let key = RegKeyGuard::open_read(HKEY_LOCAL_MACHINE, CHARGE_REG_KEY).ok()??;
    match key.read_u32(CHARGE_REG_VALUE) {
        Ok(Some(data)) => Some(Ok(data.clamp(40, 100) as u8)),
        Ok(None) => None,
        Err(e) => Some(Err(HardwareError::Registry(format!(
            "Read charging threshold: {e}"
        )))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_to_code_matches_oem_enum_and_ssdt() {
        // OEM PC Manager enum gp: One=1 (80%), Four=4 (90%), Five=5 (70%),
        // Six=6 (60%), Seven=7 (50%), Eight=8 (40%); 0 = care off.
        assert_eq!(percent_to_code(80), Some(1));
        assert_eq!(percent_to_code(90), Some(4));
        assert_eq!(percent_to_code(70), Some(5));
        assert_eq!(percent_to_code(60), Some(6));
        assert_eq!(percent_to_code(50), Some(7));
        assert_eq!(percent_to_code(40), Some(8));
        assert_eq!(percent_to_code(100), Some(0));
    }

    #[test]
    fn percent_to_code_rejects_values_the_firmware_cannot_express() {
        for bad in [0u8, 1, 79, 81, 95, 99, 101, 255] {
            assert_eq!(percent_to_code(bad), None, "{bad} must not be writable");
        }
    }

    #[test]
    fn code_roundtrip_is_identity() {
        for p in VALID_THRESHOLDS {
            let c = percent_to_code(p).expect("valid");
            assert_eq!(code_to_percent(c), Some(p), "roundtrip for {p}%");
        }
    }

    #[test]
    fn parse_level_code_accepts_only_the_oem_code_set() {
        for code in [0u32, 1, 4, 5, 6, 7, 8] {
            assert_eq!(parse_level_code(code), Some(code as u8));
        }
        for bad in [2u32, 3, 9, 80, 0x50, 0xFFFF_FFFF] {
            assert_eq!(
                parse_level_code(bad),
                None,
                "{bad:#X} is not a firmware level code"
            );
        }
    }

    #[test]
    fn threshold_is_a_code_not_a_percentage() {
        // The historical bug: the MCPI 0x1003 payload carried the percentage
        // (80) where the firmware expects the level code (1). Pin the
        // distinction so it cannot regress silently.
        assert_ne!(percent_to_code(80).unwrap(), 80);
        assert_eq!(parse_level_code(80), None);
    }

    #[test]
    fn charging_result_serializes_the_honesty_fields() {
        let r = ChargingResult {
            success: false,
            method: "unverified".into(),
            threshold: 80,
            hardware_threshold: None,
            detail: Some("access denied".into()),
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["success"], serde_json::json!(false));
        assert_eq!(v["threshold"], serde_json::json!(80));
        assert_eq!(v["hardware_threshold"], serde_json::Value::Null);
        assert_eq!(v["detail"], serde_json::json!("access denied"));
    }

    #[test]
    fn state_never_claims_applied_without_hardware() {
        let s = ChargeLimitState {
            requested_threshold: 80,
            hardware_threshold: None,
            hardware_code: None,
            applied: false,
            source: "registry".into(),
            detail: Some("ECRAM blocked".into()),
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["applied"], serde_json::json!(false));
        assert_eq!(v["source"], serde_json::json!("registry"));
    }

    // ── P1-2: the reconciler ────────────────────────────────────────────────

    /// The production drift: requested 80, firmware 100 → write.
    #[test]
    fn drift_with_care_on_is_applied() {
        let gate = ReconcileGate::new();
        assert_eq!(gate.decide(1_000, 80, Some(100)), ReconcileAction::Apply);
    }

    /// Nothing to do when the firmware already holds the request.
    #[test]
    fn matching_hardware_is_in_sync() {
        let gate = ReconcileGate::new();
        assert_eq!(gate.decide(1_000, 80, Some(80)), ReconcileAction::InSync);
    }

    /// Care off (100 %) is a user decision, not a drift to auto-correct.
    #[test]
    fn care_off_is_never_rewritten() {
        let gate = ReconcileGate::new();
        assert_eq!(gate.decide(1_000, 100, Some(80)), ReconcileAction::CareOff);
        assert_eq!(gate.decide(1_000, 100, Some(100)), ReconcileAction::CareOff);
    }

    /// An unreadable firmware must never be written blind.
    #[test]
    fn unreadable_hardware_is_not_written() {
        let gate = ReconcileGate::new();
        assert_eq!(gate.decide(1_000, 80, None), ReconcileAction::Unreadable);
    }

    /// One write per 5 min: a second call inside the window is deferred, and
    /// the deferral reports the remaining time.
    #[test]
    fn writes_are_rate_limited_to_one_per_five_minutes() {
        let mut gate = ReconcileGate::new();
        assert_eq!(gate.decide(0, 80, Some(100)), ReconcileAction::Apply);
        gate.note_attempt(0);
        gate.note_success();

        assert_eq!(
            gate.decide(60_000, 80, Some(100)),
            ReconcileAction::Deferred {
                retry_in_ms: RECONCILE_MIN_INTERVAL_MS - 60_000
            }
        );
        // Exactly at the boundary the gate reopens.
        assert_eq!(
            gate.decide(RECONCILE_MIN_INTERVAL_MS, 80, Some(100)),
            ReconcileAction::Apply
        );
    }

    /// A firmware that keeps refusing must widen the window, never loop.
    #[test]
    fn failures_back_off_exponentially_up_to_the_cap() {
        let mut gate = ReconcileGate::new();
        assert_eq!(gate.cooldown_ms(), RECONCILE_MIN_INTERVAL_MS);

        let mut seen = Vec::new();
        for _ in 0..6 {
            gate.note_failure();
            seen.push(gate.cooldown_ms());
        }
        assert_eq!(
            seen,
            vec![
                10 * 60_000,
                20 * 60_000,
                40 * 60_000,
                RECONCILE_MAX_BACKOFF_MS,
                RECONCILE_MAX_BACKOFF_MS,
                RECONCILE_MAX_BACKOFF_MS,
            ]
        );
        assert_eq!(gate.consecutive_failures(), 6);

        // A success clears the backoff entirely.
        gate.note_success();
        assert_eq!(gate.cooldown_ms(), RECONCILE_MIN_INTERVAL_MS);
        assert_eq!(gate.consecutive_failures(), 0);
    }

    /// The gate must not write twice inside a backoff window even when the
    /// drift is still present — this is the "never a write loop" property.
    #[test]
    fn backoff_window_blocks_repeated_writes() {
        let mut gate = ReconcileGate::new();
        gate.note_attempt(0);
        gate.note_failure(); // cooldown 10 min
        assert!(matches!(
            gate.decide(5 * 60_000, 80, Some(100)),
            ReconcileAction::Deferred { .. }
        ));
        assert_eq!(
            gate.decide(10 * 60_000, 80, Some(100)),
            ReconcileAction::Apply
        );
    }

    // ── F5: the hold phase ──────────────────────────────────────────────────

    #[test]
    fn hold_phase_covers_every_real_state() {
        let (on, off) = (Some(true), Some(false));
        // care off
        assert_eq!(
            decide_hold(Some(100), 100, Some(100), off, on),
            HoldPhase::Off
        );
        // above the limit — no discharge on this firmware
        assert_eq!(
            decide_hold(Some(100), 80, Some(80), off, on),
            HoldPhase::AboveLimit
        );
        // at the limit
        assert_eq!(
            decide_hold(Some(80), 80, Some(80), off, on),
            HoldPhase::Holding
        );
        // inside the band (limit - HOLD_RESUME_MARGIN)
        assert_eq!(
            decide_hold(Some(78), 80, Some(80), on, on),
            HoldPhase::Holding
        );
        // just below the band
        assert_eq!(
            decide_hold(Some(77), 80, Some(80), on, on),
            HoldPhase::Charging
        );
        // plugged but not charging right now
        assert_eq!(
            decide_hold(Some(77), 80, Some(80), off, on),
            HoldPhase::Idle
        );
        // on battery below the limit
        assert_eq!(
            decide_hold(Some(60), 80, Some(80), off, off),
            HoldPhase::OnBattery
        );
        // firmware disagrees
        assert_eq!(
            decide_hold(Some(60), 80, Some(100), on, on),
            HoldPhase::Drifted
        );
        // firmware unreadable / stale
        assert_eq!(decide_hold(Some(60), 80, None, on, on), HoldPhase::Unknown);
        // level unreadable
        assert_eq!(decide_hold(None, 80, Some(80), on, on), HoldPhase::Unknown);
    }

    /// The owner's report, exactly: limit verified at 80 % and the pack at
    /// 100 %. The app must say "above the limit", never "holding at 80".
    #[test]
    fn pack_above_the_limit_is_not_reported_as_holding() {
        let state = compute_hold_state(Some(100), 80, Some(80), Some(false), Some(true), Some(1));
        assert_eq!(state.phase, HoldPhase::AboveLimit);
        assert!(state.applied, "the limit IS on the firmware");
        assert!(
            !state.can_discharge,
            "this firmware has no discharge channel"
        );
        assert_eq!((state.band_low, state.band_high), (78, 80));
        assert!(state.detail.unwrap().contains("no discharge"));
    }

    #[test]
    fn hold_state_exposes_the_requested_band() {
        let state = compute_hold_state(Some(79), 80, Some(80), Some(false), Some(true), None);
        assert_eq!(state.phase, HoldPhase::Holding);
        assert_eq!(state.band_low, 78);
        assert_eq!(state.band_high, 80);
        assert!(state.applied);
    }

    #[test]
    fn hold_state_never_claims_applied_without_a_firmware_value() {
        let state = compute_hold_state(Some(80), 80, None, Some(false), Some(true), None);
        assert!(!state.applied);
        assert_eq!(state.phase, HoldPhase::Unknown);
    }

    // ── F5: forced re-assert on power events ───────────────────────────────

    /// A resume/AC plug must not wait out the 5 min poll window.
    #[test]
    fn forced_reassert_shortens_the_window_to_the_floor() {
        let mut gate = ReconcileGate::new();
        gate.note_attempt(0);
        gate.note_success();

        // 10 s later the normal gate defers for the rest of the window…
        assert!(matches!(
            gate.decide(10_000, 80, Some(100)),
            ReconcileAction::Deferred { .. }
        ));
        // …and the forced path still refuses before the 30 s floor.
        assert!(matches!(
            gate.decide_forced(10_000, 80, Some(100)),
            ReconcileAction::Deferred { .. }
        ));
        // At the floor it applies.
        assert_eq!(
            gate.decide_forced(FORCE_MIN_INTERVAL_MS, 80, Some(100)),
            ReconcileAction::Apply
        );
    }

    /// Forcing must never defeat the failure backoff: a firmware that refuses
    /// the write cannot be retried on every AC/DC flicker.
    #[test]
    fn forced_reassert_respects_the_failure_backoff() {
        let mut gate = ReconcileGate::new();
        gate.note_attempt(0);
        gate.note_failure(); // cooldown 10 min
        assert!(matches!(
            gate.decide_forced(FORCE_MIN_INTERVAL_MS, 80, Some(100)),
            ReconcileAction::Deferred { .. }
        ));
        // In sync is in sync, forced or not.
        assert_eq!(gate.decide_forced(0, 80, Some(80)), ReconcileAction::InSync);
        // Care off is still a user decision.
        assert_eq!(
            gate.decide_forced(0, 100, Some(80)),
            ReconcileAction::CareOff
        );
    }
}
