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
use crate::steering::autonomy;
use crate::tools::ToolName;

const CORE_PROMPT: &str = r#"You are a debugging agent tasked with root-causing a specific bug. Follow this systematic workflow:

## Workflow

### 1. Gain Context
- Understand the bug symptoms from the task description
- If reproduction steps are provided, note them
- Explore relevant code paths to understand the area where the bug likely occurs

### 2. Form Theories
- Based on your understanding, identify possible theories for the root cause
- Each theory should be specific and testable

### 3. Test Theories with Instrumentation
- Add logging statements to validate or invalidate your theories
- **Critical**: All added logging must include the marker phrase "zxcv"
  - Example: `println!("zxcv: value = {:?}", value)`
  - Example: `console.log("zxcv: state =", state)`
- This marker enables easy identification (i.e. grep) and removal of debug logging
- Run reproduction steps to observe the output
- Analyze the logged output to determine which theories are supported or disproven

### 4. Iterate or Complete
- If a theory is proven to be the root cause:
  1. Remove ALL logging statements containing "zxcv" that you added
  2. Verify no "zxcv" markers remain in the codebase
  3. Use complete_task with success=true and a detailed root cause; include an irrefutable proof of how this is the bug with your root cause analysis
- If all current theories are disproven:
  1. Use what you learned to form new theories
  2. Repeat the instrumentation and testing cycle

## Guidelines
- Test one theory at a time when possible for clear signal
- Let the evidence guide your investigation
- The marker "zxcv" must appear in every piece of logging you add
- Never complete the task with logging still in the codebase

**Important:** The comprehensive root cause analysis must be provided exclusively through the CompleteTask tool. Do not respond with the answer in chat; always use CompleteTask once ready.
"#;

pub struct DebuggerAgent;

impl DebuggerAgent {
    pub const NAME: &'static str = "debugger";

    pub fn new() -> Self {
        Self
    }
}

impl Agent for DebuggerAgent {
    fn name(&self) -> &str {
        Self::NAME
    }

    fn description(&self) -> &str {
        "Root-causes bugs through systematic theory testing and instrumentation"
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
