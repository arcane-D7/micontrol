//! Deterministic windowed statistics for OS Turbo.
//!
//! S39-004: this module is the evidence model behind OS Turbo's adaptive
//! decisions. It is a **statistical learner**, not a deep-learning model:
//! every number is produced by fixed, reproducible arithmetic (Welford
//! accumulators, fixed-size windows, fixed confidence constants). There is no
//! training, no gradient descent, no model weights, and no network access.
//!
//! Design rules enforced here:
//!
//! 1. **Determinism.** Window boundaries, ordering, rounding, confidence
//!    constants and serialization are fixed. Two runs over the same samples
//!    produce byte-identical output.
//! 2. **Bounded memory.** A window holds at most [`MAX_WINDOW_SAMPLES`]
//!    samples; older samples are dropped (ring behaviour), never accumulated
//!    without limit.
//! 3. **No causal claims without a matched control.** A comparison is only
//!    evaluated when both windows describe the same [`WorkloadClass`] and both
//!    meet [`MIN_SAMPLES_PER_WINDOW`]. Otherwise the result is
//!    [`Verdict::InsufficientEvidence`] or [`Verdict::UnmatchedWorkload`].
//! 4. **No identifying data.** Samples carry only coarse metric values and a
//!    workload class. Process names, PIDs, paths, users and device identifiers
//!    are never accepted by this module.

use serde::{Deserialize, Serialize};

/// Maximum samples retained per window. Older samples are evicted first.
pub const MAX_WINDOW_SAMPLES: usize = 240;

/// Minimum samples required in EACH window before a comparison is evaluated.
/// Below this the result is [`Verdict::InsufficientEvidence`].
pub const MIN_SAMPLES_PER_WINDOW: usize = 8;

/// Fixed two-sided z value for a 95% confidence interval. Kept as a constant
/// (not computed) so results are identical across platforms and runs.
pub const Z_95: f64 = 1.96;

/// |Cohen's d| at or above this is treated as a real effect rather than noise.
pub const MIN_EFFECT_SIZE: f64 = 0.2;

/// Decimal places used when serializing statistics. Rounding is applied at the
/// boundary so in-memory arithmetic stays full precision.
pub const SERIALIZED_DECIMALS: u32 = 4;

/// Coarse workload class. Only samples of the same class are comparable —
/// comparing an idle window against a gaming window would attribute the
/// workload change to OS Turbo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadClass {
    Idle,
    Light,
    Medium,
    Heavy,
}

impl WorkloadClass {
    /// Classify a coarse foreground CPU-busy percentage into a workload class.
    /// Thresholds are fixed so classification is reproducible.
    pub fn from_cpu_busy_percent(percent: f64) -> Self {
        if percent < 10.0 {
            Self::Idle
        } else if percent < 35.0 {
            Self::Light
        } else if percent < 70.0 {
            Self::Medium
        } else {
            Self::Heavy
        }
    }
}

/// Metrics OS Turbo tracks. Each has a fixed "better" direction so an
/// improvement is never inferred from the sign alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MetricKind {
    /// Foreground CPU busy percentage.
    ForegroundCpuPercent,
    /// Background CPU busy percentage (the metric OS Turbo targets).
    BackgroundCpuPercent,
    /// Package power draw in watts, when the platform exposes it.
    PackagePowerWatts,
    /// CPU temperature in Celsius, when the platform exposes it.
    CpuTempCelsius,
    /// Memory pressure percentage.
    MemoryPressurePercent,
}

impl MetricKind {
    /// All metrics, in a fixed order used for deterministic iteration.
    pub const ALL: [MetricKind; 5] = [
        MetricKind::ForegroundCpuPercent,
        MetricKind::BackgroundCpuPercent,
        MetricKind::PackagePowerWatts,
        MetricKind::CpuTempCelsius,
        MetricKind::MemoryPressurePercent,
    ];

    /// `true` when a lower value is the desirable direction.
    pub fn lower_is_better(self) -> bool {
        true
    }

    /// Plausible range used to reject corrupt or mis-scaled readings.
    pub fn plausible_range(self) -> (f64, f64) {
        match self {
            MetricKind::ForegroundCpuPercent | MetricKind::BackgroundCpuPercent => (0.0, 100.0),
            MetricKind::PackagePowerWatts => (0.0, 250.0),
            MetricKind::CpuTempCelsius => (0.0, 125.0),
            MetricKind::MemoryPressurePercent => (0.0, 100.0),
        }
    }
}

/// One coarse observation. Contains no identifying data by construction.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct MetricSample {
    pub workload: WorkloadClass,
    pub metric: MetricKind,
    pub value: f64,
}

impl MetricSample {
    /// Returns `None` when the value is non-finite or outside the metric's
    /// plausible range — invalid data must never enter a window.
    pub fn validated(workload: WorkloadClass, metric: MetricKind, value: f64) -> Option<Self> {
        if !value.is_finite() {
            return None;
        }
        let (lo, hi) = metric.plausible_range();
        if value < lo || value > hi {
            return None;
        }
        Some(Self {
            workload,
            metric,
            value,
        })
    }
}

/// Welford accumulator: numerically stable running mean and variance.
///
/// Uses the standard online update and the parallel merge formula, so windows
/// can be combined without re-reading samples.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Welford {
    pub count: u64,
    pub mean: f64,
    /// Sum of squared deviations from the running mean.
    pub m2: f64,
    pub min: f64,
    pub max: f64,
}

impl Default for Welford {
    fn default() -> Self {
        Self {
            count: 0,
            mean: 0.0,
            m2: 0.0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
        }
    }
}

impl Welford {
    /// Add one observation.
    pub fn push(&mut self, value: f64) {
        self.count += 1;
        let delta = value - self.mean;
        self.mean += delta / self.count as f64;
        let delta2 = value - self.mean;
        self.m2 += delta * delta2;
        if value < self.min {
            self.min = value;
        }
        if value > self.max {
            self.max = value;
        }
    }

    /// Merge another accumulator into this one (parallel Welford).
    pub fn merge(&mut self, other: &Welford) {
        if other.count == 0 {
            return;
        }
        if self.count == 0 {
            *self = *other;
            return;
        }
        let n_a = self.count as f64;
        let n_b = other.count as f64;
        let total = n_a + n_b;
        let delta = other.mean - self.mean;
        self.mean += delta * n_b / total;
        self.m2 += other.m2 + delta * delta * n_a * n_b / total;
        self.count += other.count;
        if other.min < self.min {
            self.min = other.min;
        }
        if other.max > self.max {
            self.max = other.max;
        }
    }

    /// Sample standard deviation (`n - 1` denominator). Returns `None` when
    /// fewer than two samples exist — a single sample has no spread.
    pub fn sample_sd(&self) -> Option<f64> {
        if self.count < 2 {
            return None;
        }
        let variance = self.m2 / (self.count as f64 - 1.0);
        if variance < 0.0 {
            // Guard against tiny negative values from floating-point drift.
            return Some(0.0);
        }
        Some(variance.sqrt())
    }

    /// Population standard deviation (`n` denominator), used for effect sizes
    /// where the window is treated as the whole observed population.
    pub fn population_sd(&self) -> Option<f64> {
        if self.count == 0 {
            return None;
        }
        let variance = self.m2 / self.count as f64;
        if variance < 0.0 {
            return Some(0.0);
        }
        Some(variance.sqrt())
    }
}

/// Bounded window of samples for one metric and one workload class.
///
/// Samples of a different metric or workload class are rejected, so a window
/// can never mix incomparable data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SampleWindow {
    pub metric: MetricKind,
    pub workload: WorkloadClass,
    pub stats: Welford,
    /// Ring buffer of retained values, oldest first.
    values: Vec<f64>,
}

impl SampleWindow {
    pub fn new(metric: MetricKind, workload: WorkloadClass) -> Self {
        Self {
            metric,
            workload,
            stats: Welford::default(),
            values: Vec::new(),
        }
    }

    /// Push a sample. Returns `false` when the sample belongs to a different
    /// metric or workload class (and was therefore rejected).
    pub fn push(&mut self, sample: MetricSample) -> bool {
        if sample.metric != self.metric || sample.workload != self.workload {
            return false;
        }
        if self.values.len() >= MAX_WINDOW_SAMPLES {
            // Evict the oldest value. The accumulator is rebuilt from the
            // retained values so mean/variance stay consistent with the ring.
            self.values.remove(0);
            self.rebuild();
        }
        self.values.push(sample.value);
        self.stats.push(sample.value);
        true
    }

    /// Recompute the accumulator from the retained values (deterministic).
    fn rebuild(&mut self) {
        let mut stats = Welford::default();
        for value in &self.values {
            stats.push(*value);
        }
        self.stats = stats;
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Retained values, oldest first (deterministic order).
    pub fn values(&self) -> &[f64] {
        &self.values
    }
}

/// Why a comparison could not produce a usable verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsufficientReason {
    /// One or both windows have fewer than [`MIN_SAMPLES_PER_WINDOW`] samples.
    TooFewSamples,
    /// The windows describe different workload classes.
    WorkloadMismatch,
    /// The windows describe different metrics.
    MetricMismatch,
    /// The metric has no measurable spread, so an effect size is undefined.
    NoSpread,
}

/// Direction of a measured change, from the metric's own perspective.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectDirection {
    /// The metric moved in its desirable direction.
    Improvement,
    /// The metric moved in its undesirable direction.
    Regression,
    /// The change is within noise.
    Neutral,
}

/// Outcome of comparing a before-window against an after-window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Verdict {
    /// A defensible comparison was produced.
    Measured(MeasuredEffect),
    /// No causal claim is possible; the reason is explicit.
    InsufficientEvidence { reason: InsufficientReason },
}

/// A comparison that met the evidence bar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MeasuredEffect {
    pub metric: MetricKind,
    pub workload: WorkloadClass,
    pub before_samples: u64,
    pub after_samples: u64,
    /// Rounded to [`SERIALIZED_DECIMALS`] for deterministic serialization.
    pub before_mean: f64,
    pub after_mean: f64,
    /// Signed change (`after - before`), rounded.
    pub delta: f64,
    /// Cohen's d, rounded. `None` when the pooled spread is zero.
    pub effect_size: Option<f64>,
    /// 95% confidence interval for the difference of means, rounded.
    pub ci_low: f64,
    pub ci_high: f64,
    /// `true` when the interval excludes zero.
    pub ci_excludes_zero: bool,
    pub direction: EffectDirection,
}

/// Round to a fixed number of decimals for deterministic serialization.
pub fn round_deterministic(value: f64, decimals: u32) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    let factor = 10f64.powi(decimals as i32);
    (value * factor).round() / factor
}

/// Compare two windows and produce a verdict.
///
/// The comparison refuses to claim causality unless both windows describe the
/// same metric and workload class and both meet the minimum sample count.
pub fn compare_windows(before: &SampleWindow, after: &SampleWindow) -> Verdict {
    if before.metric != after.metric {
        return Verdict::InsufficientEvidence {
            reason: InsufficientReason::MetricMismatch,
        };
    }
    if before.workload != after.workload {
        return Verdict::InsufficientEvidence {
            reason: InsufficientReason::WorkloadMismatch,
        };
    }
    if before.stats.count < MIN_SAMPLES_PER_WINDOW as u64
        || after.stats.count < MIN_SAMPLES_PER_WINDOW as u64
    {
        return Verdict::InsufficientEvidence {
            reason: InsufficientReason::TooFewSamples,
        };
    }

    let n_a = before.stats.count as f64;
    let n_b = after.stats.count as f64;
    let var_a = before.stats.m2 / (n_a - 1.0).max(1.0);
    let var_b = after.stats.m2 / (n_b - 1.0).max(1.0);

    let delta = after.stats.mean - before.stats.mean;
    let se = (var_a / n_a + var_b / n_b).sqrt();
    let margin = Z_95 * se;
    let ci_low = delta - margin;
    let ci_high = delta + margin;
    let ci_excludes_zero = ci_low > 0.0 || ci_high < 0.0;

    // Pooled standard deviation for Cohen's d.
    let pooled_var = ((n_a - 1.0) * var_a + (n_b - 1.0) * var_b) / (n_a + n_b - 2.0);
    let effect_size = if pooled_var > 0.0 {
        Some(delta / pooled_var.sqrt())
    } else {
        None
    };

    let direction = match effect_size {
        None => EffectDirection::Neutral,
        Some(d) => {
            if !ci_excludes_zero || d.abs() < MIN_EFFECT_SIZE {
                EffectDirection::Neutral
            } else {
                // `lower_is_better` metrics improve when the value decreases.
                let improved = if before.metric.lower_is_better() {
                    d < 0.0
                } else {
                    d > 0.0
                };
                if improved {
                    EffectDirection::Improvement
                } else {
                    EffectDirection::Regression
                }
            }
        }
    };

    Verdict::Measured(MeasuredEffect {
        metric: before.metric,
        workload: before.workload,
        before_samples: before.stats.count,
        after_samples: after.stats.count,
        before_mean: round_deterministic(before.stats.mean, SERIALIZED_DECIMALS),
        after_mean: round_deterministic(after.stats.mean, SERIALIZED_DECIMALS),
        delta: round_deterministic(delta, SERIALIZED_DECIMALS),
        effect_size: effect_size.map(|d| round_deterministic(d, SERIALIZED_DECIMALS)),
        ci_low: round_deterministic(ci_low, SERIALIZED_DECIMALS),
        ci_high: round_deterministic(ci_high, SERIALIZED_DECIMALS),
        ci_excludes_zero,
        direction,
    })
}

/// Confidence bucket derived from sample count and interval width.
///
/// Deliberately coarse: the policy layer consumes a bucket, not a raw float,
/// so decisions cannot hinge on floating-point noise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfidenceBucket {
    /// Not enough evidence to act on.
    None,
    Low,
    Medium,
    High,
}

/// Derive a confidence bucket from a measured effect.
///
/// Rules (fixed, deterministic):
/// - `None` when the interval includes zero or the effect is below threshold.
/// - `Low` when the interval excludes zero but the sample count is small.
/// - `Medium` when both windows have at least twice the minimum samples.
/// - `High` when both windows have at least four times the minimum samples and
///   the effect size is at least twice the minimum.
pub fn confidence_bucket(effect: &MeasuredEffect) -> ConfidenceBucket {
    if !effect.ci_excludes_zero {
        return ConfidenceBucket::None;
    }
    let Some(d) = effect.effect_size else {
        return ConfidenceBucket::None;
    };
    if d.abs() < MIN_EFFECT_SIZE {
        return ConfidenceBucket::None;
    }
    let min = MIN_SAMPLES_PER_WINDOW as u64;
    let both = effect.before_samples.min(effect.after_samples);
    if both >= min * 4 && d.abs() >= MIN_EFFECT_SIZE * 2.0 {
        ConfidenceBucket::High
    } else if both >= min * 2 {
        ConfidenceBucket::Medium
    } else {
        ConfidenceBucket::Low
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window_with(metric: MetricKind, workload: WorkloadClass, values: &[f64]) -> SampleWindow {
        let mut window = SampleWindow::new(metric, workload);
        for value in values {
            assert!(window.push(MetricSample {
                workload,
                metric,
                value: *value,
            }));
        }
        window
    }

    #[test]
    fn welford_matches_naive_mean_and_variance() {
        let values = [2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        let mut w = Welford::default();
        for v in values {
            w.push(v);
        }
        let naive_mean = values.iter().sum::<f64>() / values.len() as f64;
        assert!((w.mean - naive_mean).abs() < 1e-12);
        let naive_var = values.iter().map(|v| (v - naive_mean).powi(2)).sum::<f64>()
            / (values.len() - 1) as f64;
        let sd = w.sample_sd().expect("sd");
        assert!((sd - naive_var.sqrt()).abs() < 1e-12);
        assert_eq!(w.min, 2.0);
        assert_eq!(w.max, 9.0);
    }

    #[test]
    fn welford_merge_equals_single_pass() {
        let left = [1.0, 2.0, 3.0, 4.0];
        let right = [10.0, 12.0, 14.0];

        let mut merged = Welford::default();
        for v in left {
            merged.push(v);
        }
        let mut other = Welford::default();
        for v in right {
            other.push(v);
        }
        merged.merge(&other);

        let mut single = Welford::default();
        for v in left.iter().chain(right.iter()) {
            single.push(*v);
        }

        assert_eq!(merged.count, single.count);
        assert!((merged.mean - single.mean).abs() < 1e-12);
        assert!((merged.m2 - single.m2).abs() < 1e-9);
    }

    #[test]
    fn single_sample_has_no_spread() {
        let mut w = Welford::default();
        w.push(5.0);
        assert!(w.sample_sd().is_none());
        assert_eq!(w.population_sd(), Some(0.0));
    }

    #[test]
    fn identical_values_have_zero_variance() {
        let mut w = Welford::default();
        for _ in 0..10 {
            w.push(3.5);
        }
        assert_eq!(w.sample_sd(), Some(0.0));
    }

    #[test]
    fn invalid_samples_are_rejected() {
        assert!(
            MetricSample::validated(WorkloadClass::Idle, MetricKind::CpuTempCelsius, f64::NAN)
                .is_none()
        );
        assert!(MetricSample::validated(
            WorkloadClass::Idle,
            MetricKind::CpuTempCelsius,
            f64::INFINITY
        )
        .is_none());
        // Out of plausible range.
        assert!(MetricSample::validated(
            WorkloadClass::Idle,
            MetricKind::ForegroundCpuPercent,
            150.0
        )
        .is_none());
        assert!(
            MetricSample::validated(WorkloadClass::Idle, MetricKind::CpuTempCelsius, 42.0)
                .is_some()
        );
    }

    #[test]
    fn window_rejects_mismatched_metric_and_workload() {
        let mut window = SampleWindow::new(MetricKind::BackgroundCpuPercent, WorkloadClass::Light);
        assert!(!window.push(MetricSample {
            workload: WorkloadClass::Light,
            metric: MetricKind::CpuTempCelsius,
            value: 50.0,
        }));
        assert!(!window.push(MetricSample {
            workload: WorkloadClass::Heavy,
            metric: MetricKind::BackgroundCpuPercent,
            value: 20.0,
        }));
        assert!(window.push(MetricSample {
            workload: WorkloadClass::Light,
            metric: MetricKind::BackgroundCpuPercent,
            value: 20.0,
        }));
        assert_eq!(window.len(), 1);
    }

    #[test]
    fn window_is_bounded_and_evicts_oldest() {
        let mut window = SampleWindow::new(MetricKind::BackgroundCpuPercent, WorkloadClass::Idle);
        for i in 0..(MAX_WINDOW_SAMPLES + 50) {
            window.push(MetricSample {
                workload: WorkloadClass::Idle,
                metric: MetricKind::BackgroundCpuPercent,
                value: (i % 10) as f64,
            });
        }
        assert_eq!(window.len(), MAX_WINDOW_SAMPLES);
        assert_eq!(window.stats.count, MAX_WINDOW_SAMPLES as u64);
        // Accumulator must stay consistent with the retained ring.
        let retained_mean = window.values().iter().sum::<f64>() / window.values().len() as f64;
        assert!((window.stats.mean - retained_mean).abs() < 1e-9);
    }

    #[test]
    fn too_few_samples_is_insufficient_evidence() {
        let before = window_with(
            MetricKind::BackgroundCpuPercent,
            WorkloadClass::Light,
            &[20.0, 21.0, 19.0],
        );
        let after = window_with(
            MetricKind::BackgroundCpuPercent,
            WorkloadClass::Light,
            &[10.0, 11.0, 9.0],
        );
        assert_eq!(
            compare_windows(&before, &after),
            Verdict::InsufficientEvidence {
                reason: InsufficientReason::TooFewSamples
            }
        );
    }

    #[test]
    fn workload_mismatch_refuses_causal_claim() {
        let before = window_with(
            MetricKind::BackgroundCpuPercent,
            WorkloadClass::Idle,
            &[20.0; MIN_SAMPLES_PER_WINDOW],
        );
        let after = window_with(
            MetricKind::BackgroundCpuPercent,
            WorkloadClass::Heavy,
            &[10.0; MIN_SAMPLES_PER_WINDOW],
        );
        assert_eq!(
            compare_windows(&before, &after),
            Verdict::InsufficientEvidence {
                reason: InsufficientReason::WorkloadMismatch
            }
        );
    }

    #[test]
    fn metric_mismatch_refuses_causal_claim() {
        let before = window_with(
            MetricKind::BackgroundCpuPercent,
            WorkloadClass::Light,
            &[20.0; MIN_SAMPLES_PER_WINDOW],
        );
        let after = window_with(
            MetricKind::CpuTempCelsius,
            WorkloadClass::Light,
            &[50.0; MIN_SAMPLES_PER_WINDOW],
        );
        assert_eq!(
            compare_windows(&before, &after),
            Verdict::InsufficientEvidence {
                reason: InsufficientReason::MetricMismatch
            }
        );
    }

    #[test]
    fn clear_reduction_is_an_improvement() {
        let before = window_with(
            MetricKind::BackgroundCpuPercent,
            WorkloadClass::Light,
            &[40.0, 41.0, 39.0, 40.5, 39.5, 40.2, 39.8, 40.1, 40.3, 39.9],
        );
        let after = window_with(
            MetricKind::BackgroundCpuPercent,
            WorkloadClass::Light,
            &[20.0, 21.0, 19.0, 20.5, 19.5, 20.2, 19.8, 20.1, 20.3, 19.9],
        );
        match compare_windows(&before, &after) {
            Verdict::Measured(effect) => {
                assert_eq!(effect.direction, EffectDirection::Improvement);
                assert!(effect.ci_excludes_zero);
                assert!(effect.effect_size.expect("d").abs() > MIN_EFFECT_SIZE);
                // 10 samples clears the minimum (8) but not twice the minimum
                // (16), so the documented bucket is Low.
                assert_eq!(confidence_bucket(&effect), ConfidenceBucket::Low);
            }
            other => panic!("expected measured effect, got {other:?}"),
        }
    }

    #[test]
    fn clear_increase_is_a_regression() {
        let before = window_with(
            MetricKind::BackgroundCpuPercent,
            WorkloadClass::Light,
            &[20.0, 21.0, 19.0, 20.5, 19.5, 20.2, 19.8, 20.1, 20.3, 19.9],
        );
        let after = window_with(
            MetricKind::BackgroundCpuPercent,
            WorkloadClass::Light,
            &[40.0, 41.0, 39.0, 40.5, 39.5, 40.2, 39.8, 40.1, 40.3, 39.9],
        );
        match compare_windows(&before, &after) {
            Verdict::Measured(effect) => {
                assert_eq!(effect.direction, EffectDirection::Regression);
            }
            other => panic!("expected measured effect, got {other:?}"),
        }
    }

    #[test]
    fn overlapping_intervals_are_neutral() {
        let before = window_with(
            MetricKind::BackgroundCpuPercent,
            WorkloadClass::Light,
            &[20.0, 30.0, 10.0, 25.0, 15.0, 28.0, 12.0, 22.0, 18.0, 24.0],
        );
        let after = window_with(
            MetricKind::BackgroundCpuPercent,
            WorkloadClass::Light,
            &[21.0, 31.0, 11.0, 26.0, 16.0, 29.0, 13.0, 23.0, 19.0, 25.0],
        );
        match compare_windows(&before, &after) {
            Verdict::Measured(effect) => {
                assert_eq!(effect.direction, EffectDirection::Neutral);
                assert_eq!(confidence_bucket(&effect), ConfidenceBucket::None);
            }
            other => panic!("expected measured effect, got {other:?}"),
        }
    }

    #[test]
    fn zero_spread_yields_no_effect_size() {
        let before = window_with(
            MetricKind::BackgroundCpuPercent,
            WorkloadClass::Light,
            &[20.0; MIN_SAMPLES_PER_WINDOW],
        );
        let after = window_with(
            MetricKind::BackgroundCpuPercent,
            WorkloadClass::Light,
            &[20.0; MIN_SAMPLES_PER_WINDOW],
        );
        match compare_windows(&before, &after) {
            Verdict::Measured(effect) => {
                assert!(effect.effect_size.is_none());
                assert_eq!(effect.direction, EffectDirection::Neutral);
            }
            other => panic!("expected measured effect, got {other:?}"),
        }
    }

    #[test]
    fn high_confidence_requires_many_samples_and_large_effect() {
        let before = window_with(
            MetricKind::BackgroundCpuPercent,
            WorkloadClass::Light,
            &[
                40.0, 41.0, 39.0, 40.5, 39.5, 40.2, 39.8, 40.1, 40.3, 39.9, 40.4, 39.6, 40.0, 40.1,
                39.9, 40.2, 39.8, 40.0, 40.1, 39.9, 40.0, 40.2, 39.8, 40.1, 39.9, 40.0, 40.1, 39.9,
                40.0, 40.2, 39.8, 40.0,
            ],
        );
        let after = window_with(
            MetricKind::BackgroundCpuPercent,
            WorkloadClass::Light,
            &[
                10.0, 11.0, 9.0, 10.5, 9.5, 10.2, 9.8, 10.1, 10.3, 9.9, 10.4, 9.6, 10.0, 10.1, 9.9,
                10.2, 9.8, 10.0, 10.1, 9.9, 10.0, 10.2, 9.8, 10.1, 9.9, 10.0, 10.1, 9.9, 10.0,
                10.2, 9.8, 10.0,
            ],
        );
        match compare_windows(&before, &after) {
            Verdict::Measured(effect) => {
                assert_eq!(confidence_bucket(&effect), ConfidenceBucket::High);
            }
            other => panic!("expected measured effect, got {other:?}"),
        }
    }

    #[test]
    fn serialization_is_deterministic_and_rounded() {
        let before = window_with(
            MetricKind::BackgroundCpuPercent,
            WorkloadClass::Light,
            &[40.0, 41.0, 39.0, 40.5, 39.5, 40.2, 39.8, 40.1, 40.3, 39.9],
        );
        let after = window_with(
            MetricKind::BackgroundCpuPercent,
            WorkloadClass::Light,
            &[20.0, 21.0, 19.0, 20.5, 19.5, 20.2, 19.8, 20.1, 20.3, 19.9],
        );
        let first = serde_json::to_string(&compare_windows(&before, &after)).expect("json");
        let second = serde_json::to_string(&compare_windows(&before, &after)).expect("json");
        assert_eq!(first, second);

        // Every serialized float must be rounded to the documented precision.
        match compare_windows(&before, &after) {
            Verdict::Measured(effect) => {
                for value in [
                    effect.before_mean,
                    effect.after_mean,
                    effect.delta,
                    effect.ci_low,
                    effect.ci_high,
                ] {
                    assert_eq!(value, round_deterministic(value, SERIALIZED_DECIMALS));
                }
            }
            other => panic!("expected measured effect, got {other:?}"),
        }
    }

    #[test]
    fn workload_classification_thresholds_are_fixed() {
        assert_eq!(
            WorkloadClass::from_cpu_busy_percent(0.0),
            WorkloadClass::Idle
        );
        assert_eq!(
            WorkloadClass::from_cpu_busy_percent(9.9),
            WorkloadClass::Idle
        );
        assert_eq!(
            WorkloadClass::from_cpu_busy_percent(10.0),
            WorkloadClass::Light
        );
        assert_eq!(
            WorkloadClass::from_cpu_busy_percent(34.9),
            WorkloadClass::Light
        );
        assert_eq!(
            WorkloadClass::from_cpu_busy_percent(35.0),
            WorkloadClass::Medium
        );
        assert_eq!(
            WorkloadClass::from_cpu_busy_percent(70.0),
            WorkloadClass::Heavy
        );
    }

    #[test]
    fn metric_order_is_stable() {
        // Deterministic iteration order is part of the contract.
        assert_eq!(MetricKind::ALL[0], MetricKind::ForegroundCpuPercent);
        assert_eq!(MetricKind::ALL[4], MetricKind::MemoryPressurePercent);
    }
}
