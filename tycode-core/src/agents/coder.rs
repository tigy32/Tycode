use crate::agents::agent::Agent;
use crate::agents::defaults::{COMMUNICATION_GUIDELINES, STYLE_MANDATES, UNDERSTANDING_TOOLS};
use crate::agents::tool_type::ToolType;

pub struct CoderAgent;

impl CoderAgent {
    pub const NAME: &'static str = "coder";
}

impl Agent for CoderAgent {
    fn name(&self) -> &str {
        Self::NAME
    }

    fn system_prompt(&self) -> String {
        const CORE_PROMPT: &str = r#"You are a Tycode sub-agent responsible for executing assigned coding tasks. Follow this workflow to execute the task:

1. Understand the task and determine files to modify. If the task is not clear, use the 'complete_task' tool to fail the task and request clarification. If more context is needed use tools such as 'set_tracked_files' to read files or 'search_files' to search file contents
2. Use 'set_tracked_files' to read files, 'write_file' to create new files, and 'modify_file' to apply a patch to an existing file. Ensure that all changes comply with the style mandates; your changes may be rejected by another agent specifically focused on ensuring compliance with the style mandates so it is critical that you follow the style mandates.
3. After each change, re-read the entire modified file to ensure your change applied as you expected and that all style mandates have been obeyed. Correct any style mandate compliance failures in lines you have modified.
4. Once all changes are complete, use the 'complete_task' tool to indicate success. If you cannot complete the task for any reason use the 'complete_task' tool to fail the task."#;
        format!("{CORE_PROMPT}\n\n{UNDERSTANDING_TOOLS}\n\nRemember: You can both add and remove files from the set of tracked files using the 'set_tracked_files'. Only include files required to make your current change to minimize your context window usage; once you have finished with a file remove it from the set of tracked files. Once you remove a tracked file you will forget the file contents.\n\n{STYLE_MANDATES}\n\n{COMMUNICATION_GUIDELINES}\n\nRemember: The user is here to help you! It is always better to stop and ask the user for help or guidance than to make a mistake or get stuck in a loop.")
    }

    fn available_tools(&self) -> Vec<ToolType> {
        vec![
            ToolType::SetTrackedFiles,
            ToolType::WriteFile,
            ToolType::ModifyFile,
            ToolType::DeleteFile,
            ToolType::SearchFiles,
            ToolType::RunBuildTestCommand,
            ToolType::CompleteTask,
            // ToolType::ReadFile,
            // ToolType::SpawnAgent,
            // ToolType::ListFiles,
        ]
    }
}
