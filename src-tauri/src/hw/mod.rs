//! Hardware abstraction layer for MiControl.
//!
//! Each sub-module wraps a specific hardware domain (battery, display,
//! fan, audio, etc.) accessed via WMI, IoTService IPC, IOCTL, or
//! Windows API calls.

pub mod audio;
pub mod audio_effects;
pub mod battery;
pub mod charging;
pub mod cleanup;
pub mod color_calibration;
pub mod crash_recovery;
pub mod discovery;
pub mod display;
pub mod driver_update;
pub mod ecram;
pub mod ecram_service_mgmt;
pub mod errors;
pub mod eye_protection;
pub mod face;
pub mod fan;
pub mod fn_key;
pub mod hotkeys;
pub mod hq_wmi;
pub mod iotservice;
pub mod mic;
pub mod os_turbo;
#[cfg(windows)]
pub mod osd;
pub mod performance;
pub mod phone_link;
#[cfg(windows)]
pub mod power_listener;
pub mod processes;
pub mod screen_cast;
pub mod security_scan;
pub mod startup;
pub mod system_info;
pub mod thermal;
pub mod touchpad;
pub mod update;
pub mod wifi;
#[cfg(windows)]
pub mod wmi_cache;
pub mod wmi_ec;
