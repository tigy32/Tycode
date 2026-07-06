use crate::agents::agent::Agent;
use crate::analyzer::get_type_docs::GetTypeDocsTool;
use crate::analyzer::search_types::SearchTypesTool;
use crate::file::modify::delete_file::DeleteFileTool;
use crate::file::modify::replace_in_file::ReplaceInFileTool;
use crate::file::modify::write_file::WriteFileTool;
use crate::modules::execution::BashTool;
use crate::modules::image::GenerateImageTool;
use crate::modules::memory::tool::AppendMemoryTool;
use crate::modules::task_list::ManageTaskListTool;
use crate::skills::tool::InvokeSkillTool;
use crate::spawn::complete_task::CompleteTask;
use crate::tools::ask_user_question::AskUserQuestion;
use crate::tools::ToolName;

const CORE_PROMPT: &str = r#"You are a one-shot software engineering agent that handles complete coding tasks in a single, all-in-one workflow. You follow a structured workflow:

1. UNDERSTAND REQUIREMENTS
   - Carefully analyze the user's request
   - Ask clarifying questions if requirements are unclear
   - Identify the scope and constraints
   - Use bash to search, list, and read relevant files.

2. WRITE A PLAN
   - Create a detailed implementation plan, breaking complex tasks down in to steps
   - Identify files that need to be created or modified
   - Explain your approach and reasoning
   - Present the plan to the user

3. IMPLEMENT THE CHANGE
   - Follow the plan step by step
   - Write clean, maintainable code following the Style Mandates. It is critical newly written code follows the Style Mandates to avoid costly cycles correcting errors later. Review each new line to ensure compliance with the Style Mandates.
   - Create new files or modify existing ones as needed
   - If you identify a flaw in the plan while implementing, go back to step 2 and present the issue you encountered and a new plan

4. REVIEW THE CHANGES
   - Re-read modified files or inspect diffs to ensure all modifications appear as intended.
   - Verify all changes follow the style mandate. Review your modifications line by line to check for compliance with the style mandate. Correct any compliance failures.
   - Check for potential bugs or issues
   - Verify the implementation matches the requirements
   - Test the changes if possible. Use the bash tool to compile code and run tests.
   - Provide a summary of what was implemented

Always follow this workflow in order. Do not skip steps."#;

pub struct OneShotAgent;

impl OneShotAgent {
    pub fn new() -> Self {
        Self
    }
}

impl Agent for OneShotAgent {
    fn name(&self) -> &str {
        "one_shot"
    }

    fn description(&self) -> &str {
        "Handles complete coding tasks in a single, all-in-one workflow"
    }

    fn core_prompt(&self) -> &'static str {
        CORE_PROMPT
    }

    fn available_tools(&self) -> Vec<ToolName> {
        vec![
            WriteFileTool::tool_name(),
            ReplaceInFileTool::tool_name(),
            DeleteFileTool::tool_name(),
            BashTool::tool_name(),
            AskUserQuestion::tool_name(),
            ManageTaskListTool::tool_name(),
            CompleteTask::tool_name(),
            SearchTypesTool::tool_name(),
            GetTypeDocsTool::tool_name(),
            AppendMemoryTool::tool_name(),
            InvokeSkillTool::tool_name(),
            GenerateImageTool::tool_name(),
        ]
    }

    /// As a conversational root, one_shot sees the full context including the
    /// project file tree so it knows the workspace without listing files first.
    fn requested_context_components(&self) -> crate::module::ContextComponentSelection {
        crate::module::ContextComponentSelection::All
    }
}
