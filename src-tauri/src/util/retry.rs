//! Retry utilities for flaky operations (WMI, pipe, HID).
//!
//! Provides a `with_retry` helper that retries a fallible operation once
//! after a short delay, with logging.

/// Execute a fallible operation with one retry after a short delay.
/// Logs when a retry is used.
///
/// # Blocking note
/// This function uses `std::thread::sleep`, which is safe because all callers
/// run inside `tokio::task::spawn_blocking` (blocking thread pool). It is NOT
/// safe to call from an async context on a Tokio worker thread.
pub fn with_retry<T, E, F>(operation_name: &str, mut f: F) -> Result<T, E>
where
    F: FnMut() -> Result<T, E>,
    E: std::fmt::Display,
{
    match f() {
        Ok(result) => Ok(result),
        Err(first_err) => {
            log::warn!(
                "Operation '{}' failed ({}), retrying in 100ms...",
                operation_name,
                first_err
            );
            // thread::sleep is acceptable here — all callers are within
            // spawn_blocking (blocking thread pool), not on Tokio workers.
            std::thread::sleep(std::time::Duration::from_millis(100));
            match f() {
                Ok(result) => {
                    log::info!("Operation '{}' succeeded on retry", operation_name);
                    Ok(result)
                }
                Err(second_err) => {
                    log::error!(
                        "Operation '{}' failed after retry: {}",
                        operation_name,
                        second_err
                    );
                    Err(second_err)
                }
            }
        }
    }
}
