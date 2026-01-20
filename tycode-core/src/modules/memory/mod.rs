//! Memory module - self-contained memory management functionality.
//!
//! Provides persistent memory storage, context rendering, and the append_memory tool.

use std::sync::Arc;

use crate::module::ContextComponent;
use crate::module::Module;
use crate::module::PromptComponent;
use crate::settings::manager::SettingsManager;
use crate::tools::r#trait::ToolExecutor;

pub mod background;
pub mod compaction;
pub mod context;
pub mod log;
pub mod prompt;
pub mod tool;

use context::MemoriesManager;
use log::MemoryLog;
use prompt::CompactionPromptComponent;
use tool::AppendMemoryTool;

/// Memory module providing persistent memory storage and retrieval.
///
/// Bundles:
/// - Context: MemoriesManager (renders recent memories)
/// - Tool: AppendMemoryTool (stores new memories)
pub struct MemoryModule {
    memory_log: Arc<MemoryLog>,
    settings: SettingsManager,
}

impl MemoryModule {
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

impl Module for MemoryModule {
    fn prompt_components(&self) -> Vec<Arc<dyn PromptComponent>> {
        vec![Arc::new(CompactionPromptComponent::new(
            self.memory_log.clone(),
        ))]
    }

    fn context_components(&self) -> Vec<Arc<dyn ContextComponent>> {
        vec![Arc::new(MemoriesManager::new(
            self.memory_log.clone(),
            self.settings.clone(),
        ))]
    }

    fn tools(&self) -> Vec<Arc<dyn ToolExecutor>> {
        vec![Arc::new(AppendMemoryTool::new(self.memory_log.clone()))]
    }
}
