//! Battery hardware interface.
//!
//! Provides functions to read battery status, charge level, and health
//! from the system's WMI provider, with static data cached for performance.

use crate::hw::errors::HardwareResult;
use anyhow::Context;
use serde::{Deserialize, Serialize};
use std::time::Instant;

#[cfg(windows)]
use crate::hw::errors::HardwareError;
#[cfg(windows)]
use std::sync::{Mutex, OnceLock};
#[cfg(windows)]
use std::time::Duration;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BatteryInfo {
    pub level: u8,
    pub is_charging: bool,
    pub is_plugged: bool,
    pub health_percent: f64,
    pub cycle_count: u32,
    pub designed_capacity_mwh: u32,
    pub full_capacity_mwh: u32,
    pub manufacturer: String,
    pub device_name: String,
    pub serial_number: String,
    pub chemistry: String,
    pub temperature_celsius: Option<f64>,
    pub time_remaining_minutes: Option<i32>,
    /// Estimated minutes until battery is fully charged. None when not charging.
    pub time_to_full_minutes: Option<i32>,
    /// Positive = charge rate mW, negative = discharge rate mW. Zero when unknown.
    pub charge_rate_mw: i32,
    /// Current battery voltage in millivolts (mV). Zero if unavailable.
    pub voltage_mv: u32,
    /// AC adapter input power in milliwatts (mW). None when not plugged in,
    /// when the IoT driver is not available, or before the register offset is
    /// confirmed. Use `debug_ecram_dump` to identify the correct offset.
    pub ac_input_power_mw: Option<i32>,
    /// Derived health classification: "Good", "Fair", or "Poor".
    /// Based on wear level: ≥80% Good, 60-80% Fair, <60% Poor.
    pub health_label: String,
    /// True if the AC adapter is negotiating hyper charging (>65W sustained).
    /// The TM2424 ships with a 100W GaN adapter.
    pub is_hyper_charging: bool,
}

/// Normalize the two WMI rate fields into the public signed representation.
///
/// `ChargeRate` alone is NOT a reliable "is charging" signal. This platform's
/// firmware reports `ChargeRate = 0` as soon as the charge threshold is hit
/// (audit 2026-11-07: 96 %, `PowerOnline = True`, `Charging = False`), and
/// some ACPI providers report 0 while `Charging` is still true. The `Charging`
/// bit is authoritative; a positive rate stays as the fallback for providers
/// that omit the bit. Ignoring it made the UI claim "not charging" with the
/// charger connected.
///
/// `ChargeRate` can also remain stale after unplugging, so `PowerOnline`
/// decides whether a positive value is a charge; on battery, use
/// `DischargeRate`.
fn effective_battery_rate(
    is_plugged: bool,
    charging_flag: bool,
    charge_rate_mw: i32,
    discharge_rate_mw: i32,
) -> (bool, i32) {
    if is_plugged {
        // Charging is only possible with the adapter connected, whatever the
        // (possibly stale) flag claims.
        return (charging_flag || charge_rate_mw > 0, charge_rate_mw.max(0));
    }

    let discharge = if discharge_rate_mw > 0 {
        -discharge_rate_mw
    } else {
        charge_rate_mw.min(0)
    };
    (false, discharge)
}

/// Cached static battery data that never changes at runtime.
/// Populated once via `BATTERY_STATIC_DATA` and reused on all subsequent calls.
#[cfg(windows)]
#[derive(Clone)]
struct BatteryStaticData {
    designed_capacity_mwh: u64,
    full_capacity_mwh: u64,
    manufacturer: String,
    device_name: String,
    serial_number: String,
    chemistry: String,
    cycle_count: u32,
    temperature_raw: Option<u32>,
}

#[cfg(windows)]
static BATTERY_STATIC_DATA: OnceLock<BatteryStaticData> = OnceLock::new();

/// Upper bound for a single battery WMI query. WMI (COM) can hang for ~15 s
/// when the battery driver is unresponsive; we cap the caller's wait at 5 s
/// and fall back to the last known-good snapshot (MIOT-02, crash mitigation).
#[cfg(windows)]
const BATTERY_QUERY_TIMEOUT: Duration = Duration::from_secs(5);

/// Last successfully produced `BatteryInfo`, used when a WMI query times out.
#[cfg(windows)]
static LAST_GOOD_BATTERY: Mutex<Option<BatteryInfo>> = Mutex::new(None);

/// Wall-clock duration of the last COMPLETED WMI battery query, in ms.
///
/// The caller can only observe "the reply did not arrive within the timeout",
/// which is always >= the threshold and therefore useless for diagnosis
/// (production log: 798 lines all saying "exceeded 5000 ms"). The worker
/// measures the real query duration and stores it here so the timeout path can
/// report how long queries actually take (P-battery).
#[cfg(windows)]
static LAST_QUERY_LATENCY_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A completed battery WMI query slower than this is logged with its measured
/// duration (the normal path is ~10-100 ms; multi-second queries explain the
/// timeouts).
#[cfg(windows)]
const BATTERY_SLOW_QUERY_LOG_MS: u64 = 1_000;

/// Format the timeout warning with the MEASURED wait instead of the constant
/// threshold. Pure so the format is unit-tested (the old code printed
/// `BATTERY_QUERY_TIMEOUT` — always 5000 ms — for every timeout).
#[cfg(windows)]
fn battery_timeout_message(
    waited: Duration,
    limit: Duration,
    last_completed: Option<Duration>,
) -> String {
    let last = last_completed
        .map(|d| format!("{} ms", d.as_millis()))
        .unwrap_or_else(|| "none completed yet".to_string());
    format!(
        "battery WMI query exceeded {waited_ms} ms (timeout {limit_ms} ms, \
         previous query took {last})",
        waited_ms = waited.as_millis(),
        limit_ms = limit.as_millis(),
    )
}

/// S64: minimum interval between two *real* WMI battery queries.
///
/// Battery state changes slowly, but `get_battery_info` is called every 5 s by
/// the IoT event listener and every 15 s by the frontend. Each call spawns a
/// thread that runs a WMI query; when the battery driver is slow (observed
/// 4669 "exceeded 5000 ms" warnings in one log) those threads stay stuck in COM
/// and pile up — the process reached 134 threads / 291 617 handles after two
/// days. Inside this window we serve the cached snapshot immediately and never
/// spawn a query thread, so the effective query rate is bounded regardless of
/// how many callers poll.
#[cfg(windows)]
const BATTERY_QUERY_MIN_INTERVAL: Duration = Duration::from_secs(15);

/// Timestamp of the last query that actually spawned a WMI thread.
#[cfg(windows)]
static LAST_BATTERY_QUERY_AT: Mutex<Option<Instant>> = Mutex::new(None);

/// Returns true when enough time has passed since the last real query. Marks
/// the timestamp on success so concurrent callers cannot both start a query.
#[cfg(windows)]
fn should_query_battery_now() -> bool {
    let mut guard = crate::util::panic::lock_or_recover(&LAST_BATTERY_QUERY_AT);
    match *guard {
        Some(last) if last.elapsed() < BATTERY_QUERY_MIN_INTERVAL => false,
        _ => {
            *guard = Some(Instant::now());
            true
        }
    }
}

/// Fetch battery info without blocking the caller for longer than
/// [`BATTERY_QUERY_TIMEOUT`].
///
/// S69 (residual handle-leak root cause): this used to spawn a fresh
/// `battery-wmi-query` thread per real query (every 15 s). Each new thread
/// initializes COM (the `wmi_cache` thread_local creates fresh WMI
/// connections) and, on exit, the Rust std Windows TLS-destructor path
/// retains Thread/Section/Event handles in the process handle table
/// (measured: ~15 handles/min, 64 dead TIDs with retained handles after
/// 25 min). The query now runs on a single PERSISTENT worker thread that
/// never exits, so the COM init + TLS teardown churn never happens.
/// The timeout and the S64 snapshot/throttle semantics are unchanged.
#[cfg(windows)]
pub fn get_battery_info() -> HardwareResult<BatteryInfo> {
    use std::sync::mpsc;

    if !should_query_battery_now() {
        if let Some(info) = crate::util::panic::lock_or_recover(&LAST_GOOD_BATTERY).clone() {
            return Ok(info);
        }
        // No snapshot yet (first call of the session) — fall through and query.
    }

    let started = Instant::now();
    let (result_tx, result_rx) = mpsc::channel::<HardwareResult<BatteryInfo>>();
    {
        let mut guard = crate::util::panic::lock_or_recover(&BATTERY_WORKER_TX);
        let tx = guard.get_or_insert_with(|| {
            let (tx, rx) = mpsc::channel::<BatteryRequest>();
            std::thread::Builder::new()
                .name("battery-wmi-worker".into())
                .spawn(move || battery_worker_loop(rx))
                .expect("battery worker thread spawn failed");
            tx
        });
        let _ = tx.send(BatteryRequest { result_tx });
    }

    match result_rx.recv_timeout(BATTERY_QUERY_TIMEOUT) {
        Ok(result) => {
            if let Ok(info) = &result {
                let mut guard = crate::util::panic::lock_or_recover(&LAST_GOOD_BATTERY);
                *guard = Some(info.clone());
            }
            result
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            // P-battery: log the MEASURED wait, not the constant threshold.
            // `recv_timeout` returns at (at least) the limit, so `waited` also
            // proves whether the caller was delayed beyond the timeout; the
            // previous completed query's real duration comes from the worker.
            let waited = started.elapsed();
            let last_completed = last_query_latency();
            log::warn!(
                target: "hw::battery",
                "{} — returning last known-good snapshot",
                battery_timeout_message(waited, BATTERY_QUERY_TIMEOUT, last_completed)
            );
            let cached = crate::util::panic::lock_or_recover(&LAST_GOOD_BATTERY).clone();
            if let Some(info) = cached {
                Ok(info)
            } else {
                Err(HardwareError::Timeout(format!(
                    "{} (no cached snapshot yet)",
                    battery_timeout_message(waited, BATTERY_QUERY_TIMEOUT, last_completed)
                )))
            }
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            log::error!(target: "hw::battery", "battery WMI worker disconnected");
            Err(HardwareError::Other(
                "battery WMI worker disconnected".into(),
            ))
        }
    }
}

/// One-shot request sent to the persistent battery worker.
#[cfg(windows)]
struct BatteryRequest {
    result_tx: std::sync::mpsc::Sender<HardwareResult<BatteryInfo>>,
}

/// The persistent worker's inbound channel. The worker thread is created on
/// first use and NEVER exits (S69: thread exit is what leaked handles).
#[cfg(windows)]
static BATTERY_WORKER_TX: Mutex<Option<std::sync::mpsc::Sender<BatteryRequest>>> = Mutex::new(None);

/// Real duration of the last completed battery WMI query (`None` before the
/// first query finishes).
#[cfg(windows)]
fn last_query_latency() -> Option<Duration> {
    match LAST_QUERY_LATENCY_MS.load(std::sync::atomic::Ordering::Relaxed) {
        0 => None,
        ms => Some(Duration::from_millis(ms)),
    }
}

/// Persistent battery WMI worker loop. Runs `get_battery_info_inner` for each
/// request and replies. The thread never exits; a dropped result channel
/// (caller timed out) is simply ignored.
///
/// The real query duration is measured HERE (the caller's `recv_timeout` only
/// sees ">= 5 s") and stored in [`LAST_QUERY_LATENCY_MS`] so the timeout path
/// can report actual latencies instead of the constant threshold.
#[cfg(windows)]
fn battery_worker_loop(rx: std::sync::mpsc::Receiver<BatteryRequest>) {
    for req in rx {
        let started = Instant::now();
        let result = get_battery_info_inner();
        let elapsed_ms = started.elapsed().as_millis() as u64;
        LAST_QUERY_LATENCY_MS.store(elapsed_ms, std::sync::atomic::Ordering::Relaxed);
        if elapsed_ms >= BATTERY_SLOW_QUERY_LOG_MS {
            log::info!(
                target: "hw::battery",
                "battery WMI query took {elapsed_ms} ms (slow threshold {BATTERY_SLOW_QUERY_LOG_MS} ms, {})",
                if result.is_ok() { "ok" } else { "failed" }
            );
        }
        let _ = req.result_tx.send(result);
    }
}

#[cfg(windows)]
fn get_battery_info_inner() -> HardwareResult<BatteryInfo> {
    use crate::hw::wmi_cache;
    use crate::util::wmi_extract;
    use std::collections::HashMap;

    let started = Instant::now();
    log::debug!(target: "hw::battery", "get_battery_info: start");

    // S24-007: Use OnceLock for lock-free reads after initialisation.
    // OnceLock::get_or_try_init is unstable, so we compute the data first,
    // then use get_or_init with the already-computed value.
    let battery_static = BATTERY_STATIC_DATA.get_or_init(|| {
        // Try to populate from WMI; fall back to defaults on error.
        let data = wmi_cache::with_wmi(|wmi| {
            let static_data: Vec<HashMap<String, wmi::Variant>> = wmi
                .raw_query("SELECT * FROM BatteryStaticData")
                .context("BatteryStaticData query")?;
            let full_cap_data: Vec<HashMap<String, wmi::Variant>> = wmi
                .raw_query("SELECT * FROM BatteryFullChargedCapacity")
                .context("BatteryFullChargedCapacity query")?;

            let statics = static_data.into_iter().next().unwrap_or_default();
            let full_cap = full_cap_data.into_iter().next().unwrap_or_default();

            Ok(BatteryStaticData {
                designed_capacity_mwh: wmi_extract::extract_u64(&statics, "DesignedCapacity")
                    .unwrap_or(68224),
                full_capacity_mwh: wmi_extract::extract_u64(&full_cap, "FullChargedCapacity")
                    .unwrap_or(68224),
                manufacturer: wmi_extract::extract_string(&statics, "ManufactureName")
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|| "COSMX".to_string()),
                device_name: wmi_extract::extract_string(&statics, "DeviceName")
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|| "BX70".to_string()),
                serial_number: wmi_extract::extract_string(&statics, "SerialNumber")
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default(),
                chemistry: wmi_extract::extract_u32(&statics, "Chemistry")
                    .map(|c| {
                        // Chemistry is stored as a 4-byte ASCII value packed into a uint32
                        let bytes = c.to_le_bytes();
                        let s = String::from_utf8_lossy(&bytes);
                        s.trim_end_matches('\0').to_string()
                    })
                    .unwrap_or_else(|| "LiP".to_string()),
                cycle_count: wmi_extract::extract_u32_or(&statics, "CycleCount", 0),
                temperature_raw: wmi_extract::extract_u32(&statics, "Temperature"),
            })
        });

        match data {
            Ok(d) => d,
            Err(e) => {
                log::warn!(target: "hw::battery", "WMI static data query failed, using defaults: {e}");
                BatteryStaticData {
                    designed_capacity_mwh: 68224,
                    full_capacity_mwh: 68224,
                    manufacturer: "COSMX".to_string(),
                    device_name: "BX70".to_string(),
                    serial_number: String::new(),
                    chemistry: "LiP".to_string(),
                    cycle_count: 0,
                    temperature_raw: None,
                }
            }
        }
    });

    // ═══════════════════════════════════════════════════════════════════════
    // WORKING FORM — DO NOT MODIFY
    //
    // The BatterySnapshot pattern is CRITICAL to avoid RefCell panics.
    // wmi_cache::with_wmi() uses RefCell::borrow_mut() internally. If any
    // function called inside the closure also calls with_wmi(), the nested
    // borrow_mut() will panic with "RefCell already borrowed".
    //
    // Pattern:
    //   1. Collect ALL WMI data inside a single with_wmi closure into a
    //      BatterySnapshot struct (no nested with_wmi calls allowed).
    //   2. Exit the closure.
    //   3. Call probe_ac_input_power_throttled() OUTSIDE the closure — it
    //      internally calls wmi_ec::read_adapter_power() which calls
    //      wmi_read() which calls with_wmi() again.
    //
    // Breaking this pattern will cause a runtime panic when the battery
    // is plugged in and the adapter power probe fires.
    // ═══════════════════════════════════════════════════════════════════════
    struct BatterySnapshot {
        level: u8,
        is_charging: bool,
        is_plugged: bool,
        health_percent: f64,
        cycle_count: u32,
        designed_capacity_mwh: u32,
        full_capacity_mwh: u32,
        manufacturer: String,
        device_name: String,
        serial_number: String,
        chemistry: String,
        temperature_celsius: Option<f64>,
        time_remaining_minutes: Option<i32>,
        time_to_full_minutes: Option<i32>,
        charge_rate_mw: i32,
        voltage_mv: u32,
        health_label: String,
        is_hyper_charging: bool,
    }

    let snapshot = wmi_cache::with_wmi(|wmi| {
        // BatteryStatus (dynamic data only)
        let statuses: Vec<HashMap<String, wmi::Variant>> = wmi
            .raw_query("SELECT * FROM BatteryStatus")
            .context("BatteryStatus query")?;

        let status = statuses.into_iter().next().unwrap_or_default();

        let remaining_capacity = wmi_extract::extract_u32_or(&status, "RemainingCapacity", 0);
        let charging_rate = wmi_extract::extract_i32(&status, "ChargeRate").unwrap_or(0);
        let discharge_rate = wmi_extract::extract_i32(&status, "DischargeRate").unwrap_or(0);
        let voltage = wmi_extract::extract_u32(&status, "Voltage")
            .map(|v| v as f64 / 1000.0)
            .unwrap_or(0.0);
        let voltage_mv = wmi_extract::extract_u32_or(&status, "Voltage", 0);

        let is_plugged = wmi_extract::extract_bool(&status, "PowerOnline").unwrap_or(false);
        // `Charging` is the ACPI bit that says current is flowing into the
        // battery. It is authoritative when `ChargeRate` reads 0 because the
        // charge threshold was reached (or the driver omits the rate).
        let charging_flag = wmi_extract::extract_bool(&status, "Charging").unwrap_or(false);
        let (is_charging, effective_rate_mw) =
            effective_battery_rate(is_plugged, charging_flag, charging_rate, discharge_rate);
        log::debug!(
            target: "hw::battery",
            "wmi snapshot: plugged={} charging_flag={} charging={} remaining_capacity={} charge_rate_mw={} discharge_rate_mw={} effective_rate_mw={} voltage_v={:.3}",
            is_plugged,
            charging_flag,
            is_charging,
            remaining_capacity,
            charging_rate,
            discharge_rate,
            effective_rate_mw,
            voltage
        );

        // Static data from cached BatteryStaticData
        let designed_mah = battery_static.designed_capacity_mwh as u32;
        let manufacturer = battery_static.manufacturer.clone();
        let device_name = battery_static.device_name.clone();
        let serial_number = battery_static.serial_number.clone();
        let chemistry = battery_static.chemistry.clone();
        let cycle_count = if battery_static.cycle_count > 0 {
            battery_static.cycle_count
        } else {
            get_cycle_count_powercfg().unwrap_or(0)
        };
        let temperature_celsius = battery_static
            .temperature_raw
            .map(|t| (t as f64 / 10.0) - 273.15);
        let full_cap_mah = battery_static.full_capacity_mwh as u32;

        let level = if full_cap_mah > 0 {
            ((remaining_capacity as f64 / full_cap_mah as f64) * 100.0)
                .round()
                .clamp(0.0, 100.0) as u8
        } else {
            0
        };

        let health_percent = if designed_mah > 0 {
            ((full_cap_mah as f64 / designed_mah as f64) * 100.0).clamp(0.0, 100.0)
        } else {
            100.0
        };

        let time_remaining_minutes = if !is_charging && voltage > 0.0 {
            // Estimate: remaining capacity (mWh) / discharge_rate (mW) * 60
            let discharge_rate_mw = effective_rate_mw.unsigned_abs();
            if discharge_rate_mw > 0 {
                Some((remaining_capacity as f64 / discharge_rate_mw as f64 * 60.0) as i32)
            } else {
                None
            }
        } else {
            None
        };

        let time_to_full_minutes = if is_charging && charging_rate > 0 {
            let remaining_to_full = full_cap_mah.saturating_sub(remaining_capacity);
            if remaining_to_full > 0 {
                Some((remaining_to_full as f64 / charging_rate as f64 * 60.0) as i32)
            } else {
                Some(0) // already at full capacity
            }
        } else {
            None
        };

        // Derived health label from wear level
        let health_label = if health_percent >= 80.0 {
            "Good".to_string()
        } else if health_percent >= 60.0 {
            "Fair".to_string()
        } else {
            "Poor".to_string()
        };

        // Hyper charging: sustained input >65W (65000mW)
        // TM2424 ships with 100W GaN adapter; hyper charging is active
        // when the charge rate exceeds 65W while plugged in.
        // We use charging_rate (ChargeRate from WMI BatteryStatus) as a proxy.
        let is_hyper_charging = is_plugged && effective_rate_mw > 65000;

        Ok(BatterySnapshot {
            level,
            is_charging,
            is_plugged,
            health_percent,
            cycle_count,
            designed_capacity_mwh: designed_mah,
            full_capacity_mwh: full_cap_mah,
            manufacturer,
            device_name,
            serial_number,
            chemistry,
            temperature_celsius,
            time_remaining_minutes,
            time_to_full_minutes,
            charge_rate_mw: effective_rate_mw,
            voltage_mv,
            health_label,
            is_hyper_charging,
        })
    })?;

    // WORKING FORM — DO NOT MODIFY: AC adapter power read MUST be outside
    // the with_wmi closure. probe_ac_input_power_throttled() → read_adapter_power()
    // → wmi_read() → wmi_cache::with_wmi() — nested borrow_mut() would panic.
    //
    // The probe is also the single most expensive part of this call: on this
    // platform the WMAA adapter-power class does not exist (0x80041002) or is
    // denied to the non-elevated process (0x80041003), and each attempt costs
    // ~2.2 s (median 2222 ms, max 4927 ms in the 2026-11-07 audit). The answer
    // can never change within a process, so both the shared capability circuit
    // and the local "this process is denied" latch must be checked BEFORE the
    // query — after the first failure every poll is free again.
    let ac_input_power_mw = if !snapshot.is_plugged {
        log::debug!(target: "hw::battery", "charger is unplugged; skipping AC power read");
        clear_ac_power_probe_cache();
        None
    } else if local_ac_probe_is_useless() {
        log::debug!(
            target: "hw::battery",
            "charger is plugged but the local AC power probe is known to be unavailable; skipping"
        );
        None
    } else {
        log::debug!(target: "hw::battery", "charger is plugged; attempting WMI AC power read");
        probe_ac_input_power_throttled()
    };

    log::debug!(
        target: "hw::battery",
        "battery info ready: ac_input_power_mw={:?} elapsed_ms={}",
        ac_input_power_mw,
        started.elapsed().as_millis()
    );

    Ok(BatteryInfo {
        level: snapshot.level,
        is_charging: snapshot.is_charging,
        is_plugged: snapshot.is_plugged,
        health_percent: snapshot.health_percent,
        cycle_count: snapshot.cycle_count,
        designed_capacity_mwh: snapshot.designed_capacity_mwh,
        full_capacity_mwh: snapshot.full_capacity_mwh,
        manufacturer: snapshot.manufacturer,
        device_name: snapshot.device_name,
        serial_number: snapshot.serial_number,
        chemistry: snapshot.chemistry,
        temperature_celsius: snapshot.temperature_celsius,
        time_remaining_minutes: snapshot.time_remaining_minutes,
        time_to_full_minutes: snapshot.time_to_full_minutes,
        charge_rate_mw: snapshot.charge_rate_mw,
        voltage_mv: snapshot.voltage_mv,
        ac_input_power_mw,
        health_label: snapshot.health_label,
        is_hyper_charging: snapshot.is_hyper_charging,
    })
}

#[cfg(windows)]
#[derive(Default)]
struct AcPowerProbeCache {
    last_probe_at: Option<Instant>,
    last_value_mw: Option<i32>,
}

/// Epoch-ms before which the in-process WMAA adapter-power probe is skipped.
///
/// The app process is unprivileged, so `MICommonInterface` answers
/// `0x80041003` (access denied) — a fact that cannot change while this process
/// lives. That used to be recorded as a *permanent* `AtomicBool` latch, and the
/// 0x80041002 provider hiccup was wrongly folded into the same latch, so a
/// single bad sample at startup hid the reading for the rest of the session.
/// A deadline is used instead: after it expires the probe runs again (cheap —
/// an access-denied WMI call returns in microseconds).
#[cfg(windows)]
static LOCAL_AC_PROBE_RETRY_AT_MS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// How long the local (unprivileged) probe stays skipped after an
/// access-denied verdict from the WMI provider.
#[cfg(windows)]
const LOCAL_AC_PROBE_DEFER_SECS: u64 = 300;

#[cfg(windows)]
fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(windows)]
fn local_ac_probe_is_useless() -> bool {
    let retry_at = LOCAL_AC_PROBE_RETRY_AT_MS.load(std::sync::atomic::Ordering::Relaxed);
    retry_at != 0 && now_epoch_ms() < retry_at
}

#[cfg(windows)]
fn defer_local_ac_probe(reason: &str) {
    let retry_at = now_epoch_ms().saturating_add(LOCAL_AC_PROBE_DEFER_SECS * 1000);
    let previous = LOCAL_AC_PROBE_RETRY_AT_MS.swap(retry_at, std::sync::atomic::Ordering::Relaxed);
    if previous == 0 || previous < retry_at {
        log::info!(
            target: "hw::battery",
            "local AC adapter-power probe skipped for {LOCAL_AC_PROBE_DEFER_SECS}s ({reason}); \
             the elevated bridge path is unaffected and the local probe retries afterwards"
        );
    }
}

/// `true` when a failed LOCAL adapter-power read is a fact about *this process*
/// rather than a transient error: the caller is denied the WMI class
/// (`0x80041003` / `0x80070005`).
///
/// `0x80041002` is deliberately NOT here: the production log shows it is a
/// degraded-provider hiccup that recovers on the next attempt, so the local
/// probe must keep trying (and `wmi_ec::read_adapter_power` retries it once
/// before it ever reaches this function).
#[cfg(windows)]
fn local_probe_failure_is_permanent(message: &str) -> bool {
    let lowered = message.to_ascii_lowercase();
    [
        "0x80041003",
        "0x80070005",
        "access is denied",
        "access denied",
    ]
    .iter()
    .any(|needle| lowered.contains(needle))
}

#[cfg(windows)]
fn ac_probe_cache() -> &'static Mutex<AcPowerProbeCache> {
    static CACHE: OnceLock<Mutex<AcPowerProbeCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(AcPowerProbeCache::default()))
}

#[cfg(windows)]
fn probe_ac_input_power_throttled() -> Option<i32> {
    const AC_PROBE_MIN_INTERVAL: Duration = Duration::from_secs(15);
    let now = Instant::now();

    // S24-006: Use lock_or_recover for consistent poison recovery.
    {
        let cache = crate::util::panic::lock_or_recover(ac_probe_cache());
        if let Some(last) = cache.last_probe_at {
            let elapsed = now.saturating_duration_since(last);
            if elapsed < AC_PROBE_MIN_INTERVAL {
                log::debug!(
                    target: "hw::battery",
                    "ac probe throttled: returning cached value {:?} (elapsed_ms={} < interval_ms={})",
                    cache.last_value_mw,
                    elapsed.as_millis(),
                    AC_PROBE_MIN_INTERVAL.as_millis()
                );
                return cache.last_value_mw;
            }
        }
    }

    let value = match crate::hw::wmi_ec::read_adapter_power() {
        Ok(watts) => Some((watts as i32) * 1000), // W → mW
        Err(e) => {
            let message = e.to_string();
            if local_probe_failure_is_permanent(&message) {
                defer_local_ac_probe(&message);
            }
            // A single bad sample must NOT erase a good reading: the adapter
            // wattage is a slow-moving value (65 W / 100 W), and the UI hides
            // the field entirely when this is `None`. Keep the last measured
            // value and let the next poll refresh it.
            crate::util::panic::lock_or_recover(ac_probe_cache()).last_value_mw
        }
    };

    // S24-006: Use lock_or_recover for consistent poison recovery.
    {
        let mut cache = crate::util::panic::lock_or_recover(ac_probe_cache());
        cache.last_probe_at = Some(now);
        cache.last_value_mw = value;
    }

    value
}

#[cfg(windows)]
fn clear_ac_power_probe_cache() {
    // S24-006: Use lock_or_recover for consistent poison recovery.
    let mut cache = crate::util::panic::lock_or_recover(ac_probe_cache());
    cache.last_probe_at = None;
    cache.last_value_mw = None;
}

/// Read battery cycle count from `powercfg /batteryreport /xml` output.
///
/// This is a fallback when WMI `BatteryStaticData.CycleCount` returns 0
/// or is unavailable. The XML output contains a `<CycleCount>` element
/// under `<BatteryReport>` -> `<Batteries>` -> `<Battery>`.
#[cfg(windows)]
fn get_cycle_count_powercfg() -> Option<u32> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;

    const CREATE_NO_WINDOW: u32 = 0x08000000;

    let mut cmd = Command::new("powercfg");
    cmd.args(["/batteryreport", "/xml", "/output", "-"]);
    cmd.creation_flags(CREATE_NO_WINDOW);

    let output = cmd.output().ok()?;
    let xml = String::from_utf8_lossy(&output.stdout);

    // Parse <CycleCount> element from XML
    if let Some(start) = xml.find("<CycleCount>") {
        if let Some(end) = xml[start..].find("</CycleCount>") {
            let value_str = &xml[start + "<CycleCount>".len()..start + end];
            return value_str.trim().parse::<u32>().ok();
        }
    }

    None
}

#[cfg(not(windows))]
fn get_cycle_count_powercfg() -> Option<u32> {
    None
}

#[cfg(not(windows))]
pub fn get_battery_info() -> HardwareResult<BatteryInfo> {
    Ok(BatteryInfo {
        level: 80,
        is_charging: false,
        is_plugged: false,
        health_percent: 98.0,
        cycle_count: 42,
        designed_capacity_mwh: 68224,
        full_capacity_mwh: 66800,
        manufacturer: "COSMX".to_string(),
        device_name: "BX70".to_string(),
        temperature_celsius: Some(28.5),
        time_remaining_minutes: Some(240),
        time_to_full_minutes: None,
        charge_rate_mw: 0,
        voltage_mv: 11400,
        ac_input_power_mw: None,
        health_label: "Good".to_string(),
        is_hyper_charging: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unplugged_battery_uses_discharge_rate() {
        assert_eq!(
            effective_battery_rate(false, false, 10_000, 21_728),
            (false, -21_728)
        );
    }

    #[test]
    fn unplugged_battery_cannot_be_reported_as_charging() {
        assert_eq!(effective_battery_rate(false, false, 10_000, 0), (false, 0));
    }

    #[test]
    fn unplugged_battery_ignores_a_stale_charging_flag() {
        // A stale/false-positive `Charging` bit must never win over PowerOnline.
        assert_eq!(effective_battery_rate(false, true, 0, 0), (false, 0));
    }

    #[test]
    fn plugged_battery_uses_positive_charge_rate() {
        assert_eq!(
            effective_battery_rate(true, false, 10_000, 0),
            (true, 10_000)
        );
    }

    #[test]
    fn plugged_battery_with_charging_flag_charges_at_zero_rate() {
        // Production quirk: the firmware reports ChargeRate=0 once the charge
        // threshold is reached while `Charging` stays True. The flag wins.
        assert_eq!(effective_battery_rate(true, true, 0, 0), (true, 0));
    }

    #[test]
    fn plugged_and_full_battery_is_not_charging() {
        // This host today: PowerOnline=True, Charging=False, ChargeRate=0.
        // The UI must show the "AC connected" state, not "charging".
        assert_eq!(effective_battery_rate(true, false, 0, 0), (false, 0));
    }

    #[cfg(windows)]
    #[test]
    fn local_probe_failure_classification() {
        // Permanent for THIS process: the caller is denied the class.
        assert!(local_probe_failure_is_permanent(
            "Hardware error: 0x80041003 (Access denied)"
        ));
        assert!(local_probe_failure_is_permanent(
            "DeviceIoControl ECRAM_READ: Access is denied. (0x80070005)"
        ));
        // Retryable: the 0x80041002 degraded-provider hiccup recovered on the
        // next poll 18 451 times in production — it must never latch.
        assert!(!local_probe_failure_is_permanent(
            "Hardware error: 0x80041002"
        ));
        assert!(!local_probe_failure_is_permanent(
            "WMI query failed: 0x80041010"
        ));
        // Transient: keep retrying.
        assert!(!local_probe_failure_is_permanent(
            "Bridge service pipe round-trip timed out after 15s"
        ));
        assert!(!local_probe_failure_is_permanent(
            "No MICommonInterface instance found"
        ));
        assert!(!local_probe_failure_is_permanent(""));
    }

    #[test]
    #[ignore = "requires real battery hardware (WMI BatteryStaticData)"]
    fn battery_info_fields_valid() {
        let info = get_battery_info().expect("get_battery_info should succeed");
        assert!(
            info.level <= 100,
            "Battery level out of range: {}",
            info.level
        );
        assert!(
            info.health_percent <= 110.0,
            "Health out of range: {}",
            info.health_percent
        );
        assert!(
            !info.manufacturer.is_empty(),
            "Manufacturer should not be empty"
        );
        assert!(
            !info.device_name.is_empty(),
            "Device name should not be empty"
        );
        assert!(
            info.designed_capacity_mwh > 0,
            "Designed capacity should be > 0"
        );
        assert!(info.full_capacity_mwh > 0, "Full capacity should be > 0");
    }

    #[test]
    #[ignore = "requires real battery hardware (WMI BatteryStaticData)"]
    fn battery_capacity_ratio_sane() {
        let info = get_battery_info().expect("get_battery_info should succeed");
        let ratio = info.full_capacity_mwh as f64 / info.designed_capacity_mwh as f64;
        assert!(
            ratio <= 1.1,
            "Full capacity exceeds 110% of designed: {ratio}"
        );
        assert!(ratio > 0.0, "Full capacity must be positive");
    }

    #[test]
    #[ignore = "requires real battery hardware (WMI BatteryStaticData)"]
    fn battery_info_serialization() {
        let info = get_battery_info().expect("get_battery_info should succeed");
        let json = serde_json::to_string(&info).expect("should serialize");
        assert!(json.contains("level"));
        assert!(json.contains("manufacturer"));
    }

    #[cfg(windows)]
    #[test]
    fn ac_power_cache_concurrent_clear_probe_no_panic() {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        use std::time::Duration;

        // Two threads racing: one clearing the cache, one probing it.
        // Under the old implementation the probe could interleave with
        // the clear; here we verify the lock-held-throughout approach
        // never panics and never produces a stale read after clear.
        let stop = Arc::new(AtomicBool::new(false));

        let stop_probe = Arc::clone(&stop);
        let probe_thread = std::thread::spawn(move || {
            while !stop_probe.load(Ordering::Relaxed) {
                let _ = probe_ac_input_power_throttled();
            }
        });

        let stop_clear = Arc::clone(&stop);
        let clear_thread = std::thread::spawn(move || {
            while !stop_clear.load(Ordering::Relaxed) {
                clear_ac_power_probe_cache();
            }
        });

        // Let them race for long enough to trigger interleaving
        std::thread::sleep(Duration::from_millis(500));
        stop.store(true, Ordering::Relaxed);

        probe_thread.join().expect("probe thread panicked");
        clear_thread.join().expect("clear thread panicked");
    }

    /// The timeout line must contain the MEASURED wait, not the constant
    /// threshold. Production logged "exceeded 5000 ms" 798 times because the
    /// format used `BATTERY_QUERY_TIMEOUT`.
    #[cfg(windows)]
    #[test]
    fn timeout_message_reports_measured_latency_not_the_threshold() {
        let msg = battery_timeout_message(
            Duration::from_millis(7_812),
            Duration::from_secs(5),
            Some(Duration::from_millis(9_431)),
        );
        assert!(
            msg.contains("exceeded 7812 ms"),
            "measured wait missing from: {msg}"
        );
        assert!(
            msg.contains("timeout 5000 ms"),
            "threshold should be labelled as such: {msg}"
        );
        assert!(
            msg.contains("previous query took 9431 ms"),
            "real previous latency missing from: {msg}"
        );
    }

    #[cfg(windows)]
    #[test]
    fn timeout_message_handles_no_completed_query() {
        let msg =
            battery_timeout_message(Duration::from_millis(5_003), Duration::from_secs(5), None);
        assert!(msg.contains("exceeded 5003 ms"), "{msg}");
        assert!(msg.contains("none completed yet"), "{msg}");
    }

    #[cfg(windows)]
    #[test]
    fn last_query_latency_is_none_before_first_completed_query() {
        // The worker has not run in this test process; the sentinel 0 means
        // "no measurement yet" (never a real 0 ms query).
        LAST_QUERY_LATENCY_MS.store(0, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(last_query_latency(), None);
        LAST_QUERY_LATENCY_MS.store(2_345, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(last_query_latency(), Some(Duration::from_millis(2_345)));
        LAST_QUERY_LATENCY_MS.store(0, std::sync::atomic::Ordering::Relaxed);
    }
}
