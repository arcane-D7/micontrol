//! Per-process resource usage snapshot.
//!
//! Data source: `Win32_PerfFormattedData_PerfProc_Process` — the same cooked
//! counter that Task Manager uses.  Fields are automatically normalized by
//! the WMI performance-counter infrastructure, so a single WMI query gives
//! the current rate without needing a two-sample delta.
//!
//! CPU% is divided by the number of logical processors to give a 0-100 value
//! that matches the Task Manager "CPU" column.

use serde::{Deserialize, Serialize};

#[cfg(windows)]
use std::sync::OnceLock;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProcessInfo {
    pub name: String,
    pub pid: u32,
    /// Normalized to 0-100 (Task Manager style).
    pub cpu_percent: f64,
    /// Working set (physical RAM) in MiB.
    pub memory_mb: f64,
}

/// Cached number of logical processors (never changes at runtime).
#[cfg(windows)]
static CPU_LOGICAL_PROCESSORS: OnceLock<f64> = OnceLock::new();

/// Return up to 20 processes sorted by CPU usage descending.
/// Excludes the pseudo-process entries "_Total" and "Idle".
pub fn get_process_list() -> Vec<ProcessInfo> {
    #[cfg(windows)]
    {
        use crate::hw::wmi_cache;
        use crate::util::wmi_extract;
        use std::collections::HashMap;

        let logical_cpus = *CPU_LOGICAL_PROCESSORS.get_or_init(|| {
            // Number of logical processors — used to normalize CPU%.
            // This value is static and cached forever.
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

        let rows: Vec<HashMap<String, wmi::Variant>> = match wmi_cache::with_cimv2(|wmi| {
            Ok(wmi
                .raw_query(
                    "SELECT Name, IDProcess, PercentProcessorTime, WorkingSet \
                 FROM Win32_PerfFormattedData_PerfProc_Process",
                )
                .unwrap_or_default())
        }) {
            Ok(rows) => rows,
            Err(e) => {
                log::warn!("WMI process query failed: {e}");
                return vec![];
            }
        };

        let mut procs: Vec<ProcessInfo> = rows
            .into_iter()
            .filter_map(|row| {
                let name = wmi_extract::extract_string(&row, "Name")?;
                // Skip pseudo-processes
                if name == "_Total" || name == "Idle" {
                    return None;
                }
                let pid = wmi_extract::extract_u32_or(&row, "IDProcess", 0);
                // PercentProcessorTime is across all CPUs (0 – logical_cpus * 100).
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

                Some(ProcessInfo {
                    name,
                    pid,
                    cpu_percent,
                    memory_mb,
                })
            })
            .collect();

        procs.sort_by(|a, b| {
            b.cpu_percent
                .partial_cmp(&a.cpu_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        procs.truncate(20);
        procs
    }
    #[cfg(not(windows))]
    {
        vec![]
    }
}
