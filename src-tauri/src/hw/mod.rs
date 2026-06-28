//! Hardware abstraction layer for MiControl.
//!
//! Each sub-module wraps a specific hardware domain (battery, display,
//! fan, audio, etc.) accessed via WMI, IoTService IPC, IOCTL, or
//! Windows API calls.

pub mod audio;
pub mod battery;
pub mod charging;
pub mod discovery;
pub mod display;
pub mod ecram;
pub mod errors;
pub mod fan;
pub mod hotkeys;
pub mod iotservice;
pub mod mic;
#[cfg(windows)]
pub mod osd;
pub mod performance;
pub mod processes;
pub mod screen_cast;
pub mod startup;
pub mod system_info;
pub mod touchpad;
pub mod update;
pub mod wifi;
#[cfg(windows)]
pub mod wmi_cache;
pub mod wmi_ec;
