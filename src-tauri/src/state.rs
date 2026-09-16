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
        // S24-006: Use lock_write_or_recover for consistent poison recovery.
        let mut guard = crate::util::panic::lock_write_or_recover(&self.hardware_profile);
        *guard = Some(profile);
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

    /// S61: returns the EC power-limit mode (MiInterface WMAA FUN3 code) that
    /// matches this UI mode. The HQ WMI channel used by `set_performance_mode`
    /// drives the fan curve, but the actual PL1/PL2 package power limit lives
    /// on this EC channel — writing only HQ WMI left users on the previous
    /// mode's power draw (measured: fans slowed to Balance while the package
    /// stayed at 41 W). Mapping follows the documented EC profiles:
    /// Performance=5, Balanced=6, Quiet=7, SuperQuiet=8, UltraPerformance=9.
    pub fn to_ec_mode(self) -> crate::hw::wmi_ec::EcPerformanceMode {
        use crate::hw::wmi_ec::EcPerformanceMode as Ec;
        match self {
            Self::Silence => Ec::Quiet,
            Self::LongBattery => Ec::SuperQuiet,
            Self::Balance | Self::Smart | Self::SmartAcceleration | Self::SmartAdaptive => {
                Ec::Balanced
            }
            Self::Turbo | Self::Decepticon => Ec::Performance,
            Self::Overdrive | Self::OverdriveHigh | Self::OverdriveMax => Ec::UltraPerformance,
        }
    }
}
