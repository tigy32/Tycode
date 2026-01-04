//! Memory module - self-contained memory management functionality.
//!
//! Provides persistent memory storage, context rendering, and the append_memory tool.

use std::sync::Arc;

use crate::context::ContextComponent;
use crate::module::Module;
use crate::prompt::PromptComponent;
use crate::settings::manager::SettingsManager;
use crate::tools::r#trait::ToolExecutor;

pub mod background;
pub mod context;
pub mod log;
pub mod tool;

use context::MemoriesManager;
use log::MemoryLog;
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
        vec![]
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
