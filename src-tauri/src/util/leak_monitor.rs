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

#![cfg(windows)]

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// How often the sampler runs.
pub const SAMPLE_INTERVAL: Duration = Duration::from_secs(60);

/// Sliding window used to compute the growth rate.
pub const WINDOW: Duration = Duration::from_secs(15 * 60);

/// Handles/minute above which a leak is declared.
///
/// The audited leak ran at ~11/min. A healthy steady state is < 1/min
/// (occasional short-lived handles), so 3/min gives comfortable margin while
/// still catching a regression in ~1 hour instead of 218.
pub const LEAK_HANDLES_PER_MIN: f64 = 3.0;

/// Private/commit bytes per minute above which a leak is declared.
///
/// The audited leak grew commit by ~11.5 MB/min alongside the handles.
/// 2 MB/min is well above normal allocator noise (log rotation, caches).
pub const LEAK_COMMIT_BYTES_PER_MIN: f64 = 2.0 * 1024.0 * 1024.0;

/// Minimum number of samples before a rate is meaningful.
const MIN_SAMPLES: usize = 3;

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

            loop {
                std::thread::sleep(SAMPLE_INTERVAL);
                if let Some(report) = sample_and_evaluate(&mut ring) {
                    LAST_HANDLES.store(report.handles, Ordering::Relaxed);
                    LAST_COMMIT_MB.store(report.commit_bytes / (1024 * 1024), Ordering::Relaxed);
                    LAST_RATE_X1000
                        .store((report.handles_per_min * 1000.0) as u64, Ordering::Relaxed);
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
                                 commit {} MB (+{:.1} MB/min) over {} samples — see \
                                 memory-leak-audit.md; restart the app to reclaim",
                                report.handles,
                                report.handles_per_min,
                                report.commit_bytes / (1024 * 1024),
                                report.commit_bytes_per_min / (1024.0 * 1024.0),
                                report.samples
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
    LeakReport {
        handles,
        commit_bytes,
        handles_per_min: rate,
        commit_bytes_per_min: 0.0,
        samples: 0,
        leaking: LEAKING.load(Ordering::Relaxed),
    }
}

/// Number of leak alerts raised this session (diagnostics/UI).
pub fn alert_count() -> u64 {
    ALERTS.load(Ordering::Relaxed)
}

/// One measurement + rate evaluation. Extracted from the loop so it is
/// unit-testable without sleeping for minutes.
fn sample_and_evaluate(ring: &mut Ring) -> Option<LeakReport> {
    let handles = super::win_handle::process_handle_count()?;
    let commit_bytes = private_bytes()?;
    let now = Instant::now();
    ring.push(Sample {
        at: now,
        handles,
        commit_bytes,
    });

    let samples = ring.count_in_window(now);
    if samples < MIN_SAMPLES {
        return Some(LeakReport {
            handles,
            commit_bytes,
            handles_per_min: 0.0,
            commit_bytes_per_min: 0.0,
            samples,
            leaking: false,
        });
    }

    let oldest = ring.oldest_in_window(now)?;
    let elapsed_min = now.duration_since(oldest.at).as_secs_f64() / 60.0;
    if elapsed_min <= 0.0 {
        return None;
    }

    // Saturating subtraction: a DROP (app reclaimed memory) yields 0 growth.
    let d_handles = handles.saturating_sub(oldest.handles) as f64;
    let d_commit = commit_bytes.saturating_sub(oldest.commit_bytes) as f64;
    let handles_per_min = d_handles / elapsed_min;
    let commit_bytes_per_min = d_commit / elapsed_min;

    let leaking = handles_per_min >= LEAK_HANDLES_PER_MIN
        || commit_bytes_per_min >= LEAK_COMMIT_BYTES_PER_MIN;

    Some(LeakReport {
        handles,
        commit_bytes,
        handles_per_min,
        commit_bytes_per_min,
        samples,
        leaking,
    })
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

    fn s(secs_from_now: i64, handles: u32, commit: u64, now: Instant) -> Sample {
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

    #[test]
    fn ring_is_bounded() {
        let mut ring = Ring::new();
        let now = Instant::now();
        for i in 0..1000 {
            ring.push(s(0, i as u32, 0, now));
        }
        assert_eq!(ring.len, RING, "ring must not grow past capacity");
        assert!(ring.count_in_window(now) <= RING);
    }

    #[test]
    fn oldest_in_window_ignores_expired_samples() {
        let mut ring = Ring::new();
        let now = Instant::now();
        // One sample far outside the window, one inside.
        ring.push(s(-3600, 100, 0, now));
        ring.push(s(-60, 200, 0, now));
        let oldest = ring.oldest_in_window(now).expect("oldest");
        assert_eq!(oldest.handles, 200, "expired sample must be ignored");
    }

    #[test]
    fn rate_detects_handle_growth() {
        let mut ring = Ring::new();
        let now = Instant::now();
        // 10 minutes ago: 1000 handles. Now: 1100 → 10/min → LEAK.
        ring.push(s(-600, 1000, 500 * 1024 * 1024, now));
        ring.push(s(-300, 1050, 550 * 1024 * 1024, now));
        ring.push(Sample {
            at: now,
            handles: 1100,
            commit_bytes: 600 * 1024 * 1024,
        });

        let report = evaluate(&ring, now, 1100, 600 * 1024 * 1024);
        assert!(
            report.handles_per_min >= 9.0 && report.handles_per_min <= 11.0,
            "expected ~10/min, got {}",
            report.handles_per_min
        );
        assert!(report.leaking, "10 handles/min must be flagged as a leak");
    }

    #[test]
    fn rate_ignores_stable_state() {
        let mut ring = Ring::new();
        let now = Instant::now();
        ring.push(s(-600, 500, 200 * 1024 * 1024, now));
        ring.push(s(-300, 505, 201 * 1024 * 1024, now));
        ring.push(Sample {
            at: now,
            handles: 508,
            commit_bytes: 202 * 1024 * 1024,
        });

        let report = evaluate(&ring, now, 508, 202 * 1024 * 1024);
        assert!(!report.leaking, "stable counts must not be flagged");
    }

    #[test]
    fn rate_handles_shrinking_counts() {
        let mut ring = Ring::new();
        let now = Instant::now();
        // Samples DROP (e.g. the app restarted and reclaimed handles).
        ring.push(s(-600, 5000, 2_000 * 1024 * 1024, now));
        ring.push(Sample {
            at: now,
            handles: 100,
            commit_bytes: 100 * 1024 * 1024,
        });

        let report = evaluate(&ring, now, 100, 100 * 1024 * 1024);
        assert_eq!(
            report.handles_per_min, 0.0,
            "saturating subtraction must yield 0, never a negative rate"
        );
        assert!(!report.leaking);
    }

    #[test]
    fn commit_growth_alone_flags_a_leak() {
        let mut ring = Ring::new();
        let now = Instant::now();
        // Handles flat, commit +100 MB in 10 min = 10 MB/min → LEAK.
        ring.push(s(-600, 500, 100 * 1024 * 1024, now));
        ring.push(s(-300, 500, 150 * 1024 * 1024, now));
        ring.push(Sample {
            at: now,
            handles: 500,
            commit_bytes: 200 * 1024 * 1024,
        });

        let report = evaluate(&ring, now, 500, 200 * 1024 * 1024);
        assert!(
            report.leaking,
            "commit growth of 10 MB/min must be flagged even with flat handles"
        );
    }

    /// Mirrors the production evaluation without touching process statics.
    fn evaluate(ring: &Ring, now: Instant, handles: u32, commit: u64) -> LeakReport {
        let samples = ring.count_in_window(now);
        let oldest = ring.oldest_in_window(now).expect("oldest");
        let elapsed_min = now.duration_since(oldest.at).as_secs_f64() / 60.0;
        let handles_per_min = handles.saturating_sub(oldest.handles) as f64 / elapsed_min;
        let commit_bytes_per_min = commit.saturating_sub(oldest.commit_bytes) as f64 / elapsed_min;
        LeakReport {
            handles,
            commit_bytes: commit,
            handles_per_min,
            commit_bytes_per_min,
            samples,
            leaking: handles_per_min >= LEAK_HANDLES_PER_MIN
                || commit_bytes_per_min >= LEAK_COMMIT_BYTES_PER_MIN,
        }
    }
}
