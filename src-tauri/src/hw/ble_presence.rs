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
//!
//! ## Anti-false-lock policy (REV 2026-11)
//!
//! A single RSSI sample is **never** evidence that the bound device left. BLE
//! RSSI on a wrist-worn tracker swings 20-30 dB between advertisements (body
//! shadowing, arm position, radio duty cycle), and the 5 s scan window inside a
//! 30 s cycle *misses* most advertisements even when the device is on the desk.
//! The old code locked on either event, which produced two false locks 60 s
//! apart with the device at -52 dBm.
//!
//! Now a lock requires ALL of:
//!   * `FAR_CONFIRM_SAMPLES` consecutive far observations (seen below
//!     `RssiLockDbm`, or absent for longer than `MISS_GRACE`);
//!   * the device continuously far for at least `FAR_CONFIRM_WINDOW`;
//!   * no lock in the previous `LOCK_COOLDOWN`;
//!   * hysteresis: once far, the state only returns to `near` above
//!     `RssiLockDbm + HYSTERESIS_DBM`, so a device sitting at the threshold
//!     cannot flap.
//!
//! Every sample is logged raw (`rssi=`, `streak=`, `far_for=`) so a field test
//! can be read without a debugger.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const CONFIG_KEY: &str = r"SOFTWARE\MiControl\BlePresence";

/// How often the scanner wakes up (seconds).
const SCAN_INTERVAL: Duration = Duration::from_secs(30);

/// Consecutive far observations required before a lock is even considered.
const FAR_CONFIRM_SAMPLES: u32 = 3;

/// How often a *steady-state* sample still reaches INFO (1 scan = 30 s).
///
/// 10 samples ≈ 5 min: often enough that a field test sees the current
/// far/miss counters without waiting, rare enough that the line stops being
/// the app log. Production (recheck 2026-11-14): 6 410 `[ble_presence] sample`
/// lines in 53 h = 66 % of the whole file, all at INFO.
const SAMPLE_INFO_EVERY: u64 = 10;

/// Level for one scan result.
///
/// A state transition is news and always goes to INFO, and so does the first
/// sample after it (the `sample_log_level` caller clears the streak on a
/// change) plus one heartbeat every [`SAMPLE_INFO_EVERY`] scans. Everything
/// else is a counter tick: DEBUG.
pub(crate) fn sample_log_level(state_changed: bool, scan_index: u64) -> log::Level {
    if state_changed || scan_index.is_multiple_of(SAMPLE_INFO_EVERY) {
        log::Level::Info
    } else {
        log::Level::Debug
    }
}

/// Monotonic scan counter, used only for the heartbeat decision.
static SCAN_INDEX: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// The state string last written at INFO, so a transition is not missed when
/// it happens on a non-heartbeat scan.
static LAST_LOGGED_STATE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Decide the level for this scan and remember the state.
fn sample_level_for(outcome_state: &str) -> log::Level {
    let index = SCAN_INDEX.fetch_add(1, Ordering::Relaxed) + 1;
    let changed = {
        let mut last = match LAST_LOGGED_STATE.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let changed = last.as_deref() != Some(outcome_state);
        if changed {
            *last = Some(outcome_state.to_string());
        }
        changed
    };
    sample_log_level(changed, index)
}

/// How long the device must have been continuously far before locking.
const FAR_CONFIRM_WINDOW: Duration = Duration::from_secs(60);

/// Minimum spacing between two automatic locks.
const LOCK_COOLDOWN: Duration = Duration::from_secs(120);

/// Hysteresis width: after a `far` classification the state only returns to
/// `near` once RSSI climbs `RssiLockDbm + HYSTERESIS_DBM`.
const HYSTERESIS_DBM: i32 = 10;

/// Absence shorter than this is an advertising gap, not a departure. The scan
/// window is 5 s out of every 30 s, so missing a whole cycle proves nothing.
const MISS_GRACE: Duration = Duration::from_secs(75);

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
    /// Consecutive far observations in the current streak (0 = near/unknown).
    #[serde(default)]
    pub far_streak: u32,
    /// How long the device has been continuously far, in seconds.
    #[serde(default)]
    pub far_seconds: u64,
    /// Consecutive samples that did not match the bound device.
    #[serde(default)]
    pub miss_streak: u32,
}

/// Classified presence of the bound device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GateState {
    Unknown,
    Near,
    Far,
}

impl GateState {
    fn as_str(self) -> &'static str {
        match self {
            GateState::Unknown => "unknown",
            GateState::Near => "near",
            GateState::Far => "far",
        }
    }
}

/// One observation fed to the lock gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sample {
    /// The bound device was seen advertising; `rssi` is the raw dBm value when
    /// the advertisement carried one.
    Seen(Option<i32>),
    /// No advertisement from the bound device in this scan cycle.
    Miss,
}

/// What the gate decided for one observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct GateOutcome {
    state: GateState,
    far_streak: u32,
    far_seconds: u64,
    miss_streak: u32,
    /// True only when every confirmation rule passed.
    lock: bool,
}

/// Confirmation state machine for the auto-lock.
///
/// Pure (no I/O, `now` injected) so the policy is unit-testable without a radio
/// and without ever calling `LockWorkStation`.
#[derive(Debug, Default, Clone)]
struct LockGate {
    state: Option<GateState>,
    far_streak: u32,
    far_since: Option<Instant>,
    last_lock: Option<Instant>,
    last_seen: Option<Instant>,
    miss_streak: u32,
}

impl LockGate {
    fn state(&self) -> GateState {
        self.state.unwrap_or(GateState::Unknown)
    }

    fn clear_far(&mut self) {
        self.far_streak = 0;
        self.far_since = None;
        self.miss_streak = 0;
    }

    /// Record a far observation and, if the full confirmation policy passes,
    /// authorise a lock. Callers must not lock on anything else.
    fn observe(&mut self, sample: Sample, cfg: &PresenceConfig, now: Instant) -> GateOutcome {
        match sample {
            Sample::Seen(rssi) => {
                self.last_seen = Some(now);
                self.miss_streak = 0;
                match rssi {
                    // Presence proven, distance unmeasured: never a departure.
                    None => {
                        self.clear_far();
                        self.state = Some(GateState::Near);
                    }
                    Some(dbm) if dbm >= cfg.rssi_lock_dbm + HYSTERESIS_DBM => {
                        self.clear_far();
                        self.state = Some(GateState::Near);
                    }
                    Some(dbm) if dbm < cfg.rssi_lock_dbm => {
                        self.far_streak = self.far_streak.saturating_add(1);
                        self.far_since.get_or_insert(now);
                        self.state = Some(GateState::Far);
                    }
                    // Dead band (-70..-60 for a -70 threshold): keep the current
                    // state, count neither near nor far evidence.
                    Some(_) => {
                        if self.state.is_none() {
                            self.state = Some(GateState::Near);
                        }
                    }
                }
            }
            Sample::Miss => {
                self.miss_streak = self.miss_streak.saturating_add(1);
                let absent_for = self.last_seen.map(|t| now.saturating_duration_since(t));
                match absent_for {
                    // Never seen, or absent for less than the grace window: the
                    // 5 s/30 s scan duty cycle makes this uninformative.
                    None => {}
                    Some(d) if d < MISS_GRACE => {}
                    Some(_) => {
                        self.far_streak = self.far_streak.saturating_add(1);
                        self.far_since
                            .get_or_insert(now.checked_sub(MISS_GRACE).unwrap_or(now));
                        self.state = Some(GateState::Far);
                    }
                }
            }
        }

        let far_for = self
            .far_since
            .map(|t| now.saturating_duration_since(t))
            .unwrap_or_default();
        let cooled_down = self
            .last_lock
            .map(|t| now.saturating_duration_since(t) >= LOCK_COOLDOWN)
            .unwrap_or(true);
        let confirmed = self.state == Some(GateState::Far)
            && self.far_streak >= FAR_CONFIRM_SAMPLES
            && far_for >= FAR_CONFIRM_WINDOW
            && cooled_down;

        let lock = confirmed && cfg.lock_enabled;
        if lock {
            self.last_lock = Some(now);
            // A second lock requires a fresh, fully confirmed departure.
            self.far_streak = 0;
            self.far_since = None;
        }

        GateOutcome {
            state: self.state(),
            far_streak: self.far_streak,
            far_seconds: far_for.as_secs(),
            miss_streak: self.miss_streak,
            lock,
        }
    }
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
    /// Lock confirmation state machine (see `LockGate`).
    gate: std::sync::Mutex<LockGate>,
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
            far_streak: 0,
            far_seconds: 0,
            miss_streak: 0,
        }),
        last_seen: std::sync::Mutex::new(None),
        gate: std::sync::Mutex::new(LockGate::default()),
    })
}

fn lock_gate() -> std::sync::MutexGuard<'static, LockGate> {
    state()
        .gate
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
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

    let now = Instant::now();
    match scan_for_device(&target).await {
        Ok(Some(found)) => {
            // A device that advertises without RSSI is still *present*; the
            // threshold only applies when we actually measured a signal.
            let outcome = lock_gate().observe(Sample::Seen(found.rssi), &cfg, now);
            {
                let mut st = lock_status();
                st.state = outcome.state.as_str().to_string();
                st.last_rssi_dbm = found.rssi;
                st.last_seen_seconds_ago = Some(0);
                st.scan_ok = true;
                st.far_streak = outcome.far_streak;
                st.far_seconds = outcome.far_seconds;
                st.miss_streak = outcome.miss_streak;
                st.phone = cfg
                    .phone_name
                    .clone()
                    .or(found.name.clone())
                    .or(Some(found.address.clone()));
            }
            *lock_last_seen() = Some(now);
            // A field test reads this line. Steady-state scans are DEBUG so a
            // 53 h session does not put 6 410 counter ticks in the log.
            log::log!(
                sample_level_for(outcome.state.as_str()),
                "[ble_presence] sample rssi={:?} dbm threshold={} state={} streak={}/{} far_for={}s matched={:?} addr={}",
                found.rssi,
                cfg.rssi_lock_dbm,
                outcome.state.as_str(),
                outcome.far_streak,
                FAR_CONFIRM_SAMPLES,
                outcome.far_seconds,
                found.matched,
                found.address
            );
            if outcome.lock {
                request_lock(&cfg, found.rssi, outcome.far_seconds);
            }
        }
        Ok(None) => {
            let outcome = lock_gate().observe(Sample::Miss, &cfg, now);
            {
                let mut st = lock_status();
                st.scan_ok = true;
                st.state = outcome.state.as_str().to_string();
                st.far_streak = outcome.far_streak;
                st.far_seconds = outcome.far_seconds;
                st.miss_streak = outcome.miss_streak;
                st.last_seen_seconds_ago = last_seen_seconds_ago();
            }
            log::log!(
                sample_level_for(outcome.state.as_str()),
                "[ble_presence] sample rssi=none (no advertisement) state={} streak={}/{} far_for={}s misses={} last_seen={:?}s",
                outcome.state.as_str(),
                outcome.far_streak,
                FAR_CONFIRM_SAMPLES,
                outcome.far_seconds,
                outcome.miss_streak,
                last_seen_seconds_ago()
            );
            if outcome.lock {
                request_lock(&cfg, None, outcome.far_seconds);
            }
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

/// Lock the workstation. Only ever called after `LockGate` confirmed the
/// departure (N samples + minimum window + cooldown).
fn request_lock(cfg: &PresenceConfig, rssi: Option<i32>, far_seconds: u64) {
    log::info!(
        "[ble_presence] LOCK: device '{}' confirmed far (rssi={:?} dbm, threshold={} dbm, far for {}s, {}+ samples) — locking workstation",
        cfg.phone_name.as_deref().unwrap_or("?"),
        rssi,
        cfg.rssi_lock_dbm,
        far_seconds,
        FAR_CONFIRM_SAMPLES
    );
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
    ///
    /// **Exact MAC wins.** When a parseable MAC is configured it is the only
    /// accepted identity: a name match alone must never drive a lock, because
    /// any device can advertise any local name and a second tracker with the
    /// same model name would otherwise be able to lock the workstation. The
    /// name is used as the identity only when no MAC is configured (or the
    /// stored MAC is unparseable), and `name_only_hit` reports the near-miss
    /// case so a rotated address is visible in the log instead of silent.
    fn matches(&self, address: &str, local_name: Option<&str>) -> Option<MatchKind> {
        if let Some(want) = self.mac.as_deref() {
            return match canonical_mac(address) {
                Some(got) if got == want => Some(MatchKind::Mac),
                _ => None,
            };
        }
        if let (Some(want), Some(got)) = (self.name.as_deref(), local_name) {
            if want == normalize_name(got) {
                return Some(MatchKind::Name);
            }
        }
        None
    }

    /// True when the advertisement carries the configured name but the address
    /// is not the bound one — i.e. a possible BLE address rotation.
    fn name_only_hit(&self, local_name: Option<&str>) -> bool {
        match (self.name.as_deref(), local_name) {
            (Some(want), Some(got)) => want == normalize_name(got),
            _ => false,
        }
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
    let mut name_only: Vec<String> = Vec::new();
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
                // Diagnostic only: the configured name showed up under a
                // different address. Never used for the decision (see
                // `MatchTarget::matches`), but it is the signature of a BLE
                // address rotation and must not be silent.
                if target.mac.is_some() {
                    use btleplug::api::Peripheral as _;
                    if let Ok(Some(props)) = p.properties().await {
                        if target.name_only_hit(props.local_name.as_deref()) {
                            name_only.push(props.address.to_string());
                        }
                    }
                }
            }
        }
        let _ = central.stop_scan().await;
    }
    if !name_only.is_empty() {
        log::warn!(
            "[ble_presence] device named {:?} seen at a different address ({}) — the bound MAC {} did not match; re-bind the device if its BLE address rotated",
            target.name.as_deref().unwrap_or("?"),
            name_only.join(", "),
            target.mac.as_deref().unwrap_or("?")
        );
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

    /// A configured MAC is the ONLY accepted identity. A device that merely
    /// advertises the same name must not be able to drive a lock (two trackers
    /// of the same model, or a spoofed local name).
    #[test]
    fn target_ignores_name_when_a_mac_is_configured() {
        let t = target(Some("FC-43-45-81-4B-9A"), Some("Xiaomi 14T")).unwrap();
        assert_eq!(t.matches("6E:11:22:33:44:55", Some("Xiaomi 14T")), None);
        assert_eq!(t.matches("6E:11:22:33:44:55", Some("  xiaomi 14t ")), None);
        // ...but the near-miss is reported so an address rotation is visible.
        assert!(t.name_only_hit(Some("  xiaomi 14t ")));
        assert!(!t.name_only_hit(Some("TV Box")));
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

    // ── Lock gate: the false-lock regression suite ───────────────────────────
    //
    // Reproduces the 2026-11-08 incident: the bound device (Amazfit Balance 2,
    // D8:80:3C:D0:1C:A4) read -52 dBm, one advertisement was missed, and the
    // workstation locked twice in 60 s. None of these tests touch the radio or
    // `LockWorkStation` — the gate is pure.

    fn lock_cfg() -> PresenceConfig {
        PresenceConfig {
            enabled: true,
            phone_mac: Some("D8-80-3C-D0-1C-A4".into()),
            phone_name: Some("Amazfit Balance 2".into()),
            rssi_lock_dbm: -70,
            lock_enabled: true,
        }
    }

    /// Feed `samples` at `step` intervals starting at `t0`; return the index of
    /// the first observation that authorised a lock.
    fn run_gate(
        gate: &mut LockGate,
        cfg: &PresenceConfig,
        t0: Instant,
        step: Duration,
        samples: &[Sample],
    ) -> Option<usize> {
        let mut locked_at = None;
        for (i, s) in samples.iter().enumerate() {
            let now = t0 + step * (i as u32);
            if gate.observe(*s, cfg, now).lock && locked_at.is_none() {
                locked_at = Some(i);
            }
        }
        locked_at
    }

    #[test]
    fn one_far_sample_never_locks() {
        // The exact incident: near at -52, then a single -71 sample.
        let cfg = lock_cfg();
        let mut gate = LockGate::default();
        let t0 = Instant::now();
        let samples = [
            Sample::Seen(Some(-52)),
            Sample::Seen(Some(-71)),
            Sample::Seen(Some(-52)),
        ];
        assert_eq!(
            run_gate(&mut gate, &cfg, t0, Duration::from_secs(30), &samples),
            None
        );
    }

    #[test]
    fn one_missed_advertisement_never_locks() {
        // The second false lock: the scan cycle saw nothing and the old code
        // locked on the near -> absent transition.
        let cfg = lock_cfg();
        let mut gate = LockGate::default();
        let t0 = Instant::now();
        let samples = [
            Sample::Seen(Some(-52)),
            Sample::Miss,
            Sample::Seen(Some(-50)),
        ];
        assert_eq!(
            run_gate(&mut gate, &cfg, t0, Duration::from_secs(30), &samples),
            None
        );
    }

    #[test]
    fn absence_shorter_than_the_grace_window_is_not_far() {
        let cfg = lock_cfg();
        let mut gate = LockGate::default();
        let t0 = Instant::now();
        gate.observe(Sample::Seen(Some(-50)), &cfg, t0);
        // 30 s and 60 s of silence: still inside MISS_GRACE (75 s).
        let a = gate.observe(Sample::Miss, &cfg, t0 + Duration::from_secs(30));
        let b = gate.observe(Sample::Miss, &cfg, t0 + Duration::from_secs(60));
        assert_eq!(a.state, GateState::Near);
        assert_eq!(b.state, GateState::Near);
        assert_eq!(b.far_streak, 0);
        assert!(!a.lock && !b.lock);
    }

    #[test]
    fn three_far_samples_over_a_minute_lock() {
        let cfg = lock_cfg();
        let mut gate = LockGate::default();
        let t0 = Instant::now();
        let samples = [
            Sample::Seen(Some(-50)),
            Sample::Seen(Some(-78)),
            Sample::Seen(Some(-80)),
            Sample::Seen(Some(-79)),
        ];
        // 30 s cadence: the 3rd far sample lands at t0+90 s, i.e. 60 s after
        // the first far sample -> the window is satisfied exactly there.
        assert_eq!(
            run_gate(&mut gate, &cfg, t0, Duration::from_secs(30), &samples),
            Some(3)
        );
    }

    #[test]
    fn sustained_absence_locks_only_after_the_grace_window() {
        let cfg = lock_cfg();
        let mut gate = LockGate::default();
        let t0 = Instant::now();
        gate.observe(Sample::Seen(Some(-50)), &cfg, t0);
        // Misses at 30/60 s are advertising gaps (grace is 75 s). The first
        // counting miss is at 90 s (streak 1), then 120 s (2) and 150 s (3,
        // far for 135 s) — so a lock by absence alone needs 2.5 min, never a
        // single dropped advertisement.
        let mut locked = None;
        for i in 1..=6u32 {
            let now = t0 + Duration::from_secs(30 * i as u64);
            if gate.observe(Sample::Miss, &cfg, now).lock {
                locked = Some(30 * i);
                break;
            }
        }
        assert_eq!(locked, Some(150));
    }

    #[test]
    fn hysteresis_holds_far_until_ten_db_above_the_threshold() {
        let cfg = lock_cfg();
        let mut gate = LockGate::default();
        let t0 = Instant::now();
        gate.observe(Sample::Seen(Some(-80)), &cfg, t0);
        // -65 is inside the dead band (-70..-60): the state must stay far and
        // the streak must not be reset by a single noisy sample.
        let mid = gate.observe(Sample::Seen(Some(-65)), &cfg, t0 + Duration::from_secs(30));
        assert_eq!(mid.state, GateState::Far);
        assert_eq!(mid.far_streak, 1);
        // -60 is the release point (threshold + 10).
        let near = gate.observe(Sample::Seen(Some(-60)), &cfg, t0 + Duration::from_secs(60));
        assert_eq!(near.state, GateState::Near);
        assert_eq!(near.far_streak, 0);
    }

    #[test]
    fn cooldown_blocks_a_second_lock_within_two_minutes() {
        let cfg = lock_cfg();
        let mut gate = LockGate::default();
        let t0 = Instant::now();
        // Confirm a departure and lock.
        let first = [
            Sample::Seen(Some(-80)),
            Sample::Seen(Some(-80)),
            Sample::Seen(Some(-80)),
        ];
        assert_eq!(
            run_gate(&mut gate, &cfg, t0, Duration::from_secs(30), &first),
            Some(2)
        );
        // Device comes back, then leaves again 30 s later: the cooldown must
        // swallow the second lock.
        let t1 = t0 + Duration::from_secs(90);
        gate.observe(Sample::Seen(Some(-45)), &cfg, t1);
        let second = [
            Sample::Seen(Some(-80)),
            Sample::Seen(Some(-80)),
            Sample::Seen(Some(-80)),
        ];
        assert_eq!(
            run_gate(&mut gate, &cfg, t1, Duration::from_secs(30), &second),
            None
        );
    }

    #[test]
    fn lock_disabled_never_locks_even_when_confirmed() {
        let mut cfg = lock_cfg();
        cfg.lock_enabled = false;
        let mut gate = LockGate::default();
        let t0 = Instant::now();
        let samples = [
            Sample::Seen(Some(-90)),
            Sample::Seen(Some(-90)),
            Sample::Seen(Some(-90)),
            Sample::Seen(Some(-90)),
        ];
        assert_eq!(
            run_gate(&mut gate, &cfg, t0, Duration::from_secs(30), &samples),
            None
        );
        // The state is still reported truthfully.
        assert_eq!(gate.state(), GateState::Far);
    }

    #[test]
    fn advertisement_without_rssi_is_presence_not_departure() {
        let cfg = lock_cfg();
        let mut gate = LockGate::default();
        let t0 = Instant::now();
        gate.observe(Sample::Seen(Some(-80)), &cfg, t0);
        let out = gate.observe(Sample::Seen(None), &cfg, t0 + Duration::from_secs(30));
        assert_eq!(out.state, GateState::Near);
        assert_eq!(out.far_streak, 0);
        assert!(!out.lock);
    }

    #[test]
    fn rssi_threshold_classification() {
        let cfg = PresenceConfig::default(); // -70 dBm
                                             // "near" means signal is stronger (numerically higher) than threshold.
        assert!(-65 >= cfg.rssi_lock_dbm);
        // "far" means signal is weaker (numerically lower) than threshold.
        assert!(-80 < cfg.rssi_lock_dbm);
    }

    /// One discovery pass returning every peripheral's address and advertised
    /// name. Test-only helper.
    async fn scan_all_addresses(window: Duration) -> Vec<(String, Option<String>)> {
        use btleplug::api::{Central, Manager as _, Peripheral, ScanFilter};

        let mut out = Vec::new();
        let Ok(manager) = btleplug::platform::Manager::new().await else {
            return out;
        };
        let Ok(adapters) = manager.adapters().await else {
            return out;
        };
        for central in adapters {
            if central.start_scan(ScanFilter::default()).await.is_err() {
                continue;
            }
            tokio::time::sleep(window).await;
            if let Ok(peripherals) = central.peripherals().await {
                for p in peripherals {
                    if let Ok(Some(props)) = p.properties().await {
                        out.push((props.address.to_string(), props.local_name));
                    }
                }
            }
            let _ = central.stop_scan().await;
        }
        out
    }

    /// **Radio test — needs the phone next to the PC.** Not part of the normal
    /// suite:
    ///
    /// ```text
    /// cargo test --lib ble_presence_passive_scan -- --ignored --nocapture
    /// ```
    ///
    /// It does ONE scan pass and asserts that the configured phone is matched
    /// by canonical MAC — the exact comparison the audit found broken
    /// (`FC-43-45-81-4B-9A` stored vs `FC:43:45:81:4B:9A` reported). The
    /// address comes from `MICONTROL_BLE_TEST_MAC`, falling back to the
    /// registry `PhoneMac`, so the saved configuration is not modified.
    ///
    /// btleplug's Windows backend always probes as well as listens, so this is
    /// not strictly passive; it is a single, bounded pass with no loop.
    #[test]
    #[ignore = "radio: requires the phone nearby (run explicitly with --ignored)"]
    fn ble_presence_passive_scan_matches_canonical_mac() {
        let cfg = PresenceConfig::load();
        let raw = std::env::var("MICONTROL_BLE_TEST_MAC")
            .ok()
            .or(cfg.phone_mac.clone())
            .expect(
                "set MICONTROL_BLE_TEST_MAC or HKCU\\Software\\MiControl\\BlePresence\\PhoneMac",
            );
        let canonical =
            canonical_mac(&raw).expect("the configured MAC is not 12 hexadecimal digits");
        let target = MatchTarget {
            mac: Some(canonical.clone()),
            name: None,
        };

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("failed to build the test runtime");
        let seen = runtime.block_on(scan_all_addresses(Duration::from_secs(6)));

        println!("[ble_presence.test] looking for {canonical} ({raw})");
        println!("[ble_presence.test] {} peripheral(s) seen:", seen.len());
        for (addr, name) in &seen {
            println!("  {addr:<20} name={name:?}");
        }
        assert!(
            !seen.is_empty(),
            "no BLE advertisements at all — is the Bluetooth radio on?"
        );

        let hit = seen
            .iter()
            .find(|(addr, _)| canonical_mac(addr).as_deref() == Some(canonical.as_str()))
            .unwrap_or_else(|| {
                panic!(
                    "phone {canonical} was not seen in one pass. If it appears above under a \
                     different address it uses a randomised BLE address: configure PhoneName and \
                     the monitor will fall back to the name"
                )
            });
        assert_eq!(
            target.matches(&hit.0, hit.1.as_deref()),
            Some(MatchKind::Mac),
            "the canonical comparison must match the address btleplug prints"
        );
        println!("[ble_presence.test] matched {} ({:?})", hit.0, hit.1);
    }

    /// Recheck 2026-11-14: the sampler wrote one INFO per 30 s scan — 6 410
    /// lines in 53 h, 66 % of the app log — carrying nothing but counters. A
    /// transition is still news, and a 5-minute heartbeat keeps the line
    /// available to the field-test procedure documented in the skill.
    #[test]
    fn steady_state_samples_stop_flooding_info() {
        assert_eq!(sample_log_level(true, 3), log::Level::Info);
        assert_eq!(sample_log_level(false, 1), log::Level::Debug);
        assert_eq!(sample_log_level(false, 9), log::Level::Debug);
        assert_eq!(sample_log_level(false, SAMPLE_INFO_EVERY), log::Level::Info);
        assert_eq!(
            sample_log_level(false, SAMPLE_INFO_EVERY * 3),
            log::Level::Info
        );
        // One transition among 100 steady scans produces exactly two INFO
        // lines (the transition itself is counted separately).
        let infos = (1..=100u64)
            .filter(|i| sample_log_level(false, *i) == log::Level::Info)
            .count();
        assert_eq!(infos, 10, "one heartbeat per 10 scans");
    }

    #[test]
    fn the_state_is_remembered_so_a_transition_is_never_missed() {
        // First scan of a session is news.
        assert_eq!(sample_level_for("near"), log::Level::Info);
        // The next one is not.
        assert_eq!(sample_level_for("near"), log::Level::Debug);
        // A transition back to far is news again, even off-beat.
        assert_eq!(sample_level_for("far"), log::Level::Info);
    }
}
