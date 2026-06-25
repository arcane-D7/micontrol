//! Helper for running blocking operations on the tokio blocking thread pool.
//!
//! Wraps `tokio::task::spawn_blocking` with consistent error handling,
//! converting join errors into [`HardwareError::TaskJoin`].

use crate::hw::errors::{HardwareError, HardwareResult};

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
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| HardwareError::TaskJoin(format!("Blocking task join error: {e}")))?
}
