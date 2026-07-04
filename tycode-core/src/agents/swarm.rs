use std::collections::HashSet;
use std::path::PathBuf;

use serde::Deserialize;

use crate::agents::agent::Agent;
use crate::agents::code_review::CodeReviewAgent;
use crate::agents::coder::CoderAgent;
use crate::agents::file_impl::FileImplAgent;
use crate::agents::planner::PlannerAgent;
use crate::orchestration::{
    default_child_message, ChildAction, ChildOutcome, ConversationSeed, FanOutSpec, SpawnSpec,
    SwarmPhase, TaskAction, WorkerSpec, WorkflowState,
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

/// Plan → concurrent per-file implementation → integration review pipeline.
/// The planner decomposes the task into per-file assignments with explicit
/// shared-surface contracts; workers fork the planner's conversation so the
/// full research context transfers via prompt-cache reads instead of
/// re-distilled instructions. Degrades to a single sequential coder when the
/// plan is not parallelizable.
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

    fn on_task(
        &self,
        workflow: &mut WorkflowState,
        _settings: &Settings,
        task: &str,
    ) -> TaskAction {
        *workflow = WorkflowState::Swarm(SwarmPhase::Planning);
        TaskAction::Spawn(SpawnSpec {
            agent: PlannerAgent::NAME.to_string(),
            task: format!(
                "Produce an execution plan for the following task:\n{task}\n\n{PLAN_ASSIGNMENT_INSTRUCTIONS}"
            ),
            seed: ConversationSeed::ForkSelf,
            orientation: None,
        })
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

                let assignments = parse_assignments(&child.result).unwrap_or_default();
                if assignments.len() < 2 {
                    *phase = SwarmPhase::Implementing;
                    return ChildAction::Spawn(SpawnSpec {
                        agent: CoderAgent::NAME.to_string(),
                        task: format!(
                            "Implement the following plan exactly as specified.\n\n{}",
                            child.result
                        ),
                        seed: ConversationSeed::Fresh,
                        orientation: None,
                    });
                }

                let workers = assignments
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
                        }
                    })
                    .collect();

                *phase = SwarmPhase::FanOut {
                    plan: child.result.clone(),
                };
                ChildAction::FanOut(FanOutSpec { workers })
            }
            SwarmPhase::Implementing => ChildAction::Complete {
                success: child.success,
                result: child.result.clone(),
            },
            SwarmPhase::FanOut { plan } => {
                let task = format!(
                    "{INTEGRATION_ORIENTATION_TASK}\n\n\
                    The plan that was implemented:\n{plan}\n\n\
                    Per-file worker reports:\n{}",
                    child.result
                );
                *workflow = WorkflowState::Swarm(SwarmPhase::Integration { rounds: 0 });
                ChildAction::Spawn(SpawnSpec {
                    agent: CodeReviewAgent::NAME.to_string(),
                    task,
                    seed: ConversationSeed::Fresh,
                    orientation: None,
                })
            }
            SwarmPhase::Integration { rounds } => {
                if child.success {
                    return ChildAction::Complete {
                        success: true,
                        result: format!("Swarm complete.\n\n{}", child.result),
                    };
                }
                *rounds += 1;
                if *rounds >= settings.max_review_rounds {
                    return ChildAction::Complete {
                        success: false,
                        result: format!(
                            "[Integration review round limit ({}) reached; unresolved feedback: {}]",
                            settings.max_review_rounds, child.result
                        ),
                    };
                }
                let rounds = *rounds;
                *workflow = WorkflowState::Swarm(SwarmPhase::Fixing { rounds });
                ChildAction::Spawn(SpawnSpec {
                    agent: CoderAgent::NAME.to_string(),
                    task: format!(
                        "Address this integration review feedback from a concurrent multi-file change: {}",
                        child.result
                    ),
                    seed: ConversationSeed::Fresh,
                    orientation: None,
                })
            }
            SwarmPhase::Fixing { rounds } => {
                if !child.success {
                    return ChildAction::Complete {
                        success: false,
                        result: format!("Integration fix attempt failed: {}", child.result),
                    };
                }
                let rounds = *rounds;
                *workflow = WorkflowState::Swarm(SwarmPhase::Integration { rounds });
                ChildAction::Spawn(SpawnSpec {
                    agent: CodeReviewAgent::NAME.to_string(),
                    task: format!(
                        "{INTEGRATION_ORIENTATION_TASK}\n\nA fixer agent just addressed prior feedback; its report:\n{}",
                        child.result
                    ),
                    seed: ConversationSeed::Fresh,
                    orientation: None,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_trailing_assignment_block() {
        let plan = r#"## Plan
Do things.

```json
{"assignments": [
  {"file": "src/a.rs", "instructions": "add struct", "shared_surfaces": ["pub struct X { y: u32 }"]},
  {"file": "src/b.rs", "instructions": "use struct"}
]}
```"#;
        let assignments = parse_assignments(plan).unwrap();
        assert_eq!(assignments.len(), 2);
        assert_eq!(assignments[0].file, "src/a.rs");
        assert_eq!(assignments[0].shared_surfaces.len(), 1);
        assert!(assignments[1].shared_surfaces.is_empty());
    }

    #[test]
    fn unparseable_plan_degrades_to_none() {
        assert!(parse_assignments("no json here").is_none());
        assert!(parse_assignments("```json\nnot json\n```").is_none());
    }

    #[test]
    fn empty_assignments_parse_as_empty() {
        let plan = "plan\n```json\n{\"assignments\": []}\n```";
        assert_eq!(parse_assignments(plan).unwrap().len(), 0);
    }

    fn outcome(agent_name: &str, success: bool, result: &str) -> ChildOutcome {
        ChildOutcome {
            agent_name: agent_name.to_string(),
            success,
            result: result.to_string(),
            conversation: Vec::new(),
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

    #[test]
    fn parallelizable_plan_fans_out_with_allowlists() {
        let settings = Settings::default();
        let mut workflow = WorkflowState::None;

        let TaskAction::Spawn(spec) = SwarmAgent.on_task(&mut workflow, &settings, "wide change")
        else {
            panic!("swarm must delegate immediately");
        };
        assert_eq!(spec.agent, PlannerAgent::NAME);
        assert!(spec.task.contains("assignments"));

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
        assert!(matches!(worker.seed, ConversationSeed::ForkChild));
        assert_eq!(
            worker.write_allowlist,
            Some(HashSet::from([PathBuf::from("src/a.rs")]))
        );
        assert!(worker.task.contains("pub struct X"));
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
            WorkflowState::Swarm(SwarmPhase::Implementing)
        ));
    }

    #[test]
    fn fanout_report_flows_into_integration_review_then_completion() {
        let settings = Settings::default();
        let mut workflow = WorkflowState::Swarm(SwarmPhase::FanOut {
            plan: "the plan".to_string(),
        });

        let action = SwarmAgent.on_child_complete(
            &mut workflow,
            &settings,
            &outcome(crate::orchestration::FANOUT_AGENT, true, "all workers ok"),
        );
        let ChildAction::Spawn(spec) = action else {
            panic!("fan-out completion must spawn integration review");
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
        let mut workflow = WorkflowState::Swarm(SwarmPhase::Integration { rounds: 0 });

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
            WorkflowState::Swarm(SwarmPhase::Fixing { rounds: 1 })
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
}
