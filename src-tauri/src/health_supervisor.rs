//! Runtime health supervision and bounded self-healing for recoverable modules.

use serde::Serialize;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const CHECK_INTERVAL: Duration = Duration::from_secs(30);
const RECOVERY_COOLDOWN_MS: u64 = 60_000;
const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(5);
const HEALTH_RECOVERY_TIMEOUT: Duration = Duration::from_secs(15);

static STARTED: AtomicBool = AtomicBool::new(false);
static SNAPSHOT: OnceLock<Mutex<HealthSnapshot>> = OnceLock::new();

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Unknown,
    Healthy,
    Degraded,
    Recovering,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComponentHealth {
    pub state: HealthState,
    pub consecutive_failures: u32,
    pub last_check_ms: u64,
    pub last_recovery_ms: Option<u64>,
    pub last_error: Option<String>,
}

impl Default for ComponentHealth {
    fn default() -> Self {
        Self {
            state: HealthState::Unknown,
            consecutive_failures: 0,
            last_check_ms: 0,
            last_recovery_ms: None,
            last_error: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct HealthSnapshot {
    pub bridge: ComponentHealth,
    pub iot: ComponentHealth,
    pub face: ComponentHealth,
    pub ambient_sensor: ComponentHealth,
    pub last_cycle_ms: u64,
}

fn state() -> &'static Mutex<HealthSnapshot> {
    SNAPSHOT.get_or_init(|| Mutex::new(HealthSnapshot::default()))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn snapshot() -> HealthSnapshot {
    state()
        .lock()
        .map(|value| value.clone())
        .unwrap_or_default()
}

pub fn start() {
    if STARTED.swap(true, Ordering::AcqRel) {
        return;
    }

    tauri::async_runtime::spawn(async {
        run_cycle().await;
        let mut interval = tokio::time::interval(CHECK_INTERVAL);
        interval.tick().await;
        loop {
            interval.tick().await;
            run_cycle().await;
        }
    });
}

fn begin_check(component: &mut ComponentHealth, at: u64) {
    component.last_check_ms = at;
}

fn mark_healthy(component: &mut ComponentHealth, at: u64) {
    component.state = HealthState::Healthy;
    component.consecutive_failures = 0;
    component.last_check_ms = at;
    component.last_error = None;
}

fn mark_failure(component: &mut ComponentHealth, at: u64, error: String) -> bool {
    component.last_check_ms = at;
    component.consecutive_failures = component.consecutive_failures.saturating_add(1);
    component.last_error = Some(error);
    component.state = HealthState::Degraded;
    component
        .last_recovery_ms
        .map(|last| at.saturating_sub(last) >= RECOVERY_COOLDOWN_MS)
        .unwrap_or(true)
}

fn mark_recovering(component: &mut ComponentHealth, at: u64) {
    component.state = HealthState::Recovering;
    component.last_recovery_ms = Some(at);
}

async fn run_cycle() {
    // Keep one unavailable provider from delaying health state for every
    // other module after a sleep/resume cycle.
    let _ = tokio::join!(
        check_bridge(),
        check_iot(),
        check_face(),
        check_ambient_sensor()
    );
    if let Ok(mut snapshot) = state().lock() {
        snapshot.last_cycle_ms = now_ms();
    }
}

async fn bounded_recovery<T>(
    operation: impl Future<Output = Result<T, String>>,
    name: &str,
) -> Result<T, String> {
    tokio::time::timeout(HEALTH_RECOVERY_TIMEOUT, operation)
        .await
        .map_err(|_| format!("{name} recovery timed out"))?
}

async fn check_bridge() {
    let health = probe_bridge_health().await;
    let at = now_ms();
    // S51: keep the elev_bridge circuit breaker in sync — when the bridge
    // service comes back (or goes away) the breaker must know. Note that a
    // RUNNING service with a wedged pipe still counts as "infrastructure
    // present": the scheduled-task path may work, and P1 must not treat the
    // busy pipe as proof that the service is missing.
    if health.service_running() {
        crate::elev_bridge::note_elevated_recovery();
    }
    let should_recover = state()
        .lock()
        .map(|mut snapshot| {
            begin_check(&mut snapshot.bridge, at);
            if health.is_healthy() {
                mark_healthy(&mut snapshot.bridge, at);
                false
            } else {
                mark_failure(&mut snapshot.bridge, at, health.describe().to_string())
            }
        })
        .unwrap_or(false);

    if should_recover {
        if let Ok(mut snapshot) = state().lock() {
            mark_recovering(&mut snapshot.bridge, at);
        }
        match bounded_recovery(crate::elev_bridge::ensure_bridge_service(), "bridge").await {
            Ok(_) => {
                // Re-probe before declaring recovery: `ensure_bridge_service`
                // answering "already_running" (P1, SCM-based) says nothing about
                // the data path — C-01 leaves a RUNNING service with a dead pipe,
                // and marking it healthy here would hide that forever.
                let after = probe_bridge_health().await;
                if let Ok(mut snapshot) = state().lock() {
                    if after.is_healthy() {
                        mark_healthy(&mut snapshot.bridge, now_ms());
                    } else {
                        snapshot.bridge.state = HealthState::Degraded;
                        snapshot.bridge.last_error = Some(after.describe().to_string());
                    }
                }
            }
            Err(error) => {
                if let Ok(mut snapshot) = state().lock() {
                    snapshot.bridge.state = HealthState::Failed;
                    snapshot.bridge.last_error = Some(error);
                }
            }
        }
    }
}

/// Probe the bridge (SCM + pipe) off the async runtime.
///
/// `bridge_health()` performs two blocking Win32 calls (SCM query + a
/// `CreateFileW` on the pipe), so it must not run on a runtime worker.
async fn probe_bridge_health() -> crate::elev_bridge::BridgeHealth {
    crate::util::blocking::run_blocking_timeout(HEALTH_CHECK_TIMEOUT, || {
        Ok(crate::elev_bridge::bridge_health())
    })
    .await
    .unwrap_or(crate::elev_bridge::BridgeHealth::ServiceUnknown)
}

async fn check_iot() {
    let available = crate::util::blocking::run_blocking_timeout(HEALTH_CHECK_TIMEOUT, || {
        Ok(crate::hw::iotservice::is_pipe_available())
    })
    .await
    .unwrap_or(false);
    let at = now_ms();
    let should_recover = state()
        .lock()
        .map(|mut snapshot| {
            begin_check(&mut snapshot.iot, at);
            if available {
                mark_healthy(&mut snapshot.iot, at);
                false
            } else {
                mark_failure(&mut snapshot.iot, at, "IoT service pipe unavailable".into())
            }
        })
        .unwrap_or(false);

    if should_recover {
        if let Ok(mut snapshot) = state().lock() {
            mark_recovering(&mut snapshot.iot, at);
        }
        match bounded_recovery(
            crate::elev_bridge::run_elevated_no_prompt(
                "ensure_ecram_service",
                serde_json::Value::Null,
            ),
            "IoT service",
        )
        .await
        {
            Ok(_) => {
                // An `ok` answer is not proof: the healer returns `ok` for a
                // throttled no-op and for a healthy verification alike. Re-probe
                // the data path and only declare health on a positive readback,
                // the same rule `check_bridge` follows.
                let after =
                    crate::util::blocking::run_blocking_timeout(HEALTH_CHECK_TIMEOUT, || {
                        Ok(crate::hw::iotservice::is_pipe_available())
                    })
                    .await
                    .unwrap_or(false);
                if let Ok(mut snapshot) = state().lock() {
                    if after {
                        mark_healthy(&mut snapshot.iot, now_ms());
                    } else {
                        snapshot.iot.state = HealthState::Degraded;
                        snapshot.iot.last_error =
                            Some("IoT service pipe still unavailable after ensure".into());
                    }
                }
            }
            Err(error) => {
                if let Ok(mut snapshot) = state().lock() {
                    snapshot.iot.state = HealthState::Failed;
                    snapshot.iot.last_error = Some(error);
                }
            }
        }
    }
}

#[cfg(windows)]
async fn check_face() {
    // Lightweight probe — must NOT open the webcam (that would blink the
    // camera LED every 30 s). `face_health()` only checks the SCM service
    // state and the auth pipe, which is what "is the module alive" means.
    let healthy = crate::commands::face::face_health().await;
    let at = now_ms();
    let should_recover = state()
        .lock()
        .map(|mut snapshot| {
            begin_check(&mut snapshot.face, at);
            if healthy {
                mark_healthy(&mut snapshot.face, at);
                false
            } else {
                let error = "Face service or pipe unavailable".into();
                mark_failure(&mut snapshot.face, at, error)
            }
        })
        .unwrap_or(false);

    if should_recover {
        if let Ok(mut snapshot) = state().lock() {
            mark_recovering(&mut snapshot.face, at);
        }
        match bounded_recovery(crate::elev_bridge::ensure_face_service(), "Face service").await {
            Ok(_) => {
                if let Ok(mut snapshot) = state().lock() {
                    mark_healthy(&mut snapshot.face, now_ms());
                }
            }
            Err(error) => {
                if let Ok(mut snapshot) = state().lock() {
                    snapshot.face.state = HealthState::Failed;
                    snapshot.face.last_error = Some(error);
                }
            }
        }
    }
}

#[cfg(not(windows))]
async fn check_face() {
    let at = now_ms();
    if let Ok(mut snapshot) = state().lock() {
        begin_check(&mut snapshot.face, at);
        mark_healthy(&mut snapshot.face, at);
    }
}

async fn check_ambient_sensor() {
    let result = crate::util::blocking::run_blocking_timeout(
        HEALTH_CHECK_TIMEOUT,
        crate::hw::display::get_display_info,
    )
    .await
    .ok();
    let at = now_ms();
    // The diagnosis (Available / NoSensor / FrozenReport) decides what a
    // failed reading MEANS. A diagnosed-stable sensor must not accumulate
    // consecutive_failures forever or trigger reset bursts: the hardware is
    // known (production proof: the TCS3408 froze for 4+ days while the
    // supervisor requested a reset every cycle).
    let sensor_state = crate::hw::display::ambient_sensor_state();
    let diagnosed = sensor_state.is_diagnosed();
    let healthy = result.as_ref().is_some_and(|d| {
        d.ambient_lux
            .is_some_and(|lux| lux.is_finite() && lux > 0.5)
    });
    let mut should_request_reset = false;
    if let Ok(mut snapshot) = state().lock() {
        begin_check(&mut snapshot.ambient_sensor, at);
        if healthy {
            mark_healthy(&mut snapshot.ambient_sensor, at);
        } else if diagnosed {
            snapshot.ambient_sensor.state = HealthState::Degraded;
            snapshot.ambient_sensor.last_error = Some(match sensor_state {
                crate::hw::display::AmbientSensorState::FrozenReport => {
                    "ambient sensor present but not reporting (frozen reports)".to_string()
                }
                _ => "this device has no ambient light sensor".to_string(),
            });
        } else {
            // Undiagnosed (Unknown / transient Available): keep the old
            // behaviour — count failures and reset after the burst.
            snapshot.ambient_sensor.state = HealthState::Degraded;
            snapshot.ambient_sensor.consecutive_failures = snapshot
                .ambient_sensor
                .consecutive_failures
                .saturating_add(1);
            snapshot.ambient_sensor.last_error =
                Some("ambient sensor reading unavailable".to_string());
            should_request_reset = true;
        }
    }
    if should_request_reset {
        crate::hw::display::request_sensor_reset("ambient sensor degraded");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_is_cooldown_gated() {
        let mut component = ComponentHealth::default();
        assert!(mark_failure(&mut component, 1, "first".into()));
        mark_recovering(&mut component, 1);
        assert!(!mark_failure(&mut component, 2, "second".into()));
        assert!(mark_failure(
            &mut component,
            RECOVERY_COOLDOWN_MS + 2,
            "third".into()
        ));
    }
}
