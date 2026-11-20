//! Shared utility modules (auth, registry, retry, XML, etc.).
//!
//! Provides cross-cutting helpers used by both the hardware layer
//! and command handlers.

pub mod ai_cache;
pub mod ai_usage;
pub mod auth;
pub mod auto_update;
pub mod blocking;
pub mod consent_audit;
pub mod crash_report;
pub mod data_deletion;
pub mod error_log;
pub mod firewall;
pub mod fixed_pool;
pub mod leak_monitor;
pub mod log_fields;
pub mod log_ratelimit;
pub mod log_rotation;
pub mod mcp_config;
pub mod panic;
pub mod pdh;
pub mod process_spawn;
pub mod rate_limit;
pub mod registry;
pub mod retry;
pub mod schtasks;
pub mod session_marker;
pub mod state_store;
pub mod webview_retry;
pub mod win_handle;
pub mod win_service;
pub mod wmi_extract;
pub mod xml;
