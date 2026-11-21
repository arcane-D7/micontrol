//! Rate limiting for repeated log messages.
//!
//! Problem this solves (audit 2026-11-07, P1): a single degraded condition —
//! the bridge service being considered "not available" because its named pipe
//! was wedged — produced **597** identical INFO lines plus **4.298** identical
//! "falling back to scheduled task path" warnings in 49 h. A log line that
//! repeats every 90 s carries no extra information after the first one; it
//! only buries the signals that matter.
//!
//! [`MessageRateLimiter`] permits at most **one emission per key per window**
//! (the audit asks for 1×/5 min) and reports how many repetitions were
//! suppressed, so the next allowed line still carries the count.
//!
//! The limiter is deliberately dependency-free, `const`-constructible (so it
//! can live in a `static`) and clock-injectable ([`MessageRateLimiter::allow_at`])
//! so the window semantics are unit-testable without sleeping.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Default window used by call sites: one line per message every 5 minutes.
pub const DEFAULT_WINDOW: Duration = Duration::from_secs(300);

struct Entry {
    last_emit: Instant,
    suppressed: u32,
}

/// Per-key rate limiter for log messages.
///
/// ```ignore
/// static LIMITER: MessageRateLimiter = MessageRateLimiter::new(DEFAULT_WINDOW);
///
/// if let Some(suppressed) = LIMITER.allow("bridge_install") {
///     log::info!("bridge not available — installing{suffix}", suffix = ...);
/// }
/// ```
pub struct MessageRateLimiter {
    window: Duration,
    /// `None` until the first use — `HashMap::new()` is not a const fn on this
    /// toolchain, and this limiter must live in a `static`.
    state: Mutex<Option<HashMap<&'static str, Entry>>>,
}

impl MessageRateLimiter {
    /// Convenience alias for [`DEFAULT_WINDOW`] so call sites can write
    /// `MessageRateLimiter::new(MessageRateLimiter::DEFAULT_WINDOW)`.
    pub const DEFAULT_WINDOW: Duration = DEFAULT_WINDOW;

    /// Create a limiter with the given window. `const` so it can be used to
    /// initialise a `static`.
    pub const fn new(window: Duration) -> Self {
        Self {
            window,
            state: Mutex::new(None),
        }
    }

    /// Returns `Some(suppressed_since_last_emit)` when the message for `key`
    /// may be emitted now, or `None` when it must be suppressed because the
    /// previous emission is younger than the window.
    pub fn allow(&self, key: &'static str) -> Option<u32> {
        self.allow_at(key, Instant::now())
    }

    /// Clock-injected variant of [`Self::allow`] — used by tests.
    pub fn allow_at(&self, key: &'static str, now: Instant) -> Option<u32> {
        // A poisoned lock must never take down a logging path: recover the
        // inner map and keep going.
        let mut guard = match self.state.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        let map = guard.get_or_insert_with(HashMap::new);

        match map.get_mut(key) {
            Some(entry) => {
                if now.saturating_duration_since(entry.last_emit) < self.window {
                    entry.suppressed = entry.suppressed.saturating_add(1);
                    None
                } else {
                    let suppressed = entry.suppressed;
                    entry.suppressed = 0;
                    entry.last_emit = now;
                    Some(suppressed)
                }
            }
            None => {
                map.insert(
                    key,
                    Entry {
                        last_emit: now,
                        suppressed: 0,
                    },
                );
                Some(0)
            }
        }
    }

    /// The configured window.
    pub fn window(&self) -> Duration {
        self.window
    }
}

impl std::fmt::Debug for MessageRateLimiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MessageRateLimiter")
            .field("window", &self.window)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_message_is_always_emitted() {
        let limiter = MessageRateLimiter::new(Duration::from_secs(300));
        let now = Instant::now();
        assert_eq!(limiter.allow_at("k", now), Some(0));
    }

    #[test]
    fn repeats_inside_the_window_are_suppressed_and_counted() {
        let limiter = MessageRateLimiter::new(Duration::from_secs(300));
        let t0 = Instant::now();
        assert_eq!(limiter.allow_at("k", t0), Some(0));
        assert_eq!(limiter.allow_at("k", t0 + Duration::from_secs(1)), None);
        assert_eq!(limiter.allow_at("k", t0 + Duration::from_secs(90)), None);
        assert_eq!(limiter.allow_at("k", t0 + Duration::from_secs(299)), None);

        // Window elapsed: emit again, reporting the 3 suppressed repetitions.
        assert_eq!(
            limiter.allow_at("k", t0 + Duration::from_secs(300)),
            Some(3)
        );
        // The counter restarts after a successful emission.
        assert_eq!(limiter.allow_at("k", t0 + Duration::from_secs(301)), None);
        assert_eq!(
            limiter.allow_at("k", t0 + Duration::from_secs(600)),
            Some(1)
        );
    }

    #[test]
    fn keys_are_independent() {
        let limiter = MessageRateLimiter::new(Duration::from_secs(300));
        let t0 = Instant::now();
        assert_eq!(limiter.allow_at("a", t0), Some(0));
        assert_eq!(limiter.allow_at("b", t0), Some(0));
        assert_eq!(limiter.allow_at("a", t0 + Duration::from_secs(1)), None);
        assert_eq!(limiter.allow_at("b", t0 + Duration::from_secs(1)), None);

        // "a" reaching its window must not clear or extend "b"'s.
        assert_eq!(
            limiter.allow_at("a", t0 + Duration::from_secs(400)),
            Some(1)
        );
        assert_eq!(
            limiter.allow_at("b", t0 + Duration::from_secs(599)),
            Some(1),
            "b keeps its own suppression count and window"
        );
        assert_eq!(
            limiter.allow_at("b", t0 + Duration::from_secs(600)),
            None,
            "b re-armed its window at 599"
        );
    }

    #[test]
    fn tiny_window_never_starves_a_key_forever() {
        let limiter = MessageRateLimiter::new(Duration::from_millis(50));
        let t0 = Instant::now();
        assert_eq!(limiter.allow_at("k", t0), Some(0));
        assert_eq!(limiter.allow_at("k", t0 + Duration::from_millis(49)), None);
        assert_eq!(
            limiter.allow_at("k", t0 + Duration::from_millis(50)),
            Some(1)
        );
    }
}
