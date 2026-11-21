//! Overview snapshot — the single data source for the "Visão Geral" tab.
//!
//! The Overview is meant to be a *summary of the whole application*, so it
//! needs more than CPU% + GPU%: identity (model/SKU/BIOS), silicon specs
//! (cores, cache, clocks), the NPU, memory modules, storage, panel mode and
//! OS build/uptime. None of that changes while the app runs, so it is read
//! **once** and cached in a [`OnceLock`]; only the genuinely live values
//! (free disk space, uptime) are re-read per call and they cost microseconds.
//!
//! ## Why a separate module instead of widening `get_system_info`
//!
//! `get_system_info` is polled every 2 s by the fast tier. `Win32_Processor`
//! alone measured **1053 ms** on this host (first hit), and the storage
//! namespace connection ~580 ms. Putting those behind the hot path would
//! stall the CPU/GPU readout. Here they land in a one-shot initialisation on a
//! blocking thread, and every later call is a clone of cached strings.
//!
//! ## Honesty rules applied here
//!
//! * Anything the platform does not expose is `Option::None` — never `0`.
//!   NPU TOPS is not queried at all: Windows has no API for it, so the UI
//!   renders `—` with a hint instead of a datasheet number pretending to be a
//!   reading.
//! * `Win32_DiskDrive.InterfaceType` reports `SCSI` for every NVMe drive
//!   (storport), so the bus comes from `MSFT_PhysicalDisk.BusType`, which is
//!   the only class that actually knows.
//! * `Win32_Processor.CurrentClockSpeed` is the *rated* clock on this
//!   firmware (1900 MHz idle or loaded), so it is surfaced as `base_clock_mhz`
//!   and labelled "base" in the UI, not as a live frequency.

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// Physical machine identity (SMBIOS + BIOS).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeviceSpec {
    pub manufacturer: String,
    pub model: String,
    pub family: Option<String>,
    /// Xiaomi/ODM SKU string (e.g. `TM2424-77079`).
    pub sku: Option<String>,
    pub bios_version: Option<String>,
}

/// Processor specifications.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CpuSpec {
    pub name: String,
    pub cores: u32,
    pub threads: u32,
    /// Rated (nominal) clock from SMBIOS. Not a live frequency — see module doc.
    pub base_clock_mhz: Option<u32>,
    pub l3_cache_mb: Option<u32>,
    pub socket: Option<String>,
}

/// Primary display adapter.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GpuSpec {
    pub name: String,
    pub driver_version: Option<String>,
}

/// Neural processing unit, read from the `ComputeAccelerator` device class.
///
/// `None` on the whole struct means *no NPU was detected*; individual `None`
/// fields mean the device exists but that attribute is not exposed.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NpuSpec {
    pub name: String,
    pub vendor: Option<String>,
    pub driver_version: Option<String>,
    /// `YYYY-MM-DD`, normalised from the driver INF record.
    pub driver_date: Option<String>,
    /// `ok` | `error` | `disabled` — from the class instance's `ConfigFlags`.
    pub status: String,
    pub device_id: Option<String>,
}

/// Installed memory: total plus what the modules report about themselves.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct MemorySpec {
    pub total_gb: f64,
    pub module_count: u32,
    /// Configured speed in MT/s (`Win32_PhysicalMemory.ConfiguredClockSpeed`).
    pub speed_mt_s: Option<u32>,
    /// SMBIOS memory-type label (`DDR5`, `LPDDR5`, …).
    pub memory_type: Option<String>,
    pub manufacturer: Option<String>,
}

/// The system volume plus the physical drive behind it.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StorageSpec {
    pub model: String,
    pub capacity_gb: f64,
    /// Free bytes on the system volume, re-read per call.
    pub free_gb: f64,
    pub system_drive: String,
    /// `NVMe` | `SATA` | `USB` … from `MSFT_PhysicalDisk.BusType`.
    pub bus: Option<String>,
    /// `SSD` | `HDD` | `SCM` from `MSFT_PhysicalDisk.MediaType`.
    pub media: Option<String>,
    /// `Healthy` | `Warning` | `Unhealthy` | `Unknown`.
    pub health: Option<String>,
}

/// Active panel mode.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DisplaySpec {
    pub width: u32,
    pub height: u32,
    pub refresh_hz: u32,
    pub name: Option<String>,
}

/// OS identity and uptime.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OsSpec {
    pub caption: String,
    pub build: String,
    pub arch: String,
    pub uptime_secs: u64,
}

/// Everything the Overview tab needs that is not already in `useHardware`'s
/// per-tab state (usage, temperatures, battery and display toggles come from
/// the existing commands and keep their own poll cadence).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OverviewSnapshot {
    pub device: DeviceSpec,
    pub cpu: CpuSpec,
    pub gpu: GpuSpec,
    pub npu: Option<NpuSpec>,
    pub memory: MemorySpec,
    pub storage: StorageSpec,
    pub display: DisplaySpec,
    pub os: OsSpec,
    /// Epoch milliseconds when this snapshot was assembled.
    pub captured_at_ms: u64,
}

// ── Pure helpers (unit-tested) ───────────────────────────────────────────────

/// Resolve an INF indirection such as `@oem95.inf,%npu_b03e%;Intel(R) NPU`.
///
/// Device `DeviceDesc` values are frequently indirected through the INF that
/// installed the driver. The localised string sits *after the last `;`*; the
/// `%token%` before it cannot be resolved without the INF file, so a value
/// that has no fallback is treated as unusable rather than shown raw.
pub fn resolve_inf_string(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some((_, fallback)) = trimmed.rsplit_once(';') {
        let fallback = fallback.trim();
        return if fallback.is_empty() {
            None
        } else {
            Some(fallback.to_string())
        };
    }
    // No indirection — must not still be a `%token%` placeholder.
    if trimmed.starts_with('%') && trimmed.ends_with('%') {
        return None;
    }
    if trimmed.starts_with('@') {
        // `@file.inf,-1234` style with no fallback: nothing human-readable.
        return None;
    }
    Some(trimmed.to_string())
}

/// Map an SMBIOS `MemoryType` code to a label.
///
/// Only the codes that can appear on a machine this app targets are mapped;
/// anything else returns `None` so the UI says "—" instead of guessing.
pub fn memory_type_label(code: u32) -> Option<&'static str> {
    Some(match code {
        20 => "DDR",
        21 => "DDR2",
        22 => "DDR2 FB-DIMM",
        24 => "DDR3",
        26 => "DDR4",
        27 => "LPDDR",
        28 => "LPDDR2",
        29 => "LPDDR3",
        30 => "LPDDR4",
        34 => "DDR5",
        35 => "LPDDR5",
        36 => "HBM3",
        _ => return None,
    })
}

/// Map `MSFT_PhysicalDisk.BusType` to a label.
pub fn bus_type_label(code: u32) -> Option<&'static str> {
    Some(match code {
        1 => "SCSI",
        2 => "ATAPI",
        3 => "ATA",
        4 => "IEEE 1394",
        8 => "RAID",
        9 => "iSCSI",
        10 => "SAS",
        11 => "SATA",
        12 => "SD",
        13 => "MMC",
        17 => "NVMe",
        18 => "SCM",
        19 => "UFS",
        _ => return None,
    })
}

/// Map `MSFT_PhysicalDisk.MediaType` to a label.
pub fn media_type_label(code: u32) -> Option<&'static str> {
    Some(match code {
        3 => "HDD",
        4 => "SSD",
        5 => "SCM",
        _ => return None,
    })
}

/// Map `MSFT_PhysicalDisk.HealthStatus` to a label.
pub fn health_status_label(code: u32) -> Option<&'static str> {
    Some(match code {
        0 => "Healthy",
        1 => "Warning",
        2 => "Unhealthy",
        5 => "Unknown",
        _ => return None,
    })
}

/// Normalise the registry driver date (`7-24-2026` / `7/24/2026`) to ISO.
pub fn normalize_driver_date(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parts: Vec<&str> = trimmed
        .split(['-', '/'])
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .collect();
    if parts.len() != 3 {
        return None;
    }
    let month: u32 = parts[0].parse().ok()?;
    let day: u32 = parts[1].parse().ok()?;
    let year: u32 = parts[2].parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || !(1900..=2200).contains(&year) {
        return None;
    }
    Some(format!("{year:04}-{month:02}-{day:02}"))
}

/// Map a device `ConfigFlags` value to a coarse status.
///
/// `CONFIGFLAG_DISABLED` (0x1) and `CONFIGFLAG_FAILEDINSTALL` (0x40) are the
/// only two flags that mean "present but not working".
pub fn device_status_from_config_flags(flags: u32) -> &'static str {
    const CONFIGFLAG_DISABLED: u32 = 0x0000_0001;
    const CONFIGFLAG_FAILEDINSTALL: u32 = 0x0000_0040;
    if flags & (CONFIGFLAG_DISABLED | CONFIGFLAG_FAILEDINSTALL) != 0 {
        "error"
    } else {
        "ok"
    }
}

/// Split an uptime in seconds into `(days, hours, minutes)`.
pub fn split_uptime(total_secs: u64) -> (u64, u64, u64) {
    let days = total_secs / 86_400;
    let rem = total_secs % 86_400;
    (days, rem / 3_600, (rem % 3_600) / 60)
}

/// Bytes → GiB, rounded to one decimal in the caller.
pub fn bytes_to_gib(bytes: u64) -> f64 {
    bytes as f64 / (1024.0 * 1024.0 * 1024.0)
}

/// Read a numeric WMI field that some providers return as a string.
fn num_u64(row: &std::collections::HashMap<String, wmi::Variant>, key: &str) -> Option<u64> {
    match row.get(key) {
        Some(wmi::Variant::String(s)) => s.trim().parse::<u64>().ok(),
        _ => crate::util::wmi_extract::extract_u64(row, key),
    }
}

fn num_u32(row: &std::collections::HashMap<String, wmi::Variant>, key: &str) -> Option<u32> {
    match row.get(key) {
        Some(wmi::Variant::String(s)) => s.trim().parse::<u32>().ok(),
        _ => crate::util::wmi_extract::extract_u32(row, key),
    }
}

// ── Static cache ────────────────────────────────────────────────────────────

#[derive(Default)]
struct StaticSpecs {
    device: DeviceSpec,
    cpu: CpuSpec,
    gpu: GpuSpec,
    npu: Option<NpuSpec>,
    memory: MemorySpec,
    storage_model: String,
    storage_capacity_gb: f64,
    storage_bus: Option<String>,
    storage_media: Option<String>,
    storage_health: Option<String>,
    display_width: u32,
    display_height: u32,
    display_refresh: u32,
    display_name: Option<String>,
    os_caption: String,
    os_build: String,
    os_arch: String,
}

static STATIC_SPECS: OnceLock<StaticSpecs> = OnceLock::new();

/// The Overview snapshot. Cached statics + live free-space/uptime.
pub fn get_overview_snapshot() -> crate::hw::errors::HardwareResult<OverviewSnapshot> {
    let specs = STATIC_SPECS.get_or_init(load_static_specs);

    let (free_bytes, total_bytes) = system_drive_space();
    // Prefer the live total when the drive manager did not answer: the
    // capacity from `Win32_DiskDrive` is the *physical* disk and can exceed
    // the volume (recovery partitions).
    let capacity_gb = if total_bytes > 0 {
        bytes_to_gib(total_bytes)
    } else {
        specs.storage_capacity_gb
    };
    let free_gb = bytes_to_gib(free_bytes);

    Ok(OverviewSnapshot {
        device: specs.device.clone(),
        cpu: specs.cpu.clone(),
        gpu: specs.gpu.clone(),
        npu: specs.npu.clone(),
        memory: specs.memory.clone(),
        storage: StorageSpec {
            model: specs.storage_model.clone(),
            capacity_gb,
            free_gb,
            system_drive: system_drive_letter(),
            bus: specs.storage_bus.clone(),
            media: specs.storage_media.clone(),
            health: specs.storage_health.clone(),
        },
        display: DisplaySpec {
            width: specs.display_width,
            height: specs.display_height,
            refresh_hz: specs.display_refresh,
            name: specs.display_name.clone(),
        },
        os: OsSpec {
            caption: specs.os_caption.clone(),
            build: specs.os_build.clone(),
            arch: specs.os_arch.clone(),
            uptime_secs: uptime_secs(),
        },
        captured_at_ms: now_epoch_ms(),
    })
}

fn load_static_specs() -> StaticSpecs {
    #[cfg(windows)]
    let mut out = load_static_specs_windows();
    #[cfg(not(windows))]
    let mut out = StaticSpecs::default();

    // NPU is registry-only: no WMI round-trip, ~1 ms, and it survives the
    // elevated-bridge outages that make the WMI classes unavailable.
    out.npu = read_npu();

    out
}

/// Every `Win32_*` class in one cimv2 pass, plus the storage namespace.
///
/// Each query is independent: a failure in one leaves that field empty instead
/// of sinking the card. Static for the lifetime of the process, so the cost
/// (`Win32_Processor` alone measured 1053 ms on this host) is paid once.
#[cfg(windows)]
fn load_static_specs_windows() -> StaticSpecs {
    use crate::hw::wmi_cache;
    use crate::util::wmi_extract;
    use std::collections::HashMap;

    // `with_cimv2` takes an `Fn`, so the accumulator is built inside the
    // closure and returned; the outer `out` only receives the result.
    let mut out = wmi_cache::with_cimv2(|wmi| -> anyhow::Result<StaticSpecs> {
            let mut s = StaticSpecs::default();
            let query = |class: &str, fields: &str| -> Vec<HashMap<String, wmi::Variant>> {
                let q = format!("SELECT {fields} FROM {class}");
                match wmi.raw_query(&q) {
                    Ok(rows) => rows,
                    Err(e) => {
                        log::debug!(target: "hw::overview", "{class} query failed: {e}");
                        Vec::new()
                    }
                }
            };

            // ── Identity ────────────────────────────────────────────────────
            if let Some(row) = query(
                "Win32_ComputerSystem",
                "Manufacturer, Model, SystemFamily, SystemSKUNumber",
            )
            .into_iter()
            .next()
            {
                s.device.manufacturer =
                    wmi_extract::extract_string(&row, "Manufacturer").unwrap_or_default();
                s.device.model =
                    wmi_extract::extract_string(&row, "Model").unwrap_or_default();
                s.device.family = wmi_extract::extract_string(&row, "SystemFamily")
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                s.device.sku = wmi_extract::extract_string(&row, "SystemSKUNumber")
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
            }
            if let Some(row) = query("Win32_BIOS", "SMBIOSBIOSVersion").into_iter().next() {
                s.device.bios_version = wmi_extract::extract_string(&row, "SMBIOSBIOSVersion")
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
            }

            // ── CPU ─────────────────────────────────────────────────────────
            if let Some(row) = query(
                "Win32_Processor",
                "Name, NumberOfCores, NumberOfLogicalProcessors, MaxClockSpeed, L3CacheSize, SocketDesignation",
            )
            .into_iter()
            .next()
            {
                s.cpu.name = wmi_extract::extract_string(&row, "Name")
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();
                s.cpu.cores = num_u32(&row, "NumberOfCores").unwrap_or(0);
                s.cpu.threads = num_u32(&row, "NumberOfLogicalProcessors").unwrap_or(0);
                s.cpu.base_clock_mhz = num_u32(&row, "MaxClockSpeed").filter(|v| *v > 0);
                s.cpu.l3_cache_mb = num_u32(&row, "L3CacheSize")
                    .filter(|v| *v > 0)
                    .map(|kb| (kb as f64 / 1024.0).round() as u32);
                s.cpu.socket = wmi_extract::extract_string(&row, "SocketDesignation")
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
            }

            // ── GPU + panel mode ────────────────────────────────────────────
            if let Some(row) = query(
                "Win32_VideoController",
                "Name, DriverVersion, CurrentHorizontalResolution, CurrentVerticalResolution, CurrentRefreshRate",
            )
            .into_iter()
            .next()
            {
                s.gpu.name = wmi_extract::extract_string(&row, "Name")
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();
                s.gpu.driver_version = wmi_extract::extract_string(&row, "DriverVersion")
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                s.display_width = num_u32(&row, "CurrentHorizontalResolution").unwrap_or(0);
                s.display_height = num_u32(&row, "CurrentVerticalResolution").unwrap_or(0);
                s.display_refresh = num_u32(&row, "CurrentRefreshRate").unwrap_or(0);
            }
            if let Some(row) = query("Win32_DesktopMonitor", "Name").into_iter().next() {
                s.display_name = wmi_extract::extract_string(&row, "Name")
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
            }

            // ── Memory ──────────────────────────────────────────────────────
            let modules = query(
                "Win32_PhysicalMemory",
                "Capacity, ConfiguredClockSpeed, Speed, SMBIOSMemoryType, Manufacturer",
            );
            if !modules.is_empty() {
                s.memory.module_count = modules.len() as u32;
                let total: u64 = modules.iter().filter_map(|r| num_u64(r, "Capacity")).sum();
                s.memory.total_gb = bytes_to_gib(total);
                // Modules are usually identical; report the fastest configured
                // speed and the first recognised type.
                s.memory.speed_mt_s = modules
                    .iter()
                    .filter_map(|r| {
                        num_u32(r, "ConfiguredClockSpeed").or_else(|| num_u32(r, "Speed"))
                    })
                    .filter(|v| *v > 0)
                    .max();
                s.memory.memory_type = modules
                    .iter()
                    .filter_map(|r| num_u32(r, "SMBIOSMemoryType"))
                    .find_map(memory_type_label)
                    .map(str::to_string);
                s.memory.manufacturer = modules
                    .iter()
                    .filter_map(|r| wmi_extract::extract_string(r, "Manufacturer"))
                    .map(|s| s.trim().to_string())
                    .find(|s| !s.is_empty() && !s.eq_ignore_ascii_case("unknown"));
            }

            // ── Storage identity ────────────────────────────────────────────
            if let Some(row) = query("Win32_DiskDrive", "Model, Size").into_iter().next() {
                s.storage_model = wmi_extract::extract_string(&row, "Model")
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();
                s.storage_capacity_gb = num_u64(&row, "Size").map(bytes_to_gib).unwrap_or(0.0);
            }

            // ── OS ──────────────────────────────────────────────────────────
            if let Some(row) = query(
                "Win32_OperatingSystem",
                "Caption, BuildNumber, OSArchitecture",
            )
            .into_iter()
            .next()
            {
                s.os_caption = wmi_extract::extract_string(&row, "Caption")
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();
                s.os_build = wmi_extract::extract_string(&row, "BuildNumber")
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();
                s.os_arch = wmi_extract::extract_string(&row, "OSArchitecture")
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();
            }

            Ok(s)
        })
        .unwrap_or_default();

    // ── Storage class (separate namespace, own connection) ──────────────
    // `MSFT_PhysicalDisk` is the only class that reports the *bus*
    // (Win32_DiskDrive says "SCSI" for every NVMe device) and the drive's
    // own health. Best-effort: a machine without the Storage provider just
    // loses those two rows.
    if let Some((bus, media, health)) = read_physical_disk_traits() {
        out.storage_bus = bus;
        out.storage_media = media;
        out.storage_health = health;
    }

    out
}

/// `(BusType, MediaType, HealthStatus)` of the first physical disk.
#[cfg(windows)]
fn read_physical_disk_traits() -> Option<(Option<String>, Option<String>, Option<String>)> {
    use crate::util::wmi_extract;
    use std::collections::HashMap;
    use wmi::{COMLibrary, WMIConnection};

    let com = COMLibrary::without_security().ok()?;
    let conn = WMIConnection::with_namespace_path("ROOT\\Microsoft\\Windows\\Storage", com).ok()?;
    let rows: Vec<HashMap<String, wmi::Variant>> = conn
        .raw_query("SELECT BusType, MediaType, HealthStatus FROM MSFT_PhysicalDisk")
        .ok()?;
    let row = rows.first()?;
    Some((
        wmi_extract::extract_u32(row, "BusType")
            .and_then(bus_type_label)
            .map(str::to_string),
        wmi_extract::extract_u32(row, "MediaType")
            .and_then(media_type_label)
            .map(str::to_string),
        wmi_extract::extract_u32(row, "HealthStatus")
            .and_then(health_status_label)
            .map(str::to_string),
    ))
}

#[cfg(not(windows))]
fn read_physical_disk_traits() -> Option<(Option<String>, Option<String>, Option<String>)> {
    None
}

/// Describe the NPU from the `ComputeAccelerator` device class.
///
/// Windows 11 exposes neural accelerators as a first-class device class; the
/// class key holds the driver record and the instance's `DeviceDesc` holds the
/// marketing name (indirected through the installing INF). Returns `None` when
/// no instance exists — "no NPU", which is different from "unreadable".
#[cfg(windows)]
fn read_npu() -> Option<NpuSpec> {
    use crate::util::registry::RegKeyGuard;
    use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;

    const CLASS_ROOT: &str = r"SYSTEM\CurrentControlSet\Control\Class";
    const NPU_CLASS: &str = "ComputeAccelerator";

    let classes = RegKeyGuard::open_read(HKEY_LOCAL_MACHINE, CLASS_ROOT).ok()??;
    let class_names = classes.enum_subkeys().ok()?;

    for class_name in class_names {
        let path = format!(r"{CLASS_ROOT}\{class_name}");
        let Some(class_key) = RegKeyGuard::open_read(HKEY_LOCAL_MACHINE, &path).ok()? else {
            continue;
        };
        let is_npu = class_key
            .read_string("Class")
            .ok()
            .flatten()
            .is_some_and(|c| c.eq_ignore_ascii_case(NPU_CLASS));
        if !is_npu {
            continue;
        }

        let instances = class_key.enum_subkeys().ok()?;
        // `Properties` lives beside the device instances and is not one.
        let instance = instances
            .iter()
            .find(|name| !name.is_empty() && name.chars().all(|c| c.is_ascii_digit()))?;

        let inst_path = format!(r"{CLASS_ROOT}\{class_name}\{instance}");
        let inst_key = RegKeyGuard::open_read(HKEY_LOCAL_MACHINE, &inst_path).ok()??;

        let driver_version = inst_key
            .read_string("DriverVersion")
            .ok()
            .flatten()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let driver_date = inst_key
            .read_string("DriverDate")
            .ok()
            .flatten()
            .as_deref()
            .and_then(normalize_driver_date);
        let vendor = inst_key
            .read_string("ProviderName")
            .ok()
            .flatten()
            .and_then(|s| resolve_inf_string(&s));
        let device_id = inst_key
            .read_string("MatchingDeviceId")
            .ok()
            .flatten()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let config_flags = inst_key.read_u32("ConfigFlags").ok().flatten().unwrap_or(0);

        // Friendly name: the device instance under Enum\PCI carries DeviceDesc,
        // which the INF indirection resolves to e.g. "Intel(R) NPU".
        let name = device_id
            .as_deref()
            .and_then(read_pnp_device_desc)
            .unwrap_or_else(|| "Neural processor".to_string());

        return Some(NpuSpec {
            name,
            vendor,
            driver_version,
            driver_date,
            status: device_status_from_config_flags(config_flags).to_string(),
            device_id,
        });
    }
    None
}

/// True when an `Enum\PCI` key name belongs to a hardware id.
///
/// `MatchingDeviceId` on the class instance is the *short* id
/// (`PCI\VEN_8086&DEV_B03E`) while the registry key under `Enum\PCI` is the
/// full one (`VEN_8086&DEV_B03E&SUBSYS_24241D72&REV_04`), so the match has to be
/// by prefix of the vendor+device part — never by equality.
pub fn hardware_id_matches_key(hardware_id: &str, enum_key: &str) -> bool {
    let tail = hardware_id
        .rsplit('\\')
        .next()
        .unwrap_or(hardware_id)
        .trim();
    if tail.is_empty() {
        return false;
    }
    enum_key
        .to_ascii_uppercase()
        .starts_with(&tail.to_ascii_uppercase())
}

/// `DeviceDesc` of the first instance under `Enum\PCI\<hardware-id>`.
#[cfg(windows)]
fn read_pnp_device_desc(hardware_id: &str) -> Option<String> {
    use crate::util::registry::RegKeyGuard;
    use windows::Win32::System::Registry::HKEY_LOCAL_MACHINE;

    const ENUM_PCI: &str = r"SYSTEM\CurrentControlSet\Enum\PCI";

    let pci = RegKeyGuard::open_read(HKEY_LOCAL_MACHINE, ENUM_PCI).ok()??;
    let device_keys = pci.enum_subkeys().ok()?;
    for device_key in device_keys {
        if !hardware_id_matches_key(hardware_id, &device_key) {
            continue;
        }
        let device_path = format!(r"{ENUM_PCI}\{device_key}");
        let Some(device) = RegKeyGuard::open_read(HKEY_LOCAL_MACHINE, &device_path).ok()? else {
            continue;
        };
        let instances = device.enum_subkeys().ok()?;
        for instance in instances {
            let inst_path = format!(r"{device_path}\{instance}");
            let Some(inst) = RegKeyGuard::open_read(HKEY_LOCAL_MACHINE, &inst_path).ok()? else {
                continue;
            };
            if let Some(desc) = inst.read_string("DeviceDesc").ok().flatten() {
                if let Some(name) = resolve_inf_string(&desc) {
                    return Some(name);
                }
            }
            if let Some(friendly) = inst.read_string("FriendlyName").ok().flatten() {
                if let Some(name) = resolve_inf_string(&friendly) {
                    return Some(name);
                }
            }
        }
    }
    None
}

#[cfg(not(windows))]
fn read_npu() -> Option<NpuSpec> {
    None
}

// ── Live values ─────────────────────────────────────────────────────────────

/// `(free_bytes, total_bytes)` of the system volume.
#[cfg(windows)]
fn system_drive_space() -> (u64, u64) {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let mut root: Vec<u16> = system_drive_letter().encode_utf16().collect();
    root.push(b'\\' as u16);
    root.push(0);

    let mut free_to_caller: u64 = 0;
    let mut total: u64 = 0;
    let mut total_free: u64 = 0;
    // SAFETY: `root` is a null-terminated wide string owned by this function;
    // the three out-pointers are valid for the duration of the call.
    let ok = unsafe {
        GetDiskFreeSpaceExW(
            PCWSTR(root.as_ptr()),
            Some(&mut free_to_caller),
            Some(&mut total),
            Some(&mut total_free),
        )
    };
    if ok.is_err() {
        return (0, 0);
    }
    (free_to_caller, total)
}

#[cfg(not(windows))]
fn system_drive_space() -> (u64, u64) {
    (0, 0)
}

/// `C:` — the volume Windows is installed on.
fn system_drive_letter() -> String {
    std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string())
}

/// Milliseconds since boot.
#[cfg(windows)]
fn uptime_secs() -> u64 {
    // SAFETY: GetTickCount64 is a pure read of the system tick counter.
    unsafe { windows::Win32::System::SystemInformation::GetTickCount64() / 1000 }
}

#[cfg(not(windows))]
fn uptime_secs() -> u64 {
    0
}

fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inf_indirection_resolves_to_the_fallback_text() {
        assert_eq!(
            resolve_inf_string("@oem95.inf,%npu_b03e_w11_26100%;Intel(R) NPU").as_deref(),
            Some("Intel(R) NPU")
        );
        assert_eq!(
            resolve_inf_string("@oem95.inf,%intel%;Intel Corporation").as_deref(),
            Some("Intel Corporation")
        );
    }

    #[test]
    fn inf_indirection_without_fallback_is_not_a_name() {
        assert_eq!(resolve_inf_string("@oem95.inf,%npu_b03e%"), None);
        assert_eq!(resolve_inf_string("@oem95.inf,-1234"), None);
        assert_eq!(resolve_inf_string("%unresolved%"), None);
        assert_eq!(resolve_inf_string("   "), None);
    }

    #[test]
    fn plain_strings_pass_through_trimmed() {
        assert_eq!(
            resolve_inf_string("  Intel(R) NPU  ").as_deref(),
            Some("Intel(R) NPU")
        );
    }

    #[test]
    fn smbios_memory_types_map_to_labels() {
        assert_eq!(memory_type_label(34), Some("DDR5"));
        // The host reports 35 for its LPDDR5X modules.
        assert_eq!(memory_type_label(35), Some("LPDDR5"));
        assert_eq!(memory_type_label(0), None);
        assert_eq!(memory_type_label(255), None);
    }

    #[test]
    fn storage_bus_media_and_health_labels() {
        // 17 is what MSFT_PhysicalDisk reports for this NVMe drive.
        assert_eq!(bus_type_label(17), Some("NVMe"));
        assert_eq!(bus_type_label(11), Some("SATA"));
        assert_eq!(bus_type_label(99), None);
        assert_eq!(media_type_label(4), Some("SSD"));
        assert_eq!(media_type_label(3), Some("HDD"));
        assert_eq!(media_type_label(0), None);
        assert_eq!(health_status_label(0), Some("Healthy"));
        assert_eq!(health_status_label(2), Some("Unhealthy"));
    }

    #[test]
    fn registry_driver_dates_normalise_to_iso() {
        // The NPU driver record on this host.
        assert_eq!(
            normalize_driver_date("7-24-2026").as_deref(),
            Some("2026-07-24")
        );
        assert_eq!(
            normalize_driver_date("7/4/2026").as_deref(),
            Some("2026-07-04")
        );
        assert_eq!(
            normalize_driver_date("12-31-2025").as_deref(),
            Some("2025-12-31")
        );
    }

    #[test]
    fn malformed_driver_dates_are_rejected_not_guessed() {
        assert_eq!(normalize_driver_date(""), None);
        assert_eq!(normalize_driver_date("garbage"), None);
        assert_eq!(normalize_driver_date("13-1-2026"), None); // month 13
        assert_eq!(normalize_driver_date("7-1"), None);
        assert_eq!(normalize_driver_date("7-1-1800"), None);
    }

    #[test]
    fn config_flags_separate_healthy_from_failed_devices() {
        assert_eq!(device_status_from_config_flags(0), "ok");
        assert_eq!(device_status_from_config_flags(0x1), "error"); // disabled
        assert_eq!(device_status_from_config_flags(0x40), "error"); // failed install
                                                                    // Unrelated flags (e.g. 0x20 "no UI") are not failures.
        assert_eq!(device_status_from_config_flags(0x20), "ok");
    }

    #[test]
    fn hardware_id_matches_the_long_enum_key_by_prefix() {
        // The real pair on this host: the class instance carries the short id,
        // the Enum\PCI key carries the full one.
        assert!(hardware_id_matches_key(
            r"PCI\VEN_8086&DEV_B03E",
            "VEN_8086&DEV_B03E&SUBSYS_24241D72&REV_04"
        ));
        assert!(!hardware_id_matches_key(
            r"PCI\VEN_8086&DEV_B03E",
            "VEN_8086&DEV_1234&SUBSYS_24241D72&REV_04"
        ));
        // A different vendor with the same device number must not match.
        assert!(!hardware_id_matches_key(
            r"PCI\VEN_10DE&DEV_B03E",
            "VEN_8086&DEV_B03E&SUBSYS_24241D72&REV_04"
        ));
        assert!(!hardware_id_matches_key("", "VEN_8086&DEV_B03E"));
    }

    #[test]
    fn uptime_splits_into_days_hours_minutes() {
        assert_eq!(split_uptime(0), (0, 0, 0));
        assert_eq!(split_uptime(59), (0, 0, 0));
        assert_eq!(split_uptime(60), (0, 0, 1));
        assert_eq!(split_uptime(3_600), (0, 1, 0));
        assert_eq!(split_uptime(90_061), (1, 1, 1));
    }

    #[test]
    fn gib_conversion_uses_binary_units() {
        assert_eq!(bytes_to_gib(0), 0.0);
        assert!((bytes_to_gib(1024 * 1024 * 1024) - 1.0).abs() < 1e-9);
        // 102_324_973_9776 bytes → ~952.99 GiB (a "1 TB" volume).
        let gb = bytes_to_gib(1_023_249_739_776);
        assert!((gb - 952.98).abs() < 0.05, "gb = {gb}");
    }

    #[test]
    fn snapshot_serialises_with_null_for_unknown_optional_fields() {
        let snapshot = OverviewSnapshot {
            npu: None,
            ..Default::default()
        };
        let json = serde_json::to_string(&snapshot).expect("serialise");
        // The frontend switches on `null` to render "—", so optional fields
        // must be present as null rather than omitted.
        assert!(json.contains("\"npu\":null"), "{json}");
        assert!(json.contains("\"speed_mt_s\":null"), "{json}");
    }

    #[test]
    fn live_snapshot_reports_real_hardware() {
        // Integration-shaped: runs the real queries. It must not panic and must
        // identify *something* on any Windows host; the assertions stay loose so
        // the test is portable across machines, but they prove the parse path
        // works end to end.
        let snapshot = get_overview_snapshot().expect("snapshot");
        assert!(!snapshot.device.model.is_empty(), "model must be read");
        assert!(!snapshot.cpu.name.is_empty(), "cpu name must be read");
        assert!(snapshot.cpu.cores > 0, "cores must be > 0");
        assert!(snapshot.memory.total_gb > 0.0, "memory total must be > 0");
        assert!(snapshot.os.build.len() >= 3, "OS build must be read");
    }
}
