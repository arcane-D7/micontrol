use serde::{Deserialize, Serialize};

/// Shared application state managed by Tauri.
#[derive(Default)]
pub struct AppState {
    pub performance_mode: std::sync::Mutex<PerformanceMode>,
    pub charging_threshold: std::sync::Mutex<u8>,
}

/// Performance modes supported by Xiaomi hardware.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PerformanceMode {
    Silence,
    #[default]
    Balance,
    Turbo,
    Smart,
    LongBattery,
    Decepticon,
    SmartAcceleration,
}

impl PerformanceMode {
    /// Returns the raw value sent via DeviceIoControl to VHF device.
    pub fn to_hw_value(self) -> u32 {
        match self {
            Self::Silence => 0,
            Self::Balance => 1,
            Self::Turbo => 2,
            Self::Decepticon => 3,
            Self::Smart => 10,
            Self::LongBattery => 11,
            Self::SmartAcceleration => 14,
        }
    }
}
