//! AI usage tracking — local-only tracking of AI requests and token usage.

use serde::{Deserialize, Serialize};
use std::sync::Mutex;

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
    let mut usage = USAGE.lock().unwrap();
    let stats = usage.get_or_insert_with(AiUsageStats::default);
    stats.total_requests += 1;
    stats.total_input_tokens += input_tokens;
    stats.total_output_tokens += output_tokens;
    stats.estimated_cost_usd += (input_tokens as f64 / 1_000_000.0) * COST_PER_1M_INPUT_TOKENS
        + (output_tokens as f64 / 1_000_000.0) * COST_PER_1M_OUTPUT_TOKENS;
}

/// Get current usage statistics.
pub fn get_usage() -> AiUsageStats {
    USAGE.lock().unwrap().clone().unwrap_or_default()
}

/// Reset usage statistics.
pub fn reset_usage() {
    *USAGE.lock().unwrap() = Some(AiUsageStats::default());
}
