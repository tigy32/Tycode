use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;

use crate::context::ContextComponent;
use crate::prompt::PromptComponent;
use crate::tools::r#trait::ToolExecutor;

/// Handles session persistence for a module's state.
///
/// Modules that need to persist state across sessions should return
/// an implementation of this trait from `Module::session_state()`.
pub trait SessionStateComponent: Send + Sync {
    /// Unique key for storing this module's state in session data.
    fn key(&self) -> &str;

    /// Serialize current state for persistence.
    fn save(&self) -> Value;

    /// Restore state from persisted session data.
    fn load(&self, state: Value) -> Result<()>;
}

/// A Module bundles related prompt components, context components, and tools.
///
/// Modules represent cohesive functionality that spans multiple systems:
/// - Prompts: Instructions for how the agent should behave
/// - Context: Runtime state included in each request
/// - Tools: Actions the agent can take
///
/// Example: TaskListModule provides task tracking across all three:
/// - Prompt instructions for managing tasks
/// - Context showing current task status
/// - Tools to create and update tasks
pub trait Module: Send + Sync {
    fn prompt_components(&self) -> Vec<Arc<dyn PromptComponent>>;
    fn context_components(&self) -> Vec<Arc<dyn ContextComponent>>;
    fn tools(&self) -> Vec<Arc<dyn ToolExecutor>>;

    /// Returns a session state component if this module has persistent state.
    /// Return None if this module has no state to persist across sessions.
    fn session_state(&self) -> Option<Arc<dyn SessionStateComponent>> {
        None
    }
}
