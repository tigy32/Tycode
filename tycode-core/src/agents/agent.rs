use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use crate::{
    ai::types::Message,
    file::read_only::FILE_TREE_ID,
    module::{ContextComponentSelection, PromptComponentSelection},
    orchestration::{
        default_child_message, ChildAction, ChildOutcome, CompletionAction, TaskAction,
        WorkflowState,
    },
    settings::config::Settings,
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

    /// Orchestration hook: called when this agent receives its task, before
    /// any AI request. Mechanical orchestrators return `Spawn` and never
    /// converse.
    fn on_task(
        &self,
        _workflow: &mut WorkflowState,
        _settings: &Settings,
        _task: &str,
    ) -> TaskAction {
        TaskAction::Converse
    }

    /// Orchestration hook: called when this agent calls complete_task, before
    /// the pop is applied. Returning `Spawn` intercepts the completion (e.g.
    /// to require review); the hook must park the result in its workflow
    /// state for release when the validator approves.
    fn on_complete(
        &self,
        _workflow: &mut WorkflowState,
        _settings: &Settings,
        _success: bool,
        _result: &str,
    ) -> CompletionAction {
        CompletionAction::Finish
    }

    /// Orchestration hook: called when a child this agent spawned completes.
    fn on_child_complete(
        &self,
        _workflow: &mut WorkflowState,
        _settings: &Settings,
        child: &ChildOutcome,
    ) -> ChildAction {
        ChildAction::Resume {
            message: default_child_message(child),
        }
    }
}

/// Telemetry from this agent's most recent AI request, used by the
/// compaction planner to reason about prompt-cache state and prefix size.
#[derive(Debug, Clone)]
pub struct RequestTelemetry {
    /// Total prompt tokens of the last request: input + cached + cache-write.
    pub prefix_tokens: u64,
    pub completed_at: std::time::SystemTime,
    pub model: crate::ai::model::Model,
}

pub struct ActiveAgent {
    pub agent: Arc<dyn Agent>,
    pub conversation: Vec<Message>,
    pub workflow: WorkflowState,
    /// Files this instance may modify; enforced by the agent runner during
    /// fan-out execution.
    pub write_allowlist: Option<HashSet<PathBuf>>,
    /// Pin this instance to a specific model, overriding name-based
    /// selection. Used by multi-model consensus fan-out.
    pub model_override: Option<crate::ai::ModelSettings>,
    pub last_request: Option<RequestTelemetry>,
}

impl ActiveAgent {
    pub fn new(agent: Arc<dyn Agent>) -> Self {
        Self {
            agent,
            conversation: Vec::new(),
            workflow: WorkflowState::None,
            write_allowlist: None,
            model_override: None,
            last_request: None,
        }
    }
}
