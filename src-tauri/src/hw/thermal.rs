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
        // failure for 60 s so the 2 s fan poll doesn't spam WMI + the log.
        static LAST_FAILURE: OnceLock<AtomicU64> = OnceLock::new();
        const FAILURE_CACHE_SECS: u64 = 60;
        let last_fail = LAST_FAILURE.get_or_init(|| AtomicU64::new(0));
        let now_epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        if last_fail
            .load(AtomicOrdering::Relaxed)
            .saturating_add(FAILURE_CACHE_SECS)
            > now_epoch
        {
            log::debug!(target: "hw::thermal", "ACPI thermal WMI query in failure cache — skipping");
            return Err(HardwareError::Wmi(
                "MSAcpi_ThermalZoneTemperature denied (cached)".into(),
            ));
        }

        let query_result = wmi_cache::with_wmi(|wmi| {
            match wmi.raw_query(
                "SELECT InstanceName, Active, CurrentTemperature, CriticalTripPoint \
                 FROM MSAcpi_ThermalZoneTemperature",
            ) {
                Ok(r) => Ok(r),
                Err(e) => {
                    // No WARN here — single log on the outer error path below.
                    last_fail.store(now_epoch, AtomicOrdering::Relaxed);
                    Err(anyhow::Error::from(e))
                }
            }
        });
        let results: Vec<HashMap<String, wmi::Variant>> = match query_result {
            Ok(r) => r,
            Err(e) => {
                last_fail.store(now_epoch, AtomicOrdering::Relaxed);
                log::warn!(target: "hw::thermal", "MSAcpi_ThermalZoneTemperature query failed (retrying in {FAILURE_CACHE_SECS}s): {e}");
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

/// Get the primary thermal zone temperature (TZ00).
pub fn get_primary_thermal_zone() -> HardwareResult<ThermalZoneInfo> {
    let zones = get_thermal_zones()?;
    zones
        .iter()
        .find(|z| z.instance_name.contains("TZ00"))
        .cloned()
        .or_else(|| zones.into_iter().next())
        .ok_or_else(|| HardwareError::Wmi("No thermal zone instances available".into()))
}
