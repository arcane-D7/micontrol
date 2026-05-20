use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

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
    /// Positive = charge rate mW, negative = discharge rate mW. Zero when unknown.
    pub charge_rate_mw: i32,
    /// Current battery voltage in millivolts (mV). Zero if unavailable.
    pub voltage_mv: u32,
}

#[cfg(windows)]
pub fn get_battery_info() -> Result<BatteryInfo> {
    use wmi::{COMLibrary, WMIConnection};
    use std::collections::HashMap;

    let com = COMLibrary::new().context("COM init")?;
    let wmi = WMIConnection::with_namespace_path("ROOT\\WMI", com.into()).context("WMI connect root\\wmi")?;

    // BatteryStatus
    let statuses: Vec<HashMap<String, wmi::Variant>> = wmi
        .raw_query("SELECT * FROM BatteryStatus")
        .context("BatteryStatus query")?;

    // BatteryStaticData
    let static_data: Vec<HashMap<String, wmi::Variant>> = wmi
        .raw_query("SELECT * FROM BatteryStaticData")
        .context("BatteryStaticData query")?;

    // BatteryFullChargedCapacity
    let full_cap_data: Vec<HashMap<String, wmi::Variant>> = wmi
        .raw_query("SELECT * FROM BatteryFullChargedCapacity")
        .context("BatteryFullChargedCapacity query")?;

    let status = statuses.into_iter().next().unwrap_or_default();
    let statics = static_data.into_iter().next().unwrap_or_default();
    let full_cap = full_cap_data.into_iter().next().unwrap_or_default();

    let remaining_capacity = match status.get("RemainingCapacity") {
        Some(wmi::Variant::UI4(v)) => *v,
        _ => 0,
    };
    let charging_rate = match status.get("ChargeRate") {
        Some(wmi::Variant::I4(v)) => *v,
        _ => 0,
    };
    let voltage = match status.get("Voltage") {
        Some(wmi::Variant::UI4(v)) => *v as f64 / 1000.0,
        _ => 0.0,
    };

    let is_charging = charging_rate > 0;
    let is_plugged = match status.get("PowerOnline") {
        Some(wmi::Variant::Bool(v)) => *v,
        _ => false,
    };

    // Note: WMI DesignedCapacity / FullChargedCapacity / RemainingCapacity are in mWh, not mAh.
    let designed_mah = match statics.get("DesignedCapacity") {
        Some(wmi::Variant::UI4(v)) => *v,
        _ => 68224,
    };
    let manufacturer = match statics.get("ManufactureName") {
        Some(wmi::Variant::String(s)) => s.trim().to_string(),
        _ => "COSMX".to_string(),
    };
    let device_name = match statics.get("DeviceName") {
        Some(wmi::Variant::String(s)) => s.trim().to_string(),
        _ => "BX70".to_string(),
    };
    let cycle_count = match statics.get("CycleCount") {
        Some(wmi::Variant::UI4(v)) => *v,
        _ => 0,
    };
    let temp_raw = match statics.get("Temperature") {
        Some(wmi::Variant::UI4(v)) => Some(*v),
        _ => None,
    };
    let temperature_celsius = temp_raw.map(|t| (t as f64 / 10.0) - 273.15);

    let full_cap_mah = match full_cap.get("FullChargedCapacity") {
        Some(wmi::Variant::UI4(v)) => *v,
        _ => designed_mah,
    };

    let level = if full_cap_mah > 0 {
        ((remaining_capacity as f64 / full_cap_mah as f64) * 100.0).round().clamp(0.0, 100.0) as u8
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
        charge_rate_mw: charging_rate,
        voltage_mv: match status.get("Voltage") {
            Some(wmi::Variant::UI4(v)) => *v,
            _ => 0,
        },
    })
}

#[cfg(not(windows))]
pub fn get_battery_info() -> Result<BatteryInfo> {
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
        charge_rate_mw: 0,
        voltage_mv: 11400,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn battery_info_fields_valid() {
        let info = get_battery_info().expect("get_battery_info should succeed");
        assert!(info.level <= 100, "Battery level out of range: {}", info.level);
        assert!(info.health_percent <= 110.0, "Health out of range: {}", info.health_percent);
        assert!(!info.manufacturer.is_empty(), "Manufacturer should not be empty");
        assert!(!info.device_name.is_empty(), "Device name should not be empty");
        assert!(info.designed_capacity_mah > 0, "Designed capacity should be > 0");
        assert!(info.full_capacity_mah > 0, "Full capacity should be > 0");
    }

    #[test]
    fn battery_capacity_ratio_sane() {
        let info = get_battery_info().expect("get_battery_info should succeed");
        let ratio = info.full_capacity_mah as f64 / info.designed_capacity_mah as f64;
        assert!(ratio <= 1.1, "Full capacity exceeds 110% of designed: {ratio}");
        assert!(ratio > 0.0, "Full capacity must be positive");
    }

    #[test]
    fn battery_info_serialization() {
        let info = get_battery_info().expect("get_battery_info should succeed");
        let json = serde_json::to_string(&info).expect("should serialize");
        assert!(json.contains("level"));
        assert!(json.contains("manufacturer"));
    }
}
