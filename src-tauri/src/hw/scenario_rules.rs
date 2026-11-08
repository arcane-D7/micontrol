#![cfg(windows)]
//! Scenario rules: automatic performance-mode switching based on power events.
//!
//! Consumes the string events emitted by `hw::iot_events` via its event hook
//! (`"ac_on"`, `"ac_off"`, `"battery_low"`) and applies user-configured rules:
//!
//! - AC plug-in  → optional `OnAcMode` performance mode applied automatically.
//! - On battery  → optional `OnBatteryMode` applied automatically.
//! - Lid closed  → optional `LidAction` (reserved for the laptop-status hook).
//!
//! ## Why the feature was dead (audit 2026-11-07, P2-6)
//!
//! Two independent faults, both fixed here:
//!
//! 1. **No configuration.** `HKCU\SOFTWARE\MiControl\IotEvents` was never
//!    created, so `OnAcMode`/`OnBatteryMode` were always `None` and there was
//!    no rule to apply. The key is now created with defaults on first use
//!    ([`ScenarioRulesConfig::load_or_create`]) and written by the UI through
//!    [`set_rules`].
//! 2. **No elevation.** The old code called `performance::set_performance_mode`
//!    **in the app process**. That function starts with `persist_to_registry`,
//!    which writes `HKLM\SOFTWARE\MI\PerformanceMode` — and the ACL on that key
//!    grants `BUILTIN\Users` only `ReadKey`. Every application therefore failed
//!    with access-denied *before* touching the hardware, and nothing logged it.
//!
//! ## Privilege partition
//!
//! | half | storage | elevation |
//! |---|---|---|
//! | config (`RulesEnabled`, `OnAcMode`, `OnBatteryMode`, `LidAction`) | `HKCU\SOFTWARE\MiControl\IotEvents` | none — the app reads and creates it |
//! | effect (the perf mode itself) | `HKLM\SOFTWARE\MI\PerformanceMode` + EC/WMI | required — dispatched through `elev_bridge` |
//!
//! `RulesEnabled` is intentionally distinct from the notification master switch
//! `Enabled`: the two features must be switchable independently (the Power
//! Profiles toggle cannot silence the AC/DC OSDs, and vice versa).
//!
//! The effect half rides the **existing** elevated path
//! (`elev_bridge::run_elevated_no_prompt("set_performance_mode", …)`), the same
//! whitelisted command the UI uses (`commands::hardware::set_performance_mode`).
//! No new elevated command was added: `elevated::dispatch` already handles
//! `set_performance_mode` and the bridge pipe accepts it, so the allowlist is
//! unchanged.
//!
//! Isolation: this module never touches OSD or other hardware modules except
//! through the elevated dispatcher. All perf-mode changes are dispatched from
//! the async runtime so the IoT event tick never blocks on WMI.

use crate::hw::iot_events::IotEventsConfig;
use crate::state::PerformanceMode;
use crate::util::rate_limit::{MessageRateLimiter, DEFAULT_WINDOW};
use serde::Serialize;
use std::sync::{OnceLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Registry value holding the lid-closed action (SZ).
const LID_ACTION_VALUE: &str = "LidAction";

/// Elevated command that applies a performance mode.
///
/// Must stay in sync with the `"set_performance_mode"` arm of
/// `elevated::dispatch` — that match is the allowlist for the bridge pipe and
/// the scheduled task. A test pins the name so a rename cannot silently
/// resurrect the dead in-process path.
pub const ELEVATED_SET_MODE_CMD: &str = "set_performance_mode";

/// What to do when the laptop lid closes. `"none"` is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LidAction {
    None,
    Sleep,
}

impl LidAction {
    pub fn parse(s: &str) -> LidAction {
        match s.to_ascii_lowercase().trim() {
            "sleep" | "suspend" | "standby" => LidAction::Sleep,
            _ => LidAction::None,
        }
    }
}

/// Power event that can trigger a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleEvent {
    AcOn,
    AcOff,
    BatteryLow,
}

impl RuleEvent {
    /// Parse the identifiers emitted by `hw::iot_events::fire_hook`.
    pub fn parse(ev: &str) -> Option<Self> {
        match ev {
            "ac_on" => Some(Self::AcOn),
            "ac_off" => Some(Self::AcOff),
            "battery_low" => Some(Self::BatteryLow),
            _ => None,
        }
    }

    /// Stable identifier used in logs and in the UI status payload.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AcOn => "ac_on",
            Self::AcOff => "ac_off",
            Self::BatteryLow => "battery_low",
        }
    }
}

/// Privilege boundary a piece of the feature lives on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleScope {
    /// `HKCU\SOFTWARE\MiControl\IotEvents` — readable/writable by the app.
    User,
    /// `HKLM\SOFTWARE\MI\PerformanceMode` + EC/WMI — elevated dispatcher only.
    System,
}

/// The configuration half of the feature is user-scope: no elevation, ever.
pub const CONFIG_SCOPE: RuleScope = RuleScope::User;

/// Why an event produced no application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// Master switch off (`Enabled=0`).
    Disabled,
    /// Event is not a rule trigger, or the matching mode is "manual" (empty).
    NoRule,
}

/// A rule application, split across the privilege boundaries.
///
/// The config half was already read (user scope); this struct is the **system
/// half** — the elevated command that must carry the HKLM/hardware write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationPlan {
    pub event: RuleEvent,
    pub mode: PerformanceMode,
    /// Elevated command name; whitelisted in `elevated::dispatch`.
    pub cmd: &'static str,
}

impl ApplicationPlan {
    /// The plan always lands on the system side of the partition.
    pub fn scope(&self) -> RuleScope {
        RuleScope::System
    }
}

/// Scenario rule set, derived from the shared IoT-events registry config.
#[derive(Debug, Clone)]
pub struct ScenarioRulesConfig {
    pub enabled: bool,
    pub on_ac_mode: Option<PerformanceMode>,
    pub on_battery_mode: Option<PerformanceMode>,
    pub lid_action: LidAction,
}

impl Default for ScenarioRulesConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            on_ac_mode: None,
            on_battery_mode: None,
            lid_action: LidAction::None,
        }
    }
}

impl ScenarioRulesConfig {
    /// Derive the rule set from the shared IoT config.
    ///
    /// Pure, so the partition is testable: the **notification** master switch
    /// (`IotEventsConfig::enabled`) must not gate the rules — only
    /// `rules_enabled` does. Reusing `enabled` (the old shape) meant silencing
    /// the AC/DC OSDs also disabled the auto-switch and vice versa.
    pub fn from_iot(base: &IotEventsConfig, lid_action: LidAction) -> Self {
        Self {
            enabled: base.rules_enabled,
            on_ac_mode: base.on_ac_mode,
            on_battery_mode: base.on_battery_mode,
            lid_action,
        }
    }

    /// Load rules. `rules_enabled`/`on_ac_mode`/`on_battery_mode` are inherited
    /// from [`IotEventsConfig`] (same registry key); `lid_action` is read from
    /// the sibling `LidAction` value. Any failure falls back to defaults.
    pub fn load() -> Self {
        let base = IotEventsConfig::load();
        Self::from_iot(&base, read_lid_action())
    }

    /// Load rules, creating the `IotEvents` key with defaults when it is
    /// missing. User scope only — this never needs elevation.
    pub fn load_or_create() -> Self {
        if let Err(e) = IotEventsConfig::ensure_key() {
            log::debug!("[scenario_rules] could not create IotEvents key: {e}");
        }
        Self::load()
    }
}

/// Read the `LidAction` registry value (SZ), defaulting to `None`.
fn read_lid_action() -> LidAction {
    use crate::util::registry::RegKeyGuard;
    use windows::Win32::System::Registry::HKEY_CURRENT_USER;

    let Ok(Some(key)) =
        RegKeyGuard::open_read(HKEY_CURRENT_USER, crate::hw::iot_events::CONFIG_KEY)
    else {
        return LidAction::None;
    };
    let Ok(Some(v)) = key.read_string(LID_ACTION_VALUE) else {
        return LidAction::None;
    };
    LidAction::parse(&v)
}

/// Pick the desired performance mode for a given event, if any.
fn desired_mode(ev: RuleEvent, cfg: &ScenarioRulesConfig) -> Option<PerformanceMode> {
    match ev {
        RuleEvent::AcOn => cfg.on_ac_mode,
        RuleEvent::AcOff | RuleEvent::BatteryLow => cfg.on_battery_mode,
    }
}

/// Decide what an event should do, without touching the registry or hardware.
///
/// Pure: the whole decision table is unit-testable, and the caller logs the skip
/// reason instead of failing silently (the old code returned `()` on every path,
/// which is why 72 h of logs showed nothing).
pub fn plan_application(
    ev: &str,
    cfg: &ScenarioRulesConfig,
) -> Result<ApplicationPlan, SkipReason> {
    if !cfg.enabled {
        return Err(SkipReason::Disabled);
    }
    let event = RuleEvent::parse(ev).ok_or(SkipReason::NoRule)?;
    let mode = desired_mode(event, cfg).ok_or(SkipReason::NoRule)?;
    Ok(ApplicationPlan {
        event,
        mode,
        cmd: ELEVATED_SET_MODE_CMD,
    })
}

/// True when the target mode differs from the mode already persisted.
///
/// `current == None` means the read failed — apply rather than assume, because a
/// wrong assumption here silently drops the user's rule.
pub fn needs_application(current: Option<PerformanceMode>, target: PerformanceMode) -> bool {
    current != Some(target)
}

// ── Last-application state (for the UI subscription) ─────────────────────────

/// Outcome of the most recent rule application.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LastApplication {
    pub event: String,
    pub mode: PerformanceMode,
    pub ok: bool,
    /// `PerformanceResult.method` when the elevated dispatch succeeded.
    pub method: Option<String>,
    /// Human-readable detail (method, or the error text).
    pub detail: String,
    pub at_unix_ms: u64,
}

/// Snapshot exposed to the frontend so the UI can *subscribe* to the rules
/// instead of owning the loop.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RulesStatus {
    pub last_application: Option<LastApplication>,
    pub applied_total: u64,
    pub failed_total: u64,
    /// Last skip reason, e.g. `"disabled"` / `"no_rule"`.
    pub last_skip: Option<String>,
}

struct StatusState {
    last: Option<LastApplication>,
    applied: u64,
    failed: u64,
    last_skip: Option<String>,
}

static STATUS: OnceLock<RwLock<StatusState>> = OnceLock::new();

fn status_state() -> &'static RwLock<StatusState> {
    STATUS.get_or_init(|| {
        RwLock::new(StatusState {
            last: None,
            applied: 0,
            failed: 0,
            last_skip: None,
        })
    })
}

/// Current rules status (cheap: one lock + clone).
pub fn status() -> RulesStatus {
    let guard = match status_state().read() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    RulesStatus {
        last_application: guard.last.clone(),
        applied_total: guard.applied,
        failed_total: guard.failed,
        last_skip: guard.last_skip.clone(),
    }
}

fn record_skip(reason: SkipReason) {
    let mut guard = match status_state().write() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    guard.last_skip = Some(
        match reason {
            SkipReason::Disabled => "disabled",
            SkipReason::NoRule => "no_rule",
        }
        .to_string(),
    );
}

fn record_application(entry: LastApplication) {
    let mut guard = match status_state().write() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    if entry.ok {
        guard.applied = guard.applied.saturating_add(1);
    } else {
        guard.failed = guard.failed.saturating_add(1);
    }
    guard.last = Some(entry);
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default()
}

// ── Rate-limited logging ─────────────────────────────────────────────────────

/// One line per key per 5 min. A flapping charger (or a bridge outage) must not
/// turn this module into the next 4.000-line log storm.
static LOG_LIMITER: MessageRateLimiter = MessageRateLimiter::new(DEFAULT_WINDOW);

fn log_rate_limited(level: log::Level, key: &'static str, render: impl FnOnce(u32) -> String) {
    if let Some(suppressed) = LOG_LIMITER.allow(key) {
        let suffix = if suppressed > 0 {
            format!(" (+{suppressed} suppressed)")
        } else {
            String::new()
        };
        log::log!(level, "[scenario_rules] {}{suffix}", render(suppressed));
    }
}

// ── Application ──────────────────────────────────────────────────────────────

/// Apply scenario rules for a power event emitted by the IoT event hook.
///
/// Never blocks the caller: the elevated round-trip runs on the async runtime.
/// Every path is logged (rate-limited) so a dead rule is visible in
/// `tauri-app.log` instead of being silent.
pub fn apply_event(ev: &str) {
    // ── User scope: config (HKCU). Creates the key when missing. ────────────
    let cfg = ScenarioRulesConfig::load_or_create();

    let plan = match plan_application(ev, &cfg) {
        Ok(plan) => plan,
        Err(reason) => {
            record_skip(reason);
            let key = match reason {
                SkipReason::Disabled => "skip_disabled",
                SkipReason::NoRule => "skip_no_rule",
            };
            log_rate_limited(log::Level::Debug, key, |_| {
                format!("event {ev:?} skipped ({reason:?})")
            });
            return;
        }
    };

    // Cheap no-op guard: the mode is already persisted, so an elevated
    // round-trip would only rewrite the same value.
    let current = crate::hw::performance::get_performance_mode().ok();
    if !needs_application(current, plan.mode) {
        log_rate_limited(log::Level::Debug, "skip_already_applied", |_| {
            format!(
                "event {:?} -> {:?} already active, nothing to do",
                plan.event.as_str(),
                plan.mode
            )
        });
        return;
    }

    log_rate_limited(log::Level::Info, plan.event.as_str(), |_| {
        format!(
            "event {:?} -> applying {:?} via elevated {}",
            plan.event.as_str(),
            plan.mode,
            plan.cmd
        )
    });

    let ApplicationPlan { event, mode, cmd } = plan;
    tauri::async_runtime::spawn(async move {
        // ── System scope: HKLM + EC/WMI ride the elevated dispatcher. ───────
        let outcome =
            crate::elev_bridge::run_elevated_no_prompt(cmd, serde_json::json!({ "mode": mode }))
                .await;

        let entry = match outcome {
            Ok(raw) => {
                let method = raw
                    .get("method")
                    .and_then(|m| m.as_str())
                    .map(|m| m.to_string());
                let detail = method.clone().unwrap_or_else(|| "applied".to_string());
                LastApplication {
                    event: event.as_str().to_string(),
                    mode,
                    ok: true,
                    method,
                    detail,
                    at_unix_ms: now_unix_ms(),
                }
            }
            Err(e) => LastApplication {
                event: event.as_str().to_string(),
                mode,
                ok: false,
                method: None,
                detail: e,
                at_unix_ms: now_unix_ms(),
            },
        };

        if entry.ok {
            log_rate_limited(log::Level::Info, "applied", |_| {
                format!(
                    "applied {:?} for {:?} via {}",
                    entry.mode, entry.event, entry.detail
                )
            });
        } else {
            log_rate_limited(log::Level::Warn, "apply_failed", |_| {
                format!(
                    "failed to apply {:?} for {:?}: {}",
                    entry.mode, entry.event, entry.detail
                )
            });
        }
        record_application(entry);
    });
}

/// Persist the rule set (user scope: `HKCU`, no elevation).
///
/// Called by the UI so the toggle and the AC/DC modes survive a restart and are
/// visible to the backend poller — the previous UI kept them in `localStorage`
/// only, which the Rust side could not read.
pub fn set_rules(
    enabled: bool,
    on_ac_mode: Option<PerformanceMode>,
    on_battery_mode: Option<PerformanceMode>,
) -> Result<(), String> {
    let mut cfg = IotEventsConfig::load();
    cfg.rules_enabled = enabled;
    cfg.on_ac_mode = on_ac_mode;
    cfg.on_battery_mode = on_battery_mode;
    cfg.save()?;
    log::info!(
        "[scenario_rules] config saved: enabled={enabled} ac={on_ac_mode:?} battery={on_battery_mode:?}"
    );
    Ok(())
}

/// Apply the configured lid action. Reserved for a future laptop-status hook
/// (e.g. `iotservice` lid events); exported now so rules stay configurable.
pub fn apply_lid_action() {
    use windows::Win32::System::Power::SetSuspendState;

    let cfg = ScenarioRulesConfig::load();
    match cfg.lid_action {
        LidAction::None => {}
        LidAction::Sleep => {
            log::info!("[scenario_rules] lid action: requesting system sleep");
            // S69: FIXED pool (no tokio blocking-pool churn → no handle leak).
            tauri::async_runtime::spawn(async {
                match crate::util::fixed_pool::run_blocking(|| {
                    // hibernate=false, force=false → a normal system sleep.
                    let ok = unsafe { SetSuspendState(false, false, false) };
                    ok.as_bool()
                })
                .await
                {
                    Ok(true) => {}
                    Ok(false) => {
                        let err = windows::core::Error::from_win32();
                        log::warn!("[scenario_rules] SetSuspendState failed: {err}");
                    }
                    Err(e) => log::warn!("[scenario_rules] suspend task failed: {e}"),
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // NOTE: no test calls `apply_event` — that would dispatch a real elevated
    // `set_performance_mode` and change this machine's power profile. Every test
    // below is pure (decision table, partition, no-op guard).

    fn cfg() -> ScenarioRulesConfig {
        ScenarioRulesConfig {
            enabled: true,
            on_ac_mode: Some(PerformanceMode::Turbo),
            on_battery_mode: Some(PerformanceMode::Balance),
            lid_action: LidAction::None,
        }
    }

    #[test]
    fn desired_mode_maps_ac_and_battery_events() {
        let c = cfg();
        assert_eq!(
            desired_mode(RuleEvent::AcOn, &c),
            Some(PerformanceMode::Turbo)
        );
        assert_eq!(
            desired_mode(RuleEvent::AcOff, &c),
            Some(PerformanceMode::Balance)
        );
        assert_eq!(
            desired_mode(RuleEvent::BatteryLow, &c),
            Some(PerformanceMode::Balance)
        );
    }

    #[test]
    fn plan_maps_events_to_the_elevated_command() {
        let c = cfg();
        let ac = plan_application("ac_on", &c).expect("ac_on has a rule");
        assert_eq!(ac.event, RuleEvent::AcOn);
        assert_eq!(ac.mode, PerformanceMode::Turbo);
        assert_eq!(ac.cmd, "set_performance_mode");
        assert_eq!(ac.cmd, ELEVATED_SET_MODE_CMD);

        let dc = plan_application("ac_off", &c).expect("ac_off has a rule");
        assert_eq!(dc.mode, PerformanceMode::Balance);

        let low = plan_application("battery_low", &c).expect("battery_low has a rule");
        assert_eq!(low.mode, PerformanceMode::Balance);
    }

    #[test]
    fn plan_rejects_unknown_events_and_manual_modes() {
        let c = cfg();
        assert_eq!(plan_application("resume", &c), Err(SkipReason::NoRule));
        assert_eq!(plan_application("", &c), Err(SkipReason::NoRule));
        assert_eq!(
            plan_application("slow_charger", &c),
            Err(SkipReason::NoRule)
        );

        let manual = ScenarioRulesConfig {
            on_ac_mode: None,
            ..cfg()
        };
        assert_eq!(
            plan_application("ac_on", &manual),
            Err(SkipReason::NoRule),
            "empty OnAcMode means 'manual only'"
        );
        // The battery rule is independent of the AC rule.
        assert!(plan_application("ac_off", &manual).is_ok());
    }

    #[test]
    fn disabled_rules_never_plan_an_application() {
        let c = ScenarioRulesConfig {
            enabled: false,
            ..cfg()
        };
        assert_eq!(plan_application("ac_on", &c), Err(SkipReason::Disabled));
        assert_eq!(plan_application("ac_off", &c), Err(SkipReason::Disabled));
        assert_eq!(
            plan_application("battery_low", &c),
            Err(SkipReason::Disabled)
        );
    }

    #[test]
    fn notification_switch_does_not_gate_the_rules() {
        // `Enabled=0` silences the OSDs; the auto-switch must keep working.
        let base = IotEventsConfig {
            enabled: false,
            rules_enabled: true,
            on_ac_mode: Some(PerformanceMode::Turbo),
            on_battery_mode: Some(PerformanceMode::Balance),
            ..IotEventsConfig::default()
        };
        let rules = ScenarioRulesConfig::from_iot(&base, LidAction::None);
        assert!(rules.enabled);
        assert_eq!(
            plan_application("ac_on", &rules).map(|p| p.mode),
            Ok(PerformanceMode::Turbo)
        );

        // And the reverse: rules off does not silence the OSDs.
        let base = IotEventsConfig {
            enabled: true,
            rules_enabled: false,
            ..base
        };
        let rules = ScenarioRulesConfig::from_iot(&base, LidAction::None);
        assert!(!rules.enabled);
        assert_eq!(plan_application("ac_on", &rules), Err(SkipReason::Disabled));
    }

    #[test]
    fn partition_is_user_config_system_effect() {
        // Config half: HKCU, no elevation.
        assert_eq!(CONFIG_SCOPE, RuleScope::User);
        // Effect half: every plan is a system-scope elevated command.
        let plan = plan_application("ac_on", &cfg()).expect("rule");
        assert_eq!(plan.scope(), RuleScope::System);
        assert_eq!(plan.cmd, ELEVATED_SET_MODE_CMD);
    }

    #[test]
    fn no_op_guard_only_skips_when_mode_already_active() {
        assert!(!needs_application(
            Some(PerformanceMode::Turbo),
            PerformanceMode::Turbo
        ));
        assert!(needs_application(
            Some(PerformanceMode::Balance),
            PerformanceMode::Turbo
        ));
        // Unknown current mode must apply, never assume.
        assert!(needs_application(None, PerformanceMode::Turbo));
    }

    #[test]
    fn rule_event_round_trips_through_its_identifier() {
        for ev in [RuleEvent::AcOn, RuleEvent::AcOff, RuleEvent::BatteryLow] {
            assert_eq!(RuleEvent::parse(ev.as_str()), Some(ev));
        }
        assert_eq!(RuleEvent::parse("AC_ON"), None, "identifiers are exact");
    }

    #[test]
    fn lid_action_parse_maps_known_values() {
        assert_eq!(LidAction::parse("sleep"), LidAction::Sleep);
        assert_eq!(LidAction::parse("Suspend"), LidAction::Sleep);
        assert_eq!(LidAction::parse(""), LidAction::None);
        assert_eq!(LidAction::parse("turbo"), LidAction::None);
    }

    #[test]
    fn default_config_is_safe() {
        let c = ScenarioRulesConfig::default();
        assert!(c.enabled);
        assert!(c.on_ac_mode.is_none());
        assert!(c.on_battery_mode.is_none());
        assert_eq!(c.lid_action, LidAction::None);
    }

    #[test]
    fn status_records_the_last_application() {
        record_application(LastApplication {
            event: "ac_on".to_string(),
            mode: PerformanceMode::Turbo,
            ok: true,
            method: Some("hq_wmi+ec_wmaa+registry+overlay".to_string()),
            detail: "hq_wmi+ec_wmaa+registry+overlay".to_string(),
            at_unix_ms: 1,
        });
        let after = status();
        assert_eq!(after.last_application.expect("recorded").event, "ac_on");
        assert!(after.applied_total >= 1);
    }
}
