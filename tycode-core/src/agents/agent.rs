use std::sync::Arc;

use crate::{
    ai::types::Message, context::ContextComponentSelection, module::PromptComponentSelection,
    tools::ToolName,
};

pub trait Agent: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn core_prompt(&self) -> &'static str;
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
    /// Context messages (aka "continuous steering") is a feature where the
    /// last message to the agent always gives a big blob of up to date
    /// context. For example, the context may have a list of files in the
    /// project, some file contents, some memories, a task list, etc. All of
    /// these are updated on each request to the AI to ensure the context is
    /// always fresh. Stale versions of files, outdated task lists, etc never
    /// appear in our agents context.
    ///
    /// Generally all context is helpful for agents, however if an agent needs
    /// more fine grain control they may opt out of or control which context
    /// components are included.
    fn requested_context_components(&self) -> ContextComponentSelection {
        ContextComponentSelection::All
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
