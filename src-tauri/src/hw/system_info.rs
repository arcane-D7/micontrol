use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SystemInfo {
    pub cpu_name: String,
    pub cpu_cores: u32,
    pub cpu_threads: u32,
    pub cpu_usage: f64,
    pub gpu_name: String,
    pub ram_total_gb: f64,
    pub ram_used_gb: f64,
    pub os_version: String,
}

pub fn get_system_info() -> Result<SystemInfo> {
    #[cfg(windows)]
    {
        use wmi::{COMLibrary, WMIConnection};
        use std::collections::HashMap;

        let com = COMLibrary::new().context("COM init")?;
        let wmi = WMIConnection::new(com.into()).context("WMI connect")?;

        let cpus: Vec<HashMap<String, wmi::Variant>> = wmi
            .raw_query("SELECT Name, NumberOfCores, NumberOfLogicalProcessors, LoadPercentage FROM Win32_Processor")
            .unwrap_or_default();
        let cpu = cpus.into_iter().next().unwrap_or_default();

        let gpu_query: Vec<HashMap<String, wmi::Variant>> = wmi
            .raw_query("SELECT Name FROM Win32_VideoController")
            .unwrap_or_default();
        let gpu = gpu_query.into_iter().next().unwrap_or_default();

        let mem_query: Vec<HashMap<String, wmi::Variant>> = wmi
            .raw_query("SELECT Capacity FROM Win32_PhysicalMemory")
            .unwrap_or_default();

        let ram_total_bytes: u64 = mem_query.iter().filter_map(|row| {
            match row.get("Capacity") {
                Some(wmi::Variant::String(s)) => s.parse::<u64>().ok(),
                Some(wmi::Variant::UI8(v)) => Some(*v),
                _ => None,
            }
        }).sum();

        let os_query: Vec<HashMap<String, wmi::Variant>> = wmi
            .raw_query("SELECT Caption FROM Win32_OperatingSystem")
            .unwrap_or_default();

        // Free physical memory from Win32_OperatingSystem
        let os_info: Vec<HashMap<String, wmi::Variant>> = wmi
            .raw_query("SELECT FreePhysicalMemory, TotalVisibleMemorySize FROM Win32_OperatingSystem")
            .unwrap_or_default();
        let os_row = os_info.into_iter().next().unwrap_or_default();

        let cpu_name = match cpu.get("Name") {
            Some(wmi::Variant::String(s)) => s.trim().to_string(),
            _ => "Unknown CPU".to_string(),
        };
        let cpu_cores = match cpu.get("NumberOfCores") { Some(wmi::Variant::UI4(v)) => *v, _ => 0 };
        let cpu_threads = match cpu.get("NumberOfLogicalProcessors") { Some(wmi::Variant::UI4(v)) => *v, _ => 0 };
        let cpu_usage = match cpu.get("LoadPercentage") { Some(wmi::Variant::UI2(v)) => *v as f64, Some(wmi::Variant::UI4(v)) => *v as f64, _ => 0.0 };
        let gpu_name = match gpu.get("Name") {
            Some(wmi::Variant::String(s)) => s.trim().to_string(),
            _ => "Unknown GPU".to_string(),
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

        let os_version = match os_query.first().and_then(|r| r.get("Caption")) {
            Some(wmi::Variant::String(s)) => s.trim().to_string(),
            _ => "Windows 11".to_string(),
        };

        Ok(SystemInfo {
            cpu_name,
            cpu_cores,
            cpu_threads,
            cpu_usage,
            gpu_name,
            ram_total_gb,
            ram_used_gb,
            os_version,
        })
    }
    #[cfg(not(windows))]
    {
        Ok(SystemInfo {
            cpu_name: "Intel Core Ultra 5 125H".to_string(),
            cpu_cores: 14,
            cpu_threads: 18,
            cpu_usage: 12.5,
            gpu_name: "Intel Arc Graphics".to_string(),
            ram_total_gb: 16.0,
            ram_used_gb: 6.2,
            os_version: "Windows 11 23H2".to_string(),
        })
    }
}
