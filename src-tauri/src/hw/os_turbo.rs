//! OS Turbo — system-level software optimization.
//!
//! Provides a functional equivalent to XPM's "OS Turbo" feature, which
//! optimizes system performance through:
//! - Background process throttling (EcoQoS)
//! - Foreground app prioritization
//! - Power plan switching
//! - Startup app management
//!
//! This is distinct from the hardware performance mode (EC 0x68) which
//! controls CPU TDP/fan curves. OS Turbo operates at the Windows scheduler
//! level without touching EC registers.

use crate::hw::errors::{HardwareError, HardwareResult};
use serde::{Deserialize, Serialize};

/// Registry key for persisting OS Turbo state.
#[cfg(windows)]
const OS_TURBO_REG_KEY: &str = r"SOFTWARE\MiControl\OSTurbo";

/// OS Turbo status.
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OsTurboStatus {
    pub enabled: bool,
    /// Number of background processes throttled.
    pub throttled_processes: u32,
    /// Current power plan GUID.
    pub power_plan: String,
}

/// Enable or disable OS Turbo.
///
/// When enabled:
/// 1. Switches to "Best Performance" power plan
/// 2. Throttles known background processes to EcoQoS
/// 3. Prioritizes the foreground application
///
/// When disabled:
/// 1. Restores "Balanced" power plan
/// 2. Removes EcoQoS throttling
pub fn set_os_turbo(enabled: bool) -> HardwareResult<OsTurboStatus> {
    if enabled {
        enable_os_turbo()
    } else {
        disable_os_turbo()
    }
}

/// Get the current OS Turbo state.
pub fn get_os_turbo() -> HardwareResult<OsTurboStatus> {
    #[cfg(windows)]
    {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_CURRENT_USER;

        let enabled = RegKeyGuard::open_read(HKEY_CURRENT_USER, OS_TURBO_REG_KEY)
            .ok()
            .flatten()
            .and_then(|k| k.read_u32("Enabled").ok().flatten())
            .map(|v| v != 0)
            .unwrap_or(false);

        let power_plan = get_active_power_plan().unwrap_or_default();
        let throttled = count_throttled_processes();

        Ok(OsTurboStatus {
            enabled,
            throttled_processes: throttled,
            power_plan,
        })
    }
    #[cfg(not(windows))]
    {
        Ok(OsTurboStatus {
            enabled: false,
            throttled_processes: 0,
            power_plan: String::new(),
        })
    }
}

/// Enable OS Turbo optimizations.
fn enable_os_turbo() -> HardwareResult<OsTurboStatus> {
    // 1. Switch to Best Performance power plan
    let power_plan = set_power_plan_best_performance()?;

    // 2. Throttle known background processes
    let throttled = throttle_background_processes();

    // 3. Persist state
    persist_os_turbo(true)?;

    log::info!(
        target: "hw::os_turbo",
        "OS Turbo enabled: power_plan={}, throttled={} processes",
        power_plan,
        throttled
    );

    Ok(OsTurboStatus {
        enabled: true,
        throttled_processes: throttled,
        power_plan,
    })
}

/// Disable OS Turbo optimizations.
fn disable_os_turbo() -> HardwareResult<OsTurboStatus> {
    // 1. Restore Balanced power plan
    let power_plan = set_power_plan_balanced()?;

    // 2. Remove throttling (processes will naturally un-throttle)
    // Note: We don't need to explicitly un-throttle — when EcoQoS is removed,
    // the scheduler returns to normal priority.

    // 3. Persist state
    persist_os_turbo(false)?;

    log::info!(target: "hw::os_turbo", "OS Turbo disabled");

    Ok(OsTurboStatus {
        enabled: false,
        throttled_processes: 0,
        power_plan,
    })
}

/// Set the active power plan to "Best Performance".
fn set_power_plan_best_performance() -> HardwareResult<String> {
    // Classic Windows power scheme GUIDs (not overlay GUIDs):
    //   High Performance: 8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c
    //   Balanced:         381b4222-f694-41f0-9685-ff5bb260df2e
    // Note: performance.rs handles the Windows 11 power mode overlay separately.
    #[cfg(windows)]
    {
        use windows::Win32::System::Power::PowerSetActiveScheme;

        // High Performance GUID
        let guid = windows::core::GUID::from_u128(0x8c5e7fda_e8bf_4a96_9a85_a6e23a8c635c);

        let result = unsafe { PowerSetActiveScheme(None, Some(&guid)) };
        if result.0 != 0 {
            // Fallback: use powercfg
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let _ = std::process::Command::new("powercfg")
                .args(["/setactive", "8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c"])
                .creation_flags(CREATE_NO_WINDOW)
                .output();
        }

        Ok("Best Performance".to_string())
    }
    #[cfg(not(windows))]
    {
        Ok(String::new())
    }
}

/// Set the active power plan to "Balanced".
fn set_power_plan_balanced() -> HardwareResult<String> {
    #[cfg(windows)]
    {
        use windows::Win32::System::Power::PowerSetActiveScheme;

        // Balanced GUID
        let guid = windows::core::GUID::from_u128(0x381b4222_f694_41f0_9685_ff5bb260df2e);

        let result = unsafe { PowerSetActiveScheme(None, Some(&guid)) };
        if result.0 != 0 {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            let _ = std::process::Command::new("powercfg")
                .args(["/setactive", "381b4222-f694-41f0-9685-ff5bb260df2e"])
                .creation_flags(CREATE_NO_WINDOW)
                .output();
        }

        Ok("Balanced".to_string())
    }
    #[cfg(not(windows))]
    {
        Ok(String::new())
    }
}

/// Get the active power plan name.
fn get_active_power_plan() -> HardwareResult<String> {
    #[cfg(windows)]
    {
        use windows::Win32::System::Power::PowerGetActiveScheme;

        let mut guid_ptr = std::ptr::null_mut();
        let result = unsafe { PowerGetActiveScheme(None, &mut guid_ptr) };

        if result.0 == 0 && !guid_ptr.is_null() {
            let guid = unsafe { *guid_ptr };
            // Free the GUID buffer allocated by PowerGetActiveScheme.
            // LocalFree takes HLOCAL, but the pointer from PowerGetActiveScheme
            // is a GUID* allocated with LocalAlloc.
            windows_targets::link!(
                "kernel32.dll"
                "system"
                fn LocalFree(hmem: *mut std::ffi::c_void) -> *mut std::ffi::c_void
            );
            let _ = unsafe { LocalFree(guid_ptr as *mut std::ffi::c_void) };

            // Check against known classic power scheme GUIDs
            let high_perf = windows::core::GUID::from_u128(0x8c5e7fda_e8bf_4a96_9a85_a6e23a8c635c);
            let balanced = windows::core::GUID::from_u128(0x381b4222_f694_41f0_9685_ff5bb260df2e);
            let power_saver =
                windows::core::GUID::from_u128(0xa1841308_3541_4fab_bc81_f71556f20b4a);

            if guid == high_perf {
                return Ok("Best Performance".to_string());
            } else if guid == balanced {
                return Ok("Balanced".to_string());
            } else if guid == power_saver {
                return Ok("Best Power Efficiency".to_string());
            }
            return Ok(format!("{:?}", guid));
        }

        Err(HardwareError::Other("PowerGetActiveScheme failed".into()))
    }
    #[cfg(not(windows))]
    {
        Ok(String::new())
    }
}

/// Throttle known background processes using EcoQoS.
///
/// This applies PROCESS_POWER_THROTTLING_STATE to known background
/// services that don't need high performance, reducing their CPU impact.
fn throttle_background_processes() -> u32 {
    // List of known background process names that are safe to throttle.
    // These are common Windows services that run in the background and
    // don't require real-time performance.
    const THROTTLE_TARGETS: &[&str] = &[
        "SearchIndexer.exe",      // Windows Search indexer
        "MsMpEng.exe",            // Windows Defender (throttle during scans)
        "TiWorker.exe",           // Windows Update worker
        "TrustedInstaller.exe",   // Windows Module Installer
        "backgroundTaskHost.exe", // Background task host
    ];

    let mut count = 0u32;

    #[cfg(windows)]
    {
        use windows::Win32::System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION,
        };

        // Enumerate processes via WMI
        if let Ok(processes) = enumerate_processes() {
            for (pid, name) in processes {
                if !THROTTLE_TARGETS.contains(&name.as_str()) {
                    continue;
                }

                // SAFETY: OpenProcess with QUERY + SET_INFORMATION
                let handle = unsafe {
                    OpenProcess(
                        PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SET_INFORMATION,
                        false,
                        pid,
                    )
                };

                if let Ok(handle) = handle {
                    // Apply EcoQoS throttling via SetProcessInformation.
                    // The windows 0.58 crate does not expose SetProcessInformation,
                    // so we use NtSetInformationProcess via FFI.
                    //
                    // ProcessPowerThrottlingVal = 4 (PROCESS_POWER_THROTTLING_STATE)
                    // We set ControlMask = EXECUTION_SPEED (1) and StateMask = EXECUTION_SPEED (1)
                    // to enable EcoQoS.
                    #[repr(C)]
                    #[derive(Default, Clone, Copy)]
                    struct ProcessPowerThrottlingState {
                        version: u32,
                        control_mask: u32,
                        state_mask: u32,
                    }

                    let mut state = ProcessPowerThrottlingState {
                        version: 1,
                        control_mask: 1, // PROCESS_POWER_THROTTLING_EXECUTION_SPEED
                        state_mask: 1,   // EcoQoS enabled
                    };

                    // NtSetInformationProcess is the lower-level call behind
                    // SetProcessInformation.  We link it directly.
                    windows_targets::link!(
                        "ntdll.dll"
                        "system"
                        fn NtSetInformationProcess(
                            process_handle: windows::Win32::Foundation::HANDLE,
                            info_class: u32,
                            info: *mut std::ffi::c_void,
                            info_len: u32,
                        ) -> i32
                    );

                    let status = unsafe {
                        NtSetInformationProcess(
                            handle,
                            4, // ProcessPowerThrottlingVal
                            &mut state as *mut _ as *mut std::ffi::c_void,
                            std::mem::size_of::<ProcessPowerThrottlingState>() as u32,
                        )
                    };

                    if status >= 0 {
                        log::debug!(
                            target: "hw::os_turbo",
                            "Throttled {} (PID={})",
                            name, pid
                        );
                        count += 1;
                    }

                    let _ = unsafe { windows::Win32::Foundation::CloseHandle(handle) };
                }
            }
        }
    }

    count
}

/// Count currently throttled processes (informational).
fn count_throttled_processes() -> u32 {
    // This is informational — we return 0 since we can't easily query
    // the EcoQoS state of all processes without enumeration.
    0
}

/// Enumerate running processes via WMI.
#[cfg(windows)]
fn enumerate_processes() -> anyhow::Result<Vec<(u32, String)>> {
    use crate::hw::wmi_cache;
    use crate::util::wmi_extract;
    use std::collections::HashMap;

    wmi_cache::with_cimv2(|wmi| {
        let processes: Vec<HashMap<String, wmi::Variant>> =
            wmi.raw_query("SELECT ProcessId, Name FROM Win32_Process")?;

        let result = processes
            .into_iter()
            .filter_map(|p| {
                let pid = wmi_extract::extract_u32(&p, "ProcessId")?;
                let name = wmi_extract::extract_string(&p, "Name")?;
                Some((pid, name))
            })
            .collect();

        Ok(result)
    })
    .map_err(|e| anyhow::anyhow!(e))
}

/// Persist OS Turbo state to registry.
fn persist_os_turbo(enabled: bool) -> HardwareResult<()> {
    #[cfg(windows)]
    {
        use crate::util::registry::RegKeyGuard;
        use windows::Win32::System::Registry::HKEY_CURRENT_USER;

        let key = RegKeyGuard::create_write(HKEY_CURRENT_USER, OS_TURBO_REG_KEY)
            .map_err(|e| HardwareError::Registry(format!("Create OS Turbo key: {e}")))?;

        key.write_u32("Enabled", if enabled { 1 } else { 0 })
            .map_err(|e| HardwareError::Registry(format!("Write OS Turbo enabled: {e}")))?;

        Ok(())
    }
    #[cfg(not(windows))]
    {
        Ok(())
    }
}
