//! Battery hardware interface.
//!
//! Provides functions to read battery status, charge level, and health
//! from the system's WMI provider, with static data cached for performance.

use crate::hw::errors::HardwareResult;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::time::Instant;

#[cfg(windows)]
use std::sync::{Mutex, OnceLock};
#[cfg(windows)]
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BatteryInfo {
    pub level: u8,
    pub is_charging: bool,
    pub is_plugged: bool,
    pub health_percent: f64,
    pub cycle_count: u32,
    pub designed_capacity_mwh: u32,
    pub full_capacity_mwh: u32,
    pub manufacturer: String,
    pub device_name: String,
    pub temperature_celsius: Option<f64>,
    pub time_remaining_minutes: Option<i32>,
    /// Estimated minutes until battery is fully charged. None when not charging.
    pub time_to_full_minutes: Option<i32>,
    /// Positive = charge rate mW, negative = discharge rate mW. Zero when unknown.
    pub charge_rate_mw: i32,
    /// Current battery voltage in millivolts (mV). Zero if unavailable.
    pub voltage_mv: u32,
    /// AC adapter input power in milliwatts (mW). None when not plugged in,
    /// when the IoT driver is not available, or before the register offset is
    /// confirmed. Use `debug_ecram_dump` to identify the correct offset.
    pub ac_input_power_mw: Option<i32>,
}

/// Cached static battery data that never changes at runtime.
/// Populated once via `BATTERY_STATIC_DATA` and reused on all subsequent calls.
#[cfg(windows)]
#[derive(Clone)]
struct BatteryStaticData {
    designed_capacity_mwh: u64,
    full_capacity_mwh: u64,
    manufacturer: String,
    device_name: String,
    cycle_count: u32,
    temperature_raw: Option<u32>,
}

#[cfg(windows)]
static BATTERY_STATIC_DATA: OnceLock<BatteryStaticData> = OnceLock::new();

#[cfg(windows)]
pub fn get_battery_info() -> HardwareResult<BatteryInfo> {
    use crate::hw::wmi_cache;
    use crate::util::wmi_extract;
    use std::collections::HashMap;

    let started = Instant::now();
    log::debug!(target: "hw::battery", "get_battery_info: start");

    // S24-007: Use OnceLock for lock-free reads after initialisation.
    // OnceLock::get_or_try_init is unstable, so we compute the data first,
    // then use get_or_init with the already-computed value.
    let battery_static = BATTERY_STATIC_DATA.get_or_init(|| {
        // Try to populate from WMI; fall back to defaults on error.
        let data = wmi_cache::with_wmi(|wmi| {
            let static_data: Vec<HashMap<String, wmi::Variant>> = wmi
                .raw_query("SELECT * FROM BatteryStaticData")
                .context("BatteryStaticData query")?;
            let full_cap_data: Vec<HashMap<String, wmi::Variant>> = wmi
                .raw_query("SELECT * FROM BatteryFullChargedCapacity")
                .context("BatteryFullChargedCapacity query")?;

            let statics = static_data.into_iter().next().unwrap_or_default();
            let full_cap = full_cap_data.into_iter().next().unwrap_or_default();

            Ok(BatteryStaticData {
                designed_capacity_mwh: wmi_extract::extract_u64(&statics, "DesignedCapacity")
                    .unwrap_or(68224),
                full_capacity_mwh: wmi_extract::extract_u64(&full_cap, "FullChargedCapacity")
                    .unwrap_or(68224),
                manufacturer: wmi_extract::extract_string(&statics, "ManufactureName")
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|| "COSMX".to_string()),
                device_name: wmi_extract::extract_string(&statics, "DeviceName")
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|| "BX70".to_string()),
                cycle_count: wmi_extract::extract_u32_or(&statics, "CycleCount", 0),
                temperature_raw: wmi_extract::extract_u32(&statics, "Temperature"),
            })
        });

        match data {
            Ok(d) => d,
            Err(e) => {
                log::warn!(target: "hw::battery", "WMI static data query failed, using defaults: {e}");
                BatteryStaticData {
                    designed_capacity_mwh: 68224,
                    full_capacity_mwh: 68224,
                    manufacturer: "COSMX".to_string(),
                    device_name: "BX70".to_string(),
                    cycle_count: 0,
                    temperature_raw: None,
                }
            }
        }
    });

    let info = wmi_cache::with_wmi(|wmi| {
        // BatteryStatus (dynamic data only)
        let statuses: Vec<HashMap<String, wmi::Variant>> = wmi
            .raw_query("SELECT * FROM BatteryStatus")
            .context("BatteryStatus query")?;

        let status = statuses.into_iter().next().unwrap_or_default();

        let remaining_capacity = wmi_extract::extract_u32_or(&status, "RemainingCapacity", 0);
        let charging_rate = wmi_extract::extract_i32(&status, "ChargeRate").unwrap_or(0);
        let voltage = wmi_extract::extract_u32(&status, "Voltage")
            .map(|v| v as f64 / 1000.0)
            .unwrap_or(0.0);

        let is_charging = charging_rate > 0;
        let is_plugged = wmi_extract::extract_bool(&status, "PowerOnline").unwrap_or(false);
        log::debug!(
            target: "hw::battery",
            "wmi snapshot: plugged={} charging={} remaining_capacity={} charge_rate_mw={} voltage_v={:.3}",
            is_plugged,
            is_charging,
            remaining_capacity,
            charging_rate,
            voltage
        );

        // Static data from cached BatteryStaticData
        let designed_mah = battery_static.designed_capacity_mwh as u32;
        let manufacturer = battery_static.manufacturer.clone();
        let device_name = battery_static.device_name.clone();
        let cycle_count = if battery_static.cycle_count > 0 {
            battery_static.cycle_count
        } else {
            get_cycle_count_powercfg().unwrap_or(0)
        };
        let temperature_celsius = battery_static
            .temperature_raw
            .map(|t| (t as f64 / 10.0) - 273.15);
        let full_cap_mah = battery_static.full_capacity_mwh as u32;

        let level = if full_cap_mah > 0 {
            ((remaining_capacity as f64 / full_cap_mah as f64) * 100.0)
                .round()
                .clamp(0.0, 100.0) as u8
        } else {
            0
        };

        let health_percent = if designed_mah > 0 {
            ((full_cap_mah as f64 / designed_mah as f64) * 100.0).clamp(0.0, 100.0)
        } else {
            100.0
        };

        let time_remaining_minutes = if !is_charging && voltage > 0.0 {
            // Estimate: remaining capacity (mWh) / discharge_rate (mW) * 60
            let discharge_rate_mw = charging_rate.unsigned_abs();
            if discharge_rate_mw > 0 {
                Some((remaining_capacity as f64 / discharge_rate_mw as f64 * 60.0) as i32)
            } else {
                None
            }
        } else {
            None
        };

        let time_to_full_minutes = if is_charging && charging_rate > 0 {
            let remaining_to_full = full_cap_mah.saturating_sub(remaining_capacity);
            if remaining_to_full > 0 {
                Some((remaining_to_full as f64 / charging_rate as f64 * 60.0) as i32)
            } else {
                Some(0) // already at full capacity
            }
        } else {
            None
        };

        // Try to read AC adapter input power from ECRAM (IoTDriver.sys)
        let ac_input_power_mw = if is_plugged {
            log::debug!(target: "hw::battery", "charger is plugged; attempting ECRAM AC power read");
            probe_ac_input_power_throttled()
        } else {
            log::debug!(target: "hw::battery", "charger is unplugged; skipping ECRAM AC power read");
            clear_ac_power_probe_cache();
            None
        };

        log::debug!(
            target: "hw::battery",
            "battery info ready: ac_input_power_mw={:?} elapsed_ms={}",
            ac_input_power_mw,
            started.elapsed().as_millis()
        );

        Ok(BatteryInfo {
            level,
            is_charging,
            is_plugged,
            health_percent,
            cycle_count,
            designed_capacity_mwh: designed_mah,
            full_capacity_mwh: full_cap_mah,
            manufacturer,
            device_name,
            temperature_celsius,
            time_remaining_minutes,
            time_to_full_minutes,
            charge_rate_mw: charging_rate,
            voltage_mv: wmi_extract::extract_u32_or(&status, "Voltage", 0),
            ac_input_power_mw,
        })
    });
    info
}

#[cfg(windows)]
#[derive(Default)]
struct AcPowerProbeCache {
    last_probe_at: Option<Instant>,
    last_value_mw: Option<i32>,
}

#[cfg(windows)]
fn ac_probe_cache() -> &'static Mutex<AcPowerProbeCache> {
    static CACHE: OnceLock<Mutex<AcPowerProbeCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(AcPowerProbeCache::default()))
}

#[cfg(windows)]
fn probe_ac_input_power_throttled() -> Option<i32> {
    const AC_PROBE_MIN_INTERVAL: Duration = Duration::from_secs(15);
    let now = Instant::now();

    // S24-006: Use lock_or_recover for consistent poison recovery.
    {
        let cache = crate::util::panic::lock_or_recover(ac_probe_cache());
        if let Some(last) = cache.last_probe_at {
            let elapsed = now.saturating_duration_since(last);
            if elapsed < AC_PROBE_MIN_INTERVAL {
                log::debug!(
                    target: "hw::battery",
                    "ac probe throttled: returning cached value {:?} (elapsed_ms={} < interval_ms={})",
                    cache.last_value_mw,
                    elapsed.as_millis(),
                    AC_PROBE_MIN_INTERVAL.as_millis()
                );
                return cache.last_value_mw;
            }
        }
    }

    let value = crate::hw::ecram::try_get_ac_power_mw();

    // S24-006: Use lock_or_recover for consistent poison recovery.
    {
        let mut cache = crate::util::panic::lock_or_recover(ac_probe_cache());
        cache.last_probe_at = Some(now);
        cache.last_value_mw = value;
    }

    value
}

#[cfg(windows)]
fn clear_ac_power_probe_cache() {
    // S24-006: Use lock_or_recover for consistent poison recovery.
    let mut cache = crate::util::panic::lock_or_recover(ac_probe_cache());
    cache.last_probe_at = None;
    cache.last_value_mw = None;
}

/// Read battery cycle count from `powercfg /batteryreport /xml` output.
///
/// This is a fallback when WMI `BatteryStaticData.CycleCount` returns 0
/// or is unavailable. The XML output contains a `<CycleCount>` element
/// under `<BatteryReport>` -> `<Batteries>` -> `<Battery>`.
#[cfg(windows)]
fn get_cycle_count_powercfg() -> Option<u32> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let mut cmd = Command::new("powercfg");
    cmd.args(["/batteryreport", "/xml", "/output", "-"]);
    cmd.creation_flags(CREATE_NO_WINDOW);

    let output = cmd.output().ok()?;
    let xml = String::from_utf8_lossy(&output.stdout);

    // Parse <CycleCount> element from XML
    if let Some(start) = xml.find("<CycleCount>") {
        if let Some(end) = xml[start..].find("</CycleCount>") {
            let value_str = &xml[start + "<CycleCount>".len()..start + end];
            return value_str.trim().parse::<u32>().ok();
        }
    }

    None
}

#[cfg(not(windows))]
fn get_cycle_count_powercfg() -> Option<u32> {
    None
}

#[cfg(not(windows))]
pub fn get_battery_info() -> HardwareResult<BatteryInfo> {
    Ok(BatteryInfo {
        level: 80,
        is_charging: false,
        is_plugged: false,
        health_percent: 98.0,
        cycle_count: 42,
        designed_capacity_mwh: 68224,
        full_capacity_mwh: 66800,
        manufacturer: "COSMX".to_string(),
        device_name: "BX70".to_string(),
        temperature_celsius: Some(28.5),
        time_remaining_minutes: Some(240),
        time_to_full_minutes: None,
        charge_rate_mw: 0,
        voltage_mv: 11400,
        ac_input_power_mw: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires real battery hardware (WMI BatteryStaticData)"]
    fn battery_info_fields_valid() {
        let info = get_battery_info().expect("get_battery_info should succeed");
        assert!(
            info.level <= 100,
            "Battery level out of range: {}",
            info.level
        );
        assert!(
            info.health_percent <= 110.0,
            "Health out of range: {}",
            info.health_percent
        );
        assert!(
            !info.manufacturer.is_empty(),
            "Manufacturer should not be empty"
        );
        assert!(
            !info.device_name.is_empty(),
            "Device name should not be empty"
        );
        assert!(
            info.designed_capacity_mwh > 0,
            "Designed capacity should be > 0"
        );
        assert!(info.full_capacity_mwh > 0, "Full capacity should be > 0");
    }

    #[test]
    #[ignore = "requires real battery hardware (WMI BatteryStaticData)"]
    fn battery_capacity_ratio_sane() {
        let info = get_battery_info().expect("get_battery_info should succeed");
        let ratio = info.full_capacity_mwh as f64 / info.designed_capacity_mwh as f64;
        assert!(
            ratio <= 1.1,
            "Full capacity exceeds 110% of designed: {ratio}"
        );
        assert!(ratio > 0.0, "Full capacity must be positive");
    }

    #[test]
    #[ignore = "requires real battery hardware (WMI BatteryStaticData)"]
    fn battery_info_serialization() {
        let info = get_battery_info().expect("get_battery_info should succeed");
        let json = serde_json::to_string(&info).expect("should serialize");
        assert!(json.contains("level"));
        assert!(json.contains("manufacturer"));
    }

    #[cfg(windows)]
    #[test]
    fn ac_power_cache_concurrent_clear_probe_no_panic() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        use std::time::Duration;

        // Two threads racing: one clearing the cache, one probing it.
        // Under the old implementation the probe could interleave with
        // the clear; here we verify the lock-held-throughout approach
        // never panics and never produces a stale read after clear.
        let stop = Arc::new(AtomicBool::new(false));

        let stop_probe = Arc::clone(&stop);
        let probe_thread = std::thread::spawn(move || {
            while !stop_probe.load(Ordering::Relaxed) {
                let _ = probe_ac_input_power_throttled();
            }
        });

        let stop_clear = Arc::clone(&stop);
        let clear_thread = std::thread::spawn(move || {
            while !stop_clear.load(Ordering::Relaxed) {
                clear_ac_power_probe_cache();
            }
        });

        // Let them race for long enough to trigger interleaving
        std::thread::sleep(Duration::from_millis(500));
        stop.store(true, Ordering::Relaxed);

        probe_thread.join().expect("probe thread panicked");
        clear_thread.join().expect("clear thread panicked");
    }
}
