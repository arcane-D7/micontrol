//! Retry utilities for flaky operations (WMI, pipe, HID).
//!
//! Provides `with_retry` (one retry after a short delay) and `with_retry_backoff`
//! (configurable exponential backoff with jitter) helpers.

use rand::Rng;
use std::time::Duration;

/// Execute a fallible operation with exponential backoff and jitter.
///
/// Retries up to `max_retries` times, with delays that grow exponentially
/// (multiplied by `backoff_multiplier` each attempt, capped at `max_delay`).
/// A ±20% jitter is applied to each delay to prevent thundering herd.
///
/// # Blocking note
/// This function uses `std::thread::sleep`, which is safe because all callers
/// run inside `tokio::task::spawn_blocking` (blocking thread pool). It is NOT
/// safe to call from an async context on a Tokio worker thread.
pub fn with_retry_backoff<F, T, E>(
    max_retries: u32,
    initial_delay: Duration,
    backoff_multiplier: f64,
    max_delay: Duration,
    mut f: F,
) -> Result<T, E>
where
    F: FnMut() -> Result<T, E>,
    E: std::fmt::Display,
{
    let mut delay = initial_delay;
    let mut rng = rand::thread_rng();

    // S25-010: Restructured to avoid unreachable!() — the final attempt is
    // handled by the loop's natural fall-through, not a panic.
    for attempt in 0..max_retries {
        match f() {
            Ok(result) => return Ok(result),
            Err(e) => {
                // Apply ±20% jitter to prevent thundering herd
                let jitter_factor = 1.0 + rng.gen_range(-0.2..0.2);
                let jittered_ms = (delay.as_millis() as f64 * jitter_factor).max(0.0);
                let sleep_duration = Duration::from_millis(jittered_ms as u64);

                log::warn!(
                    "Operation failed (attempt {}/{}): {}, retrying in {:?}...",
                    attempt + 1,
                    max_retries + 1,
                    e,
                    sleep_duration
                );

                std::thread::sleep(sleep_duration);

                // Compute next delay with exponential backoff, capped at max_delay
                let next_ms = (delay.as_millis() as f64 * backoff_multiplier)
                    .min(max_delay.as_millis() as f64);
                delay = Duration::from_millis(next_ms as u64);
            }
        }
    }

    // Final attempt (no retry after this).
    match f() {
        Ok(result) => Ok(result),
        Err(e) => {
            log::error!("Operation failed after {} retries: {}", max_retries, e);
            Err(e)
        }
    }
}

/// Execute a fallible operation with default retry settings.
///
/// Thin wrapper around [`with_retry_backoff`] with defaults:
/// - Max retries: 3
/// - Initial delay: 100ms
/// - Backoff multiplier: 2.0
/// - Max delay: 1000ms
///
/// # Blocking note
/// This function uses `std::thread::sleep`, which is safe because all callers
/// run inside `tokio::task::spawn_blocking` (blocking thread pool). It is NOT
/// safe to call from an async context on a Tokio worker thread.
pub fn with_retry<T, E, F>(operation_name: &str, f: F) -> Result<T, E>
where
    F: FnMut() -> Result<T, E>,
    E: std::fmt::Display,
{
    log::trace!("with_retry: '{}'", operation_name);
    with_retry_backoff(
        3,
        Duration::from_millis(100),
        2.0,
        Duration::from_millis(1000),
        f,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::time::Instant;

    #[test]
    fn test_succeeds_on_first_attempt() {
        let count = RefCell::new(0);
        let result: Result<i32, String> = with_retry_backoff(
            3,
            Duration::from_millis(1),
            2.0,
            Duration::from_millis(10),
            || {
                *count.borrow_mut() += 1;
                Ok(42)
            },
        );
        assert_eq!(result.unwrap(), 42);
        assert_eq!(*count.borrow(), 1);
    }

    #[test]
    fn test_succeeds_after_retries() {
        let count = RefCell::new(0);
        let result: Result<i32, String> = with_retry_backoff(
            3,
            Duration::from_millis(1),
            2.0,
            Duration::from_millis(10),
            || {
                *count.borrow_mut() += 1;
                if *count.borrow() < 3 {
                    Err("fail".to_string())
                } else {
                    Ok(42)
                }
            },
        );
        assert_eq!(result.unwrap(), 42);
        assert_eq!(*count.borrow(), 3);
    }

    #[test]
    fn test_max_retries_exhausted() {
        let count = RefCell::new(0);
        let result: Result<i32, String> = with_retry_backoff(
            3,
            Duration::from_millis(1),
            2.0,
            Duration::from_millis(10),
            || {
                *count.borrow_mut() += 1;
                Err("always fails".to_string())
            },
        );
        assert!(result.is_err());
        // 1 initial attempt + 3 retries = 4 total calls
        assert_eq!(*count.borrow(), 4);
    }

    #[test]
    fn test_zero_retries() {
        let count = RefCell::new(0);
        let result: Result<i32, String> = with_retry_backoff(
            0,
            Duration::from_millis(1),
            2.0,
            Duration::from_millis(10),
            || {
                *count.borrow_mut() += 1;
                Err("fail".to_string())
            },
        );
        assert!(result.is_err());
        assert_eq!(*count.borrow(), 1);
    }

    #[test]
    fn test_backoff_timing() {
        // With initial_delay=10ms, multiplier=2.0, max_delay=100ms:
        // Delays: ~10ms, ~20ms, ~40ms (each ±20% jitter)
        // Total: 56ms–84ms
        let start = Instant::now();
        let result: Result<i32, String> = with_retry_backoff(
            3,
            Duration::from_millis(10),
            2.0,
            Duration::from_millis(100),
            || Err("fail".to_string()),
        );
        let elapsed = start.elapsed();
        assert!(result.is_err());
        assert!(
            elapsed >= Duration::from_millis(40),
            "Elapsed {:?} should be at least 40ms",
            elapsed
        );
        assert!(
            elapsed <= Duration::from_millis(200),
            "Elapsed {:?} should be at most 200ms",
            elapsed
        );
    }

    #[test]
    fn test_max_delay_cap() {
        // With initial_delay=10ms, multiplier=10.0, max_delay=15ms:
        // Delays: ~10ms, ~15ms (capped), ~15ms (capped)
        // Total: 32ms–48ms
        let start = Instant::now();
        let result: Result<i32, String> = with_retry_backoff(
            3,
            Duration::from_millis(10),
            10.0,
            Duration::from_millis(15),
            || Err("fail".to_string()),
        );
        let elapsed = start.elapsed();
        assert!(result.is_err());
        assert!(
            elapsed >= Duration::from_millis(20),
            "Elapsed {:?} should respect max_delay cap (>= 20ms)",
            elapsed
        );
        assert!(
            elapsed <= Duration::from_millis(150),
            "Elapsed {:?} should not exceed max_delay cap significantly (<= 150ms)",
            elapsed
        );
    }
}
