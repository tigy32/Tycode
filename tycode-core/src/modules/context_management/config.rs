//! Context management module configuration.
//!
//! Configures the compaction planner: when conversation history is rewritten
//! (reasoning pruned, old tool results stubbed, or the whole conversation
//! summarized) and how much material each pass removes.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn default_enabled() -> bool {
    true
}

fn default_auto_compact() -> bool {
    true
}

fn default_reasoning_prune_retain() -> usize {
    8
}

fn default_window_pressure_fraction() -> f64 {
    0.8
}

fn default_expected_remaining_requests() -> u32 {
    25
}

fn default_cache_ttl_seconds() -> u64 {
    300
}

fn default_tool_result_keep_recent_turns() -> usize {
    6
}

fn default_tool_result_min_prune_bytes() -> usize {
    2048
}

fn default_min_compaction_bytes() -> usize {
    8192
}

/// Context management settings for controlling conversation growth.
///
/// Rewriting conversation history invalidates the provider prompt cache from
/// the first changed byte, so all rewrites are batched into single compaction
/// events triggered when they are forced (context window pressure), free (the
/// cache is already cold), or profitable (expected cache-read savings exceed
/// the one-time rebuild cost).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "Context Management")]
pub struct ContextManagementConfig {
    /// Master toggle for all context management features.
    #[serde(default = "default_enabled")]
    #[schemars(default = "default_enabled")]
    pub enabled: bool,

    /// Run the compaction planner automatically before AI requests.
    #[serde(default = "default_auto_compact")]
    #[schemars(default = "default_auto_compact")]
    pub auto_compact: bool,

    /// Number of most-recent reasoning blocks to retain when compacting.
    #[serde(default = "default_reasoning_prune_retain")]
    #[schemars(default = "default_reasoning_prune_retain")]
    pub reasoning_prune_retain: usize,

    /// Fraction of the model's context window that forces compaction.
    #[serde(default = "default_window_pressure_fraction")]
    #[schemars(default = "default_window_pressure_fraction")]
    pub window_pressure_fraction: f64,

    /// Estimated number of AI requests remaining in the session, used in the
    /// break-even computation: compaction pays off when this many requests of
    /// cache-read savings exceed the one-time cache rebuild cost.
    #[serde(default = "default_expected_remaining_requests")]
    #[schemars(default = "default_expected_remaining_requests")]
    pub expected_remaining_requests: u32,

    /// Provider prompt-cache TTL in seconds. When more time than this has
    /// passed since the last request, the cache is cold and compaction is
    /// free.
    #[serde(default = "default_cache_ttl_seconds")]
    #[schemars(default = "default_cache_ttl_seconds")]
    pub cache_ttl_seconds: u64,

    /// Tool results are only stubbed once at least this many assistant
    /// messages appear after them. The most recent results are always kept.
    #[serde(default = "default_tool_result_keep_recent_turns")]
    #[schemars(default = "default_tool_result_keep_recent_turns")]
    pub tool_result_keep_recent_turns: usize,

    /// Tool results smaller than this are never stubbed.
    #[serde(default = "default_tool_result_min_prune_bytes")]
    #[schemars(default = "default_tool_result_min_prune_bytes")]
    pub tool_result_min_prune_bytes: usize,

    /// Minimum removable bytes before any non-forced compaction runs.
    /// Prevents churn when there is little to reclaim.
    #[serde(default = "default_min_compaction_bytes")]
    #[schemars(default = "default_min_compaction_bytes")]
    pub min_compaction_bytes: usize,
}

impl ContextManagementConfig {
    pub const NAMESPACE: &str = "context_management";
}

impl Default for ContextManagementConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            auto_compact: default_auto_compact(),
            reasoning_prune_retain: default_reasoning_prune_retain(),
            window_pressure_fraction: default_window_pressure_fraction(),
            expected_remaining_requests: default_expected_remaining_requests(),
            cache_ttl_seconds: default_cache_ttl_seconds(),
            tool_result_keep_recent_turns: default_tool_result_keep_recent_turns(),
            tool_result_min_prune_bytes: default_tool_result_min_prune_bytes(),
            min_compaction_bytes: default_min_compaction_bytes(),
        }
    }
}

impl ContextManagementConfig {
    /// Validates configuration values.
    pub fn validate(&self) -> Result<(), String> {
        if !(self.window_pressure_fraction > 0.0 && self.window_pressure_fraction <= 1.0) {
            return Err(format!(
                "window_pressure_fraction ({}) must be in (0, 1]",
                self.window_pressure_fraction
            ));
        }
        Ok(())
    }
}
