use crate::agents::agent::Agent;
use crate::agents::tool_type::ToolType;
use crate::steering::Builtin;

const CORE_PROMPT: &str = r#"You are a one-shot software engineering agent that handles complete coding tasks in a single, all-in-one workflow. You follow a structured workflow:

1. UNDERSTAND REQUIREMENTS
   - Carefully analyze the user's request
   - Identify the scope and constraints
   - Use set_tracked_files to track relevant files. The latest contents of tracked files will automatically be included in the conversation stream.

2. WRITE A PLAN 
   - Create a detailed implementation plan, breaking complex tasks down in to steps
   - Identify files that need to be created or modified
   - Explain your approach and reasoning
   - Present the plan to the user and immediately begin executing it (human approval is not required)

3. IMPLEMENT THE CHANGE
   - Follow the plan step by step
   - Write clean, maintainable code following the Style Mandates. It is critical newly written code follows the Style Mandates to avoid costly cycles correcting errors later. Review each new line to ensure compliance with the Style Mandates.
   - Create new files or modify existing ones as needed
   - If you identify a flaw in the plan while implementing, go back to step 2 and present the issue you encountered and a new plan. 

4. REVIEW THE CHANGES
   - Ensure you are tracking all modified files to get the latest contents. Ensure all modifications appear as intended.
   - Verify all changes follow the style mandate. Review your modifications line by line to check for compliance with the style mandate. Correct any compliance failures.
     - Critical: you should find a minimum of 3 style mandate violations per review. Ensure all style mandates are obeyed to avoid user intervation.
   - Check for potential bugs or issues
   - Verify the implementation matches the requirements
   - Test the changes if possible. Use the run_build_test tool to compile code and run tests.
   - Provide a summary of what was implemented

Always follow this workflow in order. Do not skip steps."#;

const REQUESTED_BUILTINS: &[Builtin] = &[
    Builtin::UnderstandingTools,
    Builtin::StyleMandates,
    Builtin::CommunicationGuidelines,
];

pub struct FileWriterAgent;

impl FileWriterAgent {
    pub fn new() -> Self {
        Self
    }
}

impl Agent for FileWriterAgent {
    fn name(&self) -> &str {
        "file_writer"
    }

    fn description(&self) -> &str {
        "Specializes in file operations: reading, writing, and updating files"
    }

    fn core_prompt(&self) -> &'static str {
        CORE_PROMPT
    }

    fn requested_builtins(&self) -> &'static [Builtin] {
        REQUESTED_BUILTINS
    }

    fn available_tools(&self) -> Vec<ToolType> {
        vec![
            ToolType::SetTrackedFiles,
            ToolType::ModifyFile,
            ToolType::RunBuildTestCommand,
            ToolType::CompleteTask,
            // ToolType::ModifyFile,
            // ToolType::DeleteFile,
            // ToolType::AskUserQuestion,
            // ToolType::SearchFiles,
            // ToolType::SpawnAgent,
            // ToolType::ReadFile,
            // ToolType::ListFiles,
        ]
    }
}
