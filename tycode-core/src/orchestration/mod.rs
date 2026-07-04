//! Orchestration vocabulary for mechanical agent workflows.
//!
//! Agents declare workflow transitions through hooks on the `Agent` trait
//! (`on_task`, `on_complete`, `on_child_complete`). Hooks are pure decision
//! functions of (workflow state, settings, outcome); the chat executor owns
//! conversation forking and stack manipulation. This keeps every orchestration
//! decision unit-testable without a provider or actor.

use std::collections::HashSet;
use std::path::PathBuf;

use crate::ai::model::Model;
use crate::ai::Message;

/// Synthetic agent name used for the joined fan-out outcome delivered back to
/// `on_child_complete` after concurrent workers finish.
pub const FANOUT_AGENT: &str = "fanout";

/// Decision made when an agent receives its task, before any AI request.
pub enum TaskAction {
    /// Run the normal conversational AI loop.
    Converse,
    /// Delegate immediately without conversing (mechanical orchestrators).
    Spawn(SpawnSpec),
    /// Run workers concurrently off-stack, then invoke `on_child_complete`
    /// with a synthetic [`FANOUT_AGENT`] outcome. Workers fork this agent's
    /// own conversation.
    FanOut(FanOutSpec),
}

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
    /// Push another agent (the next workflow phase).
    Spawn(SpawnSpec),
    /// Run workers concurrently off-stack, then re-invoke `on_child_complete`
    /// with a synthetic [`FANOUT_AGENT`] outcome carrying the joined report.
    FanOut(FanOutSpec),
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
    /// Pin this instance to a specific model instead of name-based selection.
    pub model: Option<Model>,
}

pub enum ConversationSeed {
    Fresh,
    /// Clone the conversation of the agent returning the action.
    ForkSelf,
    /// Clone the conversation of the child that just completed.
    ForkChild,
}

/// Outcome of a completed child, passed to the parent's `on_child_complete`.
pub struct ChildOutcome {
    pub agent_name: String,
    pub success: bool,
    pub result: String,
    /// The child's full conversation, used by [`ConversationSeed::ForkChild`].
    pub conversation: Vec<Message>,
    /// Structured per-worker results for [`FANOUT_AGENT`] outcomes, so hooks
    /// can inspect individual worker output without parsing the joined
    /// `result` string. Empty for ordinary child completions.
    pub reports: Vec<WorkerResult>,
}

/// One worker's result from a fan-out, in worker order.
#[derive(Debug, Clone)]
pub struct WorkerResult {
    pub label: String,
    pub success: bool,
    pub summary: String,
}

/// A candidate plan in a consensus round: the original per-model plans and
/// any revisions submitted during critique rounds.
#[derive(Debug, Clone)]
pub struct PlanCandidate {
    pub label: String,
    /// The model that authored this candidate; the winning author implements.
    pub author: Option<Model>,
    pub plan: String,
}

pub struct FanOutSpec {
    pub workers: Vec<WorkerSpec>,
}

pub struct WorkerSpec {
    /// Catalog name of the worker agent.
    pub agent: String,
    pub task: String,
    pub orientation: Option<String>,
    pub seed: ConversationSeed,
    /// Files this worker may modify; enforced mechanically by the runner.
    pub write_allowlist: Option<HashSet<PathBuf>>,
    /// Short label for progress reporting.
    pub label: String,
    /// Pair the worker with a reviewer that must approve before it counts
    /// as successful.
    pub reviewed: bool,
    /// Pin this worker to a specific model instead of name-based selection.
    pub model: Option<Model>,
}

/// Per-instance workflow state, stored on `ActiveAgent`. Workflow data lives
/// here rather than on agent types because agents are shared `Arc`s.
#[derive(Debug, Default)]
pub enum WorkflowState {
    #[default]
    None,
    /// A coder awaiting a review verdict for its parked completion.
    Reviewing {
        rounds: u32,
        parked_result: String,
    },
    Builder(BuilderPhase),
    Swarm(SwarmPhase),
}

#[derive(Debug)]
pub enum BuilderPhase {
    Planning,
    Implementing,
    Reviewing { rounds: u32, parked_result: String },
    Fixing { rounds: u32 },
}

#[derive(Debug)]
pub enum SwarmPhase {
    /// Single-model planning on the stack.
    Planning,
    /// Consensus planning: one planner per roster model running off-stack.
    PlanFanOut { models: Vec<Model> },
    /// Consensus rounds: every roster model either endorses one candidate
    /// plan exactly or submits a corrected plan merging the best elements.
    /// Loops until unanimous endorsement or the round cap, then falls back
    /// to plurality.
    Consensus {
        models: Vec<Model>,
        candidates: Vec<PlanCandidate>,
        round: u32,
    },
    /// Degraded to a single coder because the plan was not parallelizable.
    /// A non-empty roster still routes the result through multi-model
    /// integration review.
    Implementing {
        models: Vec<Model>,
        fixer_model: Option<Model>,
    },
    /// Per-file workers running. `model` pins workers to the winning
    /// consensus model; `models` is the roster for integration review
    /// (empty in single-model mode).
    FanOut {
        plan: String,
        model: Option<Model>,
        models: Vec<Model>,
    },
    Integration {
        rounds: u32,
        models: Vec<Model>,
        fixer_model: Option<Model>,
    },
    Fixing {
        rounds: u32,
        models: Vec<Model>,
        fixer_model: Option<Model>,
    },
}

pub fn default_child_message(child: &ChildOutcome) -> String {
    format!(
        "Sub-agent completed [success={}]: {}",
        child.success, child.result
    )
}
