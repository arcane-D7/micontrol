//! Management of the custom ecram_service (IoTService.exe replacement).
//!
//! This module ensures the custom IoTService.exe (built from `ecram_service.rs`)
//! is installed as a Windows service and running.  It provides ECRAM access
//! via named pipe `\\.\pipe\ecram_service` without requiring Xiaomi PC Manager.
//!
//! The service is installed as `IoTSvc` (matching the name the driver expects)
//! and runs as `NT AUTHORITY\SYSTEM` to pass the IoTDriver.sys security check.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const SERVICE_NAME: &str = "IoTSvc";
const SERVICE_EXE: &str = "ecram_service.exe";

/// Prevent console window flash when spawning subprocesses.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// Status of the ecram_service.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceStatus {
    pub installed: bool,
    pub running: bool,
    pub pipe_available: bool,
    pub exe_path: Option<String>,
    pub message: String,
}

/// Ensure the ecram_service is installed and running.
///
/// This function:
/// 1. Finds the ecram_service.exe (bundled with the app or in the install dir)
/// 2. Locates the IoTDriver DriverStore directory
/// 3. Copies our ecram_service.exe there as IoTService.exe (replacing the original)
///    — this is required because IoTDriver.sys checks the process path/name
/// 4. Ensures the IoTSvc service points to the DriverStore IoTService.exe
/// 5. Starts the service if it's not running
/// 6. Waits for the pipe to become available
pub fn ensure_service_running() -> Result<ServiceStatus, String> {
    let exe_path = find_ecram_service_exe();
    let pipe_path = r"\\.\pipe\ecram_service";

    // Check if pipe is already available (service already running)
    let pipe_available = std::fs::metadata(pipe_path).is_ok();
    if pipe_available {
        return Ok(ServiceStatus {
            installed: true,
            running: true,
            pipe_available: true,
            exe_path,
            message: "ecram_service already running".to_string(),
        });
    }

    let our_exe = exe_path
        .as_deref()
        .ok_or_else(|| "ecram_service.exe not found".to_string())?;

    // Find the DriverStore path where IoTService.exe lives.
    // IoTDriver.sys checks that the calling process is IoTService.exe in this path.
    let driverstore_iot_exe = find_driverstore_iot_service_exe();

    let target_exe = if let Some(ref ds_path) = driverstore_iot_exe {
        // Copy our ecram_service.exe to the DriverStore as IoTService.exe
        // (replacing the original Xiaomi IoTService.exe)
        let need_copy = match std::fs::metadata(ds_path) {
            Ok(meta) => meta.len() != std::fs::metadata(our_exe).map(|m| m.len()).unwrap_or(0),
            Err(_) => true,
        };

        if need_copy {
            // Stop the service before replacing the file
            let _ = std::process::Command::new("sc")
                .args(["stop", SERVICE_NAME])
                .creation_flags(CREATE_NO_WINDOW)
                .output();
            std::thread::sleep(std::time::Duration::from_secs(2));

            // Backup original if not already backed up
            let backup = format!("{}.bak", ds_path);
            if std::fs::metadata(&backup).is_err() {
                let _ = std::fs::copy(ds_path, &backup);
            }

            // Copy our exe to the DriverStore path
            std::fs::copy(our_exe, ds_path)
                .map_err(|e| format!("Failed to copy ecram_service.exe to DriverStore: {e}"))?;

            log::info!(
                "[ecram_service_mgmt] Replaced {} with our ecram_service.exe",
                ds_path
            );
        }

        ds_path.clone()
    } else {
        // No DriverStore path found — fall back to installing as a standalone service
        // (ECRAM access may not work if the driver checks the process path)
        log::warn!(
            "[ecram_service_mgmt] DriverStore IoTService.exe path not found — \
             installing as standalone service (ECRAM access may be restricted)"
        );

        let installed = service_exists(SERVICE_NAME);
        if !installed {
            install_service(SERVICE_NAME, our_exe)?;
        }
        our_exe.to_string()
    };

    // Ensure the service points to the target exe
    let installed = service_exists(SERVICE_NAME);
    if installed {
        let current_bin = get_service_bin_path(SERVICE_NAME);
        let expected_bin = format!("\"{}\" service", target_exe);
        let expected_bin_no_quotes = target_exe.to_string();

        let matches = current_bin
            .as_deref()
            .map(|b| {
                b.eq_ignore_ascii_case(&expected_bin)
                    || b.eq_ignore_ascii_case(&expected_bin_no_quotes)
            })
            .unwrap_or(false);

        if !matches {
            // Stop and reconfigure
            let _ = std::process::Command::new("sc")
                .args(["stop", SERVICE_NAME])
                .creation_flags(CREATE_NO_WINDOW)
                .output();
            std::thread::sleep(std::time::Duration::from_secs(2));

            // Kill any lingering process
            let _ = std::process::Command::new("taskkill")
                .args(["/F", "/IM", "IoTService.exe"])
                .creation_flags(CREATE_NO_WINDOW)
                .output();
            let _ = std::process::Command::new("taskkill")
                .args(["/F", "/IM", "ecram_service.exe"])
                .creation_flags(CREATE_NO_WINDOW)
                .output();
            std::thread::sleep(std::time::Duration::from_secs(1));

            // Delete and recreate the service with the correct path.
            // Using `sc config binPath=` with quoted paths is unreliable;
            // deleting and recreating is more robust.
            let _ = std::process::Command::new("sc")
                .args(["delete", SERVICE_NAME])
                .creation_flags(CREATE_NO_WINDOW)
                .output();
            std::thread::sleep(std::time::Duration::from_secs(2));

            install_service(SERVICE_NAME, &target_exe)?;

            log::info!(
                "[ecram_service_mgmt] Service recreated with path {}",
                expected_bin
            );
        }
    } else {
        install_service(SERVICE_NAME, &target_exe)?;
    }

    // Start the service
    start_service(SERVICE_NAME)?;

    // Wait for the pipe to become available (up to 10 seconds)
    let mut pipe_ok = false;
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        if std::fs::metadata(pipe_path).is_ok() {
            pipe_ok = true;
            break;
        }
    }

    Ok(ServiceStatus {
        installed: true,
        running: pipe_ok,
        pipe_available: pipe_ok,
        exe_path,
        message: if pipe_ok {
            "ecram_service started successfully".to_string()
        } else {
            "ecram_service installed but pipe not yet available".to_string()
        },
    })
}

/// Find the ecram_service.exe path.
///
/// Looks in:
/// 1. Same directory as the current executable (installed mode)
/// 2. App install directory (C:\Program Files\miControl\)
/// 3. Development target directory
fn find_ecram_service_exe() -> Option<String> {
    // 1. Same directory as current exe
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(SERVICE_EXE);
            if candidate.exists() {
                return Some(candidate.to_string_lossy().into_owned());
            }
        }
    }

    // 2. Program Files
    let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".to_string());
    let candidate = PathBuf::from(&pf).join("miControl").join(SERVICE_EXE);
    if candidate.exists() {
        return Some(candidate.to_string_lossy().into_owned());
    }

    // 3. LOCALAPPDATA\miControl (dev mode)
    let la = std::env::var("LOCALAPPDATA").unwrap_or_default();
    if !la.is_empty() {
        let candidate = PathBuf::from(&la).join("miControl").join(SERVICE_EXE);
        if candidate.exists() {
            return Some(candidate.to_string_lossy().into_owned());
        }
    }

    None
}

/// Find the IoTService.exe path in the Windows DriverStore.
///
/// The IoTDriver.sys driver checks that the calling process is named
/// "IoTService.exe" and is located in the DriverStore FileRepository
/// directory. We need to replace this file with our ecram_service.exe
/// to pass the driver's security check.
///
/// Searches for `iotdriver.inf_amd64_*` directories in
/// `C:\WINDOWS\System32\DriverStore\FileRepository\`.
#[cfg(windows)]
fn find_driverstore_iot_service_exe() -> Option<String> {
    let driverstore = PathBuf::from(r"C:\WINDOWS\System32\DriverStore\FileRepository");
    if !driverstore.exists() {
        return None;
    }

    // Look for iotdriver.inf_amd64_* directories
    if let Ok(entries) = std::fs::read_dir(&driverstore) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.starts_with("iotdriver.inf_amd64_") {
                let iot_exe = entry.path().join("IoTService.exe");
                if iot_exe.exists() {
                    return Some(iot_exe.to_string_lossy().into_owned());
                }
            }
        }
    }

    None
}

#[cfg(not(windows))]
fn find_driverstore_iot_service_exe() -> Option<String> {
    None
}

/// Check if a Windows service exists.
fn service_exists(name: &str) -> bool {
    #[cfg(windows)]
    {
        use std::process::Command;
        let output = Command::new("sc")
            .args(["query", name])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
        match output {
            Ok(o) => o.status.success(),
            Err(_) => false,
        }
    }
    #[cfg(not(windows))]
    {
        let _ = name;
        false
    }
}

/// Get the binary path of a Windows service (from `sc qc`).
#[cfg(windows)]
fn get_service_bin_path(name: &str) -> Option<String> {
    use std::process::Command;
    let output = Command::new("sc")
        .args(["qc", name])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let text = String::from_utf8_lossy(&output.stdout);
    // Parse line: "        BINARY_PATH_NAME   : C:\path\to\exe.exe"
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("BINARY_PATH_NAME") {
            if let Some(idx) = trimmed.rfind(':') {
                let path = trimmed[idx + 1..].trim();
                if !path.is_empty() {
                    return Some(path.to_string());
                }
            }
        }
    }
    None
}

#[cfg(not(windows))]
fn get_service_bin_path(_name: &str) -> Option<String> {
    None
}

/// Install a Windows service.
#[cfg(windows)]
fn install_service(name: &str, exe_path: &str) -> Result<(), String> {
    use std::process::Command;

    // Create the service
    let bin_path = format!("\"{}\" service", exe_path);
    let output = Command::new("sc")
        .args([
            "create",
            name,
            "binPath=",
            &bin_path,
            "start=",
            "auto",
            "DisplayName=",
            "MiControl IoT Bridge Service",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Failed to create service: {e}"))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        let out = String::from_utf8_lossy(&output.stdout);
        return Err(format!("sc create failed: {err} {out}"));
    }

    // Set the service to run as LocalSystem
    let _ = Command::new("sc")
        .args(["config", name, "obj=", "LocalSystem"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    // Set failure actions to auto-restart
    let _ = Command::new("sc")
        .args([
            "failure",
            name,
            "reset=",
            "86400",
            "actions=",
            "restart/5000/restart/10000/restart/30000",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();

    log::info!(
        "[ecram_service_mgmt] Service '{}' installed at {}",
        name,
        exe_path
    );
    Ok(())
}

#[cfg(not(windows))]
fn install_service(_name: &str, _exe_path: &str) -> Result<(), String> {
    Err("Service installation is only supported on Windows".to_string())
}

/// Start a Windows service.
#[cfg(windows)]
fn start_service(name: &str) -> Result<(), String> {
    use std::process::Command;

    let output = Command::new("sc")
        .args(["start", name])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Failed to start service: {e}"))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        let out = String::from_utf8_lossy(&output.stdout);
        // Service may already be running — check
        if out.contains("already running") || out.contains("1056") || err.contains("1056") {
            return Ok(());
        }
        return Err(format!("sc start failed: {err} {out}"));
    }

    log::info!("[ecram_service_mgmt] Service '{}' started", name);
    Ok(())
}

#[cfg(not(windows))]
fn start_service(_name: &str) -> Result<(), String> {
    Err("Service management is only supported on Windows".to_string())
}
