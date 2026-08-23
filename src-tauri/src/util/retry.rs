//! Retry utilities for flaky operations (WMI, pipe, HID).
//!
//! Provides `with_retry` (one retry after a short delay) and `with_retry_backoff`
//! (configurable exponential backoff with jitter) helpers.
//!
//! Errors implementing [`ShouldRetry`] are classified as permanent or transient.
//! Permanent errors (e.g. WMI `WBEM_E_NOT_FOUND` 0x80041002) skip all retries
//! and return immediately, avoiding wasted CPU and log spam on every poll cycle.

use rand::Rng;
use std::time::Duration;

/// Compute the next backoff delay: `current * multiplier`, capped at `max_delay`.
///
/// Pure function extracted from [`with_retry_backoff`] so the exponential
/// growth and cap can be tested deterministically without wall-clock sleeps.
/// A `backoff_multiplier` below 1.0 would shrink delays — callers pass >= 1.0.
fn backoff_delay(current: Duration, multiplier: f64, max_delay: Duration) -> Duration {
    let next_ms = (current.as_millis() as f64 * multiplier).min(max_delay.as_millis() as f64);
    Duration::from_millis(next_ms as u64)
}

/// Trait for classifying whether an error should be retried.
///
/// Implement this for error types that have permanent (non-retryable) variants.
/// The default implementation returns `true` (retry everything), preserving
/// backward compatibility for error types that don't implement this trait.
pub trait ShouldRetry {
    /// Returns `true` if the error is transient and may succeed on retry,
    /// `false` if the error is permanent and retrying will never help.
    fn should_retry(&self) -> bool {
        true
    }
}

/// Permanent WMI HRESULT codes — retrying the identical query will always fail.
const PERMANENT_HRESULTS: &[u32] = &[
    0x80041003, // WBEM_E_ACCESS_DENIED
    0x8004100E, // WBEM_E_INVALID_NAMESPACE
    0x80041010, // WBEM_E_INVALID_CLASS
    0x80041017, // WBEM_E_INVALID_QUERY
    0x80041002, // WBEM_E_NOT_FOUND
];

/// Check if a raw HRESULT code is in the permanent (non-retryable) set.
fn is_permanent_hresult(hres: u32) -> bool {
    PERMANENT_HRESULTS.contains(&hres)
}

#[cfg(windows)]
fn extract_hresult_from_error(err: &(dyn std::error::Error + 'static)) -> Option<u32> {
    // Check for wmi::WMIError::HResultError
    if let Some(wmi::WMIError::HResultError { hres }) = err.downcast_ref::<wmi::WMIError>() {
        return Some(*hres as u32);
    }
    // Check for windows::core::Error (from direct COM calls in wmi_ec.rs, hq_wmi.rs)
    if let Some(win_err) = err.downcast_ref::<windows::core::Error>() {
        let code = win_err.code();
        let hres = code.0 as u32;
        if hres != 0 {
            return Some(hres);
        }
    }
    None
}

#[cfg(not(windows))]
fn extract_hresult_from_error(_err: &(dyn std::error::Error + 'static)) -> Option<u32> {
    None
}

// Blanket implementation for anyhow::Error — checks the error chain for
// WMI permanent HRESULT codes (0x80041003, 0x8004100E, 0x80041010,
// 0x80041017, 0x80041002).
//
// Checks both `wmi::WMIError` (from raw_query) and `windows::core::Error`
// (from direct COM calls like GetObject/ExecQuery/ExecMethod in wmi_ec.rs
// and hq_wmi.rs).
impl ShouldRetry for anyhow::Error {
    fn should_retry(&self) -> bool {
        // Check the top-level error
        if let Some(hres) = extract_hresult_from_error(self.as_ref()) {
            if is_permanent_hresult(hres) {
                return false;
            }
        }

        // Walk the error chain in case the error is wrapped inside
        // another error type (e.g. HardwareError::WmiQuery).
        let mut source: Option<&dyn std::error::Error> = Some(self.as_ref());
        while let Some(err) = source {
            if let Some(hres) = extract_hresult_from_error(err) {
                if is_permanent_hresult(hres) {
                    return false;
                }
            }
            source = err.source();
        }
        true
    }
}

/// Execute a fallible operation with exponential backoff and jitter.
///
/// Retries up to `max_retries` times, with delays that grow exponentially
/// (multiplied by `backoff_multiplier` each attempt, capped at `max_delay`).
/// A ±20% jitter is applied to each delay to prevent thundering herd.
///
/// If the error type implements [`ShouldRetry`] and returns `false`, the
/// error is permanent and is returned immediately without retrying.
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
    E: std::fmt::Display + ShouldRetry,
{
    let mut delay = initial_delay;
    let mut rng = rand::thread_rng();

    // S25-010: Restructured to avoid unreachable!() — the final attempt is
    // handled by the loop's natural fall-through, not a panic.
    for attempt in 0..max_retries {
        match f() {
            Ok(result) => return Ok(result),
            Err(e) => {
                // Check if this is a permanent error that should not be retried
                if !e.should_retry() {
                    // 0x80041002 (WBEM_E_NOT_FOUND) is expected intermittently
                    // from the MICommonInterface WMI provider. Log at trace
                    // level to avoid log spam in trace mode.
                    let err_str = format!("{e}");
                    let log_level = if err_str.contains("0x80041002") {
                        log::Level::Trace
                    } else {
                        log::Level::Debug
                    };
                    log::log!(
                        log_level,
                        "Operation failed with permanent error (attempt {}/{}): {} — not retrying",
                        attempt + 1,
                        max_retries + 1,
                        e
                    );
                    return Err(e);
                }

                // Apply ±20% jitter to prevent thundering herd
                let jitter_factor = 1.0 + rng.gen_range(-0.2..0.2);
                let jittered_ms = (delay.as_millis() as f64 * jitter_factor).max(0.0);
                let sleep_duration = Duration::from_millis(jittered_ms as u64);

                log::debug!(
                    "Operation failed (attempt {}/{}): {}, retrying in {:?}...",
                    attempt + 1,
                    max_retries + 1,
                    e,
                    sleep_duration
                );

                std::thread::sleep(sleep_duration);

                // Compute next delay with exponential backoff, capped at max_delay
                delay = backoff_delay(delay, backoff_multiplier, max_delay);
            }
        }
    }

    // Final attempt (no retry after this).
    match f() {
        Ok(result) => Ok(result),
        Err(e) => {
            if e.should_retry() {
                log::warn!("Operation failed after {} retries: {}", max_retries, e);
            } else {
                log::debug!("Operation failed with permanent error: {}", e);
            }
            Err(e)
        }
    }
}

// Blanket implementation for String errors — always retry (no structured info).
impl ShouldRetry for String {}

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
    E: std::fmt::Display + ShouldRetry,
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

    /// Verify the exact delay sequence `with_retry_backoff` computes for its
    /// exponential backoff (pre-jitter): initial → *multiplier each retry, capped
    /// at max_delay. Fully deterministic — no wall-clock sleeps.
    #[test]
    fn test_backoff_delay_sequence() {
        let initial = Duration::from_millis(10);
        let max = Duration::from_millis(100);

        // Mirror the state machine inside `with_retry_backoff`:
        // delay starts at `initial`; after each retry it becomes
        // backoff_delay(delay, multiplier, max).
        let mut delay = initial;
        let mut seq = vec![delay];
        for _ in 0..2 {
            delay = backoff_delay(delay, 2.0, max);
            seq.push(delay);
        }

        assert_eq!(
            seq,
            vec![
                Duration::from_millis(10),
                Duration::from_millis(20),
                Duration::from_millis(40),
            ],
            "exponential backoff must double each retry and respect the cap"
        );
    }

    /// The same sequence with a low cap — later delays must be capped, not
    /// growing unbounded. Fully deterministic.
    #[test]
    fn test_backoff_delay_sequence_capped() {
        let initial = Duration::from_millis(10);
        let max = Duration::from_millis(15);

        let mut delay = initial;
        let mut seq = vec![delay];
        for _ in 0..2 {
            delay = backoff_delay(delay, 10.0, max);
            seq.push(delay);
        }

        assert_eq!(
            seq,
            vec![
                Duration::from_millis(10),
                Duration::from_millis(15), // 10*10=100 → capped
                Duration::from_millis(15), // stays at cap
            ],
            "backoff must cap at max_delay and never exceed it"
        );
    }

    #[test]
    fn test_backoff_delay_pure_function() {
        // Direct unit checks of the pure backoff math — fully deterministic.
        assert_eq!(
            backoff_delay(Duration::from_millis(10), 2.0, Duration::from_secs(1)),
            Duration::from_millis(20)
        );
        assert_eq!(
            backoff_delay(Duration::from_millis(20), 2.0, Duration::from_secs(1)),
            Duration::from_millis(40)
        );
        // Cap applies.
        assert_eq!(
            backoff_delay(Duration::from_millis(10), 10.0, Duration::from_millis(15)),
            Duration::from_millis(15)
        );
        // Multiplying an already-capped delay stays capped.
        assert_eq!(
            backoff_delay(Duration::from_millis(15), 10.0, Duration::from_millis(15)),
            Duration::from_millis(15)
        );
        // Multiplier below 1.0 shrinks (callers don't use it, but math holds).
        assert_eq!(
            backoff_delay(Duration::from_millis(40), 0.5, Duration::from_secs(1)),
            Duration::from_millis(20)
        );
    }
}
