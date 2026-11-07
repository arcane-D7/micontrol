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
    /// Wall-clock instant of the last successful sighting. Kept out of
    /// `PresenceStatus` (a serialised snapshot) so `last_seen_seconds_ago`
    /// can be computed on read instead of being incremented per scan cycle
    /// (cycles are 30 s apart, so the old counter reported "seconds" that
    /// were really half-minutes).
    last_seen: std::sync::Mutex<Option<std::time::Instant>>,
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
        last_seen: std::sync::Mutex::new(None),
    })
}

fn lock_last_seen() -> std::sync::MutexGuard<'static, Option<std::time::Instant>> {
    state()
        .last_seen
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Seconds since the phone was last seen, or `None` if it never was.
fn last_seen_seconds_ago() -> Option<u64> {
    let last = *lock_last_seen();
    last.map(|t| t.elapsed().as_secs())
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

    let Some(target) = MatchTarget::from_config(&cfg) else {
        log::info!("[ble_presence] enabled but no phone configured");
        return Ok(());
    };
    if cfg.phone_mac.is_some() && target.mac.is_none() {
        log::warn!(
            "[ble_presence] configured PhoneMac '{}' is not a 12-digit hex address — matching by name only",
            cfg.phone_mac.as_deref().unwrap_or_default()
        );
    }

    match scan_for_device(&target).await {
        Ok(Some(found)) => {
            // A phone that advertises without RSSI is still *present*; the
            // threshold only applies when we actually measured a signal.
            let far = found.rssi.is_some_and(|rssi| rssi < cfg.rssi_lock_dbm);
            let new_state = if far { "far" } else { "near" };
            let mut st = lock_status();
            st.state = new_state.to_string();
            st.last_rssi_dbm = found.rssi;
            st.last_seen_seconds_ago = Some(0);
            st.scan_ok = true;
            st.phone = cfg
                .phone_name
                .clone()
                .or(found.name.clone())
                .or(Some(found.address.clone()));
            log::info!(
                "[ble_presence] phone seen rssi={:?} state={new_state} matched={:?} addr={}",
                found.rssi,
                found.matched,
                found.address
            );
            *lock_last_seen() = Some(std::time::Instant::now());
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
            st.last_seen_seconds_ago = last_seen_seconds_ago();
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
    // Recompute on read: the scanner only refreshes every 30 s.
    st.last_seen_seconds_ago = last_seen_seconds_ago();
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

/// Canonical form of a Bluetooth address: exactly 12 uppercase hex digits,
/// no separators.
///
/// Three formats are in play and they must all compare equal:
///   - `AA:BB:CC:DD:EE:FF` — what `btleplug` prints for `BDAddr` on Windows
///     (`Display` impl in `bdaddr.rs`), i.e. every address the scan sees;
///   - `AA-BB-CC-DD-EE-FF` — what this module used to store and what the BLE
///     picker (`ble_scan::extract_address_from_id`) writes to the registry;
///   - `AABBCCDDEEFF` — no separators.
///
/// Returns `None` for anything that is not 12 hex digits (e.g. a synthetic
/// `"Bluetooth device"` placeholder), so callers can tell "unparseable" from
/// "does not match".
fn canonical_mac(raw: &str) -> Option<String> {
    let hex: String = raw.chars().filter(char::is_ascii_hexdigit).collect();
    (hex.len() == 12).then(|| hex.to_ascii_uppercase())
}

/// How a peripheral matched the configured phone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchKind {
    Mac,
    Name,
}

/// What the scan is looking for: the configured MAC (canonical) and/or name
/// (trimmed, lower-case). Both are held at once on purpose — the MAC is the
/// precise key, the name is the fallback for phones that advertise a
/// randomised BLE address (see H2 in the audit), and a phone may be
/// identified by either.
#[derive(Debug, Clone, PartialEq)]
struct MatchTarget {
    mac: Option<String>,
    name: Option<String>,
}

impl MatchTarget {
    fn from_config(cfg: &PresenceConfig) -> Option<Self> {
        let mac = cfg.phone_mac.as_deref().and_then(canonical_mac);
        let name = cfg
            .phone_name
            .as_deref()
            .map(normalize_name)
            .filter(|n| !n.is_empty());
        if mac.is_none() && name.is_none() {
            None
        } else {
            Some(Self { mac, name })
        }
    }

    /// Match a device identity. `address` is whatever the backend prints
    /// (dashes or colons — both are canonicalised); `local_name` is the
    /// advertisement's local name, when present.
    fn matches(&self, address: &str, local_name: Option<&str>) -> Option<MatchKind> {
        if let (Some(want), Some(got)) = (self.mac.as_deref(), canonical_mac(address)) {
            if want == got {
                return Some(MatchKind::Mac);
            }
        }
        if let (Some(want), Some(got)) = (self.name.as_deref(), local_name) {
            if want == normalize_name(got) {
                return Some(MatchKind::Name);
            }
        }
        None
    }
}

/// Name comparison basis: case-insensitive, surrounding whitespace ignored.
fn normalize_name(name: &str) -> String {
    name.trim().to_lowercase()
}

/// A peripheral that matched the configured phone.
#[derive(Debug, Clone)]
struct FoundDevice {
    rssi: Option<i32>,
    /// Address as reported by btleplug (colon-separated on Windows).
    address: String,
    name: Option<String>,
    matched: MatchKind,
}

/// Does the given btleplug peripheral match our target (MAC or name)?
async fn peripheral_match(
    peripheral: &btleplug::platform::Peripheral,
    target: &MatchTarget,
) -> Option<FoundDevice> {
    use btleplug::api::Peripheral;

    let props = peripheral
        .properties()
        .await
        .map_err(|e| log::debug!("[ble_presence] props error: {e}"))
        .ok()
        .flatten()?;
    let address = props.address.to_string();
    let matched = target.matches(&address, props.local_name.as_deref())?;
    Some(FoundDevice {
        rssi: props.rssi.map(i32::from),
        address,
        name: props.local_name,
        matched,
    })
}

/// One discovery pass returning the target device (with its RSSI) if found.
async fn scan_for_device(target: &MatchTarget) -> Result<Option<FoundDevice>, String> {
    use btleplug::api::{Central, Manager as _, ScanFilter};

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
                if let Some(found) = peripheral_match(&p, target).await {
                    // Best effort stop before returning.
                    let _ = central.stop_scan().await;
                    return Ok(Some(found));
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

    /// Temporarily hide `HKCU\SOFTWARE\MiControl\BlePresence` for the duration
    /// of a test so registry-based assertions observe a "fresh machine". On
    /// dev machines MiControl is installed and the real key exists, which
    /// would otherwise make "key missing" assumptions false. The original key
    /// is captured up front and restored on drop (including panics), so the
    /// developer's real configuration survives the test.
    ///
    /// Uses RAII via a `Drop` impl: hide in `new()`, restore in `drop()`.
    struct HiddenBlePresenceKey {
        /// Original values read before deletion. `None` entries mark values
        /// that did not exist — restoring means simply not writing them back.
        enabled: Option<u32>,
        phone_mac: Option<String>,
        phone_name: Option<String>,
        rssi_lock_dbm: Option<u32>,
        lock_enabled: Option<u32>,
        /// `true` when the real key existed and values were captured.
        had_real_key: bool,
    }

    impl HiddenBlePresenceKey {
        fn new() -> Self {
            use crate::util::registry::RegKeyGuard;
            use windows::Win32::System::Registry::{RegDeleteTreeW, HKEY_CURRENT_USER};

            let mut enabled: Option<u32> = None;
            let mut phone_mac: Option<String> = None;
            let mut phone_name: Option<String> = None;
            let mut rssi_lock_dbm: Option<u32> = None;
            let mut lock_enabled: Option<u32> = None;
            let mut had_real_key = false;

            if let Ok(Some(key)) = RegKeyGuard::open_read(HKEY_CURRENT_USER, CONFIG_KEY) {
                had_real_key = true;
                enabled = key.read_u32(K_ENABLED).ok().flatten();
                phone_mac = key.read_string(K_PHONE_MAC).ok().flatten();
                phone_name = key.read_string(K_PHONE_NAME).ok().flatten();
                rssi_lock_dbm = key.read_u32(K_RSSI_LOCK_DBM).ok().flatten();
                lock_enabled = key.read_u32(K_LOCK_ENABLED).ok().flatten();
            }

            // 2. Delete the real key so the test sees a "missing" state.
            let subkey_w: Vec<u16> = CONFIG_KEY
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let _ = unsafe {
                RegDeleteTreeW(HKEY_CURRENT_USER, windows::core::PCWSTR(subkey_w.as_ptr()))
            };
            if had_real_key {
                log::debug!("[ble_presence.test] hid BlePresence registry key");
            } else {
                log::debug!("[ble_presence.test] ensured BlePresence registry key absent");
            }

            Self {
                enabled,
                phone_mac,
                phone_name,
                rssi_lock_dbm,
                lock_enabled,
                had_real_key,
            }
        }
    }

    impl Drop for HiddenBlePresenceKey {
        /// Best-effort restore of the developer's real configuration.
        fn drop(&mut self) {
            if !self.had_real_key {
                return; // Nothing to restore — the key was already absent.
            }

            use crate::util::registry::RegKeyGuard;
            use windows::Win32::System::Registry::HKEY_CURRENT_USER;

            let guard = match RegKeyGuard::create_write(HKEY_CURRENT_USER, CONFIG_KEY) {
                Ok(g) => g,
                Err(e) => {
                    log::warn!(
                        "[ble_presence.test] failed to open BlePresence registry key for \
                         restore: {e} — developer config NOT restored, re-set manually"
                    );
                    return;
                }
            };

            if let Some(v) = self.enabled {
                let _ = guard.write_u32(K_ENABLED, v);
            }
            if let Some(v) = &self.phone_mac {
                let _ = guard.write_string(K_PHONE_MAC, v);
            }
            if let Some(v) = &self.phone_name {
                let _ = guard.write_string(K_PHONE_NAME, v);
            }
            if let Some(v) = self.rssi_lock_dbm {
                let _ = guard.write_u32(K_RSSI_LOCK_DBM, v);
            }
            if let Some(v) = self.lock_enabled {
                let _ = guard.write_u32(K_LOCK_ENABLED, v);
            }
            log::debug!("[ble_presence.test] restored original BlePresence registry values");
        }
    }

    #[test]
    fn canonical_mac_accepts_every_separator_format() {
        let cases = [
            ("aa:bb:cc:dd:ee:ff", "AABBCCDDEEFF"),
            ("AA:BB:CC:DD:EE:FF", "AABBCCDDEEFF"),
            ("AA-BB-CC-DD-EE-FF", "AABBCCDDEEFF"),
            ("aa-bb-cc-dd-ee-ff", "AABBCCDDEEFF"),
            ("AABBCCDDEEFF", "AABBCCDDEEFF"),
            (" AA:BB:CC:DD:EE:FF ", "AABBCCDDEEFF"),
            // btleplug prints BDAddr exactly like this on Windows.
            ("FC:43:45:81:4B:9A", "FC4345814B9A"),
        ];
        for (raw, expected) in cases {
            assert_eq!(
                canonical_mac(raw).as_deref(),
                Some(expected),
                "unexpected canonical form for {raw:?}"
            );
        }
    }

    #[test]
    fn canonical_mac_of_the_real_phone_in_both_formats_is_equal() {
        // Regression: the registry held `FC-43-45-81-4B-9A` while btleplug
        // reported `FC:43:45:81:4B:9A`; the comparison compared the raw
        // strings and never matched. Both must canonicalise identically.
        assert_eq!(
            canonical_mac("FC-43-45-81-4B-9A"),
            canonical_mac("FC:43:45:81:4B:9A")
        );
    }

    #[test]
    fn canonical_mac_rejects_non_addresses() {
        assert_eq!(canonical_mac(""), None);
        assert_eq!(canonical_mac("Bluetooth device"), None);
        // 11 and 13 hex digits are not addresses.
        assert_eq!(canonical_mac("FC:43:45:81:4B:9"), None);
        assert_eq!(canonical_mac("FC:43:45:81:4B:9A:00"), None);
    }

    fn target(mac: Option<&str>, name: Option<&str>) -> Option<MatchTarget> {
        MatchTarget::from_config(&PresenceConfig {
            enabled: true,
            phone_mac: mac.map(str::to_string),
            phone_name: name.map(str::to_string),
            rssi_lock_dbm: -70,
            lock_enabled: false,
        })
    }

    /// The regression test the audit asked for: the stored (dash) form must
    /// match the address btleplug actually reports (colons).
    #[test]
    fn target_matches_btleplug_colon_address_against_stored_dash_mac() {
        let t = target(Some("FC-43-45-81-4B-9A"), Some("Xiaomi 14T")).unwrap();
        assert_eq!(t.matches("FC:43:45:81:4B:9A", None), Some(MatchKind::Mac));
        // Case-insensitive as well.
        assert_eq!(t.matches("fc:43:45:81:4b:9a", None), Some(MatchKind::Mac));
    }

    /// H2: a phone that advertises a randomised BLE address must still be
    /// recognised by name even when a MAC is configured.
    #[test]
    fn target_falls_back_to_name_when_the_advertised_address_differs() {
        let t = target(Some("FC-43-45-81-4B-9A"), Some("Xiaomi 14T")).unwrap();
        assert_eq!(
            t.matches("6E:11:22:33:44:55", Some("Xiaomi 14T")),
            Some(MatchKind::Name)
        );
        assert_eq!(
            t.matches("6E:11:22:33:44:55", Some("  xiaomi 14t ")),
            Some(MatchKind::Name)
        );
    }

    #[test]
    fn target_does_not_match_other_devices() {
        let t = target(Some("FC-43-45-81-4B-9A"), Some("Xiaomi 14T")).unwrap();
        assert_eq!(t.matches("FC:43:45:81:4B:9B", Some("TV Box")), None);
        // Same name, different case-insensitive compare still works — but an
        // unrelated name must not match.
        assert_eq!(t.matches("FC:43:45:81:4B:9B", Some("Xiaomi 14")), None);
    }

    #[test]
    fn target_by_name_only_when_mac_is_blank_or_unparseable() {
        // No MAC configured: name-only target (the old code's behaviour).
        let by_name = target(None, Some("Xiaomi 14T")).unwrap();
        assert_eq!(
            by_name.matches("FC:43:45:81:4B:9A", Some("Xiaomi 14T")),
            Some(MatchKind::Name)
        );
        // A MAC that is not an address must not poison the name fallback.
        let garbage = target(Some("Xiaomi 14T's MAC"), Some("Xiaomi 14T")).unwrap();
        assert_eq!(garbage.mac, None);
        assert_eq!(
            garbage.matches("FC:43:45:81:4B:9A", Some("Xiaomi 14T")),
            Some(MatchKind::Name)
        );
    }

    #[test]
    fn target_is_none_when_nothing_usable_is_configured() {
        assert_eq!(target(None, None), None);
        assert_eq!(target(Some("   "), Some("")), None);
        assert_eq!(target(Some("not-a-mac"), None), None);
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
        let _hidden = HiddenBlePresenceKey::new();
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
