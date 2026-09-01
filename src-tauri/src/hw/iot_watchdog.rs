#![cfg(windows)]
//! IoT pipe watchdog (MIOT-03).
//!
//! Background task that periodically verifies the IoT pipe backends are alive:
//!
//! - our custom `ecram_service` named pipe (`\\.\pipe\ecram_service`), or
//! - the original Xiaomi `IoTService_IPC_Broker` pipe.
//!
//! If neither backend answers, the watchdog calls
//! `ecram_service_mgmt::ensure_service_running()` to reinstall/restart the
//! ecram service, exactly like the first-run install path does. Recovery is
//! gated by a bounded cooldown so a persisting failure does not spin.
//!
//! Isolation: fully standalone — only reads `iotservice::is_pipe_available()`
//! and calls the public service-management function. It never touches other
//! hardware modules or the UI.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// Cycle interval for the health probe.
const WATCHDOG_INTERVAL: Duration = Duration::from_secs(30);
/// Minimum delay between two service-recovery attempts.
const RECOVERY_COOLDOWN: Duration = Duration::from_secs(300);

/// Watchdog runtime state.
struct WatchdogState {
    running: AtomicBool,
    last_recovery: OnceLock<std::sync::Mutex<Option<Instant>>>,
}

impl WatchdogState {
    fn new() -> Self {
        Self {
            running: AtomicBool::new(false),
            last_recovery: OnceLock::new(),
        }
    }

    fn last_recovery(&self) -> &std::sync::Mutex<Option<Instant>> {
        self.last_recovery
            .get_or_init(|| std::sync::Mutex::new(None))
    }

    /// Returns true when a recovery attempt is allowed (cooldown elapsed).
    fn recovery_allowed(&self) -> bool {
        let mut guard = match self.last_recovery().lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        match *guard {
            Some(last) if last.elapsed() < RECOVERY_COOLDOWN => false,
            _ => {
                *guard = Some(Instant::now());
                true
            }
        }
    }
}

static WATCHDOG: OnceLock<WatchdogState> = OnceLock::new();

fn watchdog() -> &'static WatchdogState {
    WATCHDOG.get_or_init(WatchdogState::new)
}

/// Start the background watchdog loop. Idempotent.
pub fn start_iot_watchdog() {
    if watchdog().running.swap(true, Ordering::SeqCst) {
        return;
    }

    tauri::async_runtime::spawn(async {
        let mut interval = tokio::time::interval(WATCHDOG_INTERVAL);
        // First tick fires immediately.
        interval.tick().await;
        loop {
            interval.tick().await;
            check_once();
        }
    });
}

/// Probe the IoT pipes and recover the service if they are down.
///
/// Public so callers (and tests) can trigger a single check.
pub fn check_once() {
    if crate::hw::iotservice::is_pipe_available() {
        return;
    }

    log::warn!("[iot_watchdog] IoT pipe unavailable — attempting service recovery");
    if !watchdog().recovery_allowed() {
        log::debug!("[iot_watchdog] recovery on cooldown, skipping");
        return;
    }

    // Route the recovery through the elevated chain (MiControlBridge SYSTEM
    // service pipe, else the scheduled task). Creating/replacing the
    // `IoTSvc` service and copying into the DriverStore requires
    // Administrator rights — calling `sc create` from the unprivileged app
    // process ALWAYS fails with access denied (error 5), spamming the log.
    // The elevated path runs as SYSTEM, so it succeeds silently.
    tauri::async_runtime::spawn(async move {
        match crate::elev_bridge::run_elevated_no_prompt(
            "ensure_ecram_service",
            serde_json::json!({}),
        )
        .await
        {
            Ok(v) => {
                log::info!(
                    "[iot_watchdog] recovery done via elevated path: {v} \
                     (pipe_available={})",
                    crate::hw::iotservice::is_pipe_available()
                );
            }
            Err(e) => {
                log::error!("[iot_watchdog] service recovery failed: {e}");
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_allowed_initial() {
        // First call always allowed (no recorded timestamp yet).
        let w = WatchdogState::new();
        assert!(w.recovery_allowed());
    }

    #[test]
    fn recovery_disallowed_within_cooldown() {
        let w = WatchdogState::new();
        assert!(w.recovery_allowed());
        // Immediately after a recorded recovery, no new attempt.
        assert!(!w.recovery_allowed());
    }
}
