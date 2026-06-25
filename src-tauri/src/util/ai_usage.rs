//! AI usage tracking — local-only tracking of AI requests and token usage.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;

use crate::util::panic::lock_or_recover;

/// AI usage statistics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AiUsageStats {
    pub total_requests: u64,
    pub total_input_tokens: u64,
    pub total_output_tokens: u64,
    pub estimated_cost_usd: f64,
}

/// Cost per 1M tokens (example rates — adjust based on actual provider).
const COST_PER_1M_INPUT_TOKENS: f64 = 0.10;
const COST_PER_1M_OUTPUT_TOKENS: f64 = 0.30;

static USAGE: Mutex<Option<AiUsageStats>> = Mutex::new(None);

/// Record an AI request's token usage.
pub fn record_usage(input_tokens: u64, output_tokens: u64) {
    let mut usage = lock_or_recover(&USAGE);
    let stats = usage.get_or_insert_with(AiUsageStats::default);
    stats.total_requests += 1;
    stats.total_input_tokens += input_tokens;
    stats.total_output_tokens += output_tokens;
    stats.estimated_cost_usd += (input_tokens as f64 / 1_000_000.0) * COST_PER_1M_INPUT_TOKENS
        + (output_tokens as f64 / 1_000_000.0) * COST_PER_1M_OUTPUT_TOKENS;
}

/// Get current usage statistics.
pub fn get_usage() -> AiUsageStats {
    lock_or_recover(&USAGE).clone().unwrap_or_default()
}

/// Reset usage statistics.
pub fn reset_usage() {
    *lock_or_recover(&USAGE) = Some(AiUsageStats::default());
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialize tests that modify the global USAGE counter.
    static USAGE_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_record_usage_increments_counters() {
        let _lock = USAGE_LOCK.lock().unwrap();
        reset_usage();
        record_usage(100, 200);
        let stats = get_usage();
        assert_eq!(stats.total_requests, 1);
        assert_eq!(stats.total_input_tokens, 100);
        assert_eq!(stats.total_output_tokens, 200);
    }

    #[test]
    fn test_get_usage_returns_correct_stats() {
        let _lock = USAGE_LOCK.lock().unwrap();
        reset_usage();
        record_usage(50, 60);
        record_usage(70, 80);
        let stats = get_usage();
        assert_eq!(stats.total_requests, 2);
        assert_eq!(stats.total_input_tokens, 120);
        assert_eq!(stats.total_output_tokens, 140);
        // Cost: (120/1M * 0.10) + (140/1M * 0.30)
        let expected_cost = (120.0 / 1_000_000.0) * COST_PER_1M_INPUT_TOKENS
            + (140.0 / 1_000_000.0) * COST_PER_1M_OUTPUT_TOKENS;
        assert!((stats.estimated_cost_usd - expected_cost).abs() < 1e-10);
    }

    #[test]
    fn test_reset_usage_clears_counters() {
        let _lock = USAGE_LOCK.lock().unwrap();
        record_usage(100, 200);
        record_usage(300, 400);
        reset_usage();
        let stats = get_usage();
        assert_eq!(stats.total_requests, 0);
        assert_eq!(stats.total_input_tokens, 0);
        assert_eq!(stats.total_output_tokens, 0);
        assert_eq!(stats.estimated_cost_usd, 0.0);
    }

    #[test]
    fn test_concurrent_access() {
        use std::thread;

        let _lock = USAGE_LOCK.lock().unwrap();
        reset_usage();

        let handles: Vec<_> = (0..4)
            .map(|_| {
                thread::spawn(|| {
                    for _ in 0..100 {
                        record_usage(10, 20);
                    }
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap();
        }

        let stats = get_usage();
        assert_eq!(stats.total_requests, 400);
        assert_eq!(stats.total_input_tokens, 4000);
        assert_eq!(stats.total_output_tokens, 8000);
    }
}
