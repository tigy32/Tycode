//! Orchestration vocabulary for agent workflows.
//!
//! Agents declare workflow transitions through hooks on the `Agent` trait
//! (`on_complete`, `on_child_complete`). Hooks are pure decision functions of
//! (workflow state, settings, outcome); the chat executor owns conversation
//! forking and stack manipulation. This keeps every orchestration decision
//! unit-testable without a provider or actor.
//!
//! The only workflow in the tree is the coder's forced code review, gated by
//! the `review_level` setting.

use crate::ai::Message;

/// Decision made when an agent calls complete_task, before the pop applies.
pub enum CompletionAction {
    /// Complete normally; the parent's `on_child_complete` runs next.
    Finish,
    /// Intercept completion and push a validator (e.g. a reviewer). The hook
    /// is responsible for parking the completion result in its workflow state
    /// so it can be released when the validator approves.
    Spawn(SpawnSpec),
}

/// Decision made when a child agent completes and pops.
pub enum ChildAction {
    /// Inject a message into this agent's conversation and resume it.
    Resume { message: String },
    /// This agent is finished as well; cascade its completion to its parent.
    /// The completing agent's own `on_complete` hook is not consulted, so a
    /// workflow that decides to finish cannot re-intercept itself.
    Complete { success: bool, result: String },
}

pub struct SpawnSpec {
    /// Catalog name of the agent to push.
    pub agent: String,
    pub task: String,
    pub seed: ConversationSeed,
    /// Preamble injected before the task to orient a forked conversation.
    pub orientation: Option<String>,
}

pub enum ConversationSeed {
    Fresh,
    /// Clone the conversation of the agent returning the action.
    ForkSelf,
}

/// Outcome of a completed child, passed to the parent's `on_child_complete`.
pub struct ChildOutcome {
    pub agent_name: String,
    pub success: bool,
    pub result: String,
    pub conversation: Vec<Message>,
}

/// Per-instance workflow state, stored on `ActiveAgent`. Workflow data lives
/// here rather than on agent types because agents are shared `Arc`s.
#[derive(Debug, Default)]
pub enum WorkflowState {
    #[default]
    None,
    /// A coder awaiting a review verdict for its parked completion.
    Reviewing { rounds: u32, parked_result: String },
    /// A coder addressing rejected review feedback before completing again.
    /// `rounds` counts the rejections so far.
    Fixing { rounds: u32 },
}

pub fn default_child_message(child: &ChildOutcome) -> String {
    format!(
        "Sub-agent completed [success={}]: {}",
        child.success, child.result
    )
}
