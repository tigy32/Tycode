//! Memory context component for rendering memories in AI context.

use std::sync::Arc;

use crate::context::{ContextComponent, ContextComponentId};
use crate::settings::manager::SettingsManager;

use super::log::MemoryLog;

pub const ID: ContextComponentId = ContextComponentId("memories");

/// Renders recent memories in the context section.
pub struct MemoriesManager {
    memory_log: Arc<MemoryLog>,
    settings: SettingsManager,
}

impl MemoriesManager {
    pub fn new(memory_log: Arc<MemoryLog>, settings: SettingsManager) -> Self {
        Self {
            memory_log,
            settings,
        }
    }

    pub fn memory_log(&self) -> &Arc<MemoryLog> {
        &self.memory_log
    }
}

#[async_trait::async_trait(?Send)]
impl ContextComponent for MemoriesManager {
    fn id(&self) -> ContextComponentId {
        ID
    }

    async fn build_context_section(&self) -> Option<String> {
        let memories = self.memory_log.read_all().ok()?;
        if memories.is_empty() {
            return None;
        }

        let max_recent = self.settings.settings().memory.recent_memories_count;
        let recent: Vec<_> = memories.iter().rev().take(max_recent).collect();

        if recent.is_empty() {
            return None;
        }

        let mut output = String::from("Recent Memories:\n");
        for memory in recent.iter().rev() {
            let source_info = memory
                .source
                .as_ref()
                .map(|s| format!(" [{}]", s))
                .unwrap_or_default();
            output.push_str(&format!("- {}{}", memory.content, source_info));
            output.push('\n');
        }
        Some(output)
    }
}
