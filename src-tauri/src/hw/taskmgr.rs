//! Global task manager backend — per-process resource accounting
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
//! - **GPU% / NPU% per PID**: PDH `\GPU Engine(*)\Utilization Percentage`.
//!   Engines are named `pid_<Pid>_luid_..._engtype_<Type>`. GPU = sum of
//!   `engtype_3d` instances; NPU = sum of `engtype_neural` instances.
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

// ── Per-process GPU + NPU engine accounting ─────────────────────────────────
//
// The `\GPU Engine(*)\Utilization Percentage` counter has one instance per
// (process, engine type, adapter). We read it with the same PDH-via-libloading
// pattern as `system_info` and aggregate per PID:
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
    unsafe {
        use libloading::{Library, Symbol};
        use std::os::raw::c_void;
        use std::os::windows::ffi::OsStrExt;

        type FnOpenQuery = unsafe extern "system" fn(*const c_void, usize, *mut isize) -> u32;
        type FnAddCounter = unsafe extern "system" fn(isize, *const u16, usize, *mut isize) -> u32;
        type FnCollect = unsafe extern "system" fn(isize) -> u32;
        type FnGetArray = unsafe extern "system" fn(isize, u32, *mut u32, *mut u32, *mut u8) -> u32;
        type FnClose = unsafe extern "system" fn(isize) -> u32;

        let lib: &'static Library = match Library::new("pdh.dll") {
            Ok(l) => Box::leak(Box::new(l)),
            Err(_) => return,
        };
        let open_q: Symbol<'static, FnOpenQuery> = match lib.get(b"PdhOpenQueryW\0") {
            Ok(f) => f,
            Err(_) => return,
        };
        let add_c: Symbol<'static, FnAddCounter> = match lib.get(b"PdhAddEnglishCounterW\0") {
            Ok(f) => f,
            Err(_) => return,
        };
        let collect: Symbol<'static, FnCollect> = match lib.get(b"PdhCollectQueryData\0") {
            Ok(f) => f,
            Err(_) => return,
        };
        let get_array: Symbol<'static, FnGetArray> =
            match lib.get(b"PdhGetFormattedCounterArrayW\0") {
                Ok(f) => f,
                Err(_) => return,
            };
        let close_q: Symbol<'static, FnClose> = match lib.get(b"PdhCloseQuery\0") {
            Ok(f) => f,
            Err(_) => return,
        };

        let counter_text: Vec<u16> = std::ffi::OsStr::new(r"\GPU Engine(*)\Utilization Percentage")
            .encode_wide()
            .chain(Some(0))
            .collect();

        let mut query: isize = 0;
        if open_q(std::ptr::null(), 0, &mut query) != 0 {
            log::warn!(target: "hw::taskmgr", "engine poller: PdhOpenQueryW failed");
            return;
        }
        let mut counter: isize = 0;
        if add_c(query, counter_text.as_ptr(), 0, &mut counter) != 0 {
            log::warn!(target: "hw::taskmgr", "engine poller: PdhAddEnglishCounterW failed (\\GPU Engine(*) missing?)");
            close_q(query);
            return;
        }
        log::info!(target: "hw::taskmgr", "engine poller started (counter handle ok)");

        // First collect primes PDH; then loop forever.
        collect(query);
        std::thread::sleep(std::time::Duration::from_millis(500));

        loop {
            collect(query);

            // PDH_FMT_COUNTERVALUE_ITEM_W array (x64): each item is 24 bytes:
            //   szName (LPWSTR, 8 bytes) + PDH_FMT_COUNTERVALUE (16 bytes,
            //   CStatus u32 at +8, doubleValue f64 at +16). The szName pointers
            //   reference wide strings stored after the items in the buffer.
            // First call with null buffer returns required size in dwBufferSize.
            const PDH_FMT_DOUBLE: u32 = 0x00000200;
            let mut buf_size: u32 = 0;
            let mut item_count: u32 = 0;
            let rc1 = get_array(
                counter,
                PDH_FMT_DOUBLE,
                &mut buf_size,
                &mut item_count,
                std::ptr::null_mut(),
            );

            if buf_size > 0 && buf_size < 64 * 1024 * 1024 {
                let mut data = vec![0u8; buf_size as usize];
                // rc is intentionally unused: the second call is expected to
                // succeed once we pass the buffer size reported by the probe.
                let _rc2 = get_array(
                    counter,
                    PDH_FMT_DOUBLE,
                    &mut buf_size,
                    &mut item_count,
                    data.as_mut_ptr(),
                );

                let mut map: std::collections::HashMap<u32, (f64, f64)> =
                    std::collections::HashMap::new();
                parse_gpu_engine_buffer(&data, item_count as usize, &mut map);
                if !map.is_empty() {
                    let pairs: Vec<String> = map
                        .iter()
                        .map(|(pid, (g, n))| format!("pid={pid} gpu={g:.2} npu={n:.2}"))
                        .collect();
                    log::trace!(target: "hw::taskmgr", "engine poller: parsed {} pids ({} items): {}", map.len(), item_count, pairs.join("; "));
                }
                if let Some(cache) = PID_ENGINE_CACHE.get() {
                    if let Ok(mut g) = cache.lock() {
                        *g = Some(PidEngineSnapshot { gpu: map });
                    }
                }
            } else {
                log::debug!(target: "hw::taskmgr", "engine poller: no items yet (buf={buf_size} rc={rc1} items={item_count})");
            }

            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
    }
    #[cfg(not(windows))]
    {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}

/// Parse the raw `PdhGetFormattedCounterArrayW` buffer for
/// `\GPU Engine(*)\Utilization Percentage`, accumulating per-PID sums.
///
/// Buffer layout for PDH_FMT_DOUBLE (x64) — `PDH_FMT_COUNTERVALUE_ITEM_W`:
///
/// The buffer starts at the first item (ItemBuffer). Each item is 24 bytes:
///
/// - `+0`:  szName (LPWSTR, 8 bytes) — pointer to a null-terminated wide
///   instance string stored somewhere in the buffer
/// - `+8`:  PDH_FMT_COUNTERVALUE (16 bytes):
///   - `+8`: CStatus (i32)
///   - `+12`: pad
///   - `+16`: doubleValue (f64)
///
/// `lpdwItemCount` (returned by the API) tells how many items exist.
///
/// We resolve each szName pointer by translating its offset into our byte
/// slice and reading the wide string there.
fn parse_gpu_engine_buffer(
    data: &[u8],
    item_count: usize,
    map: &mut std::collections::HashMap<u32, (f64, f64)>,
) {
    if item_count == 0 || item_count > 4096 {
        return;
    }
    const ITEM_SIZE: usize = 24;
    if data.len() < item_count * ITEM_SIZE {
        return;
    }

    for i in 0..item_count {
        let off = i * ITEM_SIZE;
        // szName pointer (relative to the start of the buffer).
        // Resolve szName: PDH writes absolute heap pointers into the buffer on
        // x64 (and relative offsets on some builds). Support both by
        // converting to an offset from the buffer base.
        let base = data.as_ptr() as usize;
        let rel_ptr = usize::from_le_bytes(data[off..off + 8].try_into().unwrap_or([0; 8]));
        let off_name = if rel_ptr >= base {
            rel_ptr - base
        } else {
            rel_ptr
        };
        let value = f64::from_le_bytes(data[off + 16..off + 24].try_into().unwrap_or([0; 8]));

        if !value.is_finite() || value <= 0.0 {
            continue;
        }

        // Translate the resolved offset into a slice offset.
        if off_name == 0 || off_name >= data.len() {
            continue;
        }
        let name = read_wide_string(data, off_name).0;
        if name.is_empty() {
            continue;
        }

        // Parse `pid_<pid>_luid_..._engtype_<type>` and accumulate.
        let Some(pid) = parse_pid_from_instance(&name) else {
            continue;
        };
        let ty = name.rsplit("engtype_").next().unwrap_or("");
        let entry = map.entry(pid).or_insert((0.0, 0.0));
        // PDH canonical English instance names are mixed-case ("3D", "Neural",
        // "Copy", "Compute", ...) but can also arrive lowercased depending on
        // the counter path / provider; normalize before matching.
        match ty.to_ascii_lowercase().as_str() {
            "3d" => entry.0 += value,
            "neural" => entry.1 += value,
            _ => {}
        }
    }
}

/// Read a null-terminated UTF-16 string from `data` at `pos`.
/// Returns (string, next_position).
fn read_wide_string(data: &[u8], mut pos: usize) -> (String, usize) {
    let mut chars: Vec<u16> = Vec::new();
    while pos + 1 < data.len() {
        let u = u16::from_le_bytes([data[pos], data[pos + 1]]);
        if u == 0 {
            break;
        }
        chars.push(u);
        pos += 2;
    }
    let name = String::from_utf16_lossy(&chars);
    // Advance past the NUL terminator (2 bytes) if present.
    if pos + 1 < data.len() && data[pos] == 0 && data[pos + 1] == 0 {
        pos += 2;
    }
    (name, pos)
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
    unsafe {
        use libloading::{Library, Symbol};
        use std::os::raw::c_void;
        use std::os::windows::ffi::OsStrExt;

        type FnOpenQuery = unsafe extern "system" fn(*const c_void, usize, *mut isize) -> u32;
        type FnAddCounter = unsafe extern "system" fn(isize, *const u16, usize, *mut isize) -> u32;
        type FnCollect = unsafe extern "system" fn(isize) -> u32;
        type FnGetArray = unsafe extern "system" fn(isize, u32, *mut u32, *mut u32, *mut u8) -> u32;
        type FnClose = unsafe extern "system" fn(isize) -> u32;

        let lib: &'static Library = match Library::new("pdh.dll") {
            Ok(l) => Box::leak(Box::new(l)),
            Err(_) => {
                log::warn!(target: "hw::taskmgr", "net poller: pdh.dll load failed");
                return;
            }
        };
        let open_q: Symbol<'static, FnOpenQuery> = match lib.get(b"PdhOpenQueryW\0") {
            Ok(f) => f,
            Err(_) => {
                log::warn!(target: "hw::taskmgr", "net poller: PdhOpenQueryW symbol missing");
                return;
            }
        };
        let add_c: Symbol<'static, FnAddCounter> = match lib.get(b"PdhAddEnglishCounterW\0") {
            Ok(f) => f,
            Err(_) => {
                log::warn!(target: "hw::taskmgr", "net poller: PdhAddEnglishCounterW symbol missing");
                return;
            }
        };
        let collect: Symbol<'static, FnCollect> = match lib.get(b"PdhCollectQueryData\0") {
            Ok(f) => f,
            Err(_) => {
                log::warn!(target: "hw::taskmgr", "net poller: PdhCollectQueryData symbol missing");
                return;
            }
        };
        let get_array: Symbol<'static, FnGetArray> = match lib
            .get(b"PdhGetFormattedCounterArrayW\0")
        {
            Ok(f) => f,
            Err(_) => {
                log::warn!(target: "hw::taskmgr", "net poller: PdhGetFormattedCounterArrayW symbol missing");
                return;
            }
        };
        let close_q: Symbol<'static, FnClose> = match lib.get(b"PdhCloseQuery\0") {
            Ok(f) => f,
            Err(_) => {
                log::warn!(target: "hw::taskmgr", "net poller: PdhCloseQuery symbol missing");
                return;
            }
        };

        let counter_text: Vec<u16> = std::ffi::OsStr::new(r"\Network Interface(*)\Bytes Total/sec")
            .encode_wide()
            .chain(Some(0))
            .collect();

        let mut query: isize = 0;
        if open_q(std::ptr::null(), 0, &mut query) != 0 {
            log::warn!(target: "hw::taskmgr", "net poller: PdhOpenQueryW failed");
            return;
        }
        let mut counter: isize = 0;
        if add_c(query, counter_text.as_ptr(), 0, &mut counter) != 0 {
            log::warn!(target: "hw::taskmgr", "net poller: PdhAddEnglishCounterW failed (counter may be missing)");
            close_q(query);
            return;
        }
        log::info!(target: "hw::taskmgr", "net poller started (counter handle ok)");

        // Rate counters (Bytes Total/sec) need TWO spaced collects before the
        // first PdhGetFormattedCounterArrayW or they return PDH_INVALID_DATA.
        let rc_a = collect(query);
        std::thread::sleep(std::time::Duration::from_millis(1000));
        let rc_b = collect(query);
        log::debug!(target: "hw::taskmgr", "net poller: priming collects rc_a={rc_a} rc_b={rc_b}");

        loop {
            collect(query);
            const PDH_FMT_DOUBLE: u32 = 0x00000200;
            let mut buf_size: u32 = 0;
            let mut item_count: u32 = 0;
            let rc1 = get_array(
                counter,
                PDH_FMT_DOUBLE,
                &mut buf_size,
                &mut item_count,
                std::ptr::null_mut(),
            );

            if buf_size > 0 && buf_size < 64 * 1024 * 1024 {
                let mut data = vec![0u8; buf_size as usize];
                let rc2 = get_array(
                    counter,
                    PDH_FMT_DOUBLE,
                    &mut buf_size,
                    &mut item_count,
                    data.as_mut_ptr(),
                );

                let mut samples: Vec<NetworkInterfaceSample> = Vec::new();
                parse_net_buffer(&data, item_count as usize, &mut samples);
                if samples.is_empty() {
                    log::debug!(target: "hw::taskmgr", "net poller: got buffer (rc1={rc1} rc2={rc2} size={} items={item_count}) but no samples parsed", buf_size);
                }
                if let Some(cache) = NET_CACHE.get() {
                    if let Ok(mut g) = cache.lock() {
                        *g = samples;
                    }
                }
            } else {
                log::debug!(target: "hw::taskmgr", "net poller: buffer size {buf_size} rc={rc1}");
            }

            std::thread::sleep(std::time::Duration::from_millis(1500));
        }
    }
    #[cfg(not(windows))]
    {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}

/// Parse `\Network Interface(*)\Bytes Total/sec` buffer. Same layout as GPU
/// engine array: N × PDH_FMT_COUNTERVALUE_ITEM_W (24 bytes each: szName
/// LPWSTR pointer +8, CStatus +16, doubleValue f64 +16..+24), strings pointed
/// to by the per-item szName fields.
fn parse_net_buffer(data: &[u8], item_count: usize, samples: &mut Vec<NetworkInterfaceSample>) {
    if item_count == 0 || item_count > 4096 {
        return;
    }
    const ITEM_SIZE: usize = 24;
    if data.len() < item_count * ITEM_SIZE {
        return;
    }

    for i in 0..item_count {
        let off = i * ITEM_SIZE;
        // Resolve szName: absolute heap pointer (x64) or relative offset.
        let base = data.as_ptr() as usize;
        let rel_ptr = usize::from_le_bytes(data[off..off + 8].try_into().unwrap_or([0; 8]));
        let off_name = if rel_ptr >= base {
            rel_ptr - base
        } else {
            rel_ptr
        };
        let value = f64::from_le_bytes(data[off + 16..off + 24].try_into().unwrap_or([0; 8]));

        if !value.is_finite() || value <= 0.0 {
            continue;
        }
        if off_name == 0 || off_name >= data.len() {
            continue;
        }
        let name = read_wide_string(data, off_name).0;
        if name.is_empty() {
            continue;
        }
        // Skip pseudo interfaces.
        if name.contains("isatap") || name.contains("Teredo") || name.contains("Loopback") {
            continue;
        }
        samples.push(NetworkInterfaceSample {
            name,
            bytes_per_sec: value,
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
    /// GPU % — sum of `engtype_3d` engines for this PID (0–100 style).
    pub gpu_percent: f64,
    /// NPU % — sum of `engtype_neural` engines for this PID (0–100 style).
    pub npu_percent: f64,
    /// Combined up+down bytes/sec on all interfaces (if measurable).
    pub net_bytes_per_sec: f64,
    /// Thread count (windows only).
    pub thread_count: u32,
}

/// Kill a process by PID. Returns Ok(()) on success.
///
/// Uses `TerminateProcess` on Windows (no console window fl.) — equivalent
/// to Task Manager's "End task". Fails cleanly when the process is gone or
/// access is denied (protected/other-user process).
pub fn kill_process(pid: u32) -> HardwareResult<()> {
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

/// Fetch the full process list with per-process GPU/NPU/network, sorted by
/// CPU% descending. Includes all processes (no limit).
pub fn get_task_manager() -> Vec<ProcessTaskInfo> {
    #[cfg(windows)]
    {
        use crate::hw::wmi_cache;
        use crate::util::wmi_extract;
        use std::collections::HashMap;

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

        // Logical CPUs — cached once (like hw::processes).
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
                    net_bytes_per_sec: 0.0,
                    thread_count,
                })
            })
            .collect();

        // Sort by CPU descending, top 100.
        procs.sort_by(|a, b| {
            b.cpu_percent
                .partial_cmp(&a.cpu_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        procs.truncate(100);
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
        // pid_0 / pid_1 / pid_2 / pid_3 are system pseudo PIDs — filtered.
        assert_eq!(parse_pid_from_instance("pid_0_luid_x_engtype_3d"), None);
        assert_eq!(
            parse_pid_from_instance("pid_4_luid_0x00000000_0x00012b95_phys_0_eng_0_engtype_neural"),
            Some(4)
        );
    }

    #[test]
    fn parse_gpu_engine_buffer_empty() {
        let mut map = std::collections::HashMap::new();
        parse_gpu_engine_buffer(&[], 0, &mut map);
        assert!(map.is_empty());

        let mut map = std::collections::HashMap::new();
        parse_gpu_engine_buffer(&[0u8; 8], 1, &mut map);
        assert!(map.is_empty());
    }

    /// Build a synthetic PDH_FMT_COUNTERVALUE_ITEM_W buffer: 2 items, each
    /// 24 bytes (szName pointer, CStatus+pad, double), with wide strings
    /// appended after the items and szName entries pointing at them.
    #[test]
    fn parse_gpu_engine_buffer_24byte_item_layout() {
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
        parse_gpu_engine_buffer(&data, 2, &mut map);
        assert_eq!(map.len(), 1, "both items must aggregate to pid_1000");
        let (gpu, npu) = map.get(&1000).copied().unwrap();
        assert!((gpu - 12.5).abs() < 1e-9, "gpu = {gpu}");
        assert!((npu - 3.25).abs() < 1e-9, "npu = {npu}");
    }

    /// Real-world instance names from `PdhAddEnglishCounterW` are
    /// mixed-case ("…engtype_3D", "…engtype_Neural") — the parser must be
    /// case-insensitive. Use the same 24-byte layout with absolute-style
    /// pointers (>= buffer base → treated as relative offset after
    /// subtraction).
    #[test]
    fn parse_gpu_engine_buffer_mixed_case_engtypes() {
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
        parse_gpu_engine_buffer(&data, 2, &mut map);
        assert_eq!(map.len(), 1);
        let (gpu, npu) = map.get(&4242).copied().unwrap();
        assert!((gpu - 42.0).abs() < 1e-9, "gpu = {gpu}");
        assert!((npu - 7.5).abs() < 1e-9, "npu = {npu}");
    }

    /// Same synthetic buffer through the network parser: names are plain
    /// interface names; zero/negative values are dropped.
    #[test]
    fn parse_net_buffer_24byte_item_layout() {
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
        parse_net_buffer(&data, 2, &mut samples);
        assert_eq!(samples.len(), 1);
        assert_eq!(samples[0].name, "Intel[R] Wi-Fi 6E AX211 160MHz");
        assert!((samples[0].bytes_per_sec - 1024.5).abs() < 1e-9);
    }
}
