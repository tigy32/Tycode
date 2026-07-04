use crate::agents::agent::Agent;
use crate::agents::code_review::CodeReviewAgent;
use crate::agents::coder::{CoderAgent, REVIEW_ORIENTATION};
use crate::agents::planner::PlannerAgent;
use crate::orchestration::{
    default_child_message, BuilderPhase, ChildAction, ChildOutcome, ConversationSeed, SpawnSpec,
    TaskAction, WorkflowState,
};
use crate::settings::config::Settings;
use crate::spawn::complete_task::CompleteTask;
use crate::tools::ToolName;

const CORE_PROMPT: &str = "You are a mechanical orchestration agent that should never converse. \
If you are reading this, a workflow transition failed; use complete_task with success=false to \
report the orchestration error.";

const FIX_ORIENTATION: &str = "\
    --- AGENT TRANSITION ---\n\
    You are a coder agent. The conversation above contains a prior implementation and the \
    review that evaluated it. Address the review feedback below, then use complete_task.";

/// Deterministic plan → implement → review pipeline. Unlike prompt-driven
/// delegation, every phase transition here is mechanical: the plan always
/// happens, the review always happens, and rejected reviews loop back to a
/// fixer until approval or the round cap.
pub struct BuilderAgent;

impl BuilderAgent {
    pub const NAME: &'static str = "builder";
}

impl Agent for BuilderAgent {
    fn name(&self) -> &str {
        Self::NAME
    }

    fn description(&self) -> &str {
        "Deterministic plan → implement → review pipeline for a single coherent task"
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
        *workflow = WorkflowState::Builder(BuilderPhase::Planning);
        TaskAction::Spawn(SpawnSpec {
            agent: PlannerAgent::NAME.to_string(),
            task: format!("Produce an execution plan for the following task:\n{task}"),
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
        let WorkflowState::Builder(phase) = workflow else {
            return ChildAction::Resume {
                message: default_child_message(child),
            };
        };

        match phase {
            BuilderPhase::Planning => {
                if !child.success {
                    return ChildAction::Complete {
                        success: false,
                        result: format!("Planning failed: {}", child.result),
                    };
                }
                *phase = BuilderPhase::Implementing;
                ChildAction::Spawn(SpawnSpec {
                    agent: CoderAgent::NAME.to_string(),
                    task: format!(
                        "Implement the following plan exactly as specified.\n\n{}",
                        child.result
                    ),
                    seed: ConversationSeed::Fresh,
                    orientation: None,
                })
            }
            BuilderPhase::Implementing => {
                if !child.success {
                    return ChildAction::Complete {
                        success: false,
                        result: format!("Implementation failed: {}", child.result),
                    };
                }
                *phase = BuilderPhase::Reviewing {
                    rounds: 0,
                    parked_result: child.result.clone(),
                };
                spawn_review(&child.result)
            }
            BuilderPhase::Reviewing {
                rounds,
                parked_result,
            } => {
                if child.success {
                    return ChildAction::Complete {
                        success: true,
                        result: format!("{parked_result}\n\nReview: {}", child.result),
                    };
                }
                *rounds += 1;
                if *rounds >= settings.max_review_rounds {
                    return ChildAction::Complete {
                        success: true,
                        result: format!(
                            "{parked_result}\n\n[Review round limit ({}) reached; unresolved feedback: {}]",
                            settings.max_review_rounds, child.result
                        ),
                    };
                }
                let rounds = *rounds;
                *workflow = WorkflowState::Builder(BuilderPhase::Fixing { rounds });
                ChildAction::Spawn(SpawnSpec {
                    agent: CoderAgent::NAME.to_string(),
                    task: format!("Address this code review feedback: {}", child.result),
                    seed: ConversationSeed::ForkChild,
                    orientation: Some(FIX_ORIENTATION.to_string()),
                })
            }
            BuilderPhase::Fixing { rounds } => {
                if !child.success {
                    return ChildAction::Complete {
                        success: false,
                        result: format!("Fix attempt failed: {}", child.result),
                    };
                }
                let rounds = *rounds;
                *workflow = WorkflowState::Builder(BuilderPhase::Reviewing {
                    rounds,
                    parked_result: child.result.clone(),
                });
                spawn_review(&child.result)
            }
        }
    }
}

fn spawn_review(completed_result: &str) -> ChildAction {
    ChildAction::Spawn(SpawnSpec {
        agent: CodeReviewAgent::NAME.to_string(),
        task: format!(
            "Review the code changes for the following completed task: {}",
            completed_result
        ),
        seed: ConversationSeed::ForkChild,
        orientation: Some(REVIEW_ORIENTATION.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn outcome(success: bool, result: &str) -> ChildOutcome {
        ChildOutcome {
            agent_name: String::new(),
            success,
            result: result.to_string(),
            conversation: Vec::new(),
        }
    }

    fn spawned_agent(action: &ChildAction) -> &str {
        match action {
            ChildAction::Spawn(spec) => &spec.agent,
            _ => panic!("expected spawn"),
        }
    }

    #[test]
    fn full_pipeline_walks_plan_implement_review() {
        let settings = Settings::default();
        let mut workflow = WorkflowState::None;

        let task_action = BuilderAgent.on_task(&mut workflow, &settings, "build a widget");
        let TaskAction::Spawn(spec) = task_action else {
            panic!("builder must delegate immediately");
        };
        assert_eq!(spec.agent, PlannerAgent::NAME);
        assert!(matches!(spec.seed, ConversationSeed::ForkSelf));

        let action =
            BuilderAgent.on_child_complete(&mut workflow, &settings, &outcome(true, "the plan"));
        assert_eq!(spawned_agent(&action), CoderAgent::NAME);

        let action =
            BuilderAgent.on_child_complete(&mut workflow, &settings, &outcome(true, "implemented"));
        assert_eq!(spawned_agent(&action), CodeReviewAgent::NAME);

        let action =
            BuilderAgent.on_child_complete(&mut workflow, &settings, &outcome(true, "approved"));
        let ChildAction::Complete { success, result } = action else {
            panic!("approved review must complete the builder");
        };
        assert!(success);
        assert!(result.contains("implemented") && result.contains("approved"));
    }

    #[test]
    fn rejected_review_loops_through_fixer_until_cap() {
        let settings = Settings::default();
        let mut workflow = WorkflowState::Builder(BuilderPhase::Reviewing {
            rounds: 0,
            parked_result: "implemented".to_string(),
        });

        for _ in 1..settings.max_review_rounds {
            let action = BuilderAgent.on_child_complete(
                &mut workflow,
                &settings,
                &outcome(false, "needs work"),
            );
            assert_eq!(spawned_agent(&action), CoderAgent::NAME);

            let action =
                BuilderAgent.on_child_complete(&mut workflow, &settings, &outcome(true, "fixed"));
            assert_eq!(spawned_agent(&action), CodeReviewAgent::NAME);
        }

        let action =
            BuilderAgent.on_child_complete(&mut workflow, &settings, &outcome(false, "still bad"));
        let ChildAction::Complete { success, result } = action else {
            panic!("round cap should force completion");
        };
        assert!(success);
        assert!(result.contains("Review round limit"));
    }

    #[test]
    fn planning_failure_fails_the_builder() {
        let settings = Settings::default();
        let mut workflow = WorkflowState::Builder(BuilderPhase::Planning);
        let action =
            BuilderAgent.on_child_complete(&mut workflow, &settings, &outcome(false, "stuck"));
        assert!(matches!(
            action,
            ChildAction::Complete { success: false, .. }
        ));
    }
}
