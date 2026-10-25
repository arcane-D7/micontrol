//! Fixed-size dedicated thread pool for blocking hardware queries (S68).
//!
//! ## Why (the measured leak)
//!
//! The app's handle count grew ~15-16/min even after every RAII fix. A live
//! `handle64 -a` diff over 90 s showed the growth is:
//!   * **Thread handles** (~8/min) — threads created and dying,
//!   * **Sections** (`Theme*`, `__ComCatalogCache__`, `HWNDInterface:*`) —
//!     COM/OLE initialization artifacts,
//!   * **unnamed Events**.
//!
//! That is the signature of the **tokio blocking pool churning**: every
//! `spawn_blocking` WMI/hardware query runs on a pool thread; tokio's
//! blocking threads expire after ~10 s idle and are re-created on the next
//! burst. Each new thread initializes COM (the `wmi_cache` thread_local
//! creates fresh `WMIConnection`s), producing the Section/Event handles, and
//! the thread create/exit cycle retains Thread handles in the process handle
//! table (observed: 646 Thread handles for 318 unique TIDs, only ~82 alive).
//!
//! ## The fix
//!
//! A FIXED pool (default 4 threads) that NEVER exits — the same contract the
//! bridge service uses for its pipe workers (S55 FIX 23b) and its dispatch
//! pool (S68). Blocking work is submitted through an unbounded mpsc channel;
//! callers `recv_timeout` so a hung query cannot block the async runtime
//! (mirroring `util::blocking::run_blocking_timeout` semantics, including the
//! in-flight semaphore cap).
//!
//! Threads that never exit never run the buggy Rust std Windows TLS
//! destructor path either (the S55/S68 crash class) — a second win.
//!
//! ## Usage
//!
//! ```ignore
//! let result = crate::util::fixed_pool::run_blocking("wmi query", || {
//!     // synchronous WMI / pipe / registry work
//!     Ok(value)
//! }).await; // Result<T, HardwareError> with the same timeout semantics
//! ```

use crate::hw::errors::{HardwareError, HardwareResult};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::sync::Semaphore;

/// Default bounded wait for a submitted job (matches DEFAULT_BLOCKING_TIMEOUT).
pub const DEFAULT_FIXED_POOL_TIMEOUT: Duration = Duration::from_secs(15);

/// Pool size. 4 mirrors the bridge pipe-worker pool: enough to absorb the
/// startup burst of parallel hardware reads, small enough to bound COM
/// thread-locals and hung-query stacking.
const POOL_SIZE: usize = 4;

/// Cap on concurrent submissions (same value as util::blocking). Slots are
/// released when the wait resolves OR times out, so hung jobs cannot
/// permanently exhaust the budget.
const MAX_IN_FLIGHT: usize = 64;

type Job = Box<dyn FnOnce() + Send + 'static>;

struct Pool {
    tx: std::sync::mpsc::Sender<Job>,
}

impl Pool {
    fn new() -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<Job>();
        let rx = Arc::new(std::sync::Mutex::new(rx));
        for id in 0..POOL_SIZE {
            let rx = Arc::clone(&rx);
            // Long-lived worker threads — they NEVER exit, so:
            //   * no tokio blocking-pool churn (the measured handle leak),
            //   * no Rust std Windows TLS destructor on exit (the S55/S68
            //     crash class).
            let spawned = std::thread::Builder::new()
                .name(format!("hw-fixed-pool-{id}"))
                .spawn(move || loop {
                    let job = {
                        let guard = rx.lock().unwrap_or_else(|e| e.into_inner());
                        guard.recv()
                    };
                    let Ok(job) = job else {
                        break; // channel closed: process teardown
                    };
                    job();
                });
            if let Err(e) = spawned {
                // Thread spawn failed (resource exhaustion). The pool simply
                // runs with fewer workers; jobs still queue on the channel.
                log::error!("[fixed_pool] worker {id} failed to spawn: {e}");
            }
        }
        Self { tx }
    }

    fn submit(&self, job: Job) {
        if let Err(e) = self.tx.send(job) {
            log::error!("[fixed_pool] submit failed (pool gone): {e}");
        }
    }
}

fn pool() -> &'static Pool {
    static POOL: OnceLock<Pool> = OnceLock::new();
    POOL.get_or_init(Pool::new)
}

fn slots() -> Arc<Semaphore> {
    static SLOTS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    SLOTS
        .get_or_init(|| Arc::new(Semaphore::new(MAX_IN_FLIGHT)))
        .clone()
}

/// Run a blocking closure on the FIXED pool with the default timeout.
pub async fn run_blocking<T, F>(f: F) -> HardwareResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    run_blocking_timeout(DEFAULT_FIXED_POOL_TIMEOUT, f).await
}

/// Run a blocking closure on the FIXED pool with a bounded wait.
///
/// On timeout the underlying job may still be running on a pool thread — it
/// cannot be cancelled safely (same contract as `util::blocking`); the slot
/// is released immediately so hung jobs cannot exhaust the budget, and the
/// job's result is discarded when it eventually completes.
pub async fn run_blocking_timeout<T, F>(timeout: Duration, f: F) -> HardwareResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let permit = slots().try_acquire_owned().map_err(|_| {
        HardwareError::Timeout(
            "too many hardware operations are still blocked; retry after recovery".into(),
        )
    })?;

    let (tx, rx) = std::sync::mpsc::channel::<T>();
    pool().submit(Box::new(move || {
        // The receiver may already be gone (timeout fired) — send is
        // best-effort and the result is simply dropped.
        let _ = tx.send(f());
    }));

    match rx.recv_timeout(timeout) {
        Ok(value) => {
            drop(permit);
            Ok(value)
        }
        Err(_) => {
            // Release the slot NOW (same rationale as util::blocking): the
            // "too many hardware operations" failure must not become
            // permanent because a few hung jobs hold every slot.
            drop(permit);
            Err(HardwareError::Timeout(format!(
                "blocking hardware operation exceeded {} seconds",
                timeout.as_secs()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn runs_closure_and_returns_value() {
        let v = run_blocking(|| 42).await.expect("value");
        assert_eq!(v, 42);
    }

    #[tokio::test]
    async fn timeout_returns_error_and_releases_slot() {
        let start = std::time::Instant::now();
        let r = run_blocking_timeout(Duration::from_millis(100), || {
            std::thread::sleep(Duration::from_secs(2));
            1
        })
        .await;
        assert!(r.is_err(), "must time out");
        assert!(
            start.elapsed() < Duration::from_secs(1),
            "must return at the timeout, not after the job"
        );
        // The slot must be free again: a second submission succeeds.
        let v = run_blocking(|| 7).await.expect("slot free");
        assert_eq!(v, 7);
    }

    #[tokio::test]
    async fn pool_threads_are_stable() {
        // Submit many jobs; the pool must serve them without growing threads.
        // (We cannot observe the thread count directly from here, but a burst
        // completing quickly proves the pool is alive and not starved.)
        let mut handles = Vec::new();
        for i in 0..32 {
            handles.push(run_blocking(move || i * 2));
        }
        let mut sum = 0;
        for h in handles {
            sum += h.await.expect("burst job");
        }
        assert_eq!(sum, (0..32).map(|i| i * 2).sum::<i32>());
    }
}
