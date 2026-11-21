//! Thermal zone monitoring via MSAcpi_ThermalZoneTemperature.
//!
//! Reads temperature from ACPI thermal zones (TZ00, TZ01, etc.) via the
//! `MSAcpi_ThermalZoneTemperature` WMI class in the ROOT\WMI namespace.
//!
//! Temperature values are in tenths of Kelvin — converted to Celsius.
//! Critical trip point temperatures are also available.

use crate::hw::errors::{HardwareError, HardwareResult};
use serde::{Deserialize, Serialize};

/// Thermal zone information from ACPI.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThermalZoneInfo {
    /// ACPI instance name (e.g., "ACPI\ThermalZone\TZ00_0").
    pub instance_name: String,
    /// Current temperature in Celsius.
    pub current_temp_celsius: f64,
    /// Critical trip point in Celsius (system shutdown threshold).
    pub critical_trip_celsius: Option<f64>,
    /// Whether this thermal zone is active.
    pub active: bool,
    /// Ambient vs unknown — derived from the instance name at read time so
    /// callers never have to re-parse it.
    #[serde(default)]
    pub role: ThermalZoneRole,
}

/// What an ACPI thermal zone measures.
///
/// This platform exposes exactly one ACPI zone, `TZ00`, and it tracks the
/// chassis/ambient temperature: one sample read **27.85 °C** on `TZ00` while
/// the ESIF CPU hotspot read **57 °C**. Reporting that zone as the CPU
/// temperature would show the room as the processor, so the role is carried
/// with the reading and the CPU fallback skips ambient zones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThermalZoneRole {
    /// Chassis/ambient zone — `TZ00` on this platform. Never a CPU sensor.
    Ambient,
    /// Not a known ambient zone. It *may* track the CPU on some firmware, but
    /// nothing proves it, so it is only ever used as a labelled "ACPI zone".
    #[default]
    Unknown,
}

/// True when the ACPI instance name is the chassis/ambient zone.
///
/// `TZ00` is the ambient zone on this platform (proven: 27.85 °C while the
/// ESIF CPU hotspot was 57 °C in the same sample). It must never be reported
/// as a CPU or GPU temperature.
pub fn is_ambient_zone(instance_name: &str) -> bool {
    instance_name.to_ascii_uppercase().contains("TZ00")
}

/// Classify a zone from its ACPI instance name.
pub fn zone_role(instance_name: &str) -> ThermalZoneRole {
    if is_ambient_zone(instance_name) {
        ThermalZoneRole::Ambient
    } else {
        ThermalZoneRole::Unknown
    }
}

/// CPU temperature from ACPI zones, **skipping the ambient zone**.
///
/// Returns `None` when the only zones available are ambient ones — which is
/// the case on this platform, where `TZ00` is the sole zone. `None` is the
/// honest answer: there is no ACPI CPU sensor here, and a ~28 °C "CPU" reading
/// would be worse than no reading at all.
pub fn cpu_temp_from_zones(zones: &[ThermalZoneInfo]) -> Option<f32> {
    zones
        .iter()
        .filter(|z| z.role != ThermalZoneRole::Ambient)
        .map(|z| z.current_temp_celsius)
        .find(|t| t.is_finite() && *t > 0.0)
        .map(|t| (t as f32).clamp(0.0, 120.0))
}

/// Ambient (chassis) temperature from ACPI zones.
pub fn ambient_temp_from_zones(zones: &[ThermalZoneInfo]) -> Option<f32> {
    zones
        .iter()
        .filter(|z| z.role == ThermalZoneRole::Ambient)
        .map(|z| z.current_temp_celsius)
        .find(|t| t.is_finite() && *t > 0.0)
        .map(|t| (t as f32).clamp(0.0, 120.0))
}

/// CPU temperature from the ACPI thermal zones, ambient zone excluded.
///
/// `Ok(None)` means "ACPI has no CPU-side zone" — not an error. `Err` means the
/// WMI class itself was unreadable (denied / absent).
pub fn cpu_temp_from_acpi() -> HardwareResult<Option<f32>> {
    Ok(cpu_temp_from_zones(&get_thermal_zones()?))
}

/// Ambient (chassis) temperature from the ACPI thermal zones.
pub fn ambient_temp_from_acpi() -> HardwareResult<Option<f32>> {
    Ok(ambient_temp_from_zones(&get_thermal_zones()?))
}

/// Read all thermal zone temperatures from ACPI.
pub fn get_thermal_zones() -> HardwareResult<Vec<ThermalZoneInfo>> {
    #[cfg(windows)]
    {
        use crate::hw::wmi_cache;
        use crate::util::wmi_extract;
        use std::collections::HashMap;
        use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
        use std::sync::OnceLock;
        use std::time::{SystemTime, UNIX_EPOCH};

        // S37-005: Mirror of fan.rs — the MSAcpi_ThermalZoneTemperature WMI
        // class is also denied to unprivileged callers (0x80041003). Cache the
        // failure so the 2 s fan poll doesn't spam WMI + the log.
        //
        // P11: ACCESS_DENIED is expected before the elevated path is usable —
        // log it ONCE at INFO and grow the retry window 60 → 120 → … → 900 s
        // instead of retrying every minute forever.
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
        let window = backoff.load(AtomicOrdering::Relaxed);
        let failed_at = last_fail.load(AtomicOrdering::Relaxed);
        if failed_at.saturating_add(window) > now_epoch {
            log::debug!(
                target: "hw::thermal",
                "ACPI thermal WMI query in failure window — skipping (retry in {}s)",
                failed_at.saturating_add(window).saturating_sub(now_epoch)
            );
            return Err(HardwareError::Wmi(
                "MSAcpi_ThermalZoneTemperature denied (cached)".into(),
            ));
        }

        let access_denied = std::cell::Cell::new(false);
        let query_result = wmi_cache::with_wmi(|wmi| {
            match wmi.raw_query(
                "SELECT InstanceName, Active, CurrentTemperature, CriticalTripPoint \
                 FROM MSAcpi_ThermalZoneTemperature",
            ) {
                Ok(r) => Ok(r),
                Err(e) => {
                    // P11: classify the raw WMI error before it is erased.
                    if wmi_cache::is_access_denied(&e) {
                        access_denied.set(true);
                    }
                    // No log here — a single line is emitted below.
                    last_fail.store(now_epoch, AtomicOrdering::Relaxed);
                    Err(anyhow::Error::from(e))
                }
            }
        });
        let results: Vec<HashMap<String, wmi::Variant>> = match query_result {
            Ok(r) => {
                backoff.store(BACKOFF_START_SECS, AtomicOrdering::Relaxed);
                r
            }
            Err(e) => {
                last_fail.store(now_epoch, AtomicOrdering::Relaxed);
                let next_window =
                    wmi_cache::grow_failure_backoff(window, BACKOFF_START_SECS, BACKOFF_MAX_SECS);
                backoff.store(next_window, AtomicOrdering::Relaxed);
                if access_denied.get() {
                    // Expected while unprivileged: INFO once per process.
                    let first = DENIED_LOGGED
                        .get_or_init(|| std::sync::atomic::AtomicBool::new(false))
                        .swap(true, AtomicOrdering::Relaxed);
                    if !first {
                        log::info!(
                            target: "hw::thermal",
                            "MSAcpi_ThermalZoneTemperature access denied (0x80041003) — expected \
                             while unprivileged (thermal data comes from the elevated bridge); \
                             retry window grows to {next_window}s"
                        );
                    } else {
                        log::debug!(target: "hw::thermal", "MSAcpi_ThermalZoneTemperature denied (retry in {next_window}s): {e}");
                    }
                } else {
                    let first = OTHER_FAILURE_LOGGED
                        .get_or_init(|| std::sync::atomic::AtomicBool::new(false))
                        .swap(true, AtomicOrdering::Relaxed);
                    if !first {
                        log::warn!(target: "hw::thermal", "MSAcpi_ThermalZoneTemperature query failed (next retry in {next_window}s): {e}");
                    } else {
                        log::debug!(target: "hw::thermal", "MSAcpi_ThermalZoneTemperature query failed (retry in {next_window}s): {e}");
                    }
                }
                return Err(e);
            }
        };

        let zones: Vec<ThermalZoneInfo> = results
            .iter()
            .filter_map(|row| {
                let instance_name =
                    wmi_extract::extract_string(row, "InstanceName").unwrap_or_default();
                let active = wmi_extract::extract_bool(row, "Active").unwrap_or(false);

                // CurrentTemperature is in tenths of Kelvin
                let current_temp = wmi_extract::extract_i32(row, "CurrentTemperature")?;
                let current_temp_celsius = (current_temp as f64 / 10.0) - 273.15;

                // CriticalTripPoint is also in tenths of Kelvin
                let critical_trip_celsius = wmi_extract::extract_i32(row, "CriticalTripPoint")
                    .map(|t| (t as f64 / 10.0) - 273.15);

                Some(ThermalZoneInfo {
                    role: zone_role(&instance_name),
                    instance_name,
                    current_temp_celsius,
                    critical_trip_celsius,
                    active,
                })
            })
            .collect();

        if zones.is_empty() {
            return Err(HardwareError::Wmi(
                "No MSAcpi_ThermalZoneTemperature instances found".into(),
            ));
        }

        Ok(zones)
    }
    #[cfg(not(windows))]
    {
        Err(HardwareError::NotSupported(
            "WMI only available on Windows".into(),
        ))
    }
}

/// Get the primary ACPI thermal zone (prefers `TZ00`).
///
/// **This is not a CPU temperature.** On this platform `TZ00` is the
/// chassis/ambient zone (`role == Ambient`), so the returned reading must not
/// be substituted for the processor temperature — use [`cpu_temp_from_acpi`]
/// for that, which returns `None` rather than an ambient value.
pub fn get_primary_thermal_zone() -> HardwareResult<ThermalZoneInfo> {
    let zones = get_thermal_zones()?;
    zones
        .iter()
        .find(|z| is_ambient_zone(&z.instance_name))
        .cloned()
        .or_else(|| zones.into_iter().next())
        .ok_or_else(|| HardwareError::Wmi("No thermal zone instances available".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn zone(name: &str, temp_c: f64) -> ThermalZoneInfo {
        ThermalZoneInfo {
            instance_name: name.to_string(),
            current_temp_celsius: temp_c,
            critical_trip_celsius: None,
            active: true,
            role: zone_role(name),
        }
    }

    #[test]
    fn tz00_is_classified_as_ambient() {
        assert!(is_ambient_zone("ACPI\\ThermalZone\\TZ00_0"));
        assert_eq!(
            zone_role("ACPI\\ThermalZone\\TZ00_0"),
            ThermalZoneRole::Ambient
        );
        assert!(!is_ambient_zone("ACPI\\ThermalZone\\TZ01_0"));
        assert_eq!(
            zone_role("ACPI\\ThermalZone\\TZ01_0"),
            ThermalZoneRole::Unknown
        );
    }

    /// P2-8: the production sample was TZ00 = 27.85 °C while ESIF reported the
    /// CPU hotspot at 57 °C. A TZ00-only zone list must yield *no* CPU
    /// temperature — not 28 °C.
    #[test]
    fn ambient_only_zones_yield_no_cpu_temperature() {
        let zones = vec![zone("ACPI\\ThermalZone\\TZ00_0", 27.85)];
        assert_eq!(cpu_temp_from_zones(&zones), None);
        assert_eq!(ambient_temp_from_zones(&zones), Some(27.85));
    }

    #[test]
    fn a_non_ambient_zone_is_usable_as_cpu_but_ambient_is_preferred_last() {
        let zones = vec![
            zone("ACPI\\ThermalZone\\TZ00_0", 27.85),
            zone("ACPI\\ThermalZone\\TZ01_0", 61.5),
        ];
        assert_eq!(cpu_temp_from_zones(&zones), Some(61.5));
        assert_eq!(ambient_temp_from_zones(&zones), Some(27.85));
    }

    /// A zeroed zone (sensor not populated) is not a reading either.
    #[test]
    fn zeroed_zone_is_not_reported() {
        let zones = vec![zone("ACPI\\ThermalZone\\TZ01_0", 0.0)];
        assert_eq!(cpu_temp_from_zones(&zones), None);
        assert_eq!(ambient_temp_from_zones(&zones), None);
    }
}
