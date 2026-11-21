//! Runtime memory/handle leak detector (S67).
//!
//! Prevention layer 3: a lightweight background sampler that watches the
//! process's kernel handle count and private (commit) bytes. It converts a
//! slow leak — which previously went unnoticed for **218 hours** — into a
//! loud, timestamped WARN within minutes, plus a structured report the UI can
//! surface.
//!
//! Design constraints (deliberate):
//!
//! * **Bounded memory**: a fixed-size ring of samples (no growing vectors).
//! * **No syscalls per tick beyond 2 cheap queries** (`GetProcessHandleCount`,
//!   `GetProcessMemoryInfo`).
//! * **Throttled logging**: at most one WARN per [`LOG_COOLDOWN`].
//! * **Never panics**: every failure degrades to "no data" and the loop keeps
//!   running.
//!
//! Thresholds are expressed as *rates* over a sliding window, because absolute
//! counts legitimately vary with feature usage (Face, OS Turbo, task manager).
//!
//! Calibration history (2026-11-07 production audit):
//!
//! * The original `MIN_SAMPLES = 3` evaluated the very first sample pair, i.e.
//!   **2 minutes** after process start, and fired twice on startup ramps:
//!   925 handles (+3.5/min) and 643 handles (+5.0/min). Eleven hours later the
//!   counts were 640/647 — flat. The real historical leak was 188,715 handles
//!   (~11/min sustained for 218 h) and `RADAR_PRE_LEAK_64` is a separate,
//!   OS-side signal to be watched, not a reason to lower these thresholds.
//! * A rate is now only actionable after [`MIN_SAMPLES`] samples **and**
//!   [`MIN_SPAN`] of wall-clock history, and a handle alarm additionally
//!   requires [`ABSOLUTE_HANDLE_FLOOR`] handles — a process that ramps from
//!   300 to 900 handles is not the failure mode we care about.
//! * The first [`WARMUP`] of process life uses doubled thresholds
//!   ([`WARMUP_FACTOR`]): WebView2/COM warm-up naturally grows handles and
//!   commit, and that is exactly where the false positives came from. A real
//!   leak (11/min) still trips them during warm-up.

#![cfg(windows)]

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// How often the sampler runs.
pub const SAMPLE_INTERVAL: Duration = Duration::from_secs(60);

/// Sliding window used to compute the growth rate.
pub const WINDOW: Duration = Duration::from_secs(15 * 60);

/// Handles/minute above which a leak is declared (steady state).
///
/// The audited leak ran at ~11/min. A healthy steady state is < 1/min
/// (occasional short-lived handles), so 3/min gives comfortable margin while
/// still catching a regression in ~1 hour instead of 218.
pub const LEAK_HANDLES_PER_MIN: f64 = 3.0;

/// Minimum absolute handle count before the handle-rate alarm may fire.
///
/// Window-relative growth alone is meaningless on a small process: the 06-Nov
/// false positives sat at 643–925 handles, two orders of magnitude below the
/// audited 188,715. 5,000 is far above any healthy idle/startup count observed
/// on this app (≈600–1,000) and far below the real leak.
pub const ABSOLUTE_HANDLE_FLOOR: u32 = 5_000;

/// Private/commit bytes per minute above which a leak is declared (steady
/// state).
///
/// The audited leak grew commit by ~11.5 MB/min alongside the handles.
/// 2 MB/min is well above normal allocator noise (log rotation, caches).
pub const LEAK_COMMIT_BYTES_PER_MIN: f64 = 2.0 * 1024.0 * 1024.0;

/// Minimum number of samples before a rate is actionable.
///
/// 3 samples = 2 minutes, which is what produced the false positives. 10
/// samples with a 60 s cadence covers 9 minutes; combined with [`MIN_SPAN`]
/// the first evaluation happens at 10 minutes of history.
pub const MIN_SAMPLES: usize = 10;

/// Minimum wall-clock span between the oldest in-window sample and `now`
/// before a rate is actionable.
///
/// Making this explicit (rather than relying on sample count) keeps the
/// guarantee if [`SAMPLE_INTERVAL`] is ever shortened.
pub const MIN_SPAN: Duration = Duration::from_secs(10 * 60);

/// Process start-up period during which thresholds are multiplied by
/// [`WARMUP_FACTOR`].
pub const WARMUP: Duration = Duration::from_secs(30 * 60);

/// Multiplier applied to both rate thresholds during [`WARMUP`].
///
/// Startup work (WebView2, COM, caches) legitimately allocates; a leak at the
/// audited 11/min still clears 6/min easily.
pub const WARMUP_FACTOR: f64 = 2.0;

/// Log cooldown after a leak alert (prevents log flooding during a leak).
const LOG_COOLDOWN: Duration = Duration::from_secs(30 * 60);

/// Ring buffer capacity — 15 min window at 60 s = 15 samples; keep 32.
const RING: usize = 32;

/// One measurement.
#[derive(Debug, Clone, Copy)]
struct Sample {
    at: Instant,
    handles: u32,
    commit_bytes: u64,
}

/// Fixed-capacity ring of samples (bounded memory, no allocation churn).
struct Ring {
    buf: [Option<Sample>; RING],
    next: usize,
    len: usize,
}

impl Ring {
    const fn new() -> Self {
        Self {
            buf: [None; RING],
            next: 0,
            len: 0,
        }
    }

    fn push(&mut self, sample: Sample) {
        self.buf[self.next] = Some(sample);
        self.next = (self.next + 1) % RING;
        if self.len < RING {
            self.len += 1;
        }
    }

    /// Oldest sample still inside the window.
    fn oldest_in_window(&self, now: Instant) -> Option<Sample> {
        let mut oldest: Option<Sample> = None;
        for slot in self.buf.iter().flatten() {
            if now.duration_since(slot.at) <= WINDOW && oldest.is_none_or(|o| slot.at < o.at) {
                oldest = Some(*slot);
            }
        }
        oldest
    }

    fn count_in_window(&self, now: Instant) -> usize {
        self.buf
            .iter()
            .flatten()
            .filter(|s| now.duration_since(s.at) <= WINDOW)
            .count()
    }
}

/// Result of one evaluation pass — also the shape returned to the UI.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LeakReport {
    pub handles: u32,
    pub commit_bytes: u64,
    pub handles_per_min: f64,
    pub commit_bytes_per_min: f64,
    pub samples: usize,
    pub leaking: bool,
}

/// Live monitor state (statics: the sampler is process-wide, single instance).
static STARTED: AtomicBool = AtomicBool::new(false);
static LAST_HANDLES: AtomicU32 = AtomicU32::new(0);
static LAST_COMMIT_MB: AtomicU64 = AtomicU64::new(0);
static LAST_RATE_X1000: AtomicU64 = AtomicU64::new(0);
static LAST_COMMIT_RATE_X1000: AtomicU64 = AtomicU64::new(0);
static LAST_SAMPLES: AtomicU64 = AtomicU64::new(0);
static LEAKING: AtomicBool = AtomicBool::new(false);
static ALERTS: AtomicU64 = AtomicU64::new(0);

/// Start the monitor once (idempotent). The task lives for the process
/// lifetime — it is a detector, not a workload, so there is nothing to cancel.
pub fn start() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    std::thread::Builder::new()
        .name("handle-leak-monitor".into())
        .spawn(|| {
            let mut ring = Ring::new();
            let mut last_log: Option<Instant> = None;
            // Warm-up is measured from the sampler start, which is right after
            // the app's setup — close enough to process start for a 30 min gate.
            let started_at = Instant::now();

            loop {
                std::thread::sleep(SAMPLE_INTERVAL);
                let warmup = started_at.elapsed() < WARMUP;
                if let Some(report) = sample_and_evaluate(&mut ring, warmup) {
                    LAST_HANDLES.store(report.handles, Ordering::Relaxed);
                    LAST_COMMIT_MB.store(report.commit_bytes / (1024 * 1024), Ordering::Relaxed);
                    LAST_RATE_X1000
                        .store((report.handles_per_min * 1000.0) as u64, Ordering::Relaxed);
                    LAST_COMMIT_RATE_X1000.store(
                        (report.commit_bytes_per_min * 1000.0) as u64,
                        Ordering::Relaxed,
                    );
                    LAST_SAMPLES.store(report.samples as u64, Ordering::Relaxed);
                    LEAKING.store(report.leaking, Ordering::Relaxed);

                    if report.leaking {
                        let now = Instant::now();
                        let may_log =
                            last_log.is_none_or(|t| now.duration_since(t) >= LOG_COOLDOWN);
                        if may_log {
                            last_log = Some(now);
                            ALERTS.fetch_add(1, Ordering::Relaxed);
                            log::warn!(
                                target: "leak_monitor",
                                "KERNEL HANDLE LEAK SUSPECTED: {} handles (+{:.1}/min), \
                                 commit {} MB (+{:.1} MB/min) over {} samples{} — see \
                                 memory-leak-audit.md; restart the app to reclaim",
                                report.handles,
                                report.handles_per_min,
                                report.commit_bytes / (1024 * 1024),
                                report.commit_bytes_per_min / (1024.0 * 1024.0),
                                report.samples,
                                if warmup {
                                    " (still in warm-up: thresholds doubled)"
                                } else {
                                    ""
                                }
                            );
                        }
                    }
                }
            }
        })
        .ok();
}

/// Latest report for the UI/commands (no sampling — reads cached statics).
pub fn latest() -> LeakReport {
    let handles = LAST_HANDLES.load(Ordering::Relaxed);
    let commit_bytes = LAST_COMMIT_MB.load(Ordering::Relaxed) * 1024 * 1024;
    let rate = LAST_RATE_X1000.load(Ordering::Relaxed) as f64 / 1000.0;
    let commit_rate = LAST_COMMIT_RATE_X1000.load(Ordering::Relaxed) as f64 / 1000.0;
    LeakReport {
        handles,
        commit_bytes,
        handles_per_min: rate,
        commit_bytes_per_min: commit_rate,
        samples: LAST_SAMPLES.load(Ordering::Relaxed) as usize,
        leaking: LEAKING.load(Ordering::Relaxed),
    }
}

/// Number of leak alerts raised this session (diagnostics/UI).
pub fn alert_count() -> u64 {
    ALERTS.load(Ordering::Relaxed)
}

/// One measurement + rate evaluation. Extracted from the loop so it is
/// unit-testable without sleeping for minutes.
fn sample_and_evaluate(ring: &mut Ring, warmup: bool) -> Option<LeakReport> {
    let handles = super::win_handle::process_handle_count()?;
    let commit_bytes = private_bytes()?;
    let now = Instant::now();
    ring.push(Sample {
        at: now,
        handles,
        commit_bytes,
    });
    Some(evaluate(ring, now, handles, commit_bytes, warmup))
}

/// Pure rate evaluation — no syscalls, no statics, so the whole decision can
/// be exercised with a synthetic series.
///
/// A rate is reported as soon as there are two samples (useful for
/// diagnostics) but `leaking` stays `false` until **both** [`MIN_SAMPLES`] and
/// [`MIN_SPAN`] gates pass. The handle channel additionally requires
/// [`ABSOLUTE_HANDLE_FLOOR`]; the commit channel stands on its own (the
/// audited leak grew commit 11.5 MB/min with the handles).
fn evaluate(ring: &Ring, now: Instant, handles: u32, commit: u64, warmup: bool) -> LeakReport {
    let samples = ring.count_in_window(now);
    let Some(oldest) = ring.oldest_in_window(now) else {
        return LeakReport {
            handles,
            commit_bytes: commit,
            handles_per_min: 0.0,
            commit_bytes_per_min: 0.0,
            samples: 0,
            leaking: false,
        };
    };

    let span = now.duration_since(oldest.at);
    let elapsed_min = span.as_secs_f64() / 60.0;
    if elapsed_min <= 0.0 {
        return LeakReport {
            handles,
            commit_bytes: commit,
            handles_per_min: 0.0,
            commit_bytes_per_min: 0.0,
            samples,
            leaking: false,
        };
    }

    // Saturating subtraction: a DROP (app reclaimed memory) yields 0 growth.
    let d_handles = handles.saturating_sub(oldest.handles) as f64;
    let d_commit = commit.saturating_sub(oldest.commit_bytes) as f64;
    let handles_per_min = d_handles / elapsed_min;
    let commit_bytes_per_min = d_commit / elapsed_min;

    let enough_history = samples >= MIN_SAMPLES && span >= MIN_SPAN;
    let factor = if warmup { WARMUP_FACTOR } else { 1.0 };
    let leaking = enough_history
        && ((handles_per_min >= LEAK_HANDLES_PER_MIN * factor && handles >= ABSOLUTE_HANDLE_FLOOR)
            || commit_bytes_per_min >= LEAK_COMMIT_BYTES_PER_MIN * factor);

    LeakReport {
        handles,
        commit_bytes: commit,
        handles_per_min,
        commit_bytes_per_min,
        samples,
        leaking,
    }
}

/// Private (commit) bytes of the current process.
fn private_bytes() -> Option<u64> {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;

    let mut counters = PROCESS_MEMORY_COUNTERS {
        cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        ..Default::default()
    };
    // SAFETY: pseudo-handle (never closed) + valid, correctly sized out-struct.
    let ok = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut counters,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )
    };
    if ok.is_ok() {
        Some(counters.PagefileUsage as u64)
    } else {
        None
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const MB: u64 = 1024 * 1024;

    /// Reference "now" for tests, shifted 2 h into the future.
    ///
    /// CI runners can have <1 h of uptime, so `Instant::now() - 3600s`
    /// underflows (panics: "overflow when subtracting duration from instant").
    /// Anchoring the tests 2 h ahead makes every `now - <up to 2 h>`
    /// subtraction representable on any host, deterministically.
    fn test_now() -> Instant {
        Instant::now() + Duration::from_secs(7200)
    }

    fn sample(secs_from_now: i64, handles: u32, commit: u64, now: Instant) -> Sample {
        let at = if secs_from_now >= 0 {
            now + Duration::from_secs(secs_from_now as u64)
        } else {
            now - Duration::from_secs((-secs_from_now) as u64)
        };
        Sample {
            at,
            handles,
            commit_bytes: commit,
        }
    }

    /// Ring built from `(seconds from now, handles, commit)` — production
    /// cadence is one sample per 60 s.
    fn series(points: &[(i64, u32, u64)], now: Instant) -> Ring {
        let mut ring = Ring::new();
        for &(secs, handles, commit) in points {
            ring.push(sample(secs, handles, commit, now));
        }
        ring
    }

    /// Evaluate the last point of a synthetic series (production entry point
    /// semantics: the newest sample is "now").
    fn eval_series(points: &[(i64, u32, u64)], warmup: bool) -> LeakReport {
        let now = test_now();
        let ring = series(points, now);
        let &(_, handles, commit) = points.last().expect("series must not be empty");
        evaluate(&ring, now, handles, commit, warmup)
    }

    /// `n` samples one minute apart, oldest first, ending at "now".
    fn per_minute(
        n: i64,
        handles: impl Fn(i64) -> u32,
        commit: impl Fn(i64) -> u64,
    ) -> Vec<(i64, u32, u64)> {
        (0..n)
            .map(|step| {
                let secs = -(n - 1 - step) * 60;
                (secs, handles(step), commit(step))
            })
            .collect()
    }

    #[test]
    fn ring_is_bounded() {
        let mut ring = Ring::new();
        let now = test_now();
        for i in 0..1000 {
            ring.push(sample(0, i as u32, 0, now));
        }
        assert_eq!(ring.len, RING, "ring must not grow past capacity");
        assert!(ring.count_in_window(now) <= RING);
    }

    #[test]
    fn oldest_in_window_ignores_expired_samples() {
        let mut ring = Ring::new();
        let now = test_now();
        // One sample far outside the window, one inside.
        ring.push(sample(-3600, 100, 0, now));
        ring.push(sample(-60, 200, 0, now));
        let oldest = ring.oldest_in_window(now).expect("oldest");
        assert_eq!(oldest.handles, 200, "expired sample must be ignored");
    }

    /// The 06-Nov production series: 643 handles, completely flat.
    #[test]
    fn flat_643_handles_over_15_minutes_is_not_a_leak() {
        let points = per_minute(16, |_| 643, |_| 200 * MB);
        let report = eval_series(&points, false);
        assert_eq!(report.samples, 16);
        assert_eq!(report.handles_per_min, 0.0, "flat series has zero growth");
        assert!(
            !report.leaking,
            "643 flat handles is the healthy steady state"
        );
    }

    /// Exactly the false positive from the audit: a 2-minute startup ramp
    /// (643 → 650, +3.5/min) must NOT alert. The old MIN_SAMPLES=3 fired here.
    #[test]
    fn startup_ramp_false_positive_from_audit_is_suppressed() {
        let points = [
            (-120, 643u32, 189 * MB),
            (-60, 646, 194 * MB),
            (0, 650, 200 * MB),
        ];
        let report = eval_series(&points, false);
        assert_eq!(report.samples, 3, "only three samples exist");
        assert!(
            (report.handles_per_min - 3.5).abs() < 0.01,
            "rate must still be reported for diagnostics, got {}",
            report.handles_per_min
        );
        assert!(
            !report.leaking,
            "three samples / two minutes is not enough history to call a leak"
        );

        // Second audit FP: +5.0/min over 2 minutes at 633 → 643.
        let second = [(-120, 633u32, 11 * MB), (0, 643, 22 * MB)];
        assert!(!eval_series(&second, false).leaking);
    }

    /// A real ramp must still be caught quickly: 643 → 20k in 15 minutes.
    #[test]
    fn sustained_handle_ramp_to_20k_is_flagged() {
        let points = per_minute(
            16,
            |step| 643 + ((20_000 - 643) * step / 15) as u32,
            |_| 200 * MB,
        );
        let report = eval_series(&points, false);
        assert!(
            report.handles_per_min > 500.0,
            "expected ~1290/min, got {}",
            report.handles_per_min
        );
        assert!(
            report.leaking,
            "a ramp to {} handles must be flagged",
            report.handles
        );
    }

    /// The absolute floor keeps small-process churn quiet: 300 → 400 handles
    /// is 10/min, above the rate threshold, but nowhere near a real leak.
    #[test]
    fn handle_leak_below_absolute_floor_is_quiet() {
        let points = per_minute(11, |step| 300 + (step as u32 * 10), |_| 120 * MB);
        let report = eval_series(&points, false);
        assert!(
            (report.handles_per_min - 10.0).abs() < 0.01,
            "rate is 10/min, got {}",
            report.handles_per_min
        );
        assert!(
            report.handles < ABSOLUTE_HANDLE_FLOOR,
            "test premise: below the floor"
        );
        assert!(!report.leaking, "small counts must not page anyone");
    }

    /// Commit growth alone still flags a leak (flat handles, 6 MB/min).
    #[test]
    fn commit_growth_alone_flags_a_leak() {
        let points = per_minute(11, |_| 6_000, |step| 200 * MB + step as u64 * 6 * MB);
        let report = eval_series(&points, false);
        assert!(
            report.commit_bytes_per_min > 5.0 * MB as f64,
            "expected ~6 MB/min, got {}",
            report.commit_bytes_per_min
        );
        assert!(
            report.leaking,
            "commit growth of 6 MB/min must be flagged even with flat handles"
        );
    }

    /// Warm-up doubles the thresholds, so a 4/min ramp is quiet in the first
    /// 30 minutes and loud afterwards.
    #[test]
    fn warmup_doubles_the_threshold() {
        let points = per_minute(11, |step| 6_000 + (step as u32 * 4), |_| 200 * MB);
        assert!(
            !eval_series(&points, true).leaking,
            "4/min during warm-up is below 2x3/min"
        );
        assert!(
            eval_series(&points, false).leaking,
            "4/min in steady state must be flagged"
        );
    }

    /// Sample count alone is not enough — the span gate must also pass.
    #[test]
    fn insufficient_span_never_flags() {
        let now = test_now();
        let mut ring = Ring::new();
        for i in 0..10 {
            ring.push(sample(-(i * 30), 6_000 + (i as u32 * 600), 200 * MB, now));
        }
        let report = evaluate(&ring, now, 11_400, 200 * MB, false);
        assert_eq!(report.samples, 10, "ten samples, but only 4.5 minutes");
        assert!(!report.leaking, "a short burst must not be called a leak");
    }

    #[test]
    fn rate_handles_shrinking_counts() {
        let mut ring = Ring::new();
        let now = test_now();
        // Samples DROP (e.g. the app restarted and reclaimed handles).
        ring.push(sample(-600, 5000, 2_000 * MB, now));
        ring.push(sample(0, 100, 100 * MB, now));

        let report = evaluate(&ring, now, 100, 100 * MB, false);
        assert_eq!(
            report.handles_per_min, 0.0,
            "saturating subtraction must yield 0, never a negative rate"
        );
        assert!(!report.leaking);
    }

    /// A single sample has no rate at all and must not be evaluated.
    #[test]
    fn single_sample_yields_no_rate() {
        let now = test_now();
        let mut ring = Ring::new();
        ring.push(sample(0, 900, 100 * MB, now));
        let report = evaluate(&ring, now, 900, 100 * MB, false);
        assert_eq!(report.samples, 1);
        assert_eq!(report.handles_per_min, 0.0);
        assert!(!report.leaking);
    }
}
