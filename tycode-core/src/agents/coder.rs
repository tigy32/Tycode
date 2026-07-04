use crate::agents::agent::Agent;
use crate::agents::code_review::CodeReviewAgent;
use crate::analyzer::get_type_docs::GetTypeDocsTool;
use crate::analyzer::search_types::SearchTypesTool;
use crate::file::modify::delete_file::DeleteFileTool;
use crate::file::modify::replace_in_file::ReplaceInFileTool;
use crate::file::modify::write_file::WriteFileTool;
use crate::module::PromptComponentSelection;
use crate::modules::execution::BashTool;
use crate::modules::image::{GenerateImageTool, ReadImageTool};
use crate::modules::memory::tool::AppendMemoryTool;
use crate::modules::task_list::ManageTaskListTool;
use crate::orchestration::{
    default_child_message, ChildAction, ChildOutcome, CompletionAction, ConversationSeed,
    SpawnSpec, WorkflowState,
};
use crate::settings::config::{ReviewLevel, Settings};
use crate::skills::tool::InvokeSkillTool;
use crate::spawn::complete_task::CompleteTask;
use crate::spawn::SpawnAgent;
use crate::steering::autonomy;
use crate::tools::ToolName;

pub const REVIEW_ORIENTATION: &str = "\
    --- AGENT TRANSITION ---\n\
    You are a code review agent. The conversation above is from the parent coder agent. \
    Review all of the file modifications the parent coder agent made. \
    Evaluate correctness, style compliance, and whether the changes satisfy the task requirements. \
    When done, use complete_task to return your verdict to the parent.";

const CORE_PROMPT: &str = r#"You are a Tycode sub-agent responsible for executing assigned coding tasks. Follow this workflow to execute the task:

1. Understand the task and determine files to modify. If the task is not clear, use the 'complete_task' tool to fail the task and request clarification. If more context is needed, use 'bash' to search, list, and read files.
2. Use 'bash' to inspect files, 'write_file' to create new files, and 'modify_file' to apply a patch to an existing file. Ensure that all changes comply with the style mandates; your changes may be rejected by another agent specifically focused on ensuring compliance with the style mandates so it is critical that you follow the style mandates.
3. After each change, inspect the modified file or diff to ensure your change applied as expected and that all style mandates have been obeyed. Correct any style mandate compliance failures in lines you have modified.
4. Once all changes are complete, use the 'complete_task' tool to indicate success. If you cannot complete the task for any reason use the 'complete_task' tool to fail the task.

## It is okay to fail
- It is ok to fail. If you cannot complete the task as instructed, fail the task. This will let the parent agent (with more context) figure out what to do and how to recover. The parent agent may need to make a new plan based on your discovery!
- You can fail a task be using the complete_task tool with success = false. Give a detailed description of why you failed in the complete_task tool call so the parent agent can understand the issue.
- You are an automated agent and will not be able to interact with the user or ask for help; do your best or fail.
- Critical: Never deviate from the assigned task or plan. Do not change implementation approaches. Changing implementation approaches will cause signficant harm. Failing a task is harmless.

## Debugging
- It is critical that you never make conjectures or attempt to fix an "unproven" bug. For example, if you theorize there is a race condition but cannot write an irrefutable proof, do not attempt to fix it.
- You have access to a debugging agent. Any bug that is not immediately obvious should be delegated to the debugging agent.
- Spawn the debugging agent with a detailed description of the bug and any steps or hints to reproduce it. The debug agent will attempt to root cause the bug and give back a detailed root cause"#;

pub struct CoderAgent;

impl CoderAgent {
    pub const NAME: &'static str = "coder";

    pub fn new() -> Self {
        Self
    }
}

impl Agent for CoderAgent {
    fn name(&self) -> &str {
        Self::NAME
    }

    fn description(&self) -> &str {
        "Executes assigned coding tasks, applying patches and managing files"
    }

    fn core_prompt(&self) -> &'static str {
        CORE_PROMPT
    }

    fn requested_prompt_components(&self) -> PromptComponentSelection {
        PromptComponentSelection::Exclude(&[autonomy::ID])
    }

    fn available_tools(&self) -> Vec<ToolName> {
        vec![
            WriteFileTool::tool_name(),
            ReplaceInFileTool::tool_name(),
            DeleteFileTool::tool_name(),
            SpawnAgent::tool_name(),
            BashTool::tool_name(),
            CompleteTask::tool_name(),
            SearchTypesTool::tool_name(),
            GetTypeDocsTool::tool_name(),
            AppendMemoryTool::tool_name(),
            InvokeSkillTool::tool_name(),
            GenerateImageTool::tool_name(),
            ReadImageTool::tool_name(),
            ManageTaskListTool::tool_name(),
        ]
    }

    fn requires_tool_use(&self) -> bool {
        true
    }

    fn on_complete(
        &self,
        workflow: &mut WorkflowState,
        settings: &Settings,
        success: bool,
        result: &str,
    ) -> CompletionAction {
        if !success || settings.review_level != ReviewLevel::Task {
            return CompletionAction::Finish;
        }

        let rounds = match workflow {
            WorkflowState::Reviewing { rounds, .. } => *rounds,
            _ => 0,
        };
        *workflow = WorkflowState::Reviewing {
            rounds,
            parked_result: result.to_string(),
        };

        CompletionAction::Spawn(SpawnSpec {
            agent: CodeReviewAgent::NAME.to_string(),
            task: format!(
                "Review the code changes for the following completed task: {}",
                result
            ),
            seed: ConversationSeed::ForkSelf,
            orientation: Some(REVIEW_ORIENTATION.to_string()),
        })
    }

    fn on_child_complete(
        &self,
        workflow: &mut WorkflowState,
        settings: &Settings,
        child: &ChildOutcome,
    ) -> ChildAction {
        let WorkflowState::Reviewing {
            rounds,
            parked_result,
        } = workflow
        else {
            return ChildAction::Resume {
                message: default_child_message(child),
            };
        };

        // A non-review child (e.g. a debugger spawned while addressing
        // feedback) must not release the parked completion.
        if child.agent_name != CodeReviewAgent::NAME {
            return ChildAction::Resume {
                message: default_child_message(child),
            };
        }

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

        ChildAction::Resume {
            message: format!(
                "Code review feedback from the review agent: {}",
                child.result
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review_settings() -> Settings {
        Settings {
            review_level: ReviewLevel::Task,
            ..Settings::default()
        }
    }

    fn review_outcome(success: bool, result: &str) -> ChildOutcome {
        ChildOutcome {
            agent_name: CodeReviewAgent::NAME.to_string(),
            success,
            result: result.to_string(),
            conversation: Vec::new(),
        }
    }

    #[test]
    fn successful_completion_is_intercepted_when_review_enabled() {
        let mut workflow = WorkflowState::None;
        let action = CoderAgent.on_complete(&mut workflow, &review_settings(), true, "done");
        let CompletionAction::Spawn(spec) = action else {
            panic!("expected review spawn");
        };
        assert_eq!(spec.agent, CodeReviewAgent::NAME);
        assert!(matches!(
            workflow,
            WorkflowState::Reviewing { rounds: 0, .. }
        ));
    }

    #[test]
    fn failed_or_unreviewed_completions_finish_normally() {
        let mut workflow = WorkflowState::None;
        assert!(matches!(
            CoderAgent.on_complete(&mut workflow, &review_settings(), false, "gave up"),
            CompletionAction::Finish
        ));
        assert!(matches!(
            CoderAgent.on_complete(&mut workflow, &Settings::default(), true, "done"),
            CompletionAction::Finish
        ));
    }

    #[test]
    fn approval_releases_parked_result() {
        let mut workflow = WorkflowState::Reviewing {
            rounds: 0,
            parked_result: "done".to_string(),
        };
        let action = CoderAgent.on_child_complete(
            &mut workflow,
            &review_settings(),
            &review_outcome(true, "lgtm"),
        );
        let ChildAction::Complete { success, result } = action else {
            panic!("expected completion");
        };
        assert!(success);
        assert!(result.contains("done") && result.contains("lgtm"));
    }

    #[test]
    fn rejection_resumes_with_feedback_until_round_cap() {
        let settings = review_settings();
        let mut workflow = WorkflowState::Reviewing {
            rounds: 0,
            parked_result: "done".to_string(),
        };

        for expected_round in 1..settings.max_review_rounds {
            let action = CoderAgent.on_child_complete(
                &mut workflow,
                &settings,
                &review_outcome(false, "fix it"),
            );
            assert!(
                matches!(action, ChildAction::Resume { ref message } if message.contains("fix it"))
            );
            assert!(matches!(
                workflow,
                WorkflowState::Reviewing { rounds, .. } if rounds == expected_round
            ));
        }

        let action = CoderAgent.on_child_complete(
            &mut workflow,
            &settings,
            &review_outcome(false, "still broken"),
        );
        let ChildAction::Complete { success, result } = action else {
            panic!("round cap should force completion");
        };
        assert!(success);
        assert!(result.contains("Review round limit"));
        assert!(result.contains("still broken"));
    }

    #[test]
    fn non_review_child_does_not_release_parked_result() {
        let mut workflow = WorkflowState::Reviewing {
            rounds: 0,
            parked_result: "done".to_string(),
        };
        let debugger_outcome = ChildOutcome {
            agent_name: "debugger".to_string(),
            success: true,
            result: "root caused".to_string(),
            conversation: Vec::new(),
        };
        let action =
            CoderAgent.on_child_complete(&mut workflow, &review_settings(), &debugger_outcome);
        assert!(matches!(action, ChildAction::Resume { .. }));
        assert!(matches!(workflow, WorkflowState::Reviewing { .. }));
    }
}
