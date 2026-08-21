//! Helper for running blocking operations on the tokio blocking thread pool.
//!
//! Wraps `tokio::task::spawn_blocking` with consistent error handling,
//! converting join errors into [`HardwareError::TaskJoin`].

use crate::hw::errors::{HardwareError, HardwareResult};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::Semaphore;

/// Maximum time a synchronous hardware query may occupy an IPC command.
///
/// WMI and vendor named-pipe providers can stop responding after sleep. The
/// underlying blocking task cannot be cancelled safely, but the Tauri command
/// must still complete so the frontend can recover and show a real error.
pub const DEFAULT_BLOCKING_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_IN_FLIGHT_BLOCKING_TASKS: usize = 8;

fn blocking_slots() -> Arc<Semaphore> {
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SLOTS
        .get_or_init(|| Arc::new(Semaphore::new(MAX_IN_FLIGHT_BLOCKING_TASKS)))
        .clone()
}

/// Run a blocking closure on the tokio blocking thread pool.
///
/// This is a thin wrapper around `tokio::task::spawn_blocking` that maps
/// the `JoinError` (task panic, cancellation) into a [`HardwareError::TaskJoin`]
/// instead of requiring each call site to repeat the same `.map_err` boilerplate.
///
/// # Example
///
/// ```ignore
/// use crate::util::blocking::run_blocking;
///
/// let result: HardwareResult<u32> = run_blocking(|| Ok(42)).await;
/// ```
pub async fn run_blocking<T, F>(f: F) -> HardwareResult<T>
where
    F: FnOnce() -> HardwareResult<T> + Send + 'static,
    T: Send + 'static,
{
    run_blocking_timeout(DEFAULT_BLOCKING_TIMEOUT, f).await
}

/// Run a blocking closure with a bounded wait for its result.
pub async fn run_blocking_timeout<T, F>(timeout: Duration, f: F) -> HardwareResult<T>
where
    F: FnOnce() -> HardwareResult<T> + Send + 'static,
    T: Send + 'static,
{
    let permit = blocking_slots().try_acquire_owned().map_err(|_| {
        HardwareError::Timeout(
            "too many hardware operations are still blocked; retry after recovery".into(),
        )
    })?;
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        f()
    });
    match tokio::time::timeout(timeout, task).await {
        Ok(result) => {
            result.map_err(|e| HardwareError::TaskJoin(format!("Blocking task join error: {e}")))?
        }
        Err(_) => Err(HardwareError::Timeout(format!(
            "blocking hardware operation exceeded {} seconds",
            timeout.as_secs()
        ))),
    }
}
