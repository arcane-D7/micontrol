//! Automatic update configuration (S45-002).
//!
//! Persists two independent flags in the Windows registry under
//! `HKCU\SOFTWARE\MiControl`:
//!
//! * `AutoUpdateEnabled`  — master switch for the silent auto-update loop.
//!   **Off by default.** When ON, MiControl periodically asks the bridge to
//!   fetch and install a newer build.
//!
//! * `AutoUpdateBetaUrl`  — **hidden dev-only feed** (empty by default).
//!   Injected via CLI/MCP during development to point the updater at a
//!   specific beta installer URL ("passar via código a versão beta que
//!   queremos e o instalador faz isso sozinho"). An empty value means the
//!   regular (stable/updater) mechanism applies; a non-empty value makes the
//!   bridge download THAT installer and run it silently.
//!
//! Both live in `HKCU` so no elevation is needed to read/write them, and the
//! bridge service (SYSTEM) reads them from the *interactive user's* hive when
//! it picks the feed.

use crate::util::registry::RegKeyGuard;
use windows::Win32::System::Registry::HKEY_CURRENT_USER;

const REG_SUBKEY: &str = r"SOFTWARE\MiControl";
const REG_ENABLED: &str = "AutoUpdateEnabled";
const REG_BETA_URL: &str = "AutoUpdateBetaUrl";

/// Read the persisted auto-update master switch. Defaults to **disabled**.
pub fn is_enabled() -> bool {
    match RegKeyGuard::open_read(HKEY_CURRENT_USER, REG_SUBKEY) {
        Ok(Some(key)) => key
            .read_u32(REG_ENABLED)
            .ok()
            .flatten()
            .map(|v| v != 0)
            .unwrap_or(false),
        _ => false,
    }
}

/// Persist the auto-update master switch.
pub fn set_enabled(enabled: bool) {
    let key = match RegKeyGuard::create_write(HKEY_CURRENT_USER, REG_SUBKEY) {
        Ok(k) => k,
        Err(e) => {
            log::warn!("[auto_update] Cannot open registry key: {e}");
            return;
        }
    };
    if let Err(e) = key.write_u32(REG_ENABLED, if enabled { 1 } else { 0 }) {
        log::warn!("[auto_update] Cannot persist enabled flag: {e}");
    } else {
        log::info!(
            "[auto_update] auto-update {} (persisted)",
            if enabled { "ENABLED" } else { "disabled" }
        );
    }
}

/// Read the hidden beta feed URL. Returns `None`/empty when not configured —
/// meaning: use the regular (stable) updater, not a custom beta feed.
pub fn beta_feed_url() -> Option<String> {
    match RegKeyGuard::open_read(HKEY_CURRENT_USER, REG_SUBKEY) {
        Ok(Some(key)) => key
            .read_string(REG_BETA_URL)
            .ok()
            .flatten()
            .filter(|s| !s.trim().is_empty()),
        _ => None,
    }
}

/// Persist (or clear, with `Some("")`) the hidden beta feed URL.
pub fn set_beta_feed_url(url: Option<&str>) {
    let key = match RegKeyGuard::create_write(HKEY_CURRENT_USER, REG_SUBKEY) {
        Ok(k) => k,
        Err(e) => {
            log::warn!("[auto_update] Cannot open registry key: {e}");
            return;
        }
    };
    let value = url.unwrap_or("").trim();
    let result = key.write_string(REG_BETA_URL, value);
    if value.is_empty() {
        // Empty means "not configured" — beta_feed_url() filters it out.
        match result {
            Ok(()) => log::info!("[auto_update] beta feed URL cleared (dev feed disabled)"),
            Err(e) => log::warn!("[auto_update] Cannot clear beta feed URL: {e}"),
        }
    } else if let Err(e) = result {
        log::warn!("[auto_update] Cannot persist beta feed URL: {e}");
    } else {
        log::info!("[auto_update] beta feed URL set (dev feed enabled)");
    }
}

/// Combined config view for the Settings > Updates UI.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AutoUpdateConfig {
    pub enabled: bool,
    /// Dev-only hidden feed (empty string when not configured).
    pub beta_feed_url: String,
}

/// Read the whole config in one call (for the UI).
pub fn config() -> AutoUpdateConfig {
    AutoUpdateConfig {
        enabled: is_enabled(),
        beta_feed_url: beta_feed_url().unwrap_or_default(),
    }
}
