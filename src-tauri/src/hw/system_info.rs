use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock};

// ── CPU usage via PDH ─────────────────────────────────────────────────────────
//
// We use "\Processor Information(_Total)\% Processor Time", which measures the
// proportion of time the scheduler finds the CPU busy (non-idle).  This matches
// the intuitive number shown in Windows Task Manager Performance tab.
//
// NOTE: We deliberately do NOT use "% Processor Utility" because on Intel hybrid
// architectures (Panther Lake, Meteor Lake, Arrow Lake — all with P-cores +
// E-cores + LP-E-cores), that counter normalises by each core's max-turbo
// frequency.  When LP-E-cores run background OS tasks at moderate clock speeds,
// their per-core "utility" contribution inflates the _Total aggregate far above
// the scheduling-based percentage — producing readings like 38% when Task Manager
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
    unsafe {
        use libloading::{Library, Symbol};
        type FnOpenQuery =
            unsafe extern "system" fn(*const std::ffi::c_void, usize, *mut isize) -> u32;
        type FnAddCounter = unsafe extern "system" fn(isize, *const u16, usize, *mut isize) -> u32;
        type FnCollect = unsafe extern "system" fn(isize) -> u32;
        type FnGetValue = unsafe extern "system" fn(isize, u32, *mut u32, *mut u8) -> u32;
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
        let get_val: Symbol<'static, FnGetValue> = match lib.get(b"PdhGetFormattedCounterValue\0") {
            Ok(f) => f,
            Err(_) => return,
        };
        let close_q: Symbol<'static, FnClose> = match lib.get(b"PdhCloseQuery\0") {
            Ok(f) => f,
            Err(_) => return,
        };

        let mut query: isize = 0;
        if open_q(std::ptr::null(), 0, &mut query) != 0 {
            return;
        }

        // Primary: Processor Information provider (Vista+) — scheduling-based %.
        let path: Vec<u16> = "\\Processor Information(_Total)\\% Processor Time\0"
            .encode_utf16()
            .collect();
        let mut counter: isize = 0;
        if add_c(query, path.as_ptr(), 0, &mut counter) != 0 {
            // Fallback: legacy Processor provider (XP-era, always available)
            let path2: Vec<u16> = "\\Processor(_Total)\\% Processor Time\0"
                .encode_utf16()
                .collect();
            if add_c(query, path2.as_ptr(), 0, &mut counter) != 0 {
                close_q(query);
                return;
            }
        }

        collect(query); // baseline for rate counter

        // PDH_FMT_COUNTERVALUE (x64): CStatus u32 (4b) | pad (4b) | doubleValue f64 (8b)
        const PDH_FMT_DOUBLE: u32 = 0x00000200;
        let mut val_buf = [0u8; 16];
        let mut dummy_type: u32 = 0;

        loop {
            std::thread::sleep(std::time::Duration::from_millis(1500));
            collect(query);
            if get_val(
                counter,
                PDH_FMT_DOUBLE,
                &mut dummy_type,
                val_buf.as_mut_ptr(),
            ) != 0
            {
                continue;
            }
            let c_status = u32::from_ne_bytes(val_buf[0..4].try_into().unwrap_or([1; 4]));
            if c_status > 1 {
                continue;
            }
            let pct = f64::from_ne_bytes(val_buf[8..16].try_into().unwrap_or([0; 8]));
            if pct.is_finite() {
                if let Some(cache) = CPU_USAGE_CACHE.get() {
                    if let Ok(mut g) = cache.lock() {
                        *g = pct.clamp(0.0, 100.0);
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
    unsafe {
        use libloading::{Library, Symbol};

        // Function types from pdh.dll
        type FnOpenQuery =
            unsafe extern "system" fn(*const std::ffi::c_void, usize, *mut isize) -> u32;
        type FnAddCounter = unsafe extern "system" fn(isize, *const u16, usize, *mut isize) -> u32;
        type FnCollect = unsafe extern "system" fn(isize) -> u32;
        type FnGetArray = unsafe extern "system" fn(isize, u32, *mut u32, *mut u32, *mut u8) -> u32;
        type FnClose = unsafe extern "system" fn(isize) -> u32;

        // Box::leak keeps the library loaded for the process lifetime (this
        // thread never exits, so the lib is always needed).
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
        let get_arr: Symbol<'static, FnGetArray> = match lib.get(b"PdhGetFormattedCounterArrayW\0")
        {
            Ok(f) => f,
            Err(_) => return,
        };
        let close_q: Symbol<'static, FnClose> = match lib.get(b"PdhCloseQuery\0") {
            Ok(f) => f,
            Err(_) => return,
        };

        let mut query: isize = 0;
        if open_q(std::ptr::null(), 0, &mut query) != 0 {
            return;
        }

        // Use all GPU Engine instances (*) and filter for "3D" in the loop below
        let path: Vec<u16> = "\\GPU Engine(*)\\Utilization Percentage\0"
            .encode_utf16()
            .collect();
        let mut counter: isize = 0;
        if add_c(query, path.as_ptr(), 0, &mut counter) != 0 {
            close_q(query);
            return;
        }

        // First collection establishes the baseline for rate counters
        collect(query);

        const PDH_FMT_DOUBLE: u32 = 0x00000200;
        const PDH_MORE_DATA: u32 = 0x800007D2;
        // PDH_FMT_COUNTERVALUE_ITEM_W layout on x64:
        //   offset  0 : szName     LPWSTR (pointer, 8 bytes)
        //   offset  8 : CStatus    DWORD  (4 bytes)
        //   offset 12 : _pad               (4 bytes alignment)
        //   offset 16 : doubleValue f64    (8 bytes)
        //   total   24 bytes
        const ITEM_SIZE: usize = 24;

        loop {
            std::thread::sleep(std::time::Duration::from_millis(1500));
            collect(query);

            // First call: get required buffer size.
            // PdhGetFormattedCounterArrayW signature:
            //   (hCounter, dwFormat, lpdwBufferSize, lpdwItemCount, ItemBuffer)
            let mut buf_size: u32 = 0; // lpdwBufferSize — receives required bytes
            let mut item_count: u32 = 0; // lpdwItemCount  — receives number of items
            let st = get_arr(
                counter,
                PDH_FMT_DOUBLE,
                &mut buf_size,
                &mut item_count,
                std::ptr::null_mut(),
            );
            if st != PDH_MORE_DATA || buf_size == 0 || item_count == 0 {
                continue;
            }

            // Second call: fill the buffer (buf_size bytes allocated).
            let mut buf: Vec<u8> = vec![0u8; buf_size as usize];
            if get_arr(
                counter,
                PDH_FMT_DOUBLE,
                &mut buf_size,
                &mut item_count,
                buf.as_mut_ptr(),
            ) != 0
            {
                continue;
            }

            let n = item_count as usize;
            if buf.len() < n * ITEM_SIZE {
                continue;
            }

            let buf_base = buf.as_ptr() as usize;
            let mut total = 0.0f64;
            for i in 0..n {
                let base = i * ITEM_SIZE;
                let c_status =
                    u32::from_ne_bytes(buf[base + 8..base + 12].try_into().unwrap_or([0xff; 4]));
                // Accept valid (0) or new (1) data
                if c_status > 1 {
                    continue;
                }
                let val =
                    f64::from_ne_bytes(buf[base + 16..base + 24].try_into().unwrap_or([0; 8]));
                if !val.is_finite() || val < 0.0 {
                    continue;
                }
                // Filter: only 3D-engine instances.
                // PDH \GPU Engine(*)\Utilization Percentage has one instance per
                // (PID × LUID × phys × eng × engtype).  Summing all PID instances
                // for the same physical engine gives the correct total scheduling
                // time for that engine \u2014 equivalent to Task Manager's "GPU" column.
                // Each process gets a time-slice on the shared hardware engine; the
                // sum of their slices = total engine busy time.  Values are clamped
                // to 100 below so simultaneous use across engines never over-reports.
                let name_ptr =
                    usize::from_ne_bytes(buf[base..base + 8].try_into().unwrap_or([0; 8]));
                let contains_3d = if name_ptr >= buf_base && name_ptr + 2 <= buf_base + buf.len() {
                    let off = name_ptr - buf_base;
                    let wchars: Vec<u16> = buf[off..]
                        .chunks_exact(2)
                        .take(256)
                        .map(|c| u16::from_ne_bytes([c[0], c[1]]))
                        .take_while(|&w| w != 0)
                        .collect();
                    String::from_utf16_lossy(&wchars).contains("3D")
                } else {
                    false
                };
                if contains_3d {
                    total += val;
                }
            }

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

pub fn get_system_info() -> Result<SystemInfo> {
    #[cfg(windows)]
    {
        use std::collections::HashMap;
        use wmi::{COMLibrary, WMIConnection};

        // Start the PDH pollers on first call (no-op on subsequent calls)
        ensure_gpu_poller();
        ensure_cpu_poller();

        let com = COMLibrary::new().context("COM init")?;
        let wmi = WMIConnection::new(com.into()).context("WMI connect")?;

        // ── Static CPU identity (name, cores, threads) ────────────────────
        let cpus: Vec<HashMap<String, wmi::Variant>> = wmi
            .raw_query("SELECT Name, NumberOfCores, NumberOfLogicalProcessors FROM Win32_Processor")
            .unwrap_or_default();
        let cpu = cpus.into_iter().next().unwrap_or_default();

        // ── CPU usage — PDH "% Processor Utility" matches Task Manager ──────
        // Unlike WMI Win32_PerfFormattedData_PerfOS_Processor which reports raw
        // idle-vs-busy time, % Processor Utility accounts for P-state frequency
        // scaling, matching the value shown in Windows 11 Task Manager.
        let cpu_usage = read_cpu_usage();

        // ── GPU name ──────────────────────────────────────────────────────
        let gpu_query: Vec<HashMap<String, wmi::Variant>> = wmi
            .raw_query("SELECT Name FROM Win32_VideoController")
            .unwrap_or_default();
        let gpu_name = gpu_query
            .into_iter()
            .next()
            .and_then(|r| match r.get("Name") {
                Some(wmi::Variant::String(s)) => Some(s.trim().to_string()),
                _ => None,
            })
            .unwrap_or_else(|| "Unknown GPU".to_string());

        // ── GPU 3D engine utilization (from PDH background poller) ───────────
        // The poller maintains a persistent PDH query and writes the latest
        // value to GPU_USAGE_CACHE every ~1.5 s.  Reading from cache is instant
        // and avoids the WMI first-sample = 0 issue.
        let gpu_usage = read_gpu_usage();

        // ── Dedicated VRAM used ───────────────────────────────────────────
        let vram_q: Vec<HashMap<String, wmi::Variant>> = wmi
            .raw_query(
                "SELECT DedicatedUsage FROM Win32_PerfFormattedData_GPUAdapterMemory_GPUAdapter",
            )
            .unwrap_or_default();
        let vram_used_mb = vram_q
            .first()
            .and_then(|r| r.get("DedicatedUsage"))
            .map(|v| match v {
                wmi::Variant::UI8(n) => *n as f64 / (1024.0 * 1024.0),
                wmi::Variant::UI4(n) => *n as f64 / (1024.0 * 1024.0),
                _ => 0.0,
            })
            .unwrap_or(0.0);

        // ── Physical memory total ─────────────────────────────────────────
        let mem_query: Vec<HashMap<String, wmi::Variant>> = wmi
            .raw_query("SELECT Capacity FROM Win32_PhysicalMemory")
            .unwrap_or_default();
        let ram_total_bytes: u64 = mem_query
            .iter()
            .filter_map(|row| match row.get("Capacity") {
                Some(wmi::Variant::String(s)) => s.parse::<u64>().ok(),
                Some(wmi::Variant::UI8(v)) => Some(*v),
                _ => None,
            })
            .sum();

        // ── OS info + available (free) memory ─────────────────────────────
        let os_info: Vec<HashMap<String, wmi::Variant>> = wmi
            .raw_query("SELECT Caption, FreePhysicalMemory, TotalVisibleMemorySize FROM Win32_OperatingSystem")
            .unwrap_or_default();
        let os_row = os_info.into_iter().next().unwrap_or_default();

        let cpu_name = match cpu.get("Name") {
            Some(wmi::Variant::String(s)) => s.trim().to_string(),
            _ => "Unknown CPU".to_string(),
        };
        let cpu_cores = match cpu.get("NumberOfCores") {
            Some(wmi::Variant::UI4(v)) => *v,
            _ => 0,
        };
        let cpu_threads = match cpu.get("NumberOfLogicalProcessors") {
            Some(wmi::Variant::UI4(v)) => *v,
            _ => 0,
        };
        let ram_total_gb = ram_total_bytes as f64 / (1024.0 * 1024.0 * 1024.0);

        let free_kb = match os_row.get("FreePhysicalMemory") {
            Some(wmi::Variant::UI8(v)) => *v,
            Some(wmi::Variant::String(s)) => s.parse().unwrap_or(0),
            _ => 0,
        };
        let total_kb = match os_row.get("TotalVisibleMemorySize") {
            Some(wmi::Variant::UI8(v)) => *v,
            Some(wmi::Variant::String(s)) => s.parse().unwrap_or(0),
            _ => ram_total_bytes / 1024,
        };
        let used_kb = total_kb.saturating_sub(free_kb);
        let ram_used_gb = used_kb as f64 / (1024.0 * 1024.0);

        let os_version = match os_row.get("Caption") {
            Some(wmi::Variant::String(s)) => s.trim().to_string(),
            _ => "Windows 11".to_string(),
        };

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
