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

/// Bind address of the `tauri-plugin-mcp-bridge` WebSocket server.
///
/// The crate default is `0.0.0.0` ("all interfaces, for remote device
/// support"), which (a) exposes full app control — screenshots, DOM, input
/// simulation — to every host on the LAN and (b) is the socket that made
/// Windows raise the "Allow MiControl to communicate on these networks"
/// firewall dialog on every new build path (audit 2026-11-07 §3/§7.1).
///
/// Loopback is enough for every supported client (the MCP bridge is consumed
/// by local agents) and loopback binds never trigger the firewall prompt.
pub const BRIDGE_BIND_ADDRESS: &str = "127.0.0.1";

/// Base port of the bridge WebSocket server (the crate scans up to 100 ports
/// from here). Kept at the crate default so existing clients keep working.
pub const BRIDGE_BASE_PORT: u16 = 9223;

/// Config for `tauri_plugin_mcp_bridge::init_with_config`.
///
/// Single source of truth for the bind decision: `lib.rs` must never call the
/// bare `init()` (which silently binds `0.0.0.0`).
pub fn bridge_config() -> tauri_plugin_mcp_bridge::Config {
    tauri_plugin_mcp_bridge::Config::localhost_only()
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bridge_binds_loopback_only() {
        // Regression guard for the audit finding: the bare `init()` binds
        // 0.0.0.0, which exposes app control to the LAN and triggers the
        // Windows firewall dialog. The config must stay loopback.
        let cfg = bridge_config();
        assert_eq!(cfg.bind_address, BRIDGE_BIND_ADDRESS);
        assert_eq!(cfg.bind_address, "127.0.0.1");
        assert_ne!(cfg.bind_address, "0.0.0.0");
    }

    #[test]
    fn bridge_keeps_the_documented_port() {
        // Clients (and the audit's evidence) expect 9223; a silent port change
        // would break every local MCP client.
        assert_eq!(bridge_config().base_port, BRIDGE_BASE_PORT);
        assert_eq!(bridge_config().base_port, 9223);
    }
}
