//! Global task manager backend - per-process resource accounting
//! (CPU / GPU / NPU / RAM / network) plus process termination.
//!
//! This module powers the "Task Manager" tab that mirrors Windows Task
//! Manager: every process with per-PID GPU and NPU breakdown, network
//! throughput per interface, and a safe kill-process command.
//!
//! # Data sources
//!
//! - **CPU% / RAM**: WMI `Win32_PerfFormattedData_PerfProc_Process`
//!   (same cookbook as `hw::processes`), normalized to 0–100.
//! - **GPU% / NPU% per PID**: PDH `\GPU Engine(*)\Utilization Percentage`
//!   (via the shared engine `util::pdh`). Engines are named
//!   `pid_<Pid>_luid_..._engtype_<Type>`. GPU = sum of `engtype_3d`
//!   instances; NPU = sum of `engtype_neural` instances.
//! - **Network**: PDH `\Network Interface(*)\Bytes Total/sec` for total
//!   interface throughput (the classic TCP/IP performance counters).
//!
//! The `ProcessTaskInfo` struct returned by `get_task_manager()` is the
//! joined view used by the frontend table.

use crate::hw::errors::HardwareResult;
use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock};

/// Minimum recognized PID for the GPU/NPU engine map. Windows starts real
/// PIDs well above this; filtering out `pid_0`/partial strings keeps the map
/// clean.
const MIN_PID: i64 = 4;

/// How long the cooked process table is reused across `get_task_manager`
/// calls. The task-manager tab polls every 2.5 s and the WMI formatted
/// process query costs hundreds of milliseconds at several hundred
/// processes, so the cooked rows are cached slightly beyond that cadence:
/// every UI poll but one pays a clone instead of a query. The per-process
/// CPU/RAM/thread columns then refresh at the same 1.5-3 s cadence as the
/// GPU/NPU poller snapshots, and the engine join still uses the latest
/// snapshot. The cache lock is never held across the WMI query itself, so
/// nothing serializes the hot path.
const PROCESS_TABLE_TTL: std::time::Duration = std::time::Duration::from_millis(3000);

type CookedProcessTable = (std::time::Instant, Vec<ProcessTaskInfo>);

static PROCESS_TABLE_CACHE: OnceLock<Mutex<Option<CookedProcessTable>>> = OnceLock::new();

// ── Per-process GPU + NPU engine accounting ─────────────────────────────────
//
// The `\GPU Engine(*)\Utilization Percentage` counter has one instance per
// (process, engine type, adapter). We read it through the shared PDH engine
// (`util::pdh`) and aggregate per PID:
//   - GPU usage  → sum of `engtype_3d`
//   - NPU usage  → sum of `engtype_neural`
//
// Since the counter is relative ("% of a single engine over the last
// interval"), the per-process percentages are consistent and comparable to
// Task Manager's GPU column.

static PID_ENGINE_CACHE: OnceLock<Mutex<Option<PidEngineSnapshot>>> = OnceLock::new();
static ENGINE_POLLER_STARTED: OnceLock<()> = OnceLock::new();

#[derive(Default, Clone)]
pub struct PidEngineSnapshot {
    /// pid → (gpu pct, npu pct)
    pub gpu: std::collections::HashMap<u32, (f64, f64)>,
}

fn ensure_engine_poller() {
    PID_ENGINE_CACHE.get_or_init(|| Mutex::new(None));
    ENGINE_POLLER_STARTED.get_or_init(|| {
        std::thread::Builder::new()
            .name("pid-engine-pdh-poller".into())
            .spawn(pid_engine_pdh_poller_thread)
            .ok();
    });
}

fn pid_engine_pdh_poller_thread() {
    #[cfg(windows)]
    {
        use crate::util::pdh::{self, CounterPath};

        // Single PDH engine (util::pdh): one query owned by this poller
        // thread, no global lock, RAII close on every exit path.
        let Ok(mut query) = pdh::PdhQuery::open() else {
            log::warn!(target: "hw::taskmgr", "engine poller: pdh.dll unavailable");
            return;
        };
        let Ok(counter) = query.add_english_counter(&CounterPath::gpu_engine_utilization()) else {
            log::warn!(
                target: "hw::taskmgr",
                "engine poller: PdhAddEnglishCounterW failed (\\GPU Engine(*) missing?)"
            );
            return; // query dropped here, closing the handle
        };
        log::info!(target: "hw::taskmgr", "engine poller started (counter handle ok)");

        // First collect primes PDH; then loop forever.
        query.collect().ok();
        std::thread::sleep(std::time::Duration::from_millis(500));

        loop {
            query.collect().ok();
            match query.formatted_array(counter, pdh::MAX_ARRAY_BYTES) {
                Ok(items) => {
                    let mut map: std::collections::HashMap<u32, (f64, f64)> =
                        std::collections::HashMap::new();
                    aggregate_engine_items(&items, &mut map);
                    if !map.is_empty() {
                        let pairs: Vec<String> = map
                            .iter()
                            .map(|(pid, (g, n))| format!("pid={pid} gpu={g:.2} npu={n:.2}"))
                            .collect();
                        log::trace!(
                            target: "hw::taskmgr",
                            "engine poller: parsed {} pids ({} items): {}",
                            map.len(),
                            items.len(),
                            pairs.join("; ")
                        );
                    }
                    if let Some(cache) = PID_ENGINE_CACHE.get() {
                        if let Ok(mut g) = cache.lock() {
                            *g = Some(PidEngineSnapshot { gpu: map });
                        }
                    }
                }
                Err(e) => {
                    log::debug!(target: "hw::taskmgr", "engine poller: format failed: {e}");
                }
            }

            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
    }
    #[cfg(not(windows))]
    {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}
/// Aggregates decoded GPU-engine counter items per PID:
/// GPU = sum of `engtype_3d`, NPU = sum of `engtype_neural`.
///
/// Instance names (`pid_<pid>_luid_..._engtype_<type>`) arrive mixed-case
/// from `PdhAddEnglishCounterW` ("3D", "Neural") and sometimes lowercased,
/// so the engine-type match normalizes case. Zero and negative values carry
/// no engine time and are skipped.
fn aggregate_engine_items(
    items: &[crate::util::pdh::NamedCounterItem],
    map: &mut std::collections::HashMap<u32, (f64, f64)>,
) {
    for item in items {
        if !item.value.is_finite() || item.value <= 0.0 {
            continue;
        }
        let Some(pid) = parse_pid_from_instance(&item.name) else {
            continue;
        };
        let ty = item.name.rsplit("engtype_").next().unwrap_or("");
        let entry = map.entry(pid).or_insert((0.0, 0.0));
        match ty.to_ascii_lowercase().as_str() {
            "3d" => entry.0 += item.value,
            "neural" => entry.1 += item.value,
            _ => {}
        }
    }
}

/// Parse the PID from a GPU-engine instance name (`pid_123_luid_...`).
fn parse_pid_from_instance(instance: &str) -> Option<u32> {
    let rest = instance.strip_prefix("pid_")?;
    let pid_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if pid_str.is_empty() {
        return None;
    }
    let pid = pid_str.parse::<u32>().ok()?;
    if (pid as i64) < MIN_PID {
        return None;
    }
    Some(pid)
}

/// Current per-process GPU/NPU usage snapshot (0–100 style, per process).
pub fn get_pid_engine_snapshot() -> std::collections::HashMap<u32, (f64, f64)> {
    ensure_engine_poller();
    if let Some(cache) = PID_ENGINE_CACHE.get() {
        if let Ok(g) = cache.lock() {
            if let Some(snap) = g.as_ref() {
                return snap.gpu.clone();
            }
        }
    }
    std::collections::HashMap::new()
}

// ── Network throughput per interface ─────────────────────────────────────────
//
// `\Network Interface(*)\Bytes Total/sec` is a PDH cookbook counter that
// reports total bytes/sec per interface. We read it through the same
// dynamic-PDH helper and expose the top interfaces.

static NET_CACHE: OnceLock<Mutex<Vec<NetworkInterfaceSample>>> = OnceLock::new();
static NET_POLLER_STARTED: OnceLock<()> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInterfaceSample {
    pub name: String,
    /// Combined up+down bytes/sec.
    pub bytes_per_sec: f64,
}

fn ensure_net_poller() {
    NET_CACHE.get_or_init(|| Mutex::new(Vec::new()));
    NET_POLLER_STARTED.get_or_init(|| {
        std::thread::Builder::new()
            .name("net-pdh-poller".into())
            .spawn(net_pdh_poller_thread)
            .ok();
    });
}

fn net_pdh_poller_thread() {
    #[cfg(windows)]
    {
        use crate::util::pdh::{self, CounterPath};

        // Single PDH engine (util::pdh): one query owned by this poller
        // thread, no global lock, RAII close on every exit path.
        let Ok(mut query) = pdh::PdhQuery::open() else {
            log::warn!(target: "hw::taskmgr", "net poller: pdh.dll unavailable");
            return;
        };
        let Ok(counter) = query.add_english_counter(&CounterPath::network_bytes_total()) else {
            log::warn!(target: "hw::taskmgr", "net poller: PdhAddEnglishCounterW failed (counter may be missing)");
            return; // query dropped here, closing the handle
        };
        log::info!(target: "hw::taskmgr", "net poller started (counter handle ok)");

        // Rate counters (Bytes Total/sec) need TWO spaced collects before the
        // first PdhGetFormattedCounterArrayW or they return PDH_INVALID_DATA.
        let primed_a = query.collect();
        std::thread::sleep(std::time::Duration::from_millis(1000));
        let primed_b = query.collect();
        log::debug!(
            target: "hw::taskmgr",
            "net poller: priming collects ok_a={} ok_b={}",
            primed_a.is_ok(),
            primed_b.is_ok()
        );

        loop {
            query.collect().ok();
            match query.formatted_array(counter, pdh::MAX_ARRAY_BYTES) {
                Ok(items) => {
                    let mut samples: Vec<NetworkInterfaceSample> = Vec::new();
                    net_items_to_samples(&items, &mut samples);
                    if samples.is_empty() {
                        log::debug!(
                            target: "hw::taskmgr",
                            "net poller: got buffer ({} items) but no samples parsed",
                            items.len()
                        );
                    }
                    if let Some(cache) = NET_CACHE.get() {
                        if let Ok(mut g) = cache.lock() {
                            *g = samples;
                        }
                    }
                }
                Err(e) => {
                    log::debug!(target: "hw::taskmgr", "net poller: format failed: {e}");
                }
            }

            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
    }
    #[cfg(not(windows))]
    {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}

/// Filters decoded `\Network Interface(*)\Bytes Total/sec` items into the
/// published samples: zero/negative values are dropped and pseudo interfaces
/// (isatap, Teredo, Loopback) are skipped.
fn net_items_to_samples(
    items: &[crate::util::pdh::NamedCounterItem],
    samples: &mut Vec<NetworkInterfaceSample>,
) {
    for item in items {
        if !item.value.is_finite() || item.value <= 0.0 {
            continue;
        }
        if item.name.contains("isatap")
            || item.name.contains("Teredo")
            || item.name.contains("Loopback")
        {
            continue;
        }
        samples.push(NetworkInterfaceSample {
            name: item.name.clone(),
            bytes_per_sec: item.value,
        });
    }
}

/// Current network throughput per interface (bytes/sec, up+down combined).
pub fn get_network_interfaces() -> Vec<NetworkInterfaceSample> {
    ensure_net_poller();
    if let Some(cache) = NET_CACHE.get() {
        if let Ok(g) = cache.lock() {
            return g.clone();
        }
    }
    Vec::new()
}

// ── Process table ────────────────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProcessTaskInfo {
    pub name: String,
    pub pid: u32,
    /// Normalized 0–100 (Task Manager style).
    pub cpu_percent: f64,
    /// Working set in MiB.
    pub memory_mb: f64,
    /// GPU % - sum of `engtype_3d` engines for this PID (0–100 style).
    pub gpu_percent: f64,
    /// NPU % - sum of `engtype_neural` engines for this PID (0–100 style).
    pub npu_percent: f64,
    /// Thread count (windows only).
    ///
    /// There is deliberately no per-process network field: Windows does not
    /// expose per-PID byte counters without an ETW kernel session (which needs
    /// administrator) or a full TCP-table diff. The old `net_bytes_per_sec`
    /// was a hardcoded `0.0`, so the UI advertised a NET column that could only
    /// ever read "-" while still offering to sort by it (audit F7.3). The
    /// interface-level throughput lives in `get_network_interfaces`.
    pub thread_count: u32,
}

/// Kill a process by PID. Returns Ok(()) on success.
///
/// Uses `TerminateProcess` on Windows (no console window fl.) - equivalent
/// to Task Manager's "End task". Fails cleanly when the process is gone or
/// access is denied (protected/other-user process).
pub fn kill_process(pid: u32) -> HardwareResult<()> {
    // leak-check: audited - every exit path closes the handle explicitly (S67).
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::PROCESS_QUERY_INFORMATION;
        use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};

        // SAFETY: pid is a u32 process id; OpenProcess/PROCESS_TERMINATE +
        // PROCESS_QUERY_INFORMATION. We close the handle.
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_TERMINATE, false, pid)
                .map_err(|e| {
                    crate::hw::errors::HardwareError::Other(format!(
                        "OpenProcess({pid}) failed: {e}"
                    ))
                })?;
            let res = TerminateProcess(handle, 1);
            CloseHandle(handle).ok();
            match res {
                Ok(()) => Ok(()),
                Err(e) => Err(crate::hw::errors::HardwareError::Other(format!(
                    "TerminateProcess({pid}) failed: {e}"
                ))),
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
        Err(crate::hw::errors::HardwareError::Other(
            "kill not supported".into(),
        ))
    }
}

/// Fetch the full process list with per-process GPU/NPU, sorted by CPU%
/// descending. Includes all processes (no limit) - the UI filters and sorts the
/// complete list client-side.
pub fn get_task_manager() -> Vec<ProcessTaskInfo> {
    #[cfg(windows)]
    {
        use crate::hw::wmi_cache;
        use crate::util::wmi_extract;
        use std::collections::HashMap;

        // Fresh snapshot within the TTL? Serve it; the lock is released
        // before the WMI query below.
        let table_cache = PROCESS_TABLE_CACHE.get_or_init(|| Mutex::new(None));
        if let Ok(guard) = table_cache.lock() {
            if let Some((at, rows)) = guard.as_ref() {
                if at.elapsed() < PROCESS_TABLE_TTL {
                    return rows.clone();
                }
            }
        }

        let engine_map = get_pid_engine_snapshot();

        // ── Processes via WMI ──────────────────────────────────────────────
        let rows: Vec<HashMap<String, wmi::Variant>> = match wmi_cache::with_cimv2(|wmi| {
            Ok(wmi
                .raw_query(
                    "SELECT Name, IDProcess, PercentProcessorTime, WorkingSet, \
                     ThreadCount \
                     FROM Win32_PerfFormattedData_PerfProc_Process",
                )
                .unwrap_or_default())
        }) {
            Ok(rows) => rows,
            Err(e) => {
                log::warn!("WMI process query (task manager) failed: {e}");
                return vec![];
            }
        };

        // Logical CPUs - cached once (like hw::processes).
        static LOGICAL_CPUS: OnceLock<f64> = OnceLock::new();
        let logical_cpus = *LOGICAL_CPUS.get_or_init(|| {
            wmi_cache::with_cimv2(|wmi| {
                let cpu_q: Vec<HashMap<String, wmi::Variant>> = wmi
                    .raw_query("SELECT NumberOfLogicalProcessors FROM Win32_Processor")
                    .unwrap_or_default();
                Ok(cpu_q
                    .first()
                    .and_then(|r| wmi_extract::extract_u32(r, "NumberOfLogicalProcessors"))
                    .map(|n| n as f64)
                    .unwrap_or(1.0)
                    .max(1.0))
            })
            .unwrap_or(1.0)
        });

        let mut procs: Vec<ProcessTaskInfo> = rows
            .into_iter()
            .filter_map(|row| {
                let name = wmi_extract::extract_string(&row, "Name")?;
                if name == "_Total" || name == "Idle" {
                    return None;
                }
                let pid = wmi_extract::extract_u32_or(&row, "IDProcess", 0);
                if pid == 0 {
                    return None;
                }
                let raw_cpu = match row.get("PercentProcessorTime") {
                    Some(wmi::Variant::UI8(v)) => *v as f64,
                    Some(wmi::Variant::UI4(v)) => *v as f64,
                    Some(wmi::Variant::String(s)) => s.parse().unwrap_or(0.0),
                    _ => 0.0,
                };
                let cpu_percent = (raw_cpu / logical_cpus).clamp(0.0, 100.0);
                let memory_mb = wmi_extract::extract_u64(&row, "WorkingSet")
                    .map(|v| v as f64 / (1024.0 * 1024.0))
                    .unwrap_or(0.0);
                let thread_count = wmi_extract::extract_u32_or(&row, "ThreadCount", 0);

                let (gpu_percent, npu_percent) =
                    engine_map.get(&pid).copied().unwrap_or((0.0, 0.0));

                Some(ProcessTaskInfo {
                    name,
                    pid,
                    cpu_percent,
                    memory_mb,
                    gpu_percent,
                    npu_percent,
                    thread_count,
                })
            })
            .collect();

        // Sort by CPU descending. No truncation (F7.4): the old code kept only
        // the top-100, so processes outside it could not be seen, filtered or
        // killed - and sorting by RAM/name/GPU only reordered those 100.
        procs.sort_by(|a, b| {
            b.cpu_percent
                .partial_cmp(&a.cpu_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        // Publish the cooked snapshot for the next hot-path poll.
        if let Some(cell) = PROCESS_TABLE_CACHE.get() {
            if let Ok(mut guard) = cell.lock() {
                *guard = Some((std::time::Instant::now(), procs.clone()));
            }
        }
        procs
    }
    #[cfg(not(windows))]
    {
        vec![]
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::pdh;

    #[test]
    fn parse_pid_from_instance_basic() {
        assert_eq!(
            parse_pid_from_instance("pid_15744_luid_0x00000000_0x00e5c409_phys_0_eng_0_engtype_3d"),
            Some(15744)
        );
        assert_eq!(
            parse_pid_from_instance("pid_1916_luid_0x00000000_0x00012b0f_phys_0_eng_9_engtype_3d"),
            Some(1916)
        );
        assert_eq!(parse_pid_from_instance("_Total"), None);
        assert_eq!(parse_pid_from_instance(""), None);
    }

    #[test]
    fn parse_pid_from_instance_small_pid_filtered() {
        // pid_0 / pid_1 / pid_2 / pid_3 are system pseudo PIDs - filtered.
        assert_eq!(parse_pid_from_instance("pid_0_luid_x_engtype_3d"), None);
        assert_eq!(
            parse_pid_from_instance("pid_4_luid_0x00000000_0x00012b95_phys_0_eng_0_engtype_neural"),
            Some(4)
        );
    }

    #[test]
    fn aggregate_engine_items_empty() {
        let mut map = std::collections::HashMap::new();
        aggregate_engine_items(&pdh::decode_counter_array(&[], 0), &mut map);
        assert!(map.is_empty());

        let mut map = std::collections::HashMap::new();
        aggregate_engine_items(&pdh::decode_counter_array(&[0u8; 8], 1), &mut map);
        assert!(map.is_empty());
    }

    /// Build a synthetic PDH_FMT_COUNTERVALUE_ITEM_W buffer: 2 items, each
    /// 24 bytes (szName pointer, CStatus+pad, double), with wide strings
    /// appended after the items and szName entries pointing at them.
    #[test]
    fn aggregate_engine_items_24byte_item_layout() {
        let mut data: Vec<u8> = Vec::new();

        // Item 0: pid_1000 … engtype_3d, value 12.5 → GPU only
        // Item 1: pid_1000 … engtype_neural, value 3.25 → NPU only
        let name0: Vec<u16> = "pid_1000_luid_0x00000000_0x00e5c409_phys_0_eng_0_engtype_3d"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let name1: Vec<u16> = "pid_1000_luid_0x00000000_0x00012b95_phys_0_eng_0_engtype_neural"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let strings_off = 2 * 24;
        let bytes0: Vec<u8> = name0.iter().flat_map(|u| u.to_le_bytes()).collect();
        let bytes1: Vec<u8> = name1.iter().flat_map(|u| u.to_le_bytes()).collect();

        // Item 0
        data.extend_from_slice(&(strings_off as u64).to_le_bytes()); // szName → name0
        data.extend_from_slice(&0u32.to_le_bytes()); // CStatus
        data.extend_from_slice(&0u32.to_le_bytes()); // pad
        data.extend_from_slice(&12.5f64.to_le_bytes());
        // Item 1 (name1 must follow name0 immediately)
        let strings_off_1 = strings_off + bytes0.len();
        data.extend_from_slice(&(strings_off_1 as u64).to_le_bytes()); // szName → name1
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&3.25f64.to_le_bytes());
        // Strings
        data.extend_from_slice(&bytes0);
        data.extend_from_slice(&bytes1);

        let mut map = std::collections::HashMap::new();
        aggregate_engine_items(&pdh::decode_counter_array(&data, 2), &mut map);
        assert_eq!(map.len(), 1, "both items must aggregate to pid_1000");
        let (gpu, npu) = map.get(&1000).copied().unwrap();
        assert!((gpu - 12.5).abs() < 1e-9, "gpu = {gpu}");
        assert!((npu - 3.25).abs() < 1e-9, "npu = {npu}");
    }

    /// Real-world instance names from `PdhAddEnglishCounterW` are
    /// mixed-case ("…engtype_3D", "…engtype_Neural") - the parser must be
    /// case-insensitive. Use the same 24-byte layout with absolute-style
    /// pointers (>= buffer base → treated as relative offset after
    /// subtraction).
    #[test]
    fn aggregate_engine_items_mixed_case_engtypes() {
        let mut data: Vec<u8> = Vec::new();

        // Item 0: "…engtype_3D" (upper 'D') value 42.0 → GPU
        // Item 1: "…engtype_Neural" (capital N) value 7.5 → NPU
        let name0: Vec<u16> = "pid_4242_luid_0x00000000_0x00e5c409_phys_0_eng_0_engtype_3D"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let name1: Vec<u16> = "pid_4242_luid_0x00000000_0x00012b95_phys_0_eng_0_engtype_Neural"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let strings_off = 2 * 24;
        let bytes0: Vec<u8> = name0.iter().flat_map(|u| u.to_le_bytes()).collect();
        let bytes1: Vec<u8> = name1.iter().flat_map(|u| u.to_le_bytes()).collect();

        // Item 0
        data.extend_from_slice(&(strings_off as u64).to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&42.0f64.to_le_bytes());
        // Item 1
        let strings_off_1 = strings_off + bytes0.len();
        data.extend_from_slice(&(strings_off_1 as u64).to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&7.5f64.to_le_bytes());
        data.extend_from_slice(&bytes0);
        data.extend_from_slice(&bytes1);

        let mut map = std::collections::HashMap::new();
        aggregate_engine_items(&pdh::decode_counter_array(&data, 2), &mut map);
        assert_eq!(map.len(), 1);
        let (gpu, npu) = map.get(&4242).copied().unwrap();
        assert!((gpu - 42.0).abs() < 1e-9, "gpu = {gpu}");
        assert!((npu - 7.5).abs() < 1e-9, "npu = {npu}");
    }

    /// Same synthetic buffer through the network parser: names are plain
    /// interface names; zero/negative values are dropped.
    #[test]
    fn net_items_to_samples_24byte_item_layout() {
        let mut data: Vec<u8> = Vec::new();
        let name0: Vec<u16> = "Intel[R] Wi-Fi 6E AX211 160MHz"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let name1: Vec<u16> = "Realtek USB GbE Family Controller"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let strings_off = 2 * 24;
        let bytes0: Vec<u8> = name0.iter().flat_map(|u| u.to_le_bytes()).collect();
        let bytes1: Vec<u8> = name1.iter().flat_map(|u| u.to_le_bytes()).collect();

        data.extend_from_slice(&(strings_off as u64).to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&1024.5f64.to_le_bytes());
        let strings_off_1 = strings_off + bytes0.len();
        data.extend_from_slice(&(strings_off_1 as u64).to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0f64.to_le_bytes()); // zero value → skipped
        data.extend_from_slice(&bytes0);
        data.extend_from_slice(&bytes1);

        let mut samples = Vec::new();
        net_items_to_samples(&pdh::decode_counter_array(&data, 2), &mut samples);
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].name, "Intel[R] Wi-Fi 6E AX211 160MHz");
        assert!((samples[0].bytes_per_sec - 1024.5).abs() < 1e-9);
    }
    /// The hot path must serve the snapshot cache, not re-run the WMI
    /// process query on every call.
    #[test]
    fn process_table_snapshot_is_cached_within_ttl() {
        let first = get_task_manager();
        let started = std::time::Instant::now();
        let second = get_task_manager();
        let elapsed = started.elapsed();
        assert_eq!(
            first.len(),
            second.len(),
            "cached snapshot must have the same size"
        );
        assert!(
            elapsed.as_millis() < 50,
            "the second call must be a cache hit, took {} ms",
            elapsed.as_millis()
        );
    }

    /// U5 acceptance: the full task-manager sample (WMI process table joined
    /// with the PDH engine snapshot) must complete well under 300 ms even
    /// with the whole process list. Run explicitly (it needs the pollers
    /// warm and prints the timings):
    ///
    /// cargo test --lib --features face measure_full_task_manager_sample \
    ///   -- --ignored --nocapture
    #[test]
    #[ignore]
    fn measure_full_task_manager_sample() {
        // Warm-up: start the pollers and prime the WMI caches. The first
        // sample is cold (pollers start, WMI connects) and is not measured.
        let warm = get_task_manager();
        println!("warm-up sample: {} processes", warm.len());
        std::thread::sleep(std::time::Duration::from_millis(2500));

        let mut durations_ms: Vec<u128> = Vec::new();
        let mut processes = 0usize;
        for run in 0..10 {
            let started = std::time::Instant::now();
            let rows = get_task_manager();
            let ms = started.elapsed().as_millis();
            processes = rows.len();
            println!("sample {:2}: {:4} ms, {} processes", run, ms, processes);
            durations_ms.push(ms);
            assert!(!rows.is_empty(), "task manager sample must list processes");
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        let count = durations_ms.len() as u128;
        let total: u128 = durations_ms.iter().sum();
        let avg = total / count;
        let min = durations_ms.iter().min().copied().unwrap_or(0);
        let max = durations_ms.iter().max().copied().unwrap_or(0);
        println!(
            "full sample: {} runs, min {} ms, avg {} ms, max {} ms, {} processes",
            durations_ms.len(),
            min,
            avg,
            max,
            processes
        );
        assert!(
            avg < 300,
            "full task-manager sample avg {} ms exceeds the 300 ms goal",
            avg
        );
    }
}
