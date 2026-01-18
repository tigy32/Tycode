use std::sync::Arc;

use crate::module::Module;

pub mod command_outputs;

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
