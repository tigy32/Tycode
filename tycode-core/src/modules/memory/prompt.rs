//! Prompt component for rendering memory compaction in the system prompt.
//!
//! When a compaction exists, renders the compaction summary (compressed history).

use std::sync::Arc;

use crate::module::{PromptComponent, PromptComponentId};
use crate::settings::config::Settings;

use super::compaction::CompactionStore;
use super::log::MemoryLog;

pub const ID: PromptComponentId = PromptComponentId("memory_compaction");

/// Renders memory compaction summary in the system prompt.
pub struct CompactionPromptComponent {
    memory_log: Arc<MemoryLog>,
}

impl CompactionPromptComponent {
    pub fn new(memory_log: Arc<MemoryLog>) -> Self {
        Self { memory_log }
    }
}

impl PromptComponent for CompactionPromptComponent {
    fn id(&self) -> PromptComponentId {
        ID
    }

    fn build_prompt_section(&self, _settings: &Settings) -> Option<String> {
        let memory_dir = self.memory_log.path().parent()?;
        let store = CompactionStore::new(memory_dir.to_path_buf());

        let compaction = match store.find_latest() {
            Ok(Some(c)) => c,
            Ok(None) => return None,
            Err(e) => {
                tracing::warn!("Failed to load compaction: {e:?}");
                return None;
            }
        };

        Some(format!(
            "## Memories from Previous Conversations\n\n{}",
            compaction.summary
        ))
    }
}
