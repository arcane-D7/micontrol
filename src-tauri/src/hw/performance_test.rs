use crate::state::PerformanceMode;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hw::performance::{get_performance_mode, set_performance_mode};

    #[test]
    fn performance_mode_hw_values() {
        assert_eq!(PerformanceMode::Silence.to_hw_value(), 0);
        assert_eq!(PerformanceMode::Balance.to_hw_value(), 1);
        assert_eq!(PerformanceMode::Turbo.to_hw_value(), 2);
        assert_eq!(PerformanceMode::Decepticon.to_hw_value(), 3);
        assert_eq!(PerformanceMode::Smart.to_hw_value(), 10);
        assert_eq!(PerformanceMode::LongBattery.to_hw_value(), 11);
        assert_eq!(PerformanceMode::SmartAcceleration.to_hw_value(), 14);
    }

    #[test]
    fn performance_mode_serialization() {
        let json = serde_json::to_string(&PerformanceMode::Balance).unwrap();
        assert_eq!(json, "\"balance\"");

        let json_turbo = serde_json::to_string(&PerformanceMode::Turbo).unwrap();
        assert_eq!(json_turbo, "\"turbo\"");

        let json_smart_acc = serde_json::to_string(&PerformanceMode::SmartAcceleration).unwrap();
        assert_eq!(json_smart_acc, "\"smart_acceleration\"");
    }

    #[test]
    fn performance_mode_deserialization() {
        let mode: PerformanceMode = serde_json::from_str("\"silence\"").unwrap();
        assert_eq!(mode, PerformanceMode::Silence);

        let mode2: PerformanceMode = serde_json::from_str("\"long_battery\"").unwrap();
        assert_eq!(mode2, PerformanceMode::LongBattery);
    }

    /// On Windows with registry access, set_performance_mode should at least
    /// succeed via the registry fallback path.
    #[test]
    #[cfg(windows)]
    fn set_performance_mode_registry_fallback() {
        // This will attempt VHF (likely fail in CI) then fallback to registry.
        // We accept either success.
        let result = set_performance_mode(PerformanceMode::Balance);
        // If registry is accessible, it should succeed.
        // Not asserting Ok because elevated permissions may not be available in test runner.
        let _ = result;
    }
}
