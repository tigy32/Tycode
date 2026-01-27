//! Memory module configuration.
//!
//! This file contains the configuration struct for the memory module.
//! Module configuration should live with the module implementation,
//! not in a centralized settings file.

use crate::ai::model::ModelCost;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn default_memory_cost() -> ModelCost {
    ModelCost::High
}

fn default_context_message_count() -> usize {
    8
}

fn default_recent_memories_count() -> usize {
    16
}

fn default_auto_compaction_threshold() -> Option<usize> {
    Some(16)
}

/// Tycode allows models to store memories which persist between conversations.
/// When enabled, Tycode will also send background requests to models
/// specifically to extract memories from user input, otherwise models may
/// choose to store memories, but generally do not. Memories are appended to a
/// file (in ~/.tycode/memories/memories_log.json) and occasionally compacted
/// in to a memory summary. Memories are injected to prompts so future
/// conversations may benefit from the learnings.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[schemars(title = "Memory")]
pub struct MemoryConfig {
    /// Enable or Disable background calls to extract memories from
    /// conversation context.
    pub enabled: bool,
    /// Cost control for the background model that is used to record memories.
    #[serde(default = "default_memory_cost")]
    #[schemars(default = "default_memory_cost")]
    pub recorder_cost: ModelCost,
    /// Number of recent messages send to the background model; more messages
    /// will improve context for the background model, however will increase
    /// costs.
    #[serde(default = "default_context_message_count")]
    #[schemars(default = "default_context_message_count")]
    pub context_message_count: usize,
    /// Cost control for the model that is used to compact memories.
    #[serde(default = "default_memory_cost")]
    #[schemars(default = "default_memory_cost")]
    pub summarizer_cost: ModelCost,
    /// Number of recent memories to include in the agent's context
    #[serde(default = "default_recent_memories_count")]
    #[schemars(default = "default_recent_memories_count")]
    pub recent_memories_count: usize,
    /// When set, automatically trigger background compaction after this many
    /// new memories since the last compaction.
    #[serde(
        default = "default_auto_compaction_threshold",
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(default = "default_auto_compaction_threshold")]
    pub auto_compaction_threshold: Option<usize>,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            summarizer_cost: default_memory_cost(),
            recorder_cost: default_memory_cost(),
            context_message_count: default_context_message_count(),
            recent_memories_count: default_recent_memories_count(),
            auto_compaction_threshold: default_auto_compaction_threshold(),
        }
    }
}
