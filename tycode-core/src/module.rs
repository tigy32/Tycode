use std::sync::Arc;

use anyhow::Result;
use schemars::schema::RootSchema;
use serde_json::Value;

use crate::chat::actor::ActorState;
use crate::chat::events::ChatMessage;
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

/// A slash command that can be provided by a module.
/// Modules implement this trait for commands they want to register.
#[async_trait::async_trait(?Send)]
pub trait SlashCommand: Send + Sync {
    /// The command name without the leading slash (e.g., "memory" for /memory)
    fn name(&self) -> &'static str;

    /// Short description shown in help
    fn description(&self) -> &'static str;

    /// Usage example shown in help (e.g., "/memory summarize")
    fn usage(&self) -> &'static str;

    /// Whether to hide this command from /help output
    fn hidden(&self) -> bool {
        false
    }

    /// Execute the command with the given arguments
    async fn execute(&self, state: &mut ActorState, args: &[&str]) -> Vec<ChatMessage>;
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

    /// Returns slash commands provided by this module.
    /// Default implementation returns an empty vec (no commands).
    fn slash_commands(&self) -> Vec<Arc<dyn SlashCommand>> {
        vec![]
    }

    /// Option allows modules without configuration to opt-out, avoiding empty entries.
    fn settings_namespace(&self) -> Option<&'static str> {
        None
    }

    /// Returns JSON Schema for this module's settings configuration.
    /// Used for auto-generating settings UI.
    fn settings_json_schema(&self) -> Option<RootSchema> {
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

// === Context Components ===

/// Strongly-typed identifier for context components.
/// Using a wrapper type prevents accidental hardcoding of strings
/// and ensures compile-time checking of component references.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContextComponentId(pub &'static str);

/// Selection strategy for which context components an agent wants included.
///
/// Context components contribute to "continuous steering" - a feature where
/// the last message to the agent always contains fresh, up-to-date context.
/// Examples include:
/// - File tree listing (project structure)
/// - Tracked file contents (full source code of relevant files)
/// - Memory log (user preferences, past corrections)
/// - Task list (current work items and status)
///
/// All context is refreshed on each request, ensuring the agent never sees
/// stale file contents, outdated task lists, etc.
///
/// Most agents benefit from all context, but specialized agents may want
/// fine-grained control. For example, an agent focused on a specific task
/// might exclude irrelevant context to reduce noise.
#[derive(Clone, Copy, Debug)]
pub enum ContextComponentSelection {
    /// Include all available context components
    All,
    /// Include only the specified context components
    Only(&'static [ContextComponentId]),
    /// Include all except the specified context components
    Exclude(&'static [ContextComponentId]),
    /// Exclude all context components
    None,
}

/// A composable unit that contributes to the context message.
/// Implementations provide specific sections of context content.
/// Components should be self-contained - owning any state they need.
#[async_trait::async_trait(?Send)]
pub trait ContextComponent: Send + Sync {
    /// Returns the unique identifier for this component.
    /// This ID is used for filtering via ContextComponentSelection.
    fn id(&self) -> ContextComponentId;

    /// Returns the context section content, or None if this component
    /// should not contribute to the current context.
    async fn build_context_section(&self) -> Option<String>;
}

/// Encapsulates context component management and builds combined context sections.
#[derive(Clone)]
pub struct ContextBuilder {
    components: Vec<Arc<dyn ContextComponent>>,
}

impl ContextBuilder {
    pub fn new() -> Self {
        Self {
            components: Vec::new(),
        }
    }

    pub fn add(&mut self, component: Arc<dyn ContextComponent>) {
        self.components.push(component);
    }

    /// Builds context sections filtered by the given selection, including components from modules.
    pub async fn build(
        &self,
        selection: &ContextComponentSelection,
        modules: &[Arc<dyn Module>],
    ) -> String {
        let module_components: Vec<Arc<dyn ContextComponent>> = modules
            .iter()
            .flat_map(|m| m.context_components())
            .collect();

        let all_components: Vec<&Arc<dyn ContextComponent>> = self
            .components
            .iter()
            .chain(module_components.iter())
            .collect();

        if all_components.is_empty() {
            return String::new();
        }

        let filtered: Vec<_> = all_components
            .iter()
            .filter(|c| match selection {
                ContextComponentSelection::All => true,
                ContextComponentSelection::Only(ids) => ids.contains(&c.id()),
                ContextComponentSelection::Exclude(ids) => !ids.contains(&c.id()),
                ContextComponentSelection::None => false,
            })
            .collect();

        let mut sections = Vec::new();
        for component in filtered {
            if let Some(section) = component.build_context_section().await {
                sections.push(section);
            }
        }

        if sections.is_empty() {
            String::new()
        } else {
            format!("\n\n{}", sections.join("\n"))
        }
    }
}

impl Default for ContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}
