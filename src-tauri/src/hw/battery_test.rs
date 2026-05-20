#[cfg(test)]
mod tests {
    use crate::hw::battery::get_battery_info;

    #[test]
    fn battery_info_fields_non_empty() {
        let info = get_battery_info().expect("get_battery_info should succeed");
        assert!(info.level <= 100, "Battery level out of range: {}", info.level);
        assert!(info.health_percent <= 100.0, "Health out of range: {}", info.health_percent);
        assert!(!info.manufacturer.is_empty(), "Manufacturer should not be empty");
        assert!(!info.device_name.is_empty(), "Device name should not be empty");
        assert!(info.designed_capacity_mwh > 0, "Designed capacity should be > 0");
        assert!(info.full_capacity_mwh > 0, "Full capacity should be > 0");
        // Health should not exceed 110% (allow minor calibration variance)
        assert!(info.health_percent <= 110.0);
    }

    #[test]
    fn battery_level_consistent_with_capacities() {
        let info = get_battery_info().expect("get_battery_info should succeed");
        // Full capacity should be at most designed * 1.1
        let ratio = info.full_capacity_mwh as f64 / info.designed_capacity_mwh as f64;
        assert!(ratio <= 1.1, "Full capacity exceeds 110% of designed: {ratio}");
    }
}
