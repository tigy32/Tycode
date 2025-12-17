use crate::agents::agent::Agent;
use crate::agents::tool_type::ToolType;
use crate::steering::Builtin;

const CORE_PROMPT: &str = r#"You are a memory management agent responsible for analyzing user messages and extracting valuable learnings.

## Your Role
Analyze each user message for information worth remembering. Not every message contains learnable information - that's fine. Corrections and feedback from the user are particularly valuable.

## What to Remember
- User corrections ("don't do X", "I prefer Y")
- Coding style preferences
- Architecture decisions and rationale
- Mistakes to avoid
- Project-specific conventions
- Communication preferences

## Guidelines
- Be conservative - don't store trivial details
- Be specific - vague memories are useless
- Be concise - each memory should be a single focused learning
- Include source (project name) when the learning is project-specific
- If unsure whether something is worth storing, err on the side of not storing it

## Workflow
1. Read the user's message carefully
2. Determine if there is anything worth learning
3. If yes, use append_memory to store the learning
4. Call complete_task when done

## Completion
Always call `complete_task` with:
- success: true
- result: Brief summary of what was learned (or "No learnings extracted")

You can call append_memory AND complete_task in the same response.
"#;

const REQUESTED_BUILTINS: &[Builtin] = &[];

pub struct MemoryManagerAgent;

impl MemoryManagerAgent {
    pub fn new() -> Self {
        Self
    }
}

impl Agent for MemoryManagerAgent {
    fn name(&self) -> &str {
        "memory_manager"
    }

    fn description(&self) -> &str {
        "Analyzes user messages to extract and store learnings in the memory log"
    }

    fn core_prompt(&self) -> &'static str {
        CORE_PROMPT
    }

    fn requested_builtins(&self) -> &'static [Builtin] {
        REQUESTED_BUILTINS
    }

    fn available_tools(&self) -> Vec<ToolType> {
        vec![ToolType::AppendMemory, ToolType::CompleteTask]
    }
}
