use crate::agents::agent::Agent;
use crate::analyzer::get_type_docs::GetTypeDocsTool;
use crate::analyzer::search_types::SearchTypesTool;
use crate::file::modify::delete_file::DeleteFileTool;
use crate::file::modify::replace_in_file::ReplaceInFileTool;
use crate::file::modify::write_file::WriteFileTool;
use crate::file::read_only::TrackedFilesManager;
use crate::module::PromptComponentSelection;
use crate::modules::execution::RunBuildTestTool;
use crate::modules::memory::tool::AppendMemoryTool;
use crate::skills::tool::InvokeSkillTool;
use crate::spawn::complete_task::CompleteTask;
use crate::spawn::SpawnAgent;
use crate::steering::autonomy;
use crate::tools::ToolName;

const CORE_PROMPT: &str = r#"You are a Tycode sub-agent responsible for executing assigned coding tasks. Follow this workflow to execute the task:

1. Understand the task and determine files to modify. If the task is not clear, use the 'complete_task' tool to fail the task and request clarification. If more context is needed use tools such as 'set_tracked_files' to read files
2. Use 'set_tracked_files' to read files, 'write_file' to create new files, and 'modify_file' to apply a patch to an existing file. Ensure that all changes comply with the style mandates; your changes may be rejected by another agent specifically focused on ensuring compliance with the style mandates so it is critical that you follow the style mandates.
3. After each change, re-read the entire modified file to ensure your change applied as you expected and that all style mandates have been obeyed. Correct any style mandate compliance failures in lines you have modified.
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
            TrackedFilesManager::tool_name(),
            WriteFileTool::tool_name(),
            ReplaceInFileTool::tool_name(),
            DeleteFileTool::tool_name(),
            SpawnAgent::tool_name(),
            RunBuildTestTool::tool_name(),
            CompleteTask::tool_name(),
            SearchTypesTool::tool_name(),
            GetTypeDocsTool::tool_name(),
            AppendMemoryTool::tool_name(),
            InvokeSkillTool::tool_name(),
        ]
    }

    fn requires_tool_use(&self) -> bool {
        true
    }
}
