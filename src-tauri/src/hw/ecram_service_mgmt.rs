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

/// `taskkill /F /IM` inside the destructive recreation path. A force kill
/// completes well under a second; 10 s is headroom for a slow kill without
/// wedging the recreation (contract: UNIVERSAL-CONTRACTS.md section 4, every
/// child wait is bounded).
#[cfg(windows)]
const TASKKILL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Kill every process of one image name (best-effort `taskkill /F /IM`)
/// through the canonical spawn helper: bounded wait, tree kill on timeout,
/// CREATE_NO_WINDOW applied by the helper itself. The callers already treat
/// a missing process as fine, so failures stay best-effort there but are
/// logged.
#[cfg(windows)]
fn kill_process_image(image: &str) -> Result<(), crate::util::process_spawn::SpawnError> {
    let mut cmd = std::process::Command::new("taskkill");
    cmd.args(["/F", "/IM", image]);
    crate::util::process_spawn::run_capture(&mut cmd, TASKKILL_TIMEOUT).map(|_| ())
}

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

    // Probe the pipe with a real OPEN_EXISTING CreateFile, not `metadata`
    // (std::fs::metadata never resolves a named pipe — it always errors with
    // "The system cannot find the file specified" even when the pipe exists,
    // because pipes have no file-system metadata). Access-denied on open
    // means the pipe exists but a stale service process (old DACL) is
    // serving it, so we treat it as NOT usable and force a restart below.
    fn pipe_usable(path: &str) -> bool {
        // Any open failure (permission-denied from a stale DACL, or not-found)
        // means the pipe is NOT usable — force the restart path below.
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .is_ok()
    }

    // ── S66-005: confirm the pipe is REALLY absent before the destructive ──
    // path. A single transient connect failure (busy instance mid-accept,
    // ERROR_PIPE_BUSY) previously dropped the caller straight into
    // stop/copy/recreate — observed 2-3 recreations/day (up to 37 on
    // 2026-09-28), each disruptive (service downtime + 7045 events +
    // DriverStore writes). Retry the probe once after a short settle delay.
    let mut pipe_available = pipe_usable(pipe_path);
    if !pipe_available {
        std::thread::sleep(std::time::Duration::from_millis(250));
        pipe_available = pipe_usable(pipe_path);
    }

    // ── S66-005: persistent cooldown between full recreations. The IoT ──────
    // watchdog probes every 30 s (with its own 300 s recovery cooldown), but
    // multiple callers (health supervisor, manual UI triggers) can still
    // stack. One destructive recreation per 15 minutes is plenty for a
    // genuinely-broken service and caps the churn at 4/hour worst case.
    if !pipe_available {
        if let Some(status) = recreation_cooldown_status() {
            return Ok(status);
        }
    }

    let our_exe = exe_path
        .as_deref()
        .ok_or_else(|| "ecram_service.exe not found".to_string())?;

    // Find the DriverStore path where IoTService.exe lives.
    let driverstore_iot_exe = find_driverstore_iot_service_exe();

    // If the pipe is usable AND the DriverStore IoTService.exe is already
    // our ecram_service.exe, we're genuinely done.
    if pipe_available {
        let already_ours = driverstore_iot_exe
            .as_ref()
            .and_then(|ds_path| {
                let existing = std::fs::read(ds_path).ok()?;
                let ours = std::fs::read(our_exe).ok()?;
                Some(existing == ours)
            })
            .unwrap_or(false);

        if already_ours {
            return Ok(ServiceStatus {
                installed: true,
                running: true,
                pipe_available: true,
                exe_path,
                message: "ecram_service already running".to_string(),
            });
        }

        // Pipe usable but the DriverStore exe is NOT ours — the original
        // Xiaomi IoTService.exe is running and will reject our ioctls.
        // Fall through to the replacement logic below.
        log::warn!(
            "[ecram_service_mgmt] Pipe exists but DriverStore IoTService.exe \
             is not our ecram_service.exe — replacing it now"
        );
    } else {
        // Pipe is NOT usable. If we have already confirmed the DriverStore
        // binary is ours, the only remaining cause is a stale service
        // process still running the pre-DACL-fix binary. Restarting the
        // service re-creates the pipe with the permissive DACL.
        log::info!(
            "[ecram_service_mgmt] Pipe not usable (missing or blocked). Proceeding to ensure the service runs our binary."
        );
    }

    let target_exe = if let Some(ref ds_path) = driverstore_iot_exe {
        // Copy our ecram_service.exe to the DriverStore as IoTService.exe
        // (replacing the original Xiaomi IoTService.exe)
        //
        // We compare file hashes (not just sizes) because different builds
        // can produce binaries with identical sizes but different content.
        // Using size-only comparison caused the original Xiaomi IoTService.exe
        // to remain in place when our ecram_service.exe happened to have the
        // same file size, resulting in no \\.\pipe\ecram_service being created.
        let need_copy = match (std::fs::read(ds_path), std::fs::read(our_exe)) {
            (Ok(existing), Ok(ours)) => existing != ours,
            _ => true,
        };

        // Always stop + kill a running service when the pipe is not usable.
        // A stale process holds the OLD binary image in memory (with the
        // SYSTEM-only pipe DACL) — replacing the file on disk alone does not
        // fix the running instance; the service MUST be restarted so the new
        // permissive-DACL pipe gets created.
        if !pipe_available {
            log::info!(
                "[ecram_service_mgmt] Pipe unusable — stopping & killing stale IoTService process"
            );
            let _ = crate::util::win_service::stop_request(SERVICE_NAME);
            std::thread::sleep(std::time::Duration::from_secs(2));
            // Best-effort as before, but bounded: a spawn failure (timeout
            // included) is logged and the child tree dies on expiry.
            if let Err(e) = kill_process_image("IoTService.exe") {
                log::warn!("[ecram_service_mgmt] taskkill IoTService.exe failed: {e}");
            }
            if let Err(e) = kill_process_image("ecram_service.exe") {
                log::warn!("[ecram_service_mgmt] taskkill ecram_service.exe failed: {e}");
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        }

        if need_copy {
            // Stop the service before replacing the file
            let _ = crate::util::win_service::stop_request(SERVICE_NAME);
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
        // Canonical form (empirically REQUIRED): quoted path + `service`
        // token. A bare-path service fails to start with error 2.
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
            let _ = crate::util::win_service::stop_request(SERVICE_NAME);
            std::thread::sleep(std::time::Duration::from_secs(2));

            // Kill any lingering process
            // Best-effort as before, but bounded: a spawn failure (timeout
            // included) is logged and the child tree dies on expiry.
            if let Err(e) = kill_process_image("IoTService.exe") {
                log::warn!("[ecram_service_mgmt] taskkill IoTService.exe failed: {e}");
            }
            if let Err(e) = kill_process_image("ecram_service.exe") {
                log::warn!("[ecram_service_mgmt] taskkill ecram_service.exe failed: {e}");
            }
            std::thread::sleep(std::time::Duration::from_secs(1));

            // Delete and recreate the service with the correct path.
            // Using `sc config binPath=` with quoted paths is unreliable;
            // deleting and recreating is more robust. DeleteService is
            // asynchronous in the SCM: the sleeps below let a marked-for-
            // delete entry settle before the recreate (the install path is
            // the only user of this branch and runs elevated).
            let _ = crate::util::win_service::delete(SERVICE_NAME);
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

    // S66-005: record the recreation timestamp so the cooldown above can
    // suppress rapid follow-up recreations from stacked callers.
    note_recreation();

    // Wait for the pipe to become usable (up to 10 seconds). We probe with
    // OPEN_EXISTING (works for named pipes) rather than std::fs::metadata,
    // which never resolves pipes.
    let mut pipe_ok = false;
    for _ in 0..20 {
        std::thread::sleep(std::time::Duration::from_millis(500));
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(pipe_path)
        {
            Ok(_) => {
                pipe_ok = true;
                break;
            }
            Err(_) => continue,
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

// ── S66-005: recreation cooldown ─────────────────────────────────────────────
//
// The destructive path (stop → copy into DriverStore → delete/recreate the
// service → start) ran 2-3×/day on the live machine (up to 37× on 2026-09-28),
// each producing service downtime + a 7045 event + DriverStore writes. The
// cooldown is process-local (static) — the bridge service is the only caller
// in practice, and it is long-lived.

/// Minimum time between destructive recreations.
#[cfg(windows)]
const RECREATION_COOLDOWN_MS: u64 = 15 * 60 * 1000;

/// Timestamp (ms since epoch) of the last destructive recreation; 0 = never.
#[cfg(windows)]
static LAST_RECREATION_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Record that a destructive recreation just ran.
#[cfg(windows)]
fn note_recreation() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default();
    LAST_RECREATION_MS.store(now, std::sync::atomic::Ordering::Relaxed);
}

/// When a destructive recreation ran recently, return a cooldown status the
/// caller can surface instead of repeating the churn. `None` = proceed.
#[cfg(windows)]
fn recreation_cooldown_status() -> Option<ServiceStatus> {
    let last = LAST_RECREATION_MS.load(std::sync::atomic::Ordering::Relaxed);
    if last == 0 {
        return None;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default();
    let elapsed = now.saturating_sub(last);
    if elapsed < RECREATION_COOLDOWN_MS {
        log::warn!(
            "[ecram_service_mgmt] S66-005: pipe unusable but a destructive recreation \
             ran {}s ago (cooldown {}s) — returning cooldown status",
            elapsed / 1000,
            RECREATION_COOLDOWN_MS / 1000
        );
        return Some(ServiceStatus {
            installed: true,
            running: false,
            pipe_available: false,
            exe_path: find_ecram_service_exe(),
            message: format!(
                "recreation cooldown: last attempt {}s ago (min {}s)",
                elapsed / 1000,
                RECREATION_COOLDOWN_MS / 1000
            ),
        });
    }
    None
}

#[cfg(not(windows))]
fn note_recreation() {}

#[cfg(not(windows))]
fn recreation_cooldown_status() -> Option<ServiceStatus> {
    None
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

/// Check if a Windows service exists: the SCM is the authority (`P1`: the
/// named pipe is only evidence about the data path).
fn service_exists(name: &str) -> bool {
    #[cfg(windows)]
    {
        match crate::util::win_service::query(name) {
            // 1060 is the SCM's own answer; any other failure is NOT absence.
            Ok(info) => info.state != crate::util::win_service::ServiceState::Absent,
            Err(_) => false,
        }
    }
    #[cfg(not(windows))]
    {
        let _ = name;
        false
    }
}

/// Get the binary path of a Windows service (the stored BINARY_PATH_NAME,
/// verbatim: quotes and trailing arguments kept).
#[cfg(windows)]
fn get_service_bin_path(name: &str) -> Option<String> {
    // On error fall back to None exactly like the old parse did: the caller
    // treats "unknown" as "does not match" and repairs the service.
    crate::util::win_service::query(name)
        .ok()
        .and_then(|info| info.bin_path)
}

#[cfg(not(windows))]
fn get_service_bin_path(_name: &str) -> Option<String> {
    None
}

/// Install a Windows service (elevated; runs from the bridge service or the
/// installer hook).
#[cfg(windows)]
fn install_service(name: &str, exe_path: &str) -> Result<(), String> {
    use crate::util::win_service::{self, CreateSpec, ServiceErrorControl, ServiceStartType};

    // Create the service. binPath MUST be the QUOTED path + `service` token —
    // empirically required on the live machine: a bare binPath (`X.exe`)
    // makes the start fail with error 2 "cannot find the file specified"
    // even when the file exists. The quotes + token are the only form that
    // reliably reaches RUNNING (verified across installs/repairs).
    let bin_path = format!("\"{}\" service", exe_path);
    win_service::create(
        name,
        &CreateSpec {
            binary_path: bin_path,
            display_name: "MiControl IoT Bridge Service".to_string(),
            start_type: ServiceStartType::Auto,
            error_control: ServiceErrorControl::Normal,
            deps: Vec::new(),
        },
    )
    // LocalSystem is the CreateServiceW default account: the old
    // `sc config obj= LocalSystem` re-statement was redundant and is gone.
    .map_err(|e| format!("Failed to create service: {e}"))?;

    // Failure actions to auto-restart (`sc failure reset= 86400 actions=
    // restart/5000/restart/10000/restart/30000`); best-effort like before.
    let actions = [
        win_service::FailureAction::restart(5000),
        win_service::FailureAction::restart(10000),
        win_service::FailureAction::restart(30000),
    ];
    if let Err(e) = win_service::set_failure_actions(name, &actions, 86400) {
        log::warn!("[ecram_service_mgmt] failure actions not set for {name}: {e}");
    }

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

/// Start a Windows service. `start` tolerates ERROR_SERVICE_ALREADY_RUNNING
/// (1056) like the old `sc start` + text check did.
#[cfg(windows)]
fn start_service(name: &str) -> Result<(), String> {
    crate::util::win_service::start(name).map_err(|e| format!("Failed to start service: {e}"))?;
    log::info!("[ecram_service_mgmt] Service '{}' started", name);
    Ok(())
}

#[cfg(not(windows))]
fn start_service(_name: &str) -> Result<(), String> {
    Err("Service management is only supported on Windows".to_string())
}

// ── C2a: bounded spawn tests ─────────────────────────────────────────────────

#[cfg(all(test, windows))]
mod ecram_service_mgmt_spawn_tests {
    use crate::util::process_spawn::{self, SpawnError};
    use std::process::Command;
    use std::time::{Duration, Instant};

    /// A child sleeping past its budget answers SpawnError::Timeout and the
    /// process is really dead, not orphaned behind the timed-out wait. This
    /// pins the contract every taskkill in the recreation path now relies on.
    #[test]
    fn ecram_service_mgmt_timeout_kills_sleeping_child() {
        let mut cmd = Command::new("cmd");
        cmd.args(["/C", "ping -n 10 127.0.0.1"]);
        let start = Instant::now();
        let err = process_spawn::run_capture(&mut cmd, Duration::from_millis(500))
            .expect_err("a sleeping child must hit the timeout");
        assert!(
            matches!(err, SpawnError::Timeout { .. }),
            "expected Timeout, got {err:?}"
        );
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "kill took too long: {:?}",
            start.elapsed()
        );

        std::thread::sleep(Duration::from_millis(300));
        let mut probe = Command::new("tasklist");
        probe.args(["/FI", "IMAGENAME eq ping.exe", "/NH"]);
        let out = process_spawn::run_capture(&mut probe, Duration::from_secs(15))
            .expect("tasklist probe failed");
        assert!(
            !out.stdout_str().contains("ping.exe"),
            "ping.exe still alive after timeout kill: {}",
            out.stdout_str()
        );
    }

    /// The kill helper stays best-effort for a missing image (taskkill's
    /// nonzero exit is a result, not a spawn error), and no direct child-wait
    /// calls remain in this file: every spawn goes through process_spawn.
    #[test]
    fn ecram_service_mgmt_no_direct_child_waits() {
        super::kill_process_image("mi_control_no_such_image_xyz")
            .expect("taskkill on a missing image is not a spawn failure");

        let src = include_str!("ecram_service_mgmt.rs");
        // Assembled from parts so this test's own source text cannot match
        // the tokens it forbids.
        let direct_waits = [
            format!(".{}{}{}", "out", "put", "()"),
            format!(".{}{}{}", "sta", "tus", "()"),
            format!(".{}{}", "wait", "()"),
        ];
        for token in &direct_waits {
            assert!(
                !src.contains(token.as_str()),
                "direct child wait present: {token}"
            );
        }
    }
}
