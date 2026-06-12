use std::sync::Arc;

use crate::{
    ai::types::Message,
    file::read_only::FILE_TREE_ID,
    module::{ContextComponentSelection, PromptComponentSelection},
    tools::ToolName,
};

const DEFAULT_EXCLUDED_CONTEXT: &[crate::module::ContextComponentId] = &[FILE_TREE_ID];

pub trait Agent: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn core_prompt(&self) -> &str;
    fn available_tools(&self) -> Vec<ToolName>;

    /// If there are prompt components installed, this instructs which
    /// components should be included when building prompts for this agent.
    ///
    /// Prompt components are a way to define generic, reusable, slices of the
    /// prompt which are reused by many agents, for example, how to talk to the
    /// user, how to use your tools. Generally opting in to all prompt
    /// components is a safe default, however some agents may wish to disable
    /// or control prompt more.
    fn requested_prompt_components(&self) -> PromptComponentSelection {
        PromptComponentSelection::All
    }

    /// If there are context components, this instructs the `ContextBuilder` on
    /// which components should be included when building contexts for this
    /// agent.
    ///
    /// Context messages provide small, fresh runtime state such as task lists,
    /// memory, recent command output, and user-pinned files. Generated file
    /// trees are excluded by default because codebase discovery should happen
    /// through normal shell commands instead of being injected on every request.
    ///
    /// Generally all context is helpful for agents, however if an agent needs
    /// more fine grain control they may opt out of or control which context
    /// components are included.
    fn requested_context_components(&self) -> ContextComponentSelection {
        ContextComponentSelection::Exclude(DEFAULT_EXCLUDED_CONTEXT)
    }

    fn requires_tool_use(&self) -> bool {
        false
    }
}

pub struct ActiveAgent {
    pub agent: Arc<dyn Agent>,
    pub conversation: Vec<Message>,
    pub completion_result: Option<String>,
}

impl ActiveAgent {
    pub fn new(agent: Arc<dyn Agent>) -> Self {
        Self {
            agent,
            conversation: Vec::new(),
            completion_result: None,
        }
    }
}
