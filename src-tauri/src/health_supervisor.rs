//! Runtime health supervision and bounded self-healing for recoverable modules.

use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const CHECK_INTERVAL: Duration = Duration::from_secs(30);
const RECOVERY_COOLDOWN_MS: u64 = 60_000;

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
    check_bridge().await;
    check_iot().await;
    check_face().await;
    check_ambient_sensor().await;
    if let Ok(mut snapshot) = state().lock() {
        snapshot.last_cycle_ms = now_ms();
    }
}

async fn check_bridge() {
    let available = tokio::task::spawn_blocking(crate::elev_bridge::is_bridge_service_available)
        .await
        .unwrap_or(false);
    let at = now_ms();
    let should_recover = state()
        .lock()
        .map(|mut snapshot| {
            begin_check(&mut snapshot.bridge, at);
            if available {
                mark_healthy(&mut snapshot.bridge, at);
                false
            } else {
                mark_failure(&mut snapshot.bridge, at, "bridge pipe unavailable".into())
            }
        })
        .unwrap_or(false);

    if should_recover {
        if let Ok(mut snapshot) = state().lock() {
            mark_recovering(&mut snapshot.bridge, at);
        }
        match crate::elev_bridge::ensure_bridge_service().await {
            Ok(_) => {
                if let Ok(mut snapshot) = state().lock() {
                    mark_healthy(&mut snapshot.bridge, now_ms());
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

async fn check_iot() {
    let available = tokio::task::spawn_blocking(crate::hw::iotservice::is_pipe_available)
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
        match crate::elev_bridge::run_elevated_no_prompt(
            "ensure_ecram_service",
            serde_json::Value::Null,
        )
        .await
        {
            Ok(_) => {
                if let Ok(mut snapshot) = state().lock() {
                    mark_healthy(&mut snapshot.iot, now_ms());
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
    let result = crate::commands::face::face_status().await;
    let at = now_ms();
    let healthy = result
        .as_ref()
        .map(|status| status.service_running && status.pipe_available)
        .unwrap_or(false);
    let should_recover = state()
        .lock()
        .map(|mut snapshot| {
            begin_check(&mut snapshot.face, at);
            if healthy {
                mark_healthy(&mut snapshot.face, at);
                false
            } else {
                let error = result
                    .err()
                    .map(|value| value.message)
                    .unwrap_or_else(|| "Face service or pipe unavailable".into());
                mark_failure(&mut snapshot.face, at, error)
            }
        })
        .unwrap_or(false);

    if should_recover {
        if let Ok(mut snapshot) = state().lock() {
            mark_recovering(&mut snapshot.face, at);
        }
        match crate::elev_bridge::ensure_face_service().await {
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
    let result = tokio::task::spawn_blocking(crate::hw::display::get_display_info)
        .await
        .ok()
        .and_then(Result::ok);
    let at = now_ms();
    let healthy = result
        .as_ref()
        .and_then(|display| display.ambient_lux)
        .is_some_and(|lux| lux.is_finite() && lux > 0.5);

    if let Ok(mut snapshot) = state().lock() {
        begin_check(&mut snapshot.ambient_sensor, at);
        if healthy {
            mark_healthy(&mut snapshot.ambient_sensor, at);
        } else {
            snapshot.ambient_sensor.state = HealthState::Degraded;
            snapshot.ambient_sensor.consecutive_failures = snapshot
                .ambient_sensor
                .consecutive_failures
                .saturating_add(1);
            snapshot.ambient_sensor.last_error = Some("ambient sensor reading unavailable".into());
        }
    }

    if !healthy {
        crate::hw::display::request_sensor_reset();
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
