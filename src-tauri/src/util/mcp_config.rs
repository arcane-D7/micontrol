//! MCP integration configuration.
//!
//! Persists the "MCP Integration" toggle in the Windows registry
//! (`HKCU\SOFTWARE\MiControl\MCPIntegrationEnabled`). When enabled, the
//! tauri-plugin-mcp socket server (TCP localhost:4000) is started — giving
//! MCP clients (AI agents, debug tools) full DOM control of the app
//! (query_page / click / read_text / type_text / navigate / execute_js).
//!
//! Off by default (secure: the socket grants any local process arbitrary
//! JS execution). Opt-in via Settings → "MCP Integration".

use crate::util::registry::RegKeyGuard;
use windows::Win32::System::Registry::HKEY_CURRENT_USER;

const REG_SUBKEY: &str = r"SOFTWARE\MiControl";
const REG_VALUE: &str = "MCPIntegrationEnabled";

/// Read the persisted MCP integration flag. Defaults to **disabled**.
pub fn is_enabled() -> bool {
    match RegKeyGuard::open_read(HKEY_CURRENT_USER, REG_SUBKEY) {
        Ok(Some(key)) => key
            .read_u32(REG_VALUE)
            .ok()
            .flatten()
            .map(|v| v != 0)
            .unwrap_or(false),
        _ => false,
    }
}

/// Persist the MCP integration flag to the registry.
pub fn set_enabled(enabled: bool) {
    let key = match RegKeyGuard::create_write(HKEY_CURRENT_USER, REG_SUBKEY) {
        Ok(k) => k,
        Err(e) => {
            log::warn!("[mcp_config] Cannot open registry key: {e}");
            return;
        }
    };
    if let Err(e) = key.write_u32(REG_VALUE, if enabled { 1 } else { 0 }) {
        log::warn!("[mcp_config] Cannot persist MCP integration flag: {e}");
    } else {
        log::info!(
            "[mcp_config] MCP integration {} (persisted)",
            if enabled { "ENABLED" } else { "disabled" }
        );
    }
}
