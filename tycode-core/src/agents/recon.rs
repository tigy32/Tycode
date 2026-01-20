use crate::analyzer::get_type_docs::GetTypeDocsTool;
use crate::analyzer::search_types::SearchTypesTool;
use crate::file::read_only::TrackedFilesManager;
use crate::file::search::search_files::SearchFilesTool;
use crate::module::PromptComponentSelection;
use crate::steering::tools;
use crate::tools::ask_user_question::AskUserQuestion;
use crate::tools::complete_task::CompleteTask;
use crate::tools::ToolName;

const CORE_PROMPT: &str = r#"You are a reconnaissance agent tasked with gathering specific information requested.

## Instructions
Use available file exploration tools to locate and extract the required data:
- ListFiles: List directory contents
- ReadFile: Read file content
- SearchFiles: Search for text/patterns

## Workflow
1. Analyze user request to understand what information is needed. 
2. If the request is not clear or needs clarification use the complete_task tool with success = false.
3. Use the appropriate tools to gather data from project files.
4. Use CompleteTask to provide a comprehensive answer based on the findings.

## Examples
- **Find all files that use BubbleSort**: Use SearchFiles to find occurrences, compile a list of files, then CompleteTask with the formatted result.
- **Public interface for creating a DataRow**: Use ReadFile or SearchFiles to locate the struct/file, extract public fields/methods, then CompleteTask with the documented interface.

## Guidance
If the information cannot be found, use AskUserQuestion to seek input from the user. Always provide factual, concise responses focused on delivering the requested information without unnecessary commentary.

**Important:** The comprehensive answer must be provided exclusively through the CompleteTask tool. Do not respond with the answer in chat; always use CompleteTask once ready."#;

pub struct ReconAgent;

impl ReconAgent {
    pub fn new() -> Self {
        Self
    }
}

impl crate::agents::agent::Agent for ReconAgent {
    fn name(&self) -> &str {
        "recon"
    }

    fn description(&self) -> &str {
        "Explores files and summarizes information about project structure, existing components, and relevant file locations to aid planning"
    }

    fn core_prompt(&self) -> &'static str {
        CORE_PROMPT
    }

    fn requested_prompt_components(&self) -> PromptComponentSelection {
        PromptComponentSelection::Only(&[tools::ID])
    }

    fn available_tools(&self) -> Vec<ToolName> {
        vec![
            SearchFilesTool::tool_name(),
            TrackedFilesManager::tool_name(),
            AskUserQuestion::tool_name(),
            CompleteTask::tool_name(),
            SearchTypesTool::tool_name(),
            GetTypeDocsTool::tool_name(),
        ]
    }
}
