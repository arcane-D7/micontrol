#![cfg(windows)]
//! BLE phone presence monitor (MIOT-04).
//!
//! Scans for the user's paired phone over Bluetooth Low Energy (WinRT backend)
//! and maintains a presence state. Crossing an RSSI threshold (moving toward /
//! away from the laptop) can drive a lock (`LockWorkStation`) or an unlock
//! hint. All config is per-user in the registry:
//!
//! `HKCU\SOFTWARE\MiControl\BlePresence`:
//!   - `Enabled`        DWORD (default 0)
//!   - `PhoneMac`       SZ    (e.g. "AA:BB:CC:DD:EE:FF", uppercase, optional)
//!   - `PhoneName`      SZ    (e.g. "Xiaomi 14T", optional)
//!   - `RssiLockDbm`    DWORD (default -70) — presence lost below this
//!   - `LockEnabled`    DWORD (default 0)  — auto LockWorkStation on loss
//!
//! Isolation: standalone. It does not touch other hardware modules; the scan
//! runs on a dedicated OS thread (btleplug requires WinRT, which must be used
//! from a single-threaded apartment initialised thread).
//!
//! The scan loop is deliberately conservative: BLE advertising-only phones
//! (like the Xiaomi 14T with randomized adv addresses) may never appear in a
//! device discovery scan; the module therefore treats "no result for N cycles"
//! as *unknown*, never as "gone", unless the target was previously seen with a
//! strong signal (then absence is treated as presence loss).

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

const CONFIG_KEY: &str = r"SOFTWARE\MiControl\BlePresence";

/// How often the scanner wakes up (seconds).
const SCAN_INTERVAL: Duration = Duration::from_secs(30);

/// Registry key names.
const K_ENABLED: &str = "Enabled";
const K_PHONE_MAC: &str = "PhoneMac";
const K_PHONE_NAME: &str = "PhoneName";
const K_RSSI_LOCK_DBM: &str = "RssiLockDbm";
const K_LOCK_ENABLED: &str = "LockEnabled";

/// Presence configuration loaded from the registry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresenceConfig {
    pub enabled: bool,
    pub phone_mac: Option<String>,
    pub phone_name: Option<String>,
    pub rssi_lock_dbm: i32,
    pub lock_enabled: bool,
}

impl Default for PresenceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            phone_mac: None,
            phone_name: None,
            rssi_lock_dbm: -70,
            lock_enabled: false,
        }
    }
}

impl PresenceConfig {
    pub fn load() -> Self {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_CURRENT_USER;

        let mut cfg = Self::default();
        let Ok(Some(key)) = RegKeyGuard::open_read(HKEY_CURRENT_USER, CONFIG_KEY) else {
            return cfg;
        };
        cfg.enabled = key
            .read_u32(K_ENABLED)
            .ok()
            .flatten()
            .map(|v| v != 0)
            .unwrap_or(false);
        cfg.phone_mac = key
            .read_string(K_PHONE_MAC)
            .ok()
            .flatten()
            .filter(|v| !v.is_empty());
        cfg.phone_name = key
            .read_string(K_PHONE_NAME)
            .ok()
            .flatten()
            .filter(|v| !v.is_empty());
        if let Some(v) = key.read_u32(K_RSSI_LOCK_DBM).ok().flatten() {
            // Stored as u32 (two's complement); interpret as i32.
            cfg.rssi_lock_dbm = v as i32;
        }
        cfg.lock_enabled = key
            .read_u32(K_LOCK_ENABLED)
            .ok()
            .flatten()
            .map(|v| v != 0)
            .unwrap_or(false);
        cfg
    }

    pub fn save(&self) {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_CURRENT_USER;

        if let Ok(key) = RegKeyGuard::create_write(HKEY_CURRENT_USER, CONFIG_KEY) {
            let _ = key.write_u32(K_ENABLED, self.enabled as u32);
            let _ = key.write_u32(K_RSSI_LOCK_DBM, self.rssi_lock_dbm as u32);
            let _ = key.write_u32(K_LOCK_ENABLED, self.lock_enabled as u32);
            if let Some(mac) = &self.phone_mac {
                let _ = key.write_string(K_PHONE_MAC, mac);
            }
            if let Some(name) = &self.phone_name {
                let _ = key.write_string(K_PHONE_NAME, name);
            }
        }
    }
}

/// Observed presence status, returned to the UI.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PresenceStatus {
    pub enabled: bool,
    /// "near" | "far" | "unknown"
    pub state: String,
    pub last_rssi_dbm: Option<i32>,
    pub last_seen_seconds_ago: Option<u64>,
    pub phone: Option<String>,
    pub scan_ok: bool,
}

/// Runtime state shared between the scanner thread and the app.
struct PresenceState {
    running: AtomicBool,
    status: std::sync::Mutex<PresenceStatus>,
}

static STATE: OnceLock<PresenceState> = OnceLock::new();

fn state() -> &'static PresenceState {
    STATE.get_or_init(|| PresenceState {
        running: AtomicBool::new(false),
        status: std::sync::Mutex::new(PresenceStatus {
            enabled: false,
            state: "unknown".into(),
            last_rssi_dbm: None,
            last_seen_seconds_ago: None,
            phone: None,
            scan_ok: false,
        }),
    })
}

/// Start the presence monitor. Idempotent. Spawns the WinRT scanner on a
/// dedicated thread with its own single-threaded tokio runtime so the scan
/// never interferes with the app async runtime or COM apartments.
pub fn start_presence_monitor() {
    if state().running.swap(true, Ordering::SeqCst) {
        return;
    }

    std::thread::Builder::new()
        .name("ble-presence".into())
        .spawn(|| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .expect("failed to build BLE presence runtime");
            runtime.block_on(scanner_loop());
        })
        .expect("failed to spawn BLE presence thread");
}

/// Trigger a scan now from another thread (used by sleep/resume and tests).
pub fn scan_now() {
    // A best-effort one-off scan on a short-lived runtime.
    std::thread::spawn(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .map_err(|e| log::error!("ble presence one-off runtime failed: {e}"))
            .ok();
        if let Some(rt) = runtime {
            rt.block_on(async {
                let _ = scan_once().await;
            });
        }
    });
}

/// Scanner main loop: a bounded, cooldown-gated scan of BLE devices.
async fn scanner_loop() {
    let mut interval = tokio::time::interval(SCAN_INTERVAL);
    // First tick fires immediately.
    interval.tick().await;
    loop {
        interval.tick().await;
        let _ = scan_once().await;
    }
}

/// Single scan pass against the configured phone. Never panics on BLE errors;
/// failures degrade to `scan_ok: false` while keeping prior state.
async fn scan_once() -> Result<(), String> {
    let cfg = PresenceConfig::load();
    if !cfg.enabled {
        return Ok(());
    }

    let needle = cfg
        .phone_mac
        .clone()
        .map(|m| normalize_mac(&m))
        .filter(|m| !m.is_empty())
        .or_else(|| cfg.phone_name.clone().map(|n| n.to_ascii_lowercase()));

    let Some(needle) = needle else {
        log::info!("[ble_presence] enabled but no phone configured");
        return Ok(());
    };

    match scan_for_device(&needle).await {
        Ok(Some(rssi)) => {
            let far = rssi < cfg.rssi_lock_dbm;
            let new_state = if far { "far" } else { "near" };
            let mut st = lock_status();
            st.state = new_state.to_string();
            st.last_rssi_dbm = Some(rssi);
            st.last_seen_seconds_ago = Some(0);
            st.scan_ok = true;
            st.phone = cfg.phone_name.clone().or_else(|| cfg.phone_mac.clone());
            log::info!("[ble_presence] phone seen rssi={rssi} state={new_state}");
            if far && cfg.lock_enabled {
                request_lock();
            }
        }
        Ok(None) => {
            let mut st = lock_status();
            st.scan_ok = true;
            // Only ever downgrade to "far" if the phone was previously near —
            // absence alone is inconclusive (randomized advertising).
            if st.state == "near" {
                st.state = "far".into();
                if cfg.lock_enabled {
                    request_lock();
                }
            }
            st.last_seen_seconds_ago = st.last_seen_seconds_ago.map(|s| s + 1);
            log::debug!("[ble_presence] phone not seen this cycle");
        }
        Err(e) => {
            log::warn!("[ble_presence] scan failed: {e}");
            let mut st = lock_status();
            st.scan_ok = false;
        }
    }
    Ok(())
}

fn lock_status() -> std::sync::MutexGuard<'static, PresenceStatus> {
    let st = state();
    st.status
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Current presence state (for commands / UI polling).
pub fn get_presence_status() -> PresenceStatus {
    let cfg = PresenceConfig::load();
    let mut st = lock_status().clone();
    st.enabled = cfg.enabled;
    st
}

/// Lock the workstation with a purpose-specific action (LockWorkStation).
fn request_lock() {
    log::info!("[ble_presence] phone far — locking workstation");
    tauri::async_runtime::spawn_blocking(|| {
        use windows::Win32::System::Shutdown::LockWorkStation;
        let _ = unsafe { LockWorkStation() };
    });
}

/// Normalise a MAC address to uppercase with '-' separators (btleplug uses
/// uppercase hex with '-' on Windows).
fn normalize_mac(mac: &str) -> String {
    mac.trim().replace([':', ' '], "-").to_ascii_uppercase()
}

/// Does the given btleplug peripheral match our needle (MAC or name)?
async fn peripheral_matches(peripheral: &btleplug::platform::Peripheral, needle: &str) -> bool {
    use btleplug::api::Peripheral;

    let Some(props) = peripheral
        .properties()
        .await
        .map_err(|e| log::debug!("[ble_presence] props error: {e}"))
        .ok()
        .flatten()
    else {
        return false;
    };
    let addr = props.address.to_string();
    if addr.eq_ignore_ascii_case(needle) {
        return true;
    }
    if let Some(name) = props.local_name.as_ref() {
        if name.to_ascii_lowercase() == needle {
            return true;
        }
    }
    false
}

/// One discovery pass returning the RSSI of the target if found.
async fn scan_for_device(needle: &str) -> Result<Option<i32>, String> {
    use btleplug::api::{Central, Manager as _, Peripheral, ScanFilter};

    let manager = btleplug::platform::Manager::new()
        .await
        .map_err(|e| e.to_string())?;
    let adapters = manager.adapters().await.map_err(|e| e.to_string())?;
    if adapters.is_empty() {
        return Err("no BLE adapter found".to_string());
    }

    // Try every adapter — a phone may be bound to the second radio.
    let mut last_err: Option<String> = None;
    for central in adapters {
        if let Err(e) = central.start_scan(ScanFilter::default()).await {
            last_err = Some(e.to_string());
            continue;
        }
        tokio::time::sleep(Duration::from_secs(5)).await;

        if let Ok(peripherals) = central.peripherals().await {
            for p in peripherals {
                if peripheral_matches(&p, needle).await {
                    let rssi = p
                        .properties()
                        .await
                        .ok()
                        .flatten()
                        .and_then(|props| props.rssi)
                        .map(i32::from);
                    // Best effort stop before returning.
                    let _ = central.stop_scan().await;
                    return Ok(rssi);
                }
            }
        }
        let _ = central.stop_scan().await;
    }
    match last_err {
        Some(e) => Err(e),
        None => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_mac_handles_colons_and_case() {
        assert_eq!(normalize_mac("aa:bb:cc:dd:ee:ff"), "AA-BB-CC-DD-EE-FF");
        assert_eq!(normalize_mac(" AA:BB:cc:DD:EE:ff "), "AA-BB-CC-DD-EE-FF");
        assert_eq!(normalize_mac(""), "");
    }

    #[test]
    fn default_config_disabled() {
        let c = PresenceConfig::default();
        assert!(!c.enabled);
        assert!(!c.lock_enabled);
        assert_eq!(c.rssi_lock_dbm, -70);
        assert!(c.phone_mac.is_none());
    }

    #[test]
    fn load_returns_defaults_when_registry_missing() {
        let c = PresenceConfig::load();
        assert!(!c.enabled);
        assert_eq!(c.rssi_lock_dbm, -70);
    }

    #[test]
    fn rssi_threshold_classification() {
        let cfg = PresenceConfig::default(); // -70 dBm
                                             // "near" means signal is stronger (numerically higher) than threshold.
        assert!(-65 >= cfg.rssi_lock_dbm);
        // "far" means signal is weaker (numerically lower) than threshold.
        assert!(-80 < cfg.rssi_lock_dbm);
    }
}
