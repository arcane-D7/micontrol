//! Retry policy for WebView2 window creation.
//!
//! `HRESULT(0x800700AA)` is `HRESULT_FROM_WIN32(ERROR_BUSY)` — "The requested
//! resource is in use".  WebView2 raises it while the *user data folder* (UDF)
//! is still locked by a previous process: `msedgewebview2.exe` outlives the
//! host process for a few seconds after a crash or a `TerminateProcess`, and
//! any `WebviewWindowBuilder::build()` that lands inside that window fails.
//!
//! Observed in production (`tauri-app.log`, 2026-11-06): two failures at
//! 22:19:16 and 22:19:40, i.e. 63 s and 87 s after the 22:18:13 relaunch. The
//! old code did `let _ = ...build()` and dropped the error, so the app carried
//! on with **no webview at all** (and the pre-created tray popup ended up 0×0).
//!
//! The lock is transient, so the fix is a bounded retry with backoff.  This
//! module holds the *policy* only (no Tauri types) so it can be unit-tested by
//! injecting a builder closure that fails twice and succeeds on the third
//! attempt, and an injected sleeper that records the delays instead of
//! sleeping.
//!
//! Retry budget: the initial attempt plus one retry per entry in
//! [`WEBVIEW_RETRY_DELAYS_MS`] (3 retries → 250 ms / 1 s / 2 s, ≤ 3.25 s of
//! waiting).  Only `0x800700AA` is retried — any other failure (missing
//! WebView2 runtime, invalid URL, ACL error) is permanent and returned at once
//! so we do not stall the caller for 3 s on an unrecoverable problem.

use std::fmt::Display;
use std::time::Duration;

/// Backoff before each retry, in milliseconds.
///
/// 250 ms covers the common "previous process still releasing the UDF" case;
/// 1 s and 2 s cover the slower teardown seen after a killed WebView2 browser
/// process. The total worst-case wait is 3.25 s, which is acceptable because
/// the alternative is a windowless (or 0×0) application.
pub const WEBVIEW_RETRY_DELAYS_MS: [u64; 3] = [250, 1_000, 2_000];

/// HRESULT for `ERROR_BUSY` / "The requested resource is in use".
pub const HRESULT_ERROR_BUSY: u32 = 0x8007_00AA;

/// True when an error message looks like the WebView2 user-data-folder lock.
///
/// Matches the hex form used by `windows::core::Error`'s `Display`
/// (`HRESULT(0x800700AA)`, any case — Windows prints it upper-case today but
/// wry/`anyhow` chains sometimes lower-case it) and the signed decimal form
/// (`-2147024726`) that some wrappers emit.
pub fn is_udf_lock_error(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    if lower.contains("800700aa") {
        return true;
    }
    // 0x800700AA as a signed 32-bit value.
    let signed = (HRESULT_ERROR_BUSY as i32).to_string(); // "-2147024726"
    lower.contains(&signed)
}

/// Run `build` and retry it while the error is the transient UDF lock.
///
/// - `delays_ms` is the backoff *before each retry*; the number of retries is
///   `delays_ms.len()`, so a 3-element slice means up to 4 build attempts.
/// - `sleep` is injected so tests can run without wall-clock waits.
/// - Every failed attempt is logged at WARN with the attempt counter; the
///   final failure is logged at ERROR and returned (never panics).
/// - A non-UDF error stops the loop immediately (permanent failure).
pub fn retry_webview_create<T, E, F, S>(
    label: &str,
    delays_ms: &[u64],
    mut build: F,
    mut sleep: S,
) -> Result<T, E>
where
    F: FnMut() -> Result<T, E>,
    E: Display,
    S: FnMut(Duration),
{
    let total_attempts = delays_ms.len() + 1;
    let mut attempt = 1usize;

    loop {
        match build() {
            Ok(value) => {
                if attempt > 1 {
                    log::info!(
                        "[webview] '{label}' created on attempt {attempt}/{total_attempts} \
                         after UDF lock cleared"
                    );
                }
                return Ok(value);
            }
            Err(e) => {
                let message = e.to_string();
                if attempt >= total_attempts {
                    log::error!(
                        "[webview] creating '{label}' failed after {attempt} attempt(s) \
                         (last error: {message}) — giving up, caller continues without a webview"
                    );
                    return Err(e);
                }
                if !is_udf_lock_error(&message) {
                    // Permanent error: retrying the same build would fail the
                    // same way and only delay the caller.
                    log::error!("[webview] creating '{label}' failed permanently: {message}");
                    return Err(e);
                }
                let delay_ms = delays_ms[attempt - 1];
                log::warn!(
                    "[webview] creating '{label}' failed on attempt {attempt}/{total_attempts} \
                     ({message}) — retrying in {delay_ms} ms (WebView2 UDF still locked)"
                );
                sleep(Duration::from_millis(delay_ms));
                attempt += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    const UDF_ERR: &str = "WebView2 error: WindowsError(Error { code: HRESULT(0x800700AA), \
                           message: \"The requested resource is in use.\" })";
    const OTHER_ERR: &str = "WebView2 runtime not found";

    #[test]
    fn udf_lock_error_is_detected() {
        assert!(is_udf_lock_error(UDF_ERR));
        assert!(is_udf_lock_error(&UDF_ERR.to_ascii_lowercase()));
        assert!(is_udf_lock_error("HRESULT(0x800700AA)"));
        assert!(is_udf_lock_error(
            "error -2147024726 while creating webview"
        ));
        // Unrelated errors must not be treated as the transient lock.
        assert!(!is_udf_lock_error(OTHER_ERR));
        assert!(!is_udf_lock_error(""));
        assert!(!is_udf_lock_error("HRESULT(0x80070005) access denied"));
    }

    #[test]
    fn succeeds_first_attempt_without_sleeping() {
        let calls = RefCell::new(0u32);
        let delays: RefCell<Vec<u64>> = RefCell::new(Vec::new());

        let result: Result<&str, String> = retry_webview_create(
            "main",
            &WEBVIEW_RETRY_DELAYS_MS,
            || {
                *calls.borrow_mut() += 1;
                Ok("window")
            },
            |d| delays.borrow_mut().push(d.as_millis() as u64),
        );

        assert_eq!(result.unwrap(), "window");
        assert_eq!(*calls.borrow(), 1);
        assert!(delays.borrow().is_empty(), "no retry → no sleep");
    }

    /// The regression this module exists for: two UDF-lock failures then
    /// success on the third attempt (250 ms + 1 s backoff).
    #[test]
    fn retries_twice_then_succeeds_on_third_attempt() {
        let calls = RefCell::new(0u32);
        let delays: RefCell<Vec<u64>> = RefCell::new(Vec::new());

        let result: Result<&str, String> = retry_webview_create(
            "tray",
            &WEBVIEW_RETRY_DELAYS_MS,
            || {
                *calls.borrow_mut() += 1;
                if *calls.borrow() < 3 {
                    Err(UDF_ERR.to_string())
                } else {
                    Ok("tray-window")
                }
            },
            |d| delays.borrow_mut().push(d.as_millis() as u64),
        );

        assert_eq!(result.unwrap(), "tray-window");
        assert_eq!(*calls.borrow(), 3, "two failures + one success");
        assert_eq!(*delays.borrow(), vec![250, 1_000], "250 ms then 1 s");
    }

    #[test]
    fn exhausts_all_attempts_and_returns_error() {
        let calls = RefCell::new(0u32);
        let delays: RefCell<Vec<u64>> = RefCell::new(Vec::new());

        let result: Result<&str, String> = retry_webview_create(
            "main",
            &WEBVIEW_RETRY_DELAYS_MS,
            || {
                *calls.borrow_mut() += 1;
                Err(UDF_ERR.to_string())
            },
            |d| delays.borrow_mut().push(d.as_millis() as u64),
        );

        assert!(
            result.is_err(),
            "exhausted retries must return Err, not panic"
        );
        assert_eq!(*calls.borrow(), 4, "initial attempt + 3 retries");
        assert_eq!(*delays.borrow(), vec![250, 1_000, 2_000]);
    }

    #[test]
    fn permanent_error_is_not_retried() {
        let calls = RefCell::new(0u32);
        let delays: RefCell<Vec<u64>> = RefCell::new(Vec::new());

        let result: Result<&str, String> = retry_webview_create(
            "main",
            &WEBVIEW_RETRY_DELAYS_MS,
            || {
                *calls.borrow_mut() += 1;
                Err(OTHER_ERR.to_string())
            },
            |d| delays.borrow_mut().push(d.as_millis() as u64),
        );

        assert!(result.is_err());
        assert_eq!(*calls.borrow(), 1, "permanent errors are not retried");
        assert!(delays.borrow().is_empty());
    }

    #[test]
    fn empty_delay_slice_means_single_attempt() {
        let calls = RefCell::new(0u32);
        let result: Result<&str, String> = retry_webview_create(
            "main",
            &[],
            || {
                *calls.borrow_mut() += 1;
                Err(UDF_ERR.to_string())
            },
            |_| panic!("no delay should be requested"),
        );
        assert!(result.is_err());
        assert_eq!(*calls.borrow(), 1);
    }
}
