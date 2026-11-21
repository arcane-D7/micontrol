//! System information queries (CPU, memory, OS version).
//!
//! CPU and GPU usage come from the shared PDH engine (`util::pdh`), memory
//! and OS details from WMI, with caching via `OnceLock` and periodic refresh.

use crate::hw::errors::HardwareResult;
use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock};

// ── CPU usage via PDH ─────────────────────────────────────────────────────────
//
// We use "\Processor Information(_Total)\% Processor Time", which measures the
// proportion of time the scheduler finds the CPU busy (non-idle).  This matches
// the intuitive number shown in Windows Task Manager Performance tab.
//
// NOTE: We deliberately do NOT use "% Processor Utility" because on Intel hybrid
// architectures (Panther Lake, Meteor Lake, Arrow Lake - all with P-cores +
// E-cores + LP-E-cores), that counter normalises by each core's max-turbo
// frequency.  When LP-E-cores run background OS tasks at moderate clock speeds,
// their per-core "utility" contribution inflates the _Total aggregate far above
// the scheduling-based percentage - producing readings like 38% when Task Manager
// shows 6%.  % Processor Time is immune to this because it only tracks idle vs
// non-idle time, independent of P-state frequency.
// We use the same background PDH poller pattern as the GPU.
static CPU_USAGE_CACHE: OnceLock<Mutex<f64>> = OnceLock::new();
static CPU_POLLER_STARTED: OnceLock<()> = OnceLock::new();

fn ensure_cpu_poller() {
    CPU_USAGE_CACHE.get_or_init(|| Mutex::new(0.0));
    CPU_POLLER_STARTED.get_or_init(|| {
        std::thread::Builder::new()
            .name("cpu-pdh-poller".into())
            .spawn(cpu_pdh_poller_thread)
            .ok();
    });
}

fn cpu_pdh_poller_thread() {
    #[cfg(windows)]
    {
        use crate::util::pdh::{self, CounterPath};

        // Single PDH engine (util::pdh): one query owned by this poller
        // thread, no global lock, RAII close on every exit path.
        let Ok(mut query) = pdh::PdhQuery::open() else {
            return;
        };
        // Primary: Processor Information provider (Vista+), scheduling-based
        // %; fallback: legacy Processor provider (always available).
        let counter = match query.add_english_counter(&CounterPath::cpu_processor_time()) {
            Ok(c) => c,
            Err(_) => match query.add_english_counter(&CounterPath::cpu_processor_time_legacy()) {
                Ok(c) => c,
                Err(_) => return, // query dropped here, closing the handle
            },
        };

        query.collect().ok(); // baseline for rate counter
        loop {
            std::thread::sleep(std::time::Duration::from_millis(1500));
            query.collect().ok();
            if let Ok(value) = query.formatted_double(counter) {
                if value.is_usable() {
                    if let Some(cache) = CPU_USAGE_CACHE.get() {
                        if let Ok(mut g) = cache.lock() {
                            *g = value.value.clamp(0.0, 100.0);
                        }
                    }
                }
            }
        }
    }
}

fn read_cpu_usage() -> f64 {
    CPU_USAGE_CACHE
        .get()
        .and_then(|m| m.lock().ok())
        .map(|g| *g)
        .unwrap_or(0.0)
}

//
// WMI Win32_PerfFormattedData_GPUPerformanceCounters_GPUEngine always returns 0
// on the first sample per COM apartment (rate counters need two timed snapshots).
// Instead, we maintain a single persistent PDH query on a background thread that
// samples every 1.5 s and writes the result to GPU_USAGE_CACHE.  get_system_info()
// reads from the cache instantly.
static GPU_USAGE_CACHE: OnceLock<Mutex<f64>> = OnceLock::new();
static GPU_POLLER_STARTED: OnceLock<()> = OnceLock::new();

fn ensure_gpu_poller() {
    GPU_USAGE_CACHE.get_or_init(|| Mutex::new(0.0));
    GPU_POLLER_STARTED.get_or_init(|| {
        std::thread::Builder::new()
            .name("gpu-pdh-poller".into())
            .spawn(gpu_pdh_poller_thread)
            .ok();
    });
}

fn gpu_pdh_poller_thread() {
    #[cfg(windows)]
    {
        use crate::util::pdh::{self, CounterPath};

        // Single PDH engine (util::pdh): one persistent query on this thread
        // samples every 1.5 s and writes the aggregated 3D-engine sum to the
        // cache; get_system_info() reads the cache instantly.
        let Ok(mut query) = pdh::PdhQuery::open() else {
            return;
        };
        // All GPU Engine instances (*); the loop below sums the 3D engines.
        let Ok(counter) = query.add_english_counter(&CounterPath::gpu_engine_utilization()) else {
            return; // query dropped here, closing the handle
        };

        // First collection establishes the baseline for rate counters.
        query.collect().ok();

        loop {
            std::thread::sleep(std::time::Duration::from_millis(1500));
            query.collect().ok();

            let items = match query.formatted_array(counter, pdh::MAX_ARRAY_BYTES) {
                Ok(items) => items,
                Err(_) => continue,
            };

            // Filter: only 3D-engine instances. PDH exposes one instance per
            // (PID x LUID x phys x eng x engtype). Summing all PID instances
            // for the same physical engine gives the correct total scheduling
            // time for that engine, equivalent to Task Manager's "GPU" column:
            // each process gets a time slice on the shared hardware engine and
            // the sum of their slices is the total engine busy time. Values
            // are clamped below so simultaneous use across engines never
            // over-reports.
            let total: f64 = items
                .iter()
                .filter(|item| pdh::is_usable_status(item.status))
                .filter(|item| item.value.is_finite() && item.value >= 0.0)
                .filter(|item| item.name.contains("3D"))
                .map(|item| item.value)
                .sum();

            let clamped = total.clamp(0.0, 100.0);
            if let Some(cache) = GPU_USAGE_CACHE.get() {
                if let Ok(mut g) = cache.lock() {
                    *g = clamped;
                }
            }
        }
    }
}

fn read_gpu_usage() -> f64 {
    GPU_USAGE_CACHE
        .get()
        .and_then(|m| m.lock().ok())
        .map(|g| *g)
        .unwrap_or(0.0)
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SystemInfo {
    pub cpu_name: String,
    pub cpu_cores: u32,
    pub cpu_threads: u32,
    pub cpu_usage: f64,
    pub gpu_name: String,
    pub gpu_usage: f64,
    pub vram_used_mb: f64,
    pub ram_total_gb: f64,
    pub ram_used_gb: f64,
    pub os_version: String,
}

pub fn get_system_info() -> HardwareResult<SystemInfo> {
    #[cfg(windows)]
    {
        use crate::hw::wmi_cache;
        use crate::util::wmi_extract;
        use std::collections::HashMap;

        // Start the PDH pollers on first call (no-op on subsequent calls)
        ensure_gpu_poller();
        ensure_cpu_poller();

        let info = wmi_cache::with_cimv2(|wmi| {
            // ── Static CPU identity (name, cores, threads) ────────────────────
            // Each WMI query is independent - a failure in one should NOT fail
            // the entire SystemInfo struct. We log and use fallback values.
            let cpus: Vec<HashMap<String, wmi::Variant>> = wmi
                .raw_query(
                    "SELECT Name, NumberOfCores, NumberOfLogicalProcessors FROM Win32_Processor",
                )
                .unwrap_or_else(|e| {
                    log::debug!(target: "hw::system_info", "Win32_Processor raw_query error: {e}");
                    Vec::new()
                });
            let cpu = cpus.into_iter().next().unwrap_or_default();

            // ── CPU usage - PDH "% Processor Utility" matches Task Manager ──────
            // Unlike WMI Win32_PerfFormattedData_PerfOS_Processor which reports raw
            // idle-vs-busy time, % Processor Utility accounts for P-state frequency
            // scaling, matching the value shown in Windows 11 Task Manager.
            let cpu_usage = read_cpu_usage();

            // ── GPU name ──────────────────────────────────────────────────────
            let gpu_name = wmi
                .raw_query::<HashMap<String, wmi::Variant>>("SELECT Name FROM Win32_VideoController")
                .unwrap_or_else(|e| {
                    log::debug!(target: "hw::system_info", "Win32_VideoController raw_query error: {e}");
                    Vec::new()
                })
                .into_iter()
                .next()
                .and_then(|r| wmi_extract::extract_string(&r, "Name"))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| "Unknown GPU".to_string());

            // ── GPU 3D engine utilization (from PDH background poller) ───────────
            // The poller maintains a persistent PDH query and writes the latest
            // value to GPU_USAGE_CACHE every ~1.5 s.  Reading from cache is instant
            // and avoids the WMI first-sample = 0 issue.
            let gpu_usage = read_gpu_usage();

            // ── Dedicated VRAM used ───────────────────────────────────────────
            // The correct class name is Win32_PerfFormattedData_GPUPerformanceCounters_GPUAdapterMemory
            // (not Win32_PerfFormattedData_GPUAdapterMemory_GPUAdapter which doesn't exist)
            // This class may not exist on systems without GPU perf counter drivers -
            // don't fail the entire struct if it's missing.
            let vram_used_mb = wmi
                .raw_query::<HashMap<String, wmi::Variant>>(
                    "SELECT DedicatedUsage FROM Win32_PerfFormattedData_GPUPerformanceCounters_GPUAdapterMemory",
                )
                .unwrap_or_else(|e| {
                    log::debug!(target: "hw::system_info", "GPUAdapterMemory raw_query error: {e}");
                    Vec::new()
                })
                .first()
                .and_then(|r| wmi_extract::extract_u64(r, "DedicatedUsage"))
                .map(|n| n as f64 / (1024.0 * 1024.0))
                .unwrap_or(0.0);

            // ── Physical memory total ─────────────────────────────────────────
            let ram_total_bytes: u64 = wmi
                .raw_query::<HashMap<String, wmi::Variant>>("SELECT Capacity FROM Win32_PhysicalMemory")
                .unwrap_or_else(|e| {
                    log::debug!(target: "hw::system_info", "Win32_PhysicalMemory raw_query error: {e}");
                    Vec::new()
                })
                .iter()
                .filter_map(|row| match row.get("Capacity") {
                    Some(wmi::Variant::String(s)) => s.parse::<u64>().ok(),
                    _ => wmi_extract::extract_u64(row, "Capacity"),
                })
                .sum();

            // ── OS info + available (free) memory ─────────────────────────────
            let os_row = wmi
                .raw_query::<HashMap<String, wmi::Variant>>(
                    "SELECT Caption, FreePhysicalMemory, TotalVisibleMemorySize FROM Win32_OperatingSystem",
                )
                .unwrap_or_else(|e| {
                    log::debug!(target: "hw::system_info", "Win32_OperatingSystem raw_query error: {e}");
                    Vec::new()
                })
                .into_iter()
                .next()
                .unwrap_or_default();

            let cpu_name = wmi_extract::extract_string(&cpu, "Name")
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| "Unknown CPU".to_string());
            let cpu_cores = wmi_extract::extract_u32_or(&cpu, "NumberOfCores", 0);
            let cpu_threads = wmi_extract::extract_u32_or(&cpu, "NumberOfLogicalProcessors", 0);
            let ram_total_gb = ram_total_bytes as f64 / (1024.0 * 1024.0 * 1024.0);

            let free_kb = match os_row.get("FreePhysicalMemory") {
                Some(wmi::Variant::String(s)) => s.parse().unwrap_or(0),
                _ => wmi_extract::extract_u64(&os_row, "FreePhysicalMemory").unwrap_or(0),
            };
            let total_kb = match os_row.get("TotalVisibleMemorySize") {
                Some(wmi::Variant::String(s)) => s.parse().unwrap_or(0),
                _ => wmi_extract::extract_u64(&os_row, "TotalVisibleMemorySize")
                    .unwrap_or(ram_total_bytes / 1024),
            };
            let used_kb = total_kb.saturating_sub(free_kb);
            let ram_used_gb = used_kb as f64 / (1024.0 * 1024.0);

            let os_version = wmi_extract::extract_string(&os_row, "Caption")
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| "Windows 11".to_string());

            Ok(SystemInfo {
                cpu_name,
                cpu_cores,
                cpu_threads,
                cpu_usage,
                gpu_name,
                gpu_usage,
                vram_used_mb,
                ram_total_gb,
                ram_used_gb,
                os_version,
            })
        });
        info
    }
    #[cfg(not(windows))]
    {
        Ok(SystemInfo {
            cpu_name: "Unknown".into(),
            cpu_cores: 0,
            cpu_threads: 0,
            cpu_usage: 0.0,
            gpu_name: "Unknown".into(),
            gpu_usage: 0.0,
            vram_used_mb: 0.0,
            ram_total_gb: 0.0,
            ram_used_gb: 0.0,
            os_version: "".into(),
        })
    }
}
