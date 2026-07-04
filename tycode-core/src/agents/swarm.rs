use std::collections::HashSet;
use std::path::PathBuf;

use serde::Deserialize;

use crate::agents::agent::Agent;
use crate::agents::code_review::CodeReviewAgent;
use crate::agents::coder::CoderAgent;
use crate::agents::file_impl::FileImplAgent;
use crate::agents::plan_judge::PlanJudgeAgent;
use crate::agents::planner::PlannerAgent;
use crate::ai::model::Model;
use crate::orchestration::{
    default_child_message, ChildAction, ChildOutcome, ConversationSeed, FanOutSpec, PlanCandidate,
    SpawnSpec, SwarmPhase, TaskAction, WorkerSpec, WorkflowState,
};
use crate::settings::config::Settings;
use crate::spawn::complete_task::CompleteTask;
use crate::tools::ToolName;

const CORE_PROMPT: &str = "You are a mechanical orchestration agent that should never converse. \
If you are reading this, a workflow transition failed; use complete_task with success=false to \
report the orchestration error.";

const PLAN_ASSIGNMENT_INSTRUCTIONS: &str = r#"After the plan, append a fenced ```json code block assigning the work to per-file workers, in this exact shape:

```json
{
  "assignments": [
    {
      "file": "path/to/file.rs",
      "instructions": "Complete, self-contained instructions for all changes to this file.",
      "shared_surfaces": ["exact signatures/types this file shares with other assignments"]
    }
  ]
}
```

Rules for assignments:
- One entry per file that must change; every file edit in the plan must appear in exactly one assignment.
- Workers implement concurrently and can only see the plan, not each other's edits. Any interface two files share (struct fields, function signatures, error types) MUST be written out exactly in shared_surfaces of every involved assignment.
- If the change is too entangled to split safely, or touches only one file, emit an empty assignments array and the work will be implemented sequentially instead."#;

const WORKER_ORIENTATION: &str = "\
    --- AGENT TRANSITION ---\n\
    You are a file implementation worker. The conversation above is from the planning agent; \
    it contains the full plan and the codebase research behind it. Your assignment follows.";

const INTEGRATION_ORIENTATION_TASK: &str = "\
    All per-file workers have finished. Verify the integrated result: \
    first run the build and tests with bash, then inspect the full diff and review the \
    change as a whole for cross-file consistency (interface mismatches, missed call \
    sites, convention drift). Approve with complete_task success=true, or reject with \
    success=false and concrete fix instructions.";

#[derive(Deserialize)]
struct AssignmentList {
    assignments: Vec<Assignment>,
}

#[derive(Deserialize)]
struct Assignment {
    file: String,
    instructions: String,
    #[serde(default)]
    shared_surfaces: Vec<String>,
}

/// Parses the trailing fenced JSON assignment block from a plan. Returns None
/// when there is no parseable block, which degrades the swarm to sequential
/// implementation.
fn parse_assignments(plan: &str) -> Option<Vec<Assignment>> {
    let fence_start = plan.rfind("```json")?;
    let body = &plan[fence_start + "```json".len()..];
    let fence_end = body.find("```")?;
    let list: AssignmentList = serde_json::from_str(body[..fence_end].trim()).ok()?;
    Some(list.assignments)
}

fn plan_task(task: &str) -> String {
    format!(
        "Produce an execution plan for the following task:\n{task}\n\n{PLAN_ASSIGNMENT_INSTRUCTIONS}"
    )
}

fn plan_label(index: usize, model: Model) -> String {
    format!("plan:{}:{}", index + 1, model.name())
}

/// A panelist's parsed position for one consensus round.
enum Position {
    Endorse(usize),
    Revision(String),
}

struct PanelResponse {
    position: Option<Position>,
    worst: Option<usize>,
}

fn candidate_index(candidates: &[PlanCandidate], label: &str) -> Option<usize> {
    candidates
        .iter()
        .position(|candidate| candidate.label == label)
        .or_else(|| {
            candidates
                .iter()
                .position(|candidate| label.contains(&candidate.label))
        })
}

/// Parses a panelist's response: an `APPROVE: <label>` endorsement or a full
/// revised plan, plus the mandatory trailing `WORST: <label>` vote. All labels
/// resolve against the round-start candidate set.
fn parse_panel_response(summary: &str, candidates: &[PlanCandidate]) -> PanelResponse {
    let mut worst = None;
    let mut body_lines: Vec<&str> = Vec::new();
    for line in summary.lines() {
        if let Some(rest) = line.trim().strip_prefix("WORST:") {
            worst = candidate_index(candidates, rest.trim());
        } else {
            body_lines.push(line);
        }
    }

    let body = body_lines.join("\n");
    let trimmed = body.trim();

    let position = if let Some(rest) = trimmed.strip_prefix("APPROVE:") {
        let label = rest.trim().lines().next().unwrap_or_default().trim();
        candidate_index(candidates, label).map(Position::Endorse)
    } else if let Some(index) = candidates
        .iter()
        .position(|candidate| candidate.label == trimmed)
    {
        Some(Position::Endorse(index))
    } else if trimmed.is_empty() {
        None
    } else {
        Some(Position::Revision(trimmed.to_string()))
    };

    PanelResponse { position, worst }
}

fn consensus_round_workers(
    panel: &[Model],
    candidates: &[PlanCandidate],
    round: u32,
) -> Vec<WorkerSpec> {
    let doc = candidates
        .iter()
        .map(|candidate| format!("### {}\n{}", candidate.label, candidate.plan))
        .collect::<Vec<_>>()
        .join("\n\n");
    let task = format!(
        "Consensus round {round}: {} candidate plan(s) remain. \
        Respond with `APPROVE: <label>` if one candidate is correct as-is, or with a full \
        revised plan (replacing your own candidate) ending in the fenced json assignments \
        block. Always finish with a final line `WORST: <label>`; the plan with the most \
        WORST votes is eliminated this round along with its author's panel seat.\n\n\
        Candidates:\n\n{doc}",
        candidates.len()
    );

    panel
        .iter()
        .map(|model| WorkerSpec {
            agent: PlanJudgeAgent::NAME.to_string(),
            task: task.clone(),
            orientation: None,
            seed: ConversationSeed::Fresh,
            write_allowlist: None,
            label: format!("consensus:r{round}:{}", model.name()),
            reviewed: false,
            model: Some(*model),
        })
        .collect()
}

fn build_file_workers(assignments: Vec<Assignment>, model: Option<Model>) -> Vec<WorkerSpec> {
    assignments
        .into_iter()
        .map(|assignment| {
            let shared = if assignment.shared_surfaces.is_empty() {
                String::from("none")
            } else {
                assignment.shared_surfaces.join("\n")
            };
            WorkerSpec {
                agent: FileImplAgent::NAME.to_string(),
                task: format!(
                    "Your assignment: implement all planned changes to `{}`.\n\n\
                    Instructions:\n{}\n\n\
                    Shared surfaces (must match EXACTLY as written):\n{}\n\n\
                    Modify ONLY `{}`. Use complete_task when done.",
                    assignment.file, assignment.instructions, shared, assignment.file
                ),
                orientation: Some(WORKER_ORIENTATION.to_string()),
                seed: ConversationSeed::ForkChild,
                write_allowlist: Some(HashSet::from([PathBuf::from(&assignment.file)])),
                label: assignment.file,
                reviewed: true,
                model,
            }
        })
        .collect()
}

fn integration_review_workers(models: &[Model], task: &str) -> Vec<WorkerSpec> {
    models
        .iter()
        .map(|model| WorkerSpec {
            agent: CodeReviewAgent::NAME.to_string(),
            task: task.to_string(),
            orientation: None,
            seed: ConversationSeed::Fresh,
            write_allowlist: None,
            label: format!("review:{}", model.name()),
            reviewed: false,
            model: Some(*model),
        })
        .collect()
}

fn spawn_coder(task: String, model: Option<Model>) -> ChildAction {
    ChildAction::Spawn(SpawnSpec {
        agent: CoderAgent::NAME.to_string(),
        task,
        seed: ConversationSeed::Fresh,
        orientation: None,
        model,
    })
}

/// Route a winning plan into implementation: fan out per-file workers pinned
/// to the winning model, or degrade to a single coder when the plan is not
/// parallelizable. `models` is the consensus roster (empty in single-model
/// mode) and controls whether integration review is multi-model.
fn advance_with_plan(
    workflow: &mut WorkflowState,
    plan: String,
    winner: Option<Model>,
    models: Vec<Model>,
) -> ChildAction {
    let assignments = parse_assignments(&plan).unwrap_or_default();
    if assignments.len() < 2 {
        let task = format!("Implement the following plan exactly as specified.\n\n{plan}");
        *workflow = WorkflowState::Swarm(SwarmPhase::Implementing {
            models,
            fixer_model: winner,
        });
        return spawn_coder(task, winner);
    }

    let workers = build_file_workers(assignments, winner);
    *workflow = WorkflowState::Swarm(SwarmPhase::FanOut {
        plan,
        model: winner,
        models,
    });
    ChildAction::FanOut(FanOutSpec { workers })
}

/// Enter the integration review phase: a fan-out over every roster model, or
/// a single stack review when the roster is empty.
fn start_integration_review(
    workflow: &mut WorkflowState,
    rounds: u32,
    models: Vec<Model>,
    fixer_model: Option<Model>,
    task: String,
) -> ChildAction {
    let action = if models.len() >= 2 {
        ChildAction::FanOut(FanOutSpec {
            workers: integration_review_workers(&models, &task),
        })
    } else {
        ChildAction::Spawn(SpawnSpec {
            agent: CodeReviewAgent::NAME.to_string(),
            task,
            seed: ConversationSeed::Fresh,
            orientation: None,
            model: None,
        })
    };
    *workflow = WorkflowState::Swarm(SwarmPhase::Integration {
        rounds,
        models,
        fixer_model,
    });
    action
}

/// Plan → concurrent per-file implementation → integration review pipeline.
/// The planner decomposes the task into per-file assignments with explicit
/// shared-surface contracts; workers fork the planner's conversation so the
/// full research context transfers without re-distilled instructions.
/// Degrades to a single sequential coder when the plan is not parallelizable.
///
/// With two or more models in `swarm_models`, planning becomes an elimination
/// tournament: every model plans, then each round every surviving panelist
/// either approves one candidate or submits a revision merging the best
/// elements, and always votes for the worst candidate — which is eliminated
/// together with its author's seat. Unanimous approval (or a single survivor)
/// decides the plan; its author's model implements, and integration review
/// requires approval from every roster model.
pub struct SwarmAgent;

impl SwarmAgent {
    pub const NAME: &'static str = "swarm";
}

impl Agent for SwarmAgent {
    fn name(&self) -> &str {
        Self::NAME
    }

    fn description(&self) -> &str {
        "Plans a change, implements per-file assignments concurrently, then integration-reviews; best for wide multi-file changes"
    }

    fn core_prompt(&self) -> &'static str {
        CORE_PROMPT
    }

    fn available_tools(&self) -> Vec<ToolName> {
        vec![CompleteTask::tool_name()]
    }

    fn requires_tool_use(&self) -> bool {
        true
    }

    fn on_task(&self, workflow: &mut WorkflowState, settings: &Settings, task: &str) -> TaskAction {
        let roster = settings.swarm_models.clone();
        if roster.len() < 2 {
            *workflow = WorkflowState::Swarm(SwarmPhase::Planning);
            return TaskAction::Spawn(SpawnSpec {
                agent: PlannerAgent::NAME.to_string(),
                task: plan_task(task),
                seed: ConversationSeed::ForkSelf,
                orientation: None,
                model: None,
            });
        }

        let workers = roster
            .iter()
            .enumerate()
            .map(|(index, model)| WorkerSpec {
                agent: PlannerAgent::NAME.to_string(),
                task: plan_task(task),
                orientation: None,
                seed: ConversationSeed::ForkSelf,
                write_allowlist: None,
                label: plan_label(index, *model),
                reviewed: false,
                model: Some(*model),
            })
            .collect();

        *workflow = WorkflowState::Swarm(SwarmPhase::PlanFanOut { models: roster });
        TaskAction::FanOut(FanOutSpec { workers })
    }

    fn on_child_complete(
        &self,
        workflow: &mut WorkflowState,
        settings: &Settings,
        child: &ChildOutcome,
    ) -> ChildAction {
        let WorkflowState::Swarm(phase) = workflow else {
            return ChildAction::Resume {
                message: default_child_message(child),
            };
        };

        match phase {
            SwarmPhase::Planning => {
                if !child.success {
                    return ChildAction::Complete {
                        success: false,
                        result: format!("Planning failed: {}", child.result),
                    };
                }
                advance_with_plan(workflow, child.result.clone(), None, Vec::new())
            }
            SwarmPhase::PlanFanOut { models } => {
                let models = models.clone();
                let mut panel: Vec<Model> = Vec::new();
                let mut candidates: Vec<PlanCandidate> = Vec::new();
                for (index, report) in child.reports.iter().enumerate() {
                    if !report.success {
                        continue;
                    }
                    let Some(model) = models.get(index) else {
                        continue;
                    };
                    panel.push(*model);
                    candidates.push(PlanCandidate {
                        label: report.label.clone(),
                        author: Some(*model),
                        plan: report.summary.clone(),
                    });
                }

                match candidates.len() {
                    0 => ChildAction::Complete {
                        success: false,
                        result: format!("All consensus planners failed:\n{}", child.result),
                    },
                    1 => {
                        let winner = candidates.remove(0);
                        advance_with_plan(
                            workflow,
                            winner.plan,
                            winner.author,
                            settings.swarm_models.clone(),
                        )
                    }
                    _ => {
                        let workers = consensus_round_workers(&panel, &candidates, 1);
                        *workflow = WorkflowState::Swarm(SwarmPhase::Consensus {
                            models: panel,
                            candidates,
                            round: 1,
                        });
                        ChildAction::FanOut(FanOutSpec { workers })
                    }
                }
            }
            SwarmPhase::Consensus {
                models,
                candidates,
                round,
            } => {
                let mut panel = models.clone();
                let mut candidates = candidates.clone();
                let round = *round;

                // Parse every panelist's response against the round-start
                // candidate set so labels stay stable within the round.
                let mut endorsements: Vec<usize> = Vec::new();
                let mut revisions: Vec<(usize, String)> = Vec::new();
                let mut worst_votes = vec![0usize; candidates.len()];
                for (index, report) in child.reports.iter().enumerate() {
                    if !report.success {
                        continue;
                    }
                    let response = parse_panel_response(&report.summary, &candidates);
                    match response.position {
                        Some(Position::Endorse(candidate)) => endorsements.push(candidate),
                        Some(Position::Revision(plan)) => revisions.push((index, plan)),
                        None => {}
                    }
                    if let Some(candidate) = response.worst {
                        worst_votes[candidate] += 1;
                    }
                }

                // Unanimous approval among respondents ends the tournament.
                if revisions.is_empty() {
                    if let Some((&first, rest)) = endorsements.split_first() {
                        if rest.iter().all(|&candidate| candidate == first) {
                            let winner = candidates.swap_remove(first);
                            return advance_with_plan(
                                workflow,
                                winner.plan,
                                winner.author,
                                settings.swarm_models.clone(),
                            );
                        }
                    }
                }

                // A panelist's revision replaces its own candidate, so a good
                // idea survives even if its author is eliminated this round.
                for (index, plan) in revisions {
                    if let Some(model) = panel.get(index) {
                        candidates[index] = PlanCandidate {
                            label: format!("plan:{}:{}:r{}", index + 1, model.name(), round + 1),
                            author: Some(*model),
                            plan,
                        };
                    }
                }

                // Eliminate the most worst-voted plan and its author's seat.
                // Ties (and rounds with no usable votes) eliminate the lowest
                // priority roster entry, so the tournament always terminates.
                let mut eliminated = 0;
                let mut max_votes = worst_votes[0];
                for (index, &votes) in worst_votes.iter().enumerate() {
                    if votes >= max_votes {
                        max_votes = votes;
                        eliminated = index;
                    }
                }
                panel.remove(eliminated);
                candidates.remove(eliminated);

                if candidates.len() == 1 {
                    let winner = candidates.remove(0);
                    return advance_with_plan(
                        workflow,
                        winner.plan,
                        winner.author,
                        settings.swarm_models.clone(),
                    );
                }

                let workers = consensus_round_workers(&panel, &candidates, round + 1);
                *workflow = WorkflowState::Swarm(SwarmPhase::Consensus {
                    models: panel,
                    candidates,
                    round: round + 1,
                });
                ChildAction::FanOut(FanOutSpec { workers })
            }
            SwarmPhase::Implementing {
                models,
                fixer_model,
            } => {
                let models = models.clone();
                let fixer_model = *fixer_model;
                if !child.success {
                    return ChildAction::Complete {
                        success: false,
                        result: format!("Implementation failed: {}", child.result),
                    };
                }
                if models.len() < 2 {
                    return ChildAction::Complete {
                        success: true,
                        result: child.result.clone(),
                    };
                }
                let task = format!(
                    "{INTEGRATION_ORIENTATION_TASK}\n\nImplementation report:\n{}",
                    child.result
                );
                start_integration_review(workflow, 0, models, fixer_model, task)
            }
            SwarmPhase::FanOut {
                plan,
                model,
                models,
            } => {
                let models = models.clone();
                let fixer_model = *model;
                let task = format!(
                    "{INTEGRATION_ORIENTATION_TASK}\n\n\
                    The plan that was implemented:\n{plan}\n\n\
                    Per-file worker reports:\n{}",
                    child.result
                );
                start_integration_review(workflow, 0, models, fixer_model, task)
            }
            SwarmPhase::Integration {
                rounds,
                models,
                fixer_model,
            } => {
                if child.success {
                    return ChildAction::Complete {
                        success: true,
                        result: format!("Swarm complete.\n\n{}", child.result),
                    };
                }
                let models = models.clone();
                let fixer_model = *fixer_model;
                let next_rounds = *rounds + 1;
                if next_rounds >= settings.max_review_rounds {
                    return ChildAction::Complete {
                        success: false,
                        result: format!(
                            "[Integration review round limit ({}) reached; unresolved feedback: {}]",
                            settings.max_review_rounds, child.result
                        ),
                    };
                }
                *workflow = WorkflowState::Swarm(SwarmPhase::Fixing {
                    rounds: next_rounds,
                    models,
                    fixer_model,
                });
                spawn_coder(
                    format!(
                        "Address this integration review feedback from a concurrent multi-file change: {}",
                        child.result
                    ),
                    fixer_model,
                )
            }
            SwarmPhase::Fixing {
                rounds,
                models,
                fixer_model,
            } => {
                if !child.success {
                    return ChildAction::Complete {
                        success: false,
                        result: format!("Integration fix attempt failed: {}", child.result),
                    };
                }
                let rounds = *rounds;
                let models = models.clone();
                let fixer_model = *fixer_model;
                let task = format!(
                    "{INTEGRATION_ORIENTATION_TASK}\n\nA fixer agent just addressed prior feedback; its report:\n{}",
                    child.result
                );
                start_integration_review(workflow, rounds, models, fixer_model, task)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::orchestration::WorkerResult;

    fn outcome(agent_name: &str, success: bool, result: &str) -> ChildOutcome {
        ChildOutcome {
            agent_name: agent_name.to_string(),
            success,
            result: result.to_string(),
            conversation: Vec::new(),
            reports: Vec::new(),
        }
    }

    fn fanout_outcome(reports: Vec<WorkerResult>) -> ChildOutcome {
        let success = reports.iter().all(|report| report.success);
        ChildOutcome {
            agent_name: crate::orchestration::FANOUT_AGENT.to_string(),
            success,
            result: String::new(),
            conversation: Vec::new(),
            reports,
        }
    }

    fn report(label: &str, success: bool, summary: &str) -> WorkerResult {
        WorkerResult {
            label: label.to_string(),
            success,
            summary: summary.to_string(),
        }
    }

    fn candidate(label: &str, author: Model, plan: &str) -> PlanCandidate {
        PlanCandidate {
            label: label.to_string(),
            author: Some(author),
            plan: plan.to_string(),
        }
    }

    const PARALLEL_PLAN: &str = r#"## Plan
Change both files.

```json
{"assignments": [
  {"file": "src/a.rs", "instructions": "define struct", "shared_surfaces": ["pub struct X"]},
  {"file": "src/b.rs", "instructions": "use struct", "shared_surfaces": ["pub struct X"]}
]}
```"#;

    fn consensus_settings() -> Settings {
        Settings {
            swarm_models: vec![Model::ClaudeFable, Model::Gpt, Model::Grok],
            ..Settings::default()
        }
    }

    fn consensus_phase(models: Vec<Model>, candidates: Vec<PlanCandidate>) -> WorkflowState {
        WorkflowState::Swarm(SwarmPhase::Consensus {
            models,
            candidates,
            round: 1,
        })
    }

    #[test]
    fn parses_trailing_assignment_block() {
        let assignments = parse_assignments(PARALLEL_PLAN).unwrap();
        assert_eq!(assignments.len(), 2);
        assert_eq!(assignments[0].file, "src/a.rs");
        assert_eq!(assignments[0].shared_surfaces.len(), 1);
    }

    #[test]
    fn unparseable_plan_degrades_to_none() {
        assert!(parse_assignments("no json here").is_none());
        assert!(parse_assignments("```json\nnot json\n```").is_none());
    }

    #[test]
    fn single_model_plan_fans_out_with_allowlists() {
        let settings = Settings::default();
        let mut workflow = WorkflowState::None;

        let TaskAction::Spawn(spec) = SwarmAgent.on_task(&mut workflow, &settings, "wide change")
        else {
            panic!("swarm without a roster must spawn a single planner");
        };
        assert_eq!(spec.agent, PlannerAgent::NAME);
        assert!(spec.model.is_none());

        let action = SwarmAgent.on_child_complete(
            &mut workflow,
            &settings,
            &outcome(PlannerAgent::NAME, true, PARALLEL_PLAN),
        );
        let ChildAction::FanOut(fanout) = action else {
            panic!("two assignments must fan out");
        };
        assert_eq!(fanout.workers.len(), 2);
        let worker = &fanout.workers[0];
        assert_eq!(worker.agent, FileImplAgent::NAME);
        assert!(worker.reviewed);
        assert!(worker.model.is_none());
        assert_eq!(
            worker.write_allowlist,
            Some(HashSet::from([PathBuf::from("src/a.rs")]))
        );
    }

    #[test]
    fn unparseable_plan_degrades_to_single_coder() {
        let settings = Settings::default();
        let mut workflow = WorkflowState::Swarm(SwarmPhase::Planning);
        let action = SwarmAgent.on_child_complete(
            &mut workflow,
            &settings,
            &outcome(PlannerAgent::NAME, true, "a plan with no assignment block"),
        );
        let ChildAction::Spawn(spec) = action else {
            panic!("expected sequential degrade");
        };
        assert_eq!(spec.agent, CoderAgent::NAME);
        assert!(matches!(
            workflow,
            WorkflowState::Swarm(SwarmPhase::Implementing { .. })
        ));
    }

    #[test]
    fn single_model_fanout_flows_into_integration_review_then_completion() {
        let settings = Settings::default();
        let mut workflow = WorkflowState::Swarm(SwarmPhase::FanOut {
            plan: "the plan".to_string(),
            model: None,
            models: Vec::new(),
        });

        let action = SwarmAgent.on_child_complete(
            &mut workflow,
            &settings,
            &outcome(crate::orchestration::FANOUT_AGENT, true, "all workers ok"),
        );
        let ChildAction::Spawn(spec) = action else {
            panic!("empty roster must use a single integration review");
        };
        assert_eq!(spec.agent, CodeReviewAgent::NAME);
        assert!(spec.task.contains("the plan") && spec.task.contains("all workers ok"));

        let action = SwarmAgent.on_child_complete(
            &mut workflow,
            &settings,
            &outcome(CodeReviewAgent::NAME, true, "integrated fine"),
        );
        assert!(matches!(
            action,
            ChildAction::Complete { success: true, .. }
        ));
    }

    #[test]
    fn integration_rejection_spawns_fixer_then_rereview() {
        let settings = Settings::default();
        let mut workflow = WorkflowState::Swarm(SwarmPhase::Integration {
            rounds: 0,
            models: Vec::new(),
            fixer_model: None,
        });

        let action = SwarmAgent.on_child_complete(
            &mut workflow,
            &settings,
            &outcome(CodeReviewAgent::NAME, false, "mismatched interface"),
        );
        let ChildAction::Spawn(spec) = action else {
            panic!("rejection must spawn fixer");
        };
        assert_eq!(spec.agent, CoderAgent::NAME);
        assert!(matches!(
            workflow,
            WorkflowState::Swarm(SwarmPhase::Fixing { rounds: 1, .. })
        ));

        let action = SwarmAgent.on_child_complete(
            &mut workflow,
            &settings,
            &outcome(CoderAgent::NAME, true, "aligned interface"),
        );
        let ChildAction::Spawn(spec) = action else {
            panic!("fix must trigger re-review");
        };
        assert_eq!(spec.agent, CodeReviewAgent::NAME);
    }

    #[test]
    fn consensus_roster_fans_out_planners_per_model() {
        let settings = consensus_settings();
        let mut workflow = WorkflowState::None;

        let TaskAction::FanOut(fanout) = SwarmAgent.on_task(&mut workflow, &settings, "task")
        else {
            panic!("roster of 3 must fan out planners");
        };
        assert_eq!(fanout.workers.len(), 3);
        assert_eq!(fanout.workers[0].agent, PlannerAgent::NAME);
        assert_eq!(fanout.workers[0].model, Some(Model::ClaudeFable));
        assert_eq!(fanout.workers[0].label, "plan:1:claude-fable");
        assert!(!fanout.workers[0].reviewed);
    }

    #[test]
    fn plan_fanout_enters_consensus_tournament() {
        let settings = consensus_settings();
        let mut workflow = WorkflowState::Swarm(SwarmPhase::PlanFanOut {
            models: settings.swarm_models.clone(),
        });

        let reports = vec![
            report("plan:1:claude-fable", true, PARALLEL_PLAN),
            report("plan:2:gpt", true, PARALLEL_PLAN),
            report("plan:3:grok", false, "planner crashed"),
        ];
        let action =
            SwarmAgent.on_child_complete(&mut workflow, &settings, &fanout_outcome(reports));
        let ChildAction::FanOut(fanout) = action else {
            panic!("multiple surviving plans must enter the tournament");
        };
        assert_eq!(
            fanout.workers.len(),
            2,
            "failed planners lose their panel seat before round 1"
        );
        assert_eq!(fanout.workers[0].agent, PlanJudgeAgent::NAME);
        assert!(fanout.workers[0].task.contains("plan:1:claude-fable"));
        assert!(fanout.workers[0].task.contains("WORST"));
        assert!(
            !fanout.workers[0].task.contains("planner crashed"),
            "failed plans must not reach the panel"
        );
        assert!(matches!(
            workflow,
            WorkflowState::Swarm(SwarmPhase::Consensus { round: 1, .. })
        ));
    }

    #[test]
    fn single_successful_plan_skips_tournament() {
        let settings = consensus_settings();
        let mut workflow = WorkflowState::Swarm(SwarmPhase::PlanFanOut {
            models: settings.swarm_models.clone(),
        });

        let reports = vec![
            report("plan:1:claude-fable", false, "failed"),
            report("plan:2:gpt", true, PARALLEL_PLAN),
            report("plan:3:grok", false, "failed"),
        ];
        let action =
            SwarmAgent.on_child_complete(&mut workflow, &settings, &fanout_outcome(reports));
        let ChildAction::FanOut(fanout) = action else {
            panic!("sole surviving plan should fan out file workers directly");
        };
        assert_eq!(
            fanout.workers[0].model,
            Some(Model::Gpt),
            "workers must be pinned to the surviving plan's model"
        );
    }

    #[test]
    fn unanimous_approval_ends_tournament_with_author_pinned() {
        let settings = consensus_settings();
        let mut workflow = consensus_phase(
            vec![Model::ClaudeFable, Model::Gpt, Model::Grok],
            vec![
                candidate("plan:1:claude-fable", Model::ClaudeFable, "meh plan"),
                candidate("plan:2:gpt", Model::Gpt, PARALLEL_PLAN),
                candidate("plan:3:grok", Model::Grok, "another plan"),
            ],
        );

        let responses = vec![
            report(
                "consensus:r1:claude-fable",
                true,
                "APPROVE: plan:2:gpt\nWORST: plan:3:grok",
            ),
            report(
                "consensus:r1:gpt",
                true,
                "APPROVE: plan:2:gpt\nWORST: plan:1:claude-fable",
            ),
            report(
                "consensus:r1:grok",
                true,
                "APPROVE: plan:2:gpt\nWORST: plan:1:claude-fable",
            ),
        ];
        let action =
            SwarmAgent.on_child_complete(&mut workflow, &settings, &fanout_outcome(responses));
        let ChildAction::FanOut(fanout) = action else {
            panic!("unanimous approval of a parallelizable plan must fan out file workers");
        };
        assert_eq!(
            fanout.workers[0].model,
            Some(Model::Gpt),
            "file workers must be pinned to the winning author"
        );
    }

    #[test]
    fn split_round_eliminates_most_worst_voted_and_applies_revisions() {
        let settings = consensus_settings();
        let mut workflow = consensus_phase(
            vec![Model::ClaudeFable, Model::Gpt, Model::Grok],
            vec![
                candidate("plan:1:claude-fable", Model::ClaudeFable, "plan A"),
                candidate("plan:2:gpt", Model::Gpt, "plan B"),
                candidate("plan:3:grok", Model::Grok, "plan C"),
            ],
        );

        let revision = "Critique: merged grok's caching idea into plan A.\nFull revised plan...";
        let responses = vec![
            report(
                "consensus:r1:claude-fable",
                true,
                &format!("{revision}\nWORST: plan:2:gpt"),
            ),
            report(
                "consensus:r1:gpt",
                true,
                "APPROVE: plan:2:gpt\nWORST: plan:3:grok",
            ),
            report(
                "consensus:r1:grok",
                true,
                "APPROVE: plan:3:grok\nWORST: plan:2:gpt",
            ),
        ];
        let action =
            SwarmAgent.on_child_complete(&mut workflow, &settings, &fanout_outcome(responses));
        let ChildAction::FanOut(fanout) = action else {
            panic!("split round must continue the tournament");
        };
        assert_eq!(fanout.workers.len(), 2, "gpt's seat is eliminated");
        assert_eq!(fanout.workers[0].model, Some(Model::ClaudeFable));
        assert_eq!(fanout.workers[1].model, Some(Model::Grok));
        assert!(
            fanout.workers[0]
                .task
                .contains("merged grok's caching idea"),
            "fable's revision must replace its candidate"
        );
        assert!(
            !fanout.workers[0].task.contains("plan B"),
            "eliminated candidate must leave the pool"
        );

        let WorkflowState::Swarm(SwarmPhase::Consensus {
            models,
            candidates,
            round,
        }) = &workflow
        else {
            panic!("must remain in consensus");
        };
        assert_eq!(*round, 2);
        assert_eq!(models.as_slice(), &[Model::ClaudeFable, Model::Grok]);
        assert!(candidates[0].label.ends_with(":r2"));
    }

    #[test]
    fn elimination_down_to_one_survivor_wins() {
        let settings = consensus_settings();
        let mut workflow = consensus_phase(
            vec![Model::ClaudeFable, Model::Gpt],
            vec![
                candidate("plan:1:claude-fable", Model::ClaudeFable, PARALLEL_PLAN),
                candidate("plan:2:gpt", Model::Gpt, "plan B"),
            ],
        );

        let responses = vec![
            report(
                "consensus:r1:claude-fable",
                true,
                "APPROVE: plan:1:claude-fable\nWORST: plan:2:gpt",
            ),
            report(
                "consensus:r1:gpt",
                true,
                "APPROVE: plan:2:gpt\nWORST: plan:2:gpt",
            ),
        ];
        let action =
            SwarmAgent.on_child_complete(&mut workflow, &settings, &fanout_outcome(responses));
        let ChildAction::FanOut(fanout) = action else {
            panic!("single survivor's parallelizable plan must fan out file workers");
        };
        assert_eq!(
            fanout.workers[0].model,
            Some(Model::ClaudeFable),
            "survivor implements"
        );
    }

    #[test]
    fn garbage_round_still_terminates_via_priority_elimination() {
        let settings = consensus_settings();
        let mut workflow = consensus_phase(
            vec![Model::ClaudeFable, Model::Gpt, Model::Grok],
            vec![
                candidate("plan:1:claude-fable", Model::ClaudeFable, "plan A"),
                candidate("plan:2:gpt", Model::Gpt, "plan B"),
                candidate("plan:3:grok", Model::Grok, "plan C"),
            ],
        );

        let responses = vec![
            report("consensus:r1:claude-fable", false, "provider error"),
            report("consensus:r1:gpt", false, "provider error"),
            report("consensus:r1:grok", false, "provider error"),
        ];
        let action =
            SwarmAgent.on_child_complete(&mut workflow, &settings, &fanout_outcome(responses));
        let ChildAction::FanOut(fanout) = action else {
            panic!("tournament must continue even on a garbage round");
        };
        assert_eq!(
            fanout.workers.len(),
            2,
            "lowest-priority seat is eliminated when no votes are cast"
        );
        assert_eq!(fanout.workers[1].model, Some(Model::Gpt));
    }

    #[test]
    fn parse_panel_response_handles_all_forms() {
        let candidates = vec![
            candidate("plan:1:a", Model::ClaudeFable, "p1"),
            candidate("plan:2:b", Model::Gpt, "p2"),
        ];

        let approve = parse_panel_response("APPROVE: plan:2:b\nWORST: plan:1:a", &candidates);
        assert!(matches!(approve.position, Some(Position::Endorse(1))));
        assert_eq!(approve.worst, Some(0));

        let bare = parse_panel_response("plan:1:a", &candidates);
        assert!(matches!(bare.position, Some(Position::Endorse(0))));
        assert_eq!(bare.worst, None);

        let revision = parse_panel_response(
            "Critique of plan:1:a...\nNew plan\nWORST: plan:2:b",
            &candidates,
        );
        assert!(matches!(revision.position, Some(Position::Revision(_))));
        assert_eq!(revision.worst, Some(1));

        let empty = parse_panel_response("WORST: plan:1:a", &candidates);
        assert!(empty.position.is_none());
        assert_eq!(empty.worst, Some(0));
    }

    #[test]
    fn consensus_integration_review_fans_out_every_model() {
        let settings = consensus_settings();
        let mut workflow = WorkflowState::Swarm(SwarmPhase::FanOut {
            plan: "the plan".to_string(),
            model: Some(Model::Gpt),
            models: settings.swarm_models.clone(),
        });

        let action = SwarmAgent.on_child_complete(
            &mut workflow,
            &settings,
            &outcome(crate::orchestration::FANOUT_AGENT, true, "all workers ok"),
        );
        let ChildAction::FanOut(fanout) = action else {
            panic!("consensus integration review must fan out");
        };
        assert_eq!(fanout.workers.len(), 3);
        assert_eq!(fanout.workers[0].agent, CodeReviewAgent::NAME);
        assert_eq!(fanout.workers[2].model, Some(Model::Grok));

        let rejections = vec![
            report("review:claude-fable", true, "approved"),
            report("review:gpt", false, "missed call site"),
            report("review:grok", true, "approved"),
        ];
        let action =
            SwarmAgent.on_child_complete(&mut workflow, &settings, &fanout_outcome(rejections));
        let ChildAction::Spawn(spec) = action else {
            panic!("rejection must spawn fixer");
        };
        assert_eq!(spec.agent, CoderAgent::NAME);
        assert_eq!(spec.model, Some(Model::Gpt));
    }
}
