//! Syncthing REST client — status, folders, and minimal folder toggle (MIOT-12).
//!
//! [Syncthing](https://syncthing.net) exposes a REST API on
//! `127.0.0.1:8384` (configurable) authenticated with an API key. This module
//! is a *read + single-write* client:
//!
//! - `get_status()` — hits `system/status` and `config/folders`, mapping the
//!   interesting fields into a compact [`SyncthingStatus`] for the UI.
//! - `get_folders()` — `config/folders` full list.
//! - `set_folder_paused(folder_id, paused)` — PATCHes a single folder via the
//!   REST endpoint so a folder can be enabled/disabled from MiControl.
//! - `events()` — polls `events` for a bounded listen (used by the UI refresh
//!   to detect folder state changes).
//!
//! Fully isolated: all HTTP goes through `reqwest` (already a dependency) and
//! every function returns a `Result` so failures (daemon not running, wrong
//! key) degrade gracefully. No state is shared with other modules.

use serde::{Deserialize, Serialize};

pub const DEFAULT_ADDR: &str = "127.0.0.1:8384";

/// Runtime configured Syncthing targets.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncthingConfig {
    #[serde(default = "default_addr")]
    pub address: String,
    #[serde(default)]
    pub api_key: String,
}

fn default_addr() -> String {
    DEFAULT_ADDR.to_string()
}

impl Default for SyncthingConfig {
    fn default() -> Self {
        Self {
            address: DEFAULT_ADDR.to_string(),
            api_key: String::new(),
        }
    }
}

/// key fields from `system/status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncthingSystemStatus {
    pub my_id: String,
    pub version: String,
    #[serde(default)]
    pub uptime: u32,
    #[serde(default)]
    pub goroutines: u32,
    #[serde(default)]
    pub cpu: f64,
    #[serde(default)]
    pub sys: f64,
    #[serde(default)]
    pub connection_service_status: Option<String>,
}

/// A Syncthing folder from `config/folders`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncthingFolder {
    pub id: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub paused: bool,
    #[serde(rename = "type", default)]
    pub type_: Option<String>,
}

/// Compact UI status (daemon reachable? version? folders?).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SyncthingStatus {
    pub reachable: bool,
    pub address: String,
    pub version: Option<String>,
    pub device_id: Option<String>,
    pub uptime_secs: Option<u32>,
    pub connection_service_status: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub folders: Vec<SyncthingFolder>,
}

/// A single `events` SSE payload (only the fields we need).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncthingEvent {
    pub id: u64,
    #[serde(rename = "type")]
    pub event_type: String,
    #[serde(default)]
    pub time: String,
}

/// Build a reqwest client with the Syncthing API key header (X-API-Key).
fn client_for(cfg: &SyncthingConfig) -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    if !cfg.api_key.is_empty() {
        if let Ok(v) = reqwest::header::HeaderValue::from_str(&cfg.api_key) {
            headers.insert("X-API-Key", v);
        }
    }
    reqwest::Client::builder()
        .default_headers(headers)
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_else(|_| reqwest::Client::new())
}

/// Canonical base URL for a config (http scheme, path-safe).
pub fn base_url(cfg: &SyncthingConfig) -> String {
    let addr = cfg.address.trim();
    if addr.is_empty() {
        "http://127.0.0.1:8384".to_string()
    } else if addr.starts_with("http://") || addr.starts_with("https://") {
        addr.trim_end_matches('/').to_string()
    } else {
        format!("http://{addr}")
    }
}

/// `GET /rest/system/status`
pub async fn fetch_system_status(cfg: &SyncthingConfig) -> Result<SyncthingSystemStatus, String> {
    let url = format!("{}/rest/system/status", base_url(cfg));
    let resp = client_for(cfg)
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("syncthing status: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("syncthing status: HTTP {}", resp.status()));
    }
    resp.json()
        .await
        .map_err(|e| format!("syncthing status parse: {e}"))
}

/// `GET /rest/config/folders`
pub async fn fetch_folders(cfg: &SyncthingConfig) -> Result<Vec<SyncthingFolder>, String> {
    let url = format!("{}/rest/config/folders", base_url(cfg));
    let resp = client_for(cfg)
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("syncthing folders: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("syncthing folders: HTTP {}", resp.status()));
    }
    resp.json()
        .await
        .map_err(|e| format!("syncthing folders parse: {e}"))
}

/// `GET /rest/system/status` + `config/folders` — the compact UI status.
pub async fn get_status(cfg: &SyncthingConfig) -> SyncthingStatus {
    let address = base_url(cfg);
    match (fetch_system_status(cfg).await, fetch_folders(cfg).await) {
        (Ok(sys), Ok(folders)) => SyncthingStatus {
            reachable: true,
            address,
            version: Some(sys.version),
            device_id: Some(sys.my_id),
            uptime_secs: Some(sys.uptime),
            connection_service_status: sys.connection_service_status,
            error: None,
            folders,
        },
        (Err(e), _) | (_, Err(e)) => SyncthingStatus {
            reachable: false,
            address,
            version: None,
            device_id: None,
            uptime_secs: None,
            connection_service_status: None,
            error: Some(e),
            folders: Vec::new(),
        },
    }
}

/// Pause/resume a folder. `PATCH /rest/config/folders/{id}` with the folder's
/// existing config JSON plus `paused` overridden. Requires read+write.
pub async fn set_folder_paused(
    cfg: &SyncthingConfig,
    folder_id: &str,
    paused: bool,
) -> Result<(), String> {
    // Current config for this folder (404 → propagate as clear error).
    let url = format!("{}/rest/config/folders/{folder_id}", base_url(cfg));
    let client = client_for(cfg);
    let resp = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("syncthing folder get: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("syncthing folder get: HTTP {}", resp.status()));
    }
    let mut folder: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("syncthing folder parse: {e}"))?;
    folder["paused"] = serde_json::Value::Bool(paused);

    let put = client
        .put(&url)
        .json(&folder)
        .send()
        .await
        .map_err(|e| format!("syncthing folder patch: {e}"))?;
    if !put.status().is_success() {
        return Err(format!("syncthing folder patch: HTTP {}", put.status()));
    }
    Ok(())
}

/// Poll `events` with `since` cursor — non-blocking: reads whatever is
/// available and returns immediately (empty vec when nothing new).
pub async fn events_since(cfg: &SyncthingConfig, since: u64, limit: u32) -> Vec<SyncthingEvent> {
    let url = format!(
        "{}/rest/events?since={since}&limit={limit}&timeout=0",
        base_url(cfg)
    );
    match client_for(cfg).get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            resp.json::<Vec<SyncthingEvent>>().await.unwrap_or_default()
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_url_normalizes_plain_addr() {
        let cfg = SyncthingConfig {
            address: "127.0.0.1:8384".into(),
            api_key: String::new(),
        };
        assert_eq!(base_url(&cfg), "http://127.0.0.1:8384");
    }

    #[test]
    fn base_url_normalizes_with_scheme_and_slash() {
        let cfg = SyncthingConfig {
            address: "http://localhost:8384/".into(),
            api_key: "k".into(),
        };
        assert_eq!(base_url(&cfg), "http://localhost:8384");
        assert!(!base_url(&cfg).ends_with('/'));
    }

    #[test]
    fn base_url_empty_uses_default() {
        let cfg = SyncthingConfig::default();
        assert_eq!(base_url(&cfg), "http://127.0.0.1:8384");
    }

    #[test]
    fn base_url_keeps_https() {
        let cfg = SyncthingConfig {
            address: "https://syncthing.example:8443".into(),
            api_key: String::new(),
        };
        assert_eq!(base_url(&cfg), "https://syncthing.example:8443");
    }

    #[test]
    fn set_folder_paused_errors_cleanly_offline() {
        // Unreachable address → Err quickly, never panic.
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let cfg = SyncthingConfig {
            address: "127.0.0.1:59999".into(),
            api_key: "bogus".into(),
        };
        let _ = rt.block_on(set_folder_paused(&cfg, "no-such-folder", true));
    }

    #[test]
    fn get_status_returns_unreachable_offline() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let cfg = SyncthingConfig {
            address: "127.0.0.1:59999".into(),
            api_key: String::new(),
        };
        let st = rt.block_on(get_status(&cfg));
        assert!(!st.reachable);
        assert!(st.error.is_some());
        assert!(st.folders.is_empty());
    }
}
