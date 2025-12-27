pub mod autonomy;
pub mod communication;
pub mod style;
pub mod tools;

use crate::settings::config::Settings;
use std::sync::Arc;

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

    /// Builds combined prompt sections from all components.
    /// Returns empty string if no components produce content.
    pub fn build(&self, settings: &Settings) -> String {
        self.build_filtered(settings, &PromptComponentSelection::All)
    }

    /// Builds prompt sections filtered by the given selection.
    pub fn build_filtered(
        &self,
        settings: &Settings,
        selection: &PromptComponentSelection,
    ) -> String {
        if self.components.is_empty() {
            return String::new();
        }

        let sections: Vec<String> = self
            .components
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
