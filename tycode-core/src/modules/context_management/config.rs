//! Context management module configuration.
//!
//! Configures when the conversation is summarized to stay inside the model's
//! context window, and how many reasoning blocks `/compact reasoning` keeps.

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

/// Context management settings for controlling conversation growth.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "Context Management")]
pub struct ContextManagementConfig {
    /// Master toggle for all context management features.
    #[serde(default = "default_enabled")]
    #[schemars(default = "default_enabled")]
    pub enabled: bool,

    /// Summarize the conversation automatically when it nears the window.
    #[serde(default = "default_auto_compact")]
    #[schemars(default = "default_auto_compact")]
    pub auto_compact: bool,

    /// Number of most-recent reasoning blocks to retain when compacting.
    #[serde(default = "default_reasoning_prune_retain")]
    #[schemars(default = "default_reasoning_prune_retain")]
    pub reasoning_prune_retain: usize,

    /// Fraction of the model's context window that triggers summarization.
    #[serde(default = "default_window_pressure_fraction")]
    #[schemars(default = "default_window_pressure_fraction")]
    pub window_pressure_fraction: f64,
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
