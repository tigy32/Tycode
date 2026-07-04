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
    default_child_message, ChildAction, ChildOutcome, ConversationSeed, FanOutSpec, SpawnSpec,
    SwarmPhase, TaskAction, WorkerResult, WorkerSpec, WorkflowState,
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

fn joined_successful_plans(plans: &[WorkerResult]) -> String {
    plans
        .iter()
        .filter(|plan| plan.success)
        .map(|plan| format!("### {} [ok]\n{}", plan.label, plan.summary))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Tally judge votes and return the index of the winning plan. Votes match a
/// candidate's exact label first, then fall back to label containment;
/// ties resolve to the earliest roster entry. Returns the first successful
/// plan when no judge produced a usable vote, and None when every plan
/// failed.
fn tally_votes(plans: &[WorkerResult], judges: &[WorkerResult]) -> Option<usize> {
    let candidates: Vec<usize> = plans
        .iter()
        .enumerate()
        .filter(|(_, plan)| plan.success)
        .map(|(index, _)| index)
        .collect();
    let first = *candidates.first()?;

    let mut votes = vec![0usize; plans.len()];
    for judge in judges.iter().filter(|judge| judge.success) {
        let verdict = judge.summary.trim();
        let vote = candidates
            .iter()
            .copied()
            .find(|&index| verdict == plans[index].label)
            .or_else(|| {
                candidates
                    .iter()
                    .copied()
                    .find(|&index| verdict.contains(&plans[index].label))
            });
        if let Some(index) = vote {
            votes[index] += 1;
        }
    }

    let mut best = first;
    for &index in &candidates {
        if votes[index] > votes[best] {
            best = index;
        }
    }
    Some(best)
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
/// With two or more models in `swarm_models`, planning fans out one planner
/// per model, a judge panel of all models votes on the best plan, the winning
/// model implements, and integration review requires approval from every
/// model.
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
                let successful: Vec<usize> = child
                    .reports
                    .iter()
                    .enumerate()
                    .filter(|(_, report)| report.success)
                    .map(|(index, _)| index)
                    .collect();

                match successful.as_slice() {
                    [] => ChildAction::Complete {
                        success: false,
                        result: format!("All consensus planners failed:\n{}", child.result),
                    },
                    [only] => {
                        let plan = child.reports[*only].summary.clone();
                        let winner = models.get(*only).copied();
                        advance_with_plan(workflow, plan, winner, models)
                    }
                    _ => {
                        let judge_task = format!(
                            "Pick the best plan from the candidates below.\n\n{}",
                            joined_successful_plans(&child.reports)
                        );
                        let workers = models
                            .iter()
                            .map(|model| WorkerSpec {
                                agent: PlanJudgeAgent::NAME.to_string(),
                                task: judge_task.clone(),
                                orientation: None,
                                seed: ConversationSeed::Fresh,
                                write_allowlist: None,
                                label: format!("judge:{}", model.name()),
                                reviewed: false,
                                model: Some(*model),
                            })
                            .collect();
                        *workflow = WorkflowState::Swarm(SwarmPhase::Judging {
                            models,
                            plans: child.reports.clone(),
                        });
                        ChildAction::FanOut(FanOutSpec { workers })
                    }
                }
            }
            SwarmPhase::Judging { models, plans } => {
                let models = models.clone();
                let Some(winner_index) = tally_votes(plans, &child.reports) else {
                    return ChildAction::Complete {
                        success: false,
                        result: "Consensus judging failed: no successful plans".to_string(),
                    };
                };
                let plan = plans[winner_index].summary.clone();
                let winner = models.get(winner_index).copied();
                advance_with_plan(workflow, plan, winner, models)
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
        assert_eq!(fanout.workers[1].model, Some(Model::Gpt));
        assert_eq!(fanout.workers[0].label, "plan:1:claude-fable");
        assert!(!fanout.workers[0].reviewed);
    }

    #[test]
    fn plan_fanout_proceeds_to_judging_with_all_models() {
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
            panic!("multiple successful plans must be judged");
        };
        assert_eq!(fanout.workers.len(), 3, "every roster model votes");
        assert_eq!(fanout.workers[0].agent, PlanJudgeAgent::NAME);
        assert!(fanout.workers[0].task.contains("plan:1:claude-fable"));
        assert!(
            !fanout.workers[0].task.contains("planner crashed"),
            "failed plans must not reach the judges"
        );
    }

    #[test]
    fn single_successful_plan_skips_judging() {
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
    fn all_plans_failed_completes_with_failure() {
        let settings = consensus_settings();
        let mut workflow = WorkflowState::Swarm(SwarmPhase::PlanFanOut {
            models: settings.swarm_models.clone(),
        });
        let reports = vec![
            report("plan:1:claude-fable", false, "failed"),
            report("plan:2:gpt", false, "failed"),
            report("plan:3:grok", false, "failed"),
        ];
        let action =
            SwarmAgent.on_child_complete(&mut workflow, &settings, &fanout_outcome(reports));
        assert!(matches!(
            action,
            ChildAction::Complete { success: false, .. }
        ));
    }

    #[test]
    fn judging_majority_pins_winner_model() {
        let settings = consensus_settings();
        let plans = vec![
            report("plan:1:claude-fable", true, "plan without assignments"),
            report("plan:2:gpt", true, PARALLEL_PLAN),
        ];
        let mut workflow = WorkflowState::Swarm(SwarmPhase::Judging {
            models: settings.swarm_models.clone(),
            plans,
        });

        let judges = vec![
            report("judge:claude-fable", true, "plan:2:gpt"),
            report("judge:gpt", true, "plan:2:gpt"),
            report("judge:grok", true, "plan:1:claude-fable"),
        ];
        let action =
            SwarmAgent.on_child_complete(&mut workflow, &settings, &fanout_outcome(judges));
        let ChildAction::FanOut(fanout) = action else {
            panic!("winning parallelizable plan must fan out file workers");
        };
        assert_eq!(
            fanout.workers[0].model,
            Some(Model::Gpt),
            "file workers must be pinned to the winning model"
        );
        assert!(matches!(
            workflow,
            WorkflowState::Swarm(SwarmPhase::FanOut {
                model: Some(Model::Gpt),
                ..
            })
        ));
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

        // Any rejection routes feedback to a fixer pinned to the winner.
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

    #[test]
    fn tally_prefers_majority_and_breaks_ties_by_roster_order() {
        let plans = vec![
            report("plan:1:a", true, "p1"),
            report("plan:2:b", true, "p2"),
        ];

        let majority = vec![
            report("j1", true, "plan:2:b"),
            report("j2", true, "The best is plan:2:b because..."),
            report("j3", true, "plan:1:a"),
        ];
        assert_eq!(tally_votes(&plans, &majority), Some(1));

        let tie = vec![
            report("j1", true, "plan:1:a"),
            report("j2", true, "plan:2:b"),
        ];
        assert_eq!(tally_votes(&plans, &tie), Some(0));

        let no_votes = vec![report("j1", true, "gibberish")];
        assert_eq!(tally_votes(&plans, &no_votes), Some(0));

        let failed_plan = vec![report("plan:1:a", false, "x")];
        assert_eq!(tally_votes(&failed_plan, &[]), None);
    }
}
