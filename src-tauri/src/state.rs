//! Shared application state managed by Tauri.
//!
//! Defines `AppState` holding performance mode, charging threshold,
//! and the hardware profile cache.

use crate::hw::discovery::HardwareProfile;
use serde::{Deserialize, Serialize};
use std::sync::RwLock;

/// Shared application state managed by Tauri.
pub struct AppState {
    pub performance_mode: std::sync::Mutex<PerformanceMode>,
    pub charging_threshold: std::sync::Mutex<u8>,
    pub hardware_profile: RwLock<Option<HardwareProfile>>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            performance_mode: std::sync::Mutex::new(PerformanceMode::default()),
            charging_threshold: std::sync::Mutex::new(0),
            hardware_profile: RwLock::new(None),
        }
    }
}

impl AppState {
    /// Set the hardware profile.
    pub fn set_profile(&self, profile: HardwareProfile) {
        if let Ok(mut guard) = self.hardware_profile.write() {
            *guard = Some(profile);
        }
    }
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
    /// Hidden DPTF profile ODV1=4 — unique firmware profile not used by any stock mode.
    Overdrive,
    /// Hidden DPTF profile ODV1=5 — unique firmware profile.
    OverdriveHigh,
    /// Hidden DPTF profile ODV1=6 — highest known DPTF profile in the firmware.
    OverdriveMax,
    /// EC SMMT-driven adaptive mode — the EC updates the sub-mode register dynamically
    /// based on workload and feeds it to NTDP, providing true auto-scaling.
    SmartAdaptive,
}

impl PerformanceMode {
    /// Returns the raw value sent via DeviceIoControl to VHF device.
    pub fn to_hw_value(self) -> u32 {
        match self {
            Self::Silence => 0,
            Self::Balance => 1,
            Self::Turbo => 2,
            Self::Decepticon => 3,
            Self::Overdrive => 4,
            Self::OverdriveHigh => 5,
            Self::OverdriveMax => 6,
            Self::SmartAdaptive => 9,
            Self::Smart => 10,
            Self::LongBattery => 11,
            Self::SmartAcceleration => 14,
        }
    }
}
