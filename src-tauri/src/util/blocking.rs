//! Helper for running blocking operations on a FIXED dedicated thread pool.
//!
//! Wraps [`crate::util::fixed_pool::run_blocking`] with consistent error
//! handling, converting join errors into [`HardwareError::TaskJoin`].
//!
//! S68 (handle-leak root cause): this used `tokio::task::spawn_blocking`,
//! whose pool threads expire after ~10 s idle and are re-created on every
//! poll burst. Each new thread initializes COM (the `wmi_cache` thread_local
//! creates fresh WMIConnections), producing Section/Event handles, and the
//! create/exit churn retained Thread handles in the process handle table
//! (measured: ~15-16 handles/min, 646 Thread handles for 318 TIDs with only
//! ~82 alive). The fixed pool never exits its threads, so the churn — and
//! the buggy Rust std Windows TLS destructor path on thread exit — never
//! runs. Same public API; all 129 call sites migrated at once.

use crate::hw::errors::HardwareResult;
use std::time::Duration;

/// Maximum time a synchronous hardware query may occupy an IPC command.
///
/// WMI and vendor named-pipe providers can stop responding after sleep. The
/// underlying blocking task cannot be cancelled safely, but the Tauri command
/// must still complete so the frontend can recover and show a real error.
pub const DEFAULT_BLOCKING_TIMEOUT: Duration = Duration::from_secs(15);

/// Run a blocking closure on the fixed pool.
///
/// This is a thin wrapper around [`crate::util::fixed_pool::run_blocking`]
/// that maps the timeout error into [`HardwareError::Timeout`] (the fixed
/// pool already returns that shape; the wrapper keeps the historical public
/// surface stable for the 129 call sites).
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
    crate::util::fixed_pool::run_blocking(f).await?
}

/// Run a blocking closure with a bounded wait for its result.
pub async fn run_blocking_timeout<T, F>(timeout: Duration, f: F) -> HardwareResult<T>
where
    F: FnOnce() -> HardwareResult<T> + Send + 'static,
    T: Send + 'static,
{
    crate::util::fixed_pool::run_blocking_timeout(timeout, f).await?
}
