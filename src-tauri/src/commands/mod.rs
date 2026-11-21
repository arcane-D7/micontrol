//! Tauri command handlers exposing hardware and system functionality to the frontend.
//!
//! Each sub-module maps to a family of `#[tauri::command]` functions
//! that delegate to the corresponding `hw` or service layer.

pub mod ai;
pub mod ai_logs;
#[cfg(test)]
pub mod cast_command_tests;
#[cfg(test)]
pub mod color_command_tests;
pub mod credentials;
pub mod crossdevice;
#[cfg(windows)]
pub mod face;
pub mod hardware;
pub mod hotkeys;
pub mod privacy;
pub mod system;
