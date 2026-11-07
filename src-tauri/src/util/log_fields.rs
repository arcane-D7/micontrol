//! Formatting helpers for "time since" fields in log records.
//!
//! The audit of 2026-11-07 (finding **P6**) found lines like
//!
//! ```text
//! 22:19:40.517Z [INFO ] micontrol_lib: [tray] elapsed_since_hidden=1794003580517ms
//! ```
//!
//! `1794003580517` is the *current epoch* in milliseconds (2026-11-06
//! 22:19:40.517Z), not a duration — 56.8 years. The timestamp slot was simply
//! never written (`0` sentinel) and `now - 0` was printed as an age.
//!
//! These helpers make that mistake impossible:
//! - [`elapsed_since`] returns `None` for the `0` sentinel ("never recorded"),
//! - [`format_duration_ms`] renders a real duration,
//! - [`duration_field`] renders the `name=1234ms (1.234s)` log field and the
//!   explicit `name=unset` for a timestamp that was never stored.

/// Sentinel stored in the atomic timestamp slots when the event never happened
/// in this session.
pub const UNSET_MS: u64 = 0;

/// Duration between `now_ms` and `since_ms`, or `None` when `since_ms` is the
/// "never recorded" sentinel.
///
/// `saturating_sub` keeps a backwards wall-clock step from wrapping into a
/// 584-million-year age.
pub fn elapsed_since(now_ms: u64, since_ms: u64) -> Option<u64> {
    if since_ms == UNSET_MS {
        None
    } else {
        Some(now_ms.saturating_sub(since_ms))
    }
}

/// Render a millisecond duration in the shortest human form.
///
/// ```text
/// 0        → "0ms"
/// 850      → "850ms"
/// 1234     → "1.234s"
/// 123_456  → "2m03.456s"
/// 3_723_000→ "1h02m"
/// 90_000_000 → "1d01h"
/// ```
pub fn format_duration_ms(ms: u64) -> String {
    const SECOND: u64 = 1_000;
    const MINUTE: u64 = 60 * SECOND;
    const HOUR: u64 = 60 * MINUTE;
    const DAY: u64 = 24 * HOUR;

    if ms < SECOND {
        format!("{ms}ms")
    } else if ms < MINUTE {
        format!("{}.{:03}s", ms / SECOND, ms % SECOND)
    } else if ms < HOUR {
        format!(
            "{}m{:02}.{:03}s",
            ms / MINUTE,
            (ms % MINUTE) / SECOND,
            ms % SECOND
        )
    } else if ms < DAY {
        format!("{}h{:02}m", ms / HOUR, (ms % HOUR) / MINUTE)
    } else {
        format!("{}d{:02}h", ms / DAY, (ms % DAY) / HOUR)
    }
}

/// Render a log field such as `elapsed_since_hidden=1234ms (1.234s)`.
///
/// When the timestamp was never recorded the field is rendered as
/// `elapsed_since_hidden=unset` — never as a bogus epoch-sized age.
pub fn duration_field(name: &str, now_ms: u64, since_ms: u64) -> String {
    match elapsed_since(now_ms, since_ms) {
        None => format!("{name}=unset"),
        Some(ms) => {
            let human = format_duration_ms(ms);
            if human == format!("{ms}ms") {
                format!("{name}={ms}ms")
            } else {
                format!("{name}={ms}ms ({human})")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sentinel_is_unset_not_epoch() {
        // The exact failure from the audit: stored timestamp 0 must NOT produce
        // the current epoch as an "age".
        let now = 1_794_003_580_517u64;
        assert_eq!(elapsed_since(now, UNSET_MS), None);
        assert_eq!(
            duration_field("elapsed_since_hidden", now, UNSET_MS),
            "elapsed_since_hidden=unset"
        );
        assert!(
            !duration_field("elapsed_since_hidden", now, UNSET_MS).contains("1794"),
            "no epoch may leak into the field"
        );
    }

    #[test]
    fn real_durations_are_computed_from_stored_timestamps() {
        assert_eq!(elapsed_since(1_000, 400), Some(600));
        assert_eq!(
            duration_field("age_since_shown", 1_000, 400),
            "age_since_shown=600ms"
        );
        assert_eq!(
            duration_field("age_since_shown", 2_234, 1_000),
            "age_since_shown=1234ms (1.234s)"
        );
    }

    #[test]
    fn backwards_clock_saturates_instead_of_wrapping() {
        assert_eq!(elapsed_since(10, 5_000), Some(0));
    }

    #[test]
    fn duration_formats_cover_every_magnitude() {
        assert_eq!(format_duration_ms(0), "0ms");
        assert_eq!(format_duration_ms(1), "1ms");
        assert_eq!(format_duration_ms(999), "999ms");
        assert_eq!(format_duration_ms(1_000), "1.000s");
        assert_eq!(format_duration_ms(1_234), "1.234s");
        assert_eq!(format_duration_ms(59_999), "59.999s");
        assert_eq!(format_duration_ms(60_000), "1m00.000s");
        assert_eq!(format_duration_ms(123_456), "2m03.456s");
        assert_eq!(format_duration_ms(3_600_000), "1h00m");
        assert_eq!(format_duration_ms(3_723_000), "1h02m");
        assert_eq!(format_duration_ms(86_400_000), "1d00h");
        assert_eq!(format_duration_ms(90_000_000), "1d01h");
    }

    #[test]
    fn sub_second_field_is_not_duplicated() {
        assert_eq!(duration_field("x", 500, 100), "x=400ms");
    }
}
