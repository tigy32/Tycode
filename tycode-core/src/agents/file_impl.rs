use crate::agents::agent::Agent;
use crate::file::modify::delete_file::DeleteFileTool;
use crate::file::modify::replace_in_file::ReplaceInFileTool;
use crate::file::modify::write_file::WriteFileTool;
use crate::module::PromptComponentSelection;
use crate::modules::execution::BashTool;
use crate::spawn::complete_task::CompleteTask;
use crate::steering::autonomy;
use crate::tools::ToolName;

const CORE_PROMPT: &str = r#"You are a file implementation worker executing one assignment of a larger plan. Sibling workers are implementing other files of the same plan concurrently.

## Rules
1. You may ONLY modify your assigned file (stated in your task). Modifications to any other file will be rejected. If your assignment cannot be completed without modifying another file, use complete_task with success=false explaining why.
2. You may read anything: use bash to inspect any file for context. Note that sibling files may still be mid-change; the plan (not the current content of sibling files) is the source of truth for shared interfaces.
3. Implement shared interfaces EXACTLY as specified in the plan. Sibling workers are implementing against the same specification; any deviation breaks their work.
4. Do NOT run builds or tests. Your file may not compile until sibling assignments land; the orchestrator validates the integrated result after all workers finish.
5. When your assignment is complete, use complete_task with a concise summary of the changes you made. If you cannot complete it as specified, use complete_task with success=false and a detailed reason. Never deviate from the assignment."#;

pub struct FileImplAgent;

impl FileImplAgent {
    pub const NAME: &'static str = "file_impl";
}

impl Agent for FileImplAgent {
    fn name(&self) -> &str {
        Self::NAME
    }

    fn description(&self) -> &str {
        "Implements a single-file assignment of a plan; used by swarm fan-out"
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
            BashTool::tool_name(),
            CompleteTask::tool_name(),
        ]
    }

    fn requires_tool_use(&self) -> bool {
        true
    }
}
