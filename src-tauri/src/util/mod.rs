//! Shared utility modules (auth, registry, retry, XML, etc.).
//!
//! Provides cross-cutting helpers used by both the hardware layer
//! and command handlers.

pub mod ai_usage;
pub mod auth;
pub mod blocking;
pub mod consent_audit;
pub mod data_deletion;
pub mod panic;
pub mod registry;
pub mod retry;
pub mod wmi_extract;
pub mod xml;
