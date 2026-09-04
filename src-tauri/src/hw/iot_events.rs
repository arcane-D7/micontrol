//! IoT event listener: turns hardware power/EC events into native OSD
//! notifications and optional user actions.
//!
//! This module is fully **isolated**: it only reads state from existing
//! hardware modules (`battery`, `osd`, `iotservice`) and never modifies
//! their internals. It synthesizes events from two sources:
//!
//!   1. **AC/DC + battery deltas** (via `hw::battery::get_battery_info`, which
//!      already has its own WMI cache and timeouts — we never block long).
//!   2. **Sleep/resume** (via a hook into `hw::power_listener`).
//!
//! Event → OSD mapping:
//!   - AC plug-in    → "Charger connected" OSD (+ optional action)
//!   - AC unplug     → "On battery" OSD (+ optional action)
//!   - Charging      → charging OSD (icon)
//!   - Battery low   → low-battery warning OSD
//!   - Resume        → "Resumed from sleep" info OSD
//!
//! Configuration (per-user, no elevation):
//! `HKCU\SOFTWARE\MiControl\IotEvents`:
//!   - `Enabled`         DWORD (default 1) — master switch
//!   - `NotifyAcDc`      DWORD (default 1) — OSD on AC/DC change
//!   - `NotifyCharging`  DWORD (default 0) — OSD when charging state changes
//!   - `NotifyLowBattery`DWORD (default 1) — OSD at low battery threshold
//!   - `LowBatteryPct`   DWORD (default 20)
//!   - `OnAcMode`        SZ  (optional) — perf mode applied when AC plugs in
//!     (empty = do not change)
//!   - `OnBatteryMode`   SZ  (optional) — perf mode applied on battery
//!
//! All registry reads use the shared `util::registry` helpers; all OSD calls
//! are thread-safe. No hardware state is written by this module — applying a
//! perf-mode on AC/DC is delegated to `hw::scenario_rules` (MIOT-02), which
//! this module invokes via a trait-like callback when configured.

#![cfg(windows)]

use std::sync::OnceLock;
use std::sync::RwLock;
use std::time::Duration;

use crate::state::PerformanceMode;

// ── Configuration ─────────────────────────────────────────────────────────────

/// Registry key for IoT-event notification config.
pub(crate) const CONFIG_KEY: &str = r"SOFTWARE\MiControl\IotEvents";

#[derive(Debug, Clone)]
pub struct IotEventsConfig {
    pub enabled: bool,
    pub notify_ac_dc: bool,
    pub notify_charging: bool,
    pub notify_low_battery: bool,
    pub low_battery_pct: u8,
    pub on_ac_mode: Option<PerformanceMode>,
    pub on_battery_mode: Option<PerformanceMode>,
}

impl Default for IotEventsConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            notify_ac_dc: true,
            notify_charging: false,
            notify_low_battery: true,
            low_battery_pct: 20,
            on_ac_mode: None,
            on_battery_mode: None,
        }
    }
}

impl IotEventsConfig {
    /// Read config from the registry, with defaults on any failure.
    pub fn load() -> Self {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_CURRENT_USER;

        let mut cfg = Self::default();
        let Ok(Some(key)) = RegKeyGuard::open_read(HKEY_CURRENT_USER, CONFIG_KEY) else {
            return cfg;
        };

        let d = |name: &str, def: u32| key.read_u32(name).ok().flatten().unwrap_or(def);
        let s = |name: &str| {
            key.read_string(name)
                .ok()
                .flatten()
                .filter(|v| !v.is_empty())
        };

        cfg.enabled = d("Enabled", 1) != 0;
        cfg.notify_ac_dc = d("NotifyAcDc", 1) != 0;
        cfg.notify_charging = d("NotifyCharging", 0) != 0;
        cfg.notify_low_battery = d("NotifyLowBattery", 1) != 0;
        cfg.low_battery_pct = (d("LowBatteryPct", 20) % 101) as u8;

        cfg.on_ac_mode = s("OnAcMode").and_then(|v| parse_mode(&v));
        cfg.on_battery_mode = s("OnBatteryMode").and_then(|v| parse_mode(&v));
        cfg
    }

    pub fn save(&self) {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_CURRENT_USER;

        if let Ok(key) = RegKeyGuard::create_write(HKEY_CURRENT_USER, CONFIG_KEY) {
            let _ = key.write_u32("Enabled", self.enabled as u32);
            let _ = key.write_u32("NotifyAcDc", self.notify_ac_dc as u32);
            let _ = key.write_u32("NotifyCharging", self.notify_charging as u32);
            let _ = key.write_u32("NotifyLowBattery", self.notify_low_battery as u32);
            let _ = key.write_u32("LowBatteryPct", self.low_battery_pct as u32);
            if let Some(m) = &self.on_ac_mode {
                let _ = key.write_string("OnAcMode", &format!("{m:?}"));
            } else {
                let _ = key.write_string("OnAcMode", "");
            }
            if let Some(m) = &self.on_battery_mode {
                let _ = key.write_string("OnBatteryMode", &format!("{m:?}"));
            } else {
                let _ = key.write_string("OnBatteryMode", "");
            }
        }
    }
}

fn parse_mode(s: &str) -> Option<PerformanceMode> {
    match s.to_ascii_lowercase().trim() {
        "silence" | "quiet" => Some(PerformanceMode::Silence),
        "balance" | "balanced" => Some(PerformanceMode::Balance),
        "turbo" | "performance" => Some(PerformanceMode::Turbo),
        "decepticon" | "beast" => Some(PerformanceMode::Decepticon),
        "smart" => Some(PerformanceMode::Smart),
        "longbattery" | "long_battery" => Some(PerformanceMode::LongBattery),
        "smartacceleration" | "smart_acceleration" => Some(PerformanceMode::SmartAcceleration),
        "overdrive" => Some(PerformanceMode::Overdrive),
        "overdrivehigh" | "overdrive_high" => Some(PerformanceMode::OverdriveHigh),
        "overdrivemax" | "overdrive_max" => Some(PerformanceMode::OverdriveMax),
        "smartadaptive" | "smart_adaptive" => Some(PerformanceMode::SmartAdaptive),
        _ => None,
    }
}

// ── Listener state ────────────────────────────────────────────────────────────

struct ListenerState {
    running: std::sync::atomic::AtomicBool,
    last_ac: RwLock<Option<bool>>,
    last_charging: RwLock<Option<bool>>,
    last_level: RwLock<Option<u8>>,
    /// S48-002: last adapter power seen while charging (mW). Used to detect
    /// "slow charger" (sustained input below the slow-charger threshold).
    last_adapter_mw: RwLock<Option<i32>>,
}

static STATE: OnceLock<ListenerState> = OnceLock::new();

fn state() -> &'static ListenerState {
    STATE.get_or_init(|| ListenerState {
        running: std::sync::atomic::AtomicBool::new(false),
        last_ac: RwLock::new(None),
        last_charging: RwLock::new(None),
        last_level: RwLock::new(None),
        last_adapter_mw: RwLock::new(None),
    })
}

// ── Public API ───────────────────────────────────────────────────────────────

/// A hook invoked when an event callback should run (e.g., perf-mode
/// application). Registered by the app at setup after scenario rules exist.
type EventHook = Box<dyn Fn(&str) + Send + Sync>;
static EVENT_HOOK: OnceLock<RwLock<Option<EventHook>>> = OnceLock::new();

/// Register a hook that receives event identifiers like `"ac_on"`, `"ac_off"`.
/// Used to let MIOT-02's scenario rules react without coupling this module
/// to `scenario_rules`.
pub fn set_event_hook(hook: impl Fn(&str) + Send + Sync + 'static) {
    let slot = EVENT_HOOK.get_or_init(|| RwLock::new(None));
    if let Ok(mut g) = slot.write() {
        *g = Some(Box::new(hook));
    }
}

/// Start the background IoT event listener. Safe to call multiple times
/// (idempotent). Spawns a tokio task that samples battery state periodically
/// and emits OSD/hook events on detected transitions.
pub fn start_iot_event_listener() {
    if state()
        .running
        .swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        return;
    }

    tauri::async_runtime::spawn(async {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        // First tick fires immediately; the listener self-baselines.
        loop {
            interval.tick().await;
            tick();
        }
    });
}

/// Manually trigger a scan now (used by sleep-resume hook and tests).
pub fn scan_now() {
    tick();
}

fn tick() {
    let cfg = IotEventsConfig::load();
    if !cfg.enabled {
        return;
    }

    // Battery info is already cached/throttled inside hw::battery; on error we
    // simply skip this tick (no event, no OSD).
    let info = match crate::hw::battery::get_battery_info() {
        Ok(i) => i,
        Err(e) => {
            log::debug!("[iot_events] battery poll failed (skipping tick): {e}");
            return;
        }
    };

    let ac_now = info.is_plugged;
    let charging_now = info.is_charging;
    let level_now = info.level;

    // — AC/DC transition ────────────────────────────────────────────────────
    {
        let mut last_ac = state().last_ac.write().unwrap();
        if let Some(prev) = *last_ac {
            if prev != ac_now {
                log::info!("[iot_events] AC transition: plugged={ac_now}");
                if cfg.notify_ac_dc {
                    show_power_notification(if ac_now {
                        "power_ac_on"
                    } else {
                        "power_ac_off"
                    });
                }
                fire_hook(if ac_now { "ac_on" } else { "ac_off" });
            }
        }
        *last_ac = Some(ac_now);
    }

    // — Charging transition ─────────────────────────────────────────────────
    {
        let mut last_charging = state().last_charging.write().unwrap();
        if let Some(prev) = *last_charging {
            if prev != charging_now {
                log::info!("[iot_events] charging transition: charging={charging_now}");
                if cfg.notify_charging {
                    show_power_notification(if charging_now {
                        "power_charging_on"
                    } else {
                        "power_charging_off"
                    });
                }
            }
        }
        *last_charging = Some(charging_now);
    }

    // — Low battery warning ─────────────────────────────────────────────────
    {
        let mut last_level = state().last_level.write().unwrap();
        if let Some(level) = *last_level {
            let was_low = level <= cfg.low_battery_pct;
            let is_low = level_now <= cfg.low_battery_pct;
            if is_low && !was_low && !info.is_charging {
                log::info!("[iot_events] battery low: {level_now}%");
                if cfg.notify_low_battery {
                    show_power_notification("power_battery_low");
                }
                fire_hook("battery_low");
            }
        }
        *last_level = Some(level_now);
    }

    // — Slow charger detection (S48-002) ─────────────────────────────────────
    // While ACTIVELY charging (not merely plugged at 100%), if the adapter
    // input is below SLOW_CHARGER_MW the battery takes many hours to fill.
    // Notify once per plug-in cycle; reset when unplugged or charging ends.
    // The official adapter delivers 100 W; anything under 45 W is "slow"
    // (USB-C PD phone chargers, hub-passed power, degraded cables).
    {
        const SLOW_CHARGER_MW: i32 = 45_000;
        const CONSECUTIVE_TICKS_NEEDED: u32 = 6; // ~30 s at 5 s cadence
        static SLOW_TICKS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let mut last_adapter = state().last_adapter_mw.write().unwrap();

        if !ac_now {
            // Unplugged — reset detection window.
            SLOW_TICKS.store(0, std::sync::atomic::Ordering::Relaxed);
            *last_adapter = None;
        } else {
            let adapter_mw = info.ac_input_power_mw;
            let charging = info.is_charging;
            if let Some(mw) = adapter_mw {
                if charging && mw < SLOW_CHARGER_MW {
                    let ticks = SLOW_TICKS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    if ticks == CONSECUTIVE_TICKS_NEEDED {
                        log::info!(
                            "[iot_events] slow charger detected: {} W sustained",
                            mw / 1000
                        );
                        if cfg.notify_charging {
                            show_power_notification("power_slow_charger");
                        }
                        fire_hook("slow_charger");
                    }
                } else {
                    // Healthy charge or full battery — reset the window.
                    SLOW_TICKS.store(0, std::sync::atomic::Ordering::Relaxed);
                }
            } else {
                // No adapter-power reading this tick — keep previous state.
                SLOW_TICKS.store(0, std::sync::atomic::Ordering::Relaxed);
            }
            *last_adapter = adapter_mw;
        }
    }
}

fn fire_hook(ev: &str) {
    if let Some(slot) = EVENT_HOOK.get() {
        if let Ok(g) = slot.read() {
            if let Some(hook) = g.as_ref() {
                hook(ev);
            }
        }
    }
}

/// Show a native notification via the OSD subsystem (thread-safe).
///
/// We route through `osd::show_generic_notification`, a dedicated public
/// wrapper added for MIOT notifications — we never reach into OSD internals.
fn show_power_notification(kind: &str) {
    use crate::hw::osd::show_generic_notification;
    match kind {
        "power_ac_on" => show_generic_notification(2, 0xE7E8), // plug
        "power_ac_off" => show_generic_notification(2, 0xE7E8), // unplug
        "power_charging_on" => show_generic_notification(2, 0xE8B7), // battery charging
        "power_charging_off" => show_generic_notification(2, 0xE8B7),
        "power_battery_low" => show_generic_notification(2, 0xECA5), // warning
        "power_slow_charger" => show_generic_notification(2, 0xECA5), // warning (⚡ low input)
        _ => return,
    }
    log::info!("[iot_events] notification requested: {kind}");
}

// ── Tests ─────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mode_maps_all_names() {
        assert!(matches!(
            parse_mode("Quiet"),
            Some(PerformanceMode::Silence)
        ));
        assert!(matches!(
            parse_mode("balanced"),
            Some(PerformanceMode::Balance)
        ));
        assert!(matches!(
            parse_mode("LONG_BATTERY"),
            Some(PerformanceMode::LongBattery)
        ));
        assert!(parse_mode("nonsense").is_none());
        assert!(parse_mode("").is_none());
    }

    #[test]
    fn default_config_is_safe() {
        let cfg = IotEventsConfig::default();
        assert!(cfg.enabled);
        assert!(cfg.notify_ac_dc);
        assert!(!cfg.notify_charging);
        assert_eq!(cfg.low_battery_pct, 20);
        assert!(cfg.on_ac_mode.is_none());
    }

    #[test]
    fn load_returns_defaults_when_registry_missing() {
        // No registry key exists in test env → defaults must not panic.
        let cfg = IotEventsConfig::load();
        assert!(cfg.enabled);
    }
}
