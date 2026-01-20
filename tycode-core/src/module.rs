use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;

use crate::context::ContextComponent;
use crate::settings::config::Settings;
use crate::tools::r#trait::ToolExecutor;

/// Strongly-typed identifier for prompt components.
/// Using a wrapper type prevents accidental hardcoding of strings
/// and ensures compile-time checking of component references.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PromptComponentId(pub &'static str);

/// Selection strategy for which prompt components an agent wants included.
///
/// Prompt components contribute to the system prompt - the initial instructions
/// given to the AI that shape its behavior. Examples include style mandates,
/// autonomy level instructions, tool usage guidelines, etc.
///
/// Most agents want all components, but specialized agents (like memory manager)
/// may want to exclude certain components (like autonomy instructions) because
/// they have their own bespoke prompts.
#[derive(Clone, Copy, Debug)]
pub enum PromptComponentSelection {
    /// Include all available prompt components
    All,
    /// Include only the specified prompt components
    Only(&'static [PromptComponentId]),
    /// Include all except the specified prompt components
    Exclude(&'static [PromptComponentId]),
    /// Exclude all prompt components (agent has its own complete prompt)
    None,
}

/// A composable unit that contributes to the system prompt.
/// Implementations provide specific sections of prompt content.
pub trait PromptComponent: Send + Sync {
    /// Returns the unique identifier for this component.
    /// This ID is used for filtering via PromptComponentSelection.
    fn id(&self) -> PromptComponentId;

    /// Returns the prompt section content, or None if this component
    /// should not contribute to the current prompt.
    fn build_prompt_section(&self, settings: &Settings) -> Option<String>;
}

// === Session State ===

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

/// Encapsulates prompt component management and builds the combined prompt.
#[derive(Clone)]
pub struct PromptBuilder {
    components: Vec<Arc<dyn PromptComponent>>,
}

impl PromptBuilder {
    pub fn new() -> Self {
        Self {
            components: Vec::new(),
        }
    }

    pub fn add(&mut self, component: Arc<dyn PromptComponent>) {
        self.components.push(component);
    }

    /// Builds prompt sections filtered by the given selection, including components from modules.
    pub fn build(
        &self,
        settings: &Settings,
        selection: &PromptComponentSelection,
        modules: &[Arc<dyn Module>],
    ) -> String {
        let module_components: Vec<Arc<dyn PromptComponent>> =
            modules.iter().flat_map(|m| m.prompt_components()).collect();

        let all_components: Vec<&Arc<dyn PromptComponent>> = self
            .components
            .iter()
            .chain(module_components.iter())
            .collect();

        if all_components.is_empty() {
            return String::new();
        }

        let sections: Vec<String> = all_components
            .iter()
            .filter(|c| match selection {
                PromptComponentSelection::All => true,
                PromptComponentSelection::Only(ids) => ids.contains(&c.id()),
                PromptComponentSelection::Exclude(ids) => !ids.contains(&c.id()),
                PromptComponentSelection::None => false,
            })
            .filter_map(|c| c.build_prompt_section(settings))
            .collect();

        if sections.is_empty() {
            String::new()
        } else {
            format!("\n\n{}", sections.join("\n\n"))
        }
    }
}

impl Default for PromptBuilder {
    fn default() -> Self {
        Self::new()
    }
}
