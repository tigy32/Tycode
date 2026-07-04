//! Structured orchestration events for machine consumption.
//!
//! Consuming UIs (VS Code, Tyde) render sub-agent and workflow progress from
//! these typed events instead of parsing human-readable system messages. Each
//! event is wrapped in `ChatEvent::Orchestration` and travels the same
//! JSON-lines subprocess stream as every other chat event; unknown kinds must
//! be ignored by consumers so the payload set can grow.
//!
//! Emission sources:
//! - Agent lifecycle and phase events come from the chat executor, which owns
//!   the agent stack (`chat/tools.rs`).
//! - Fan-out and worker events come from the fan-out runner itself.
//! - Domain resolutions (consensus rounds, review verdicts, plan selection)
//!   are returned by the `on_child_complete` orchestration hooks through an
//!   event sink, so the data comes from the decision site rather than from
//!   re-parsing rendered text.
//!
//! The human-readable progress strings remain for CLI users; UIs consuming
//! these events can disable them with the `orchestration_progress_messages`
//! setting.
//!
//! Known limitations, called out so consumers do not infer support:
//! - Review/fix rounds inside a fan-out worker pair are not individually
//!   surfaced; the worker's final `WorkerCompleted` summary carries the
//!   review verdict text.
//! - Cancelling a turn drops any in-flight fan-out without terminal worker
//!   events. Consumers should treat `ChatEvent::OperationCancelled` as
//!   aborting every fan-out and worker that has started but not completed.
//!   Agents discarded by an agent switch or session change do get a terminal
//!   `AgentCompleted` with [`OutcomeStatus::Aborted`].

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::ai::model::Model;
use crate::orchestration::PlanCandidate;

/// Stable identifier for an agent instance, fan-out, or worker slot.
/// Consumers must treat it as an opaque string; ids minted by different
/// process runs never collide, so replayed session events and live events
/// can share one tree.
pub type AgentId = String;

static NEXT_SEQ: AtomicU64 = AtomicU64::new(1);
static PROCESS_PREFIX: OnceLock<String> = OnceLock::new();

/// Allocates an orchestration id unique across process restarts: a per-process
/// prefix (start time + pid + random entropy, so even a same-millisecond
/// restart with pid reuse cannot collide) plus a monotonic sequence number.
pub fn next_orchestration_id() -> AgentId {
    let prefix = PROCESS_PREFIX.get_or_init(|| {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0);
        let entropy: u32 = rand::random();
        format!("{millis:x}-{:x}-{entropy:x}", std::process::id())
    });
    format!("{prefix}-{}", NEXT_SEQ.fetch_add(1, Ordering::Relaxed))
}

/// One structured orchestration event, wrapped in `ChatEvent::Orchestration`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrchestrationEvent {
    /// The agent instance this event describes. For fan-out, workflow, and
    /// phase events this is the orchestrating on-stack agent; worker payloads
    /// carry their own worker ids.
    pub agent_id: AgentId,
    /// Catalog name of that agent (e.g. "coder", "swarm").
    pub agent_type: String,
    pub payload: OrchestrationPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum OrchestrationPayload {
    /// An agent joined the interactive stack. The wrapper's
    /// `agent_id`/`agent_type` identify the new agent. Root agents are
    /// announced lazily with [`AgentOrigin::Root`] before their first child
    /// event, so `parent_agent_id` always resolves to an announced agent.
    AgentStarted {
        parent_agent_id: Option<AgentId>,
        /// First line of the agent's task, truncated for display. Full tasks
        /// can embed entire plans and are intentionally not carried on the
        /// event stream.
        task_preview: String,
        origin: AgentOrigin,
        /// Stack depth including this agent; the root agent is depth 1.
        depth: usize,
        /// On-stack agents receive user input when at the top of the stack.
        /// Fan-out workers are announced with worker events instead and are
        /// never interactive.
        interactive: bool,
        /// Model pinned by orchestration; None means the agent's model comes
        /// from per-agent settings at request time.
        model: Option<Model>,
    },
    /// A sub-agent left the stack: popped with its final result, or
    /// [`OutcomeStatus::Aborted`] when discarded by an agent switch or
    /// session change. Root agents never pop, so no AgentCompleted is
    /// emitted at depth 1.
    AgentCompleted {
        status: OutcomeStatus,
        result: String,
    },
    /// The agent's mechanical workflow moved to a new phase.
    PhaseChanged { phase: WorkflowPhase },
    /// Concurrent off-stack workers are launching.
    FanOutStarted {
        fanout_id: AgentId,
        total: usize,
        concurrency: usize,
        workers: Vec<WorkerInfo>,
    },
    /// A worker began: it acquired an execution slot, or failed preflight
    /// validation. Every worker emits WorkerStarted before WorkerCompleted.
    WorkerStarted {
        fanout_id: AgentId,
        worker_id: AgentId,
        label: String,
    },
    WorkerCompleted {
        fanout_id: AgentId,
        worker_id: AgentId,
        label: String,
        status: OutcomeStatus,
        summary: String,
    },
    /// All workers finished; Failed when any worker failed.
    FanOutCompleted {
        fanout_id: AgentId,
        status: OutcomeStatus,
    },
    /// One multi-model consensus tournament round resolved. `verdicts` are
    /// per-judge positions against the round-start candidate set; on an
    /// elimination round `eliminated` names the removed candidate and
    /// `remaining` lists the survivors (revisions already applied).
    ConsensusRoundResolved {
        round: u32,
        verdicts: Vec<PanelVerdict>,
        eliminated: Option<CandidateInfo>,
        remaining: Vec<CandidateInfo>,
    },
    /// Planning finished and implementation is starting. `candidate` is the
    /// winning consensus plan; None when a single planner produced the plan.
    PlanSelected { candidate: Option<CandidateInfo> },
    /// A mechanically forced review round resolved (coder task review,
    /// builder pipeline review, or swarm integration review).
    ReviewRoundResolved {
        round: u32,
        verdict: ReviewVerdict,
        feedback: String,
    },
}

/// How an on-stack sub-agent came to exist.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum AgentOrigin {
    /// A spawn_agent tool call from the parent agent's conversation.
    Tool { tool_call_id: String },
    /// A mechanical workflow transition decided by an orchestration hook.
    Workflow,
    /// An agent that predates structured events — the root of the interactive
    /// stack — announced lazily before its first child event so parent ids
    /// always resolve.
    Root,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutcomeStatus {
    Succeeded,
    Failed,
    /// Terminated without completing: the agent was discarded by an agent
    /// switch, conversation reset, or session change.
    Aborted,
}

impl From<bool> for OutcomeStatus {
    fn from(success: bool) -> Self {
        if success {
            OutcomeStatus::Succeeded
        } else {
            OutcomeStatus::Failed
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReviewVerdict {
    Approved,
    /// Rejected; a fixer round follows.
    Rejected,
    /// Rejected at the round cap; no further rounds run. Coder and builder
    /// reviews accept the parked result with the unresolved feedback
    /// attached; swarm integration review fails the workflow instead.
    RoundLimitReached,
}

/// One fan-out worker slot as announced in FanOutStarted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerInfo {
    pub worker_id: AgentId,
    pub label: String,
    pub agent_type: String,
    pub model: Option<Model>,
    /// Paired with a reviewer that must approve before the worker counts as
    /// successful.
    pub reviewed: bool,
    /// First line of the worker's task, truncated for display. The full task
    /// is intentionally omitted: it can embed entire plans.
    pub task_preview: String,
}

/// A candidate plan in the consensus tournament.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateInfo {
    pub label: String,
    /// The model that authored the candidate; the winning author implements.
    pub author: Option<Model>,
}

impl From<&PlanCandidate> for CandidateInfo {
    fn from(candidate: &PlanCandidate) -> Self {
        Self {
            label: candidate.label.clone(),
            author: candidate.author,
        }
    }
}

/// One panelist's parsed response in a consensus round.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PanelVerdict {
    /// The judge's model seat on the panel.
    pub judge: Option<Model>,
    pub position: PanelPosition,
    /// The candidate this judge voted worst, when parseable.
    pub worst_vote: Option<CandidateInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum PanelPosition {
    /// Endorsed a candidate as correct as-is.
    Endorsed { candidate: CandidateInfo },
    /// Submitted a revised plan replacing its own candidate.
    Revised,
    /// Responded without a parseable endorsement or revision.
    NoPosition,
    /// The judge worker itself failed.
    Failed,
}

/// Typed snapshot of a mechanical workflow phase, emitted as PhaseChanged
/// whenever an orchestration hook moves the workflow to a new phase.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum WorkflowPhase {
    /// A coder awaiting a forced review verdict for its parked completion.
    Reviewing {
        round: u32,
    },
    /// A coder addressing feedback from the rejected review round `round`.
    Fixing {
        round: u32,
    },
    BuilderPlanning,
    BuilderImplementing,
    BuilderReviewing {
        round: u32,
    },
    BuilderFixing {
        round: u32,
    },
    SwarmPlanning,
    SwarmPlanFanOut {
        models: Vec<Model>,
    },
    SwarmConsensus {
        round: u32,
        candidates: Vec<CandidateInfo>,
    },
    SwarmImplementing {
        fixer_model: Option<Model>,
    },
    SwarmFanOut {
        model: Option<Model>,
    },
    SwarmIntegration {
        round: u32,
        models: Vec<Model>,
    },
    SwarmFixing {
        round: u32,
    },
}

/// First line of a task, truncated for event payloads.
pub fn task_preview(task: &str) -> String {
    const MAX_CHARS: usize = 160;
    let first_line = task.lines().next().unwrap_or_default();
    let mut preview: String = first_line.chars().take(MAX_CHARS).collect();
    if first_line.chars().count() > MAX_CHARS || task.lines().nth(1).is_some() {
        preview.push('…');
    }
    preview
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The JSON wire shape is a public contract mirrored in
    /// tycode-client-typescript/src/types.ts; changing it breaks consumers.
    #[test]
    fn payloads_serialize_with_kind_tags() {
        let event = OrchestrationEvent {
            agent_id: "boot-1-7".to_string(),
            agent_type: "swarm".to_string(),
            payload: OrchestrationPayload::WorkerCompleted {
                fanout_id: "boot-1-8".to_string(),
                worker_id: "boot-1-9".to_string(),
                label: "src/a.rs".to_string(),
                status: OutcomeStatus::Succeeded,
                summary: "done".to_string(),
            },
        };
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["agent_id"], "boot-1-7");
        assert_eq!(json["agent_type"], "swarm");
        assert_eq!(json["payload"]["kind"], "WorkerCompleted");
        assert_eq!(json["payload"]["status"], "Succeeded");

        let origin = serde_json::to_value(AgentOrigin::Workflow).unwrap();
        assert_eq!(origin["kind"], "Workflow");

        // Option fields serialize as explicit null on the wire; the TS
        // mirror types them as `T | null`.
        let started = serde_json::to_value(OrchestrationPayload::AgentStarted {
            parent_agent_id: None,
            task_preview: "task".to_string(),
            origin: AgentOrigin::Root,
            depth: 1,
            interactive: true,
            model: None,
        })
        .unwrap();
        assert!(started["parent_agent_id"].is_null());
        assert!(started["model"].is_null());
        assert_eq!(started["origin"]["kind"], "Root");

        let phase = serde_json::to_value(WorkflowPhase::SwarmConsensus {
            round: 2,
            candidates: vec![CandidateInfo {
                label: "plan:1:gpt".to_string(),
                author: Some(Model::Gpt),
            }],
        })
        .unwrap();
        assert_eq!(phase["kind"], "SwarmConsensus");
        assert_eq!(phase["round"], 2);
        assert_eq!(phase["candidates"][0]["label"], "plan:1:gpt");

        let verdict = serde_json::to_value(ReviewVerdict::RoundLimitReached).unwrap();
        assert_eq!(verdict, "RoundLimitReached");

        let roundtrip: OrchestrationEvent =
            serde_json::from_value(serde_json::to_value(&event).unwrap()).unwrap();
        assert_eq!(roundtrip.agent_id, "boot-1-7");
    }

    #[test]
    fn ids_are_unique_and_process_scoped() {
        let first = next_orchestration_id();
        let second = next_orchestration_id();
        assert_ne!(first, second);
        let (first_prefix, _) = first.rsplit_once('-').unwrap();
        let (second_prefix, _) = second.rsplit_once('-').unwrap();
        assert_eq!(
            first_prefix, second_prefix,
            "ids within a process share its prefix"
        );
    }

    #[test]
    fn task_preview_keeps_first_line_only() {
        assert_eq!(task_preview("short task"), "short task");
        assert_eq!(task_preview("line one\nline two"), "line one…");
        let long = "x".repeat(200);
        let preview = task_preview(&long);
        assert_eq!(preview.chars().count(), 161);
        assert!(preview.ends_with('…'));
    }
}
