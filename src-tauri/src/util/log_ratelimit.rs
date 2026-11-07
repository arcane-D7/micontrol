//! Repeat suppression for `WARN` / `ERROR` records.
//!
//! The audit of 2026-11-07 found a single failing condition (bridge probe,
//! battery WMI timeout, IGCL fallback) producing **hundreds of identical
//! lines** per hour — 665 lines for one constant condition, 82 KB of noise
//! that buries the lines that actually matter.
//!
//! This module implements the cheap, bounded deduplication the audit asks for:
//!
//! - the **same signature** (level + target + message, with digit runs
//!   collapsed) repeated less than [`DEFAULT_WINDOW_MS`] after its previous
//!   occurrence is suppressed — a rolling quiet-period window, not fixed
//!   clock buckets, so a steady 1/second trickle is also silenced;
//! - the suppression is not silent: the next emitted record is prefixed with
//!   `×N repetições suprimidas …`, and while the spam continues a summary is
//!   emitted at most once per window;
//! - state is a fixed-size slot table (64 entries) with LRU eviction, so the
//!   cost per record is one short scan and the memory is constant — no map
//!   growth, no allocations on the hot path.
//!
//! Only `WARN` and `ERROR` are filtered (see `debug_log::emit_record`), so
//! lifecycle markers and `PANIC:` lines are never affected.

use std::sync::{Mutex, OnceLock};

/// Window during which an identical signature is suppressed.
pub const DEFAULT_WINDOW_MS: u64 = 60_000;

/// Maximum number of distinct signatures tracked at once.
pub const DEFAULT_CAPACITY: usize = 64;

/// What the formatter should do with a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Emit the record. `suppressed` identical records were dropped since the
    /// previous emission and must be reported as `×N repetições suprimidas`.
    Emit { suppressed: u32 },
    /// Drop the record. `summary` is `Some(n)` when a periodic summary line
    /// (`×n repetições suprimidas`) is due; the caller should write it even
    /// though the record itself is dropped.
    Suppress { summary: Option<u32> },
}

#[derive(Debug, Clone, Copy)]
struct Slot {
    key: u64,
    /// Records suppressed since the last emission / summary.
    count: u32,
    last_seen_ms: u64,
    last_summary_ms: u64,
    /// Monotonic usage counter — the LRU key. A millisecond clock is too
    /// coarse to order a burst of records (ties would make eviction pick slot
    /// 0 and could drop a signature that was just used).
    last_used_seq: u64,
    used: bool,
}

impl Slot {
    const EMPTY: Slot = Slot {
        key: 0,
        count: 0,
        last_seen_ms: 0,
        last_summary_ms: 0,
        last_used_seq: 0,
        used: false,
    };
}

/// Fixed-size, LRU-evicting repeat suppressor.
///
/// The type is deliberately free of I/O and of the `log` crate so it can be
/// unit-tested with synthetic clocks.
pub struct RateLimiter {
    window_ms: u64,
    slots: Vec<Slot>,
    /// Monotonic counter incremented on every `check`; orders slot usage.
    seq: u64,
}

impl RateLimiter {
    /// Create a limiter with an explicit window and capacity (tests, tuning).
    pub fn new(window_ms: u64, capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            window_ms: window_ms.max(1),
            slots: vec![Slot::EMPTY; capacity],
            seq: 0,
        }
    }

    /// Create a limiter with the shipping defaults (60 s, 64 signatures).
    pub fn with_defaults() -> Self {
        Self::new(DEFAULT_WINDOW_MS, DEFAULT_CAPACITY)
    }

    /// Number of signatures the limiter can track.
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// Current suppression window in milliseconds.
    pub fn window_ms(&self) -> u64 {
        self.window_ms
    }

    /// Decide what to do with a record identified by `signature` at `now_ms`.
    ///
    /// Time is passed in (not read from the clock) so the window logic is
    /// deterministic under test. Callers must pass a monotonic-enough value;
    /// `saturating_sub` tolerates a clock that steps backwards.
    pub fn check(&mut self, signature: u64, now_ms: u64) -> Decision {
        self.seq = self.seq.wrapping_add(1);
        let seq = self.seq;

        if let Some(index) = self
            .slots
            .iter()
            .position(|slot| slot.used && slot.key == signature)
        {
            let slot = &mut self.slots[index];
            slot.last_used_seq = seq;
            if now_ms.saturating_sub(slot.last_seen_ms) >= self.window_ms {
                // The quiet window elapsed: this is a new occurrence, report
                // whatever was suppressed in the previous burst.
                let suppressed = slot.count;
                slot.count = 0;
                slot.last_seen_ms = now_ms;
                slot.last_summary_ms = now_ms;
                return Decision::Emit { suppressed };
            }

            // Still inside the window: suppress and count.
            slot.count = slot.count.saturating_add(1);
            slot.last_seen_ms = now_ms;
            if now_ms.saturating_sub(slot.last_summary_ms) >= self.window_ms {
                let total = slot.count;
                slot.count = 0;
                slot.last_summary_ms = now_ms;
                return Decision::Suppress {
                    summary: Some(total),
                };
            }
            return Decision::Suppress { summary: None };
        }

        self.insert(signature, now_ms, seq);
        Decision::Emit { suppressed: 0 }
    }

    /// Insert a new signature, evicting the least-recently-used slot when full.
    fn insert(&mut self, signature: u64, now_ms: u64, seq: u64) {
        if let Some(slot) = self.slots.iter_mut().find(|slot| !slot.used) {
            *slot = Slot {
                key: signature,
                count: 0,
                last_seen_ms: now_ms,
                last_summary_ms: now_ms,
                last_used_seq: seq,
                used: true,
            };
            return;
        }

        // Full: replace the entry used longest ago. This keeps the memory
        // bounded and only costs a re-emission of a signature that went quiet
        // longest ago.
        let mut oldest = 0usize;
        for (index, slot) in self.slots.iter().enumerate() {
            if slot.last_used_seq < self.slots[oldest].last_used_seq {
                oldest = index;
            }
        }
        self.slots[oldest] = Slot {
            key: signature,
            count: 0,
            last_seen_ms: now_ms,
            last_summary_ms: now_ms,
            last_used_seq: seq,
            used: true,
        };
    }
}

/// Process-wide limiter used by the logger formatter.
///
/// `try_lock` is used by the caller: if the mutex is busy (e.g. the panic hook
/// firing while another thread formats a record) the record is emitted without
/// deduplication instead of deadlocking or blocking.
pub fn global() -> &'static Mutex<RateLimiter> {
    static LIMITER: OnceLock<Mutex<RateLimiter>> = OnceLock::new();
    LIMITER.get_or_init(|| Mutex::new(RateLimiter::with_defaults()))
}

/// Build the deduplication signature of a record.
///
/// `level_rank` is `log::Level as u8`. Digit runs in the message are collapsed
/// to a single `#` so retry counters, durations and ids (`attempt=1`,
/// `attempt=2`, `took 5012 ms`) map to the same signature — that is exactly the
/// spam shape the audit found. The emitted message is never altered.
pub fn signature(level_rank: u8, target: &str, message: &str) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    level_rank.hash(&mut hasher);
    target.hash(&mut hasher);

    let mut in_digits = false;
    for ch in message.chars() {
        if ch.is_ascii_digit() {
            if !in_digits {
                '#'.hash(&mut hasher);
                in_digits = true;
            }
        } else {
            in_digits = false;
            ch.hash(&mut hasher);
        }
    }
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u64 = DEFAULT_WINDOW_MS;

    #[test]
    fn first_occurrence_is_emitted() {
        let mut limiter = RateLimiter::with_defaults();
        assert_eq!(limiter.check(7, 1_000), Decision::Emit { suppressed: 0 });
    }

    #[test]
    fn repeats_inside_the_window_are_suppressed() {
        let mut limiter = RateLimiter::with_defaults();
        assert_eq!(limiter.check(7, 0), Decision::Emit { suppressed: 0 });
        for t in [1, 2, 59_999] {
            assert_eq!(limiter.check(7, t), Decision::Suppress { summary: None });
        }
    }

    #[test]
    fn quiet_gap_reemits_and_reports_the_suppressed_count() {
        let mut limiter = RateLimiter::with_defaults();
        assert_eq!(limiter.check(7, 0), Decision::Emit { suppressed: 0 });
        assert_eq!(limiter.check(7, 10), Decision::Suppress { summary: None });
        assert_eq!(limiter.check(7, 20), Decision::Suppress { summary: None });
        // The window is measured from the LAST occurrence: exactly one window
        // of silence (60 s after t=20) re-opens emission and reports how many
        // records were dropped in between.
        assert_eq!(
            limiter.check(7, W + 20),
            Decision::Emit { suppressed: 2 },
            "one full window of silence opens a new emission"
        );
        // …and the count restarts.
        assert_eq!(
            limiter.check(7, W + 30),
            Decision::Suppress { summary: None }
        );
        assert_eq!(
            limiter.check(7, W * 2 + 30),
            Decision::Emit { suppressed: 1 }
        );
    }

    /// A steady stream (one record every 15 s, the bridge-probe cadence) never
    /// goes quiet, so it stays suppressed across window boundaries — only the
    /// periodic summaries reach the file.
    #[test]
    fn steady_trickle_never_reemits_even_though_it_crosses_windows() {
        let mut limiter = RateLimiter::with_defaults();
        assert_eq!(limiter.check(9, 0), Decision::Emit { suppressed: 0 });
        let mut summaries = 0;
        for tick in 1..=40u64 {
            let decision = limiter.check(9, tick * 15_000);
            assert!(
                !matches!(decision, Decision::Emit { .. }),
                "tick {tick} must stay suppressed, got {decision:?}"
            );
            if matches!(decision, Decision::Suppress { summary: Some(_) }) {
                summaries += 1;
            }
        }
        // 600 s of spam → a summary at each 60 s boundary.
        assert_eq!(summaries, 10, "one summary per 60 s window");
    }

    #[test]
    fn continuous_spam_gets_a_periodic_summary() {
        let mut limiter = RateLimiter::new(W, 8);
        assert_eq!(limiter.check(1, 0), Decision::Emit { suppressed: 0 });
        // One record per second for 3 minutes → 3 summaries (t=60/120/180).
        let mut summaries = Vec::new();
        for second in 1..=180u64 {
            let t = second * 1_000;
            if let Decision::Suppress { summary: Some(n) } = limiter.check(1, t) {
                summaries.push((t, n));
            }
        }
        assert_eq!(
            summaries,
            vec![(W, 60), (W * 2, 60), (W * 3, 60)],
            "a summary per window, counting the records actually suppressed"
        );
    }

    #[test]
    fn distinct_signatures_do_not_interfere() {
        let mut limiter = RateLimiter::with_defaults();
        assert_eq!(limiter.check(1, 0), Decision::Emit { suppressed: 0 });
        assert_eq!(limiter.check(2, 0), Decision::Emit { suppressed: 0 });
        assert_eq!(limiter.check(1, 1), Decision::Suppress { summary: None });
        assert_eq!(limiter.check(2, 1), Decision::Suppress { summary: None });
    }

    #[test]
    fn capacity_is_bounded_and_evicts_the_oldest_signature() {
        let mut limiter = RateLimiter::new(W, 2);
        assert_eq!(limiter.capacity(), 2);
        assert_eq!(limiter.check(1, 0), Decision::Emit { suppressed: 0 });
        assert_eq!(limiter.check(2, 1), Decision::Emit { suppressed: 0 });
        // Third signature evicts #1 (least recently seen).
        assert_eq!(limiter.check(3, 2), Decision::Emit { suppressed: 0 });
        // #2 and #3 are still tracked → suppressed.
        assert_eq!(limiter.check(2, 3), Decision::Suppress { summary: None });
        assert_eq!(limiter.check(3, 3), Decision::Suppress { summary: None });
        // #1 was evicted, so it is emitted again even inside the window.
        assert_eq!(limiter.check(1, 4), Decision::Emit { suppressed: 0 });
    }

    /// A burst of records inside one millisecond must still evict by *usage*
    /// order, not by clock time (all timestamps are equal there). The most
    /// recently used signature has to survive the eviction.
    #[test]
    fn same_millisecond_burst_evicts_the_least_recently_used_signature() {
        let mut limiter = RateLimiter::new(W, 2);
        assert_eq!(limiter.check(1, 5_000), Decision::Emit { suppressed: 0 });
        assert_eq!(limiter.check(2, 5_000), Decision::Emit { suppressed: 0 });
        // Touch #1 so #2 becomes the least recently used.
        assert_eq!(
            limiter.check(1, 5_000),
            Decision::Suppress { summary: None }
        );
        // #3 evicts #2, never #1.
        assert_eq!(limiter.check(3, 5_000), Decision::Emit { suppressed: 0 });
        assert_eq!(
            limiter.check(1, 5_000),
            Decision::Suppress { summary: None },
            "the just-used signature must not be evicted on a clock tie"
        );
        assert_eq!(
            limiter.check(2, 5_000),
            Decision::Emit { suppressed: 0 },
            "the least recently used signature is the one evicted"
        );
    }

    #[test]
    fn backwards_clock_does_not_panic_or_epoch() {
        let mut limiter = RateLimiter::with_defaults();
        assert_eq!(limiter.check(1, 10_000), Decision::Emit { suppressed: 0 });
        // A clock step backwards must not underflow; it is treated as "inside
        // the window" and suppresses.
        assert_eq!(
            limiter.check(1, 5_000),
            Decision::Suppress { summary: None }
        );
    }

    #[test]
    fn signature_collapses_digit_runs_but_keeps_text() {
        let a = signature(3, "hw::battery", "probe attempt=1 failed after 5000 ms");
        let b = signature(3, "hw::battery", "probe attempt=2 failed after 5001 ms");
        assert_eq!(a, b, "digit runs collapse to the same signature");

        let c = signature(3, "hw::battery", "probe attempt=# failed after # ms");
        assert_eq!(a, c, "literal # and digits hash identically");

        let other_text = signature(3, "hw::battery", "fan probe failed");
        assert_ne!(a, other_text, "different text is a different signature");
        assert_ne!(
            signature(2, "hw::battery", "probe failed"),
            signature(4, "hw::battery", "probe failed"),
            "level is part of the signature"
        );
        assert_ne!(
            signature(3, "hw::fan", "probe failed"),
            signature(3, "hw::battery", "probe failed"),
            "target is part of the signature"
        );
    }

    #[test]
    fn default_limiter_matches_the_documented_window() {
        let limiter = RateLimiter::with_defaults();
        assert_eq!(limiter.window_ms(), 60_000);
        assert_eq!(limiter.capacity(), 64);
    }
}
