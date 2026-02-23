//! Context management module configuration.
//!
//! This file contains the configuration for managing conversation context,
//! including reasoning block pruning to control context window growth.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn default_reasoning_prune_trigger() -> usize {
    16
}

fn default_reasoning_prune_retain() -> usize {
    8
}

fn default_enabled() -> bool {
    true
}

fn default_auto_compact_reasoning() -> bool {
    false
}

/// Context management settings for controlling conversation growth.
///
/// Reasoning blocks accumulate over long conversations and can consume
/// significant context window. These settings use a hysteresis approach
/// to prune reasoning blocks in batches, preserving prompt caching.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "Context Management")]
pub struct ContextManagementConfig {
    /// Master toggle for all context management features.
    #[serde(default = "default_enabled")]
    #[schemars(default = "default_enabled")]
    pub enabled: bool,
    /// Enable automatic reasoning compaction during request preparation.
    #[serde(default = "default_auto_compact_reasoning")]
    #[schemars(default = "default_auto_compact_reasoning")]
    pub auto_compact_reasoning: bool,
    /// Number of reasoning blocks that triggers pruning.
    /// When the count reaches this threshold, oldest blocks are removed.
    #[serde(default = "default_reasoning_prune_trigger")]
    #[schemars(default = "default_reasoning_prune_trigger")]
    pub reasoning_prune_trigger: usize,
    /// Number of reasoning blocks to retain after pruning.
    /// Must be less than reasoning_prune_trigger.
    #[serde(default = "default_reasoning_prune_retain")]
    #[schemars(default = "default_reasoning_prune_retain")]
    pub reasoning_prune_retain: usize,
}

impl ContextManagementConfig {
    pub const NAMESPACE: &str = "context_management";
}

impl Default for ContextManagementConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            auto_compact_reasoning: default_auto_compact_reasoning(),
            reasoning_prune_trigger: default_reasoning_prune_trigger(),
            reasoning_prune_retain: default_reasoning_prune_retain(),
        }
    }
}

impl ContextManagementConfig {
    /// Validates configuration values.
    /// Returns an error if retain count is not less than trigger count.
    pub fn validate(&self) -> Result<(), String> {
        if self.reasoning_prune_retain >= self.reasoning_prune_trigger {
            return Err(format!(
                "reasoning_prune_retain ({}) must be less than reasoning_prune_trigger ({})",
                self.reasoning_prune_retain, self.reasoning_prune_trigger
            ));
        }
        Ok(())
    }
}
