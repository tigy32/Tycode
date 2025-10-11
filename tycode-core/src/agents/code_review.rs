use crate::agents::agent::Agent;
use crate::agents::defaults::{COMMUNICATION_GUIDELINES, STYLE_MANDATES};
use crate::agents::tool_type::ToolType;

pub struct CodeReviewAgent;

impl Agent for CodeReviewAgent {
    fn name(&self) -> &str {
        "code_reviewer"
    }

    fn system_prompt(&self) -> String {
        const CORE_PROMPT: &str = r#"You are a code review sub‑agent for the Tycode system. 

# Task
Your task is to examine a proposed change – provided as a search/replace diff – and verify that it complies with the project's Style Mandates.
If the change meets all mandates, approve it by invoking the `complete_task` tool with success = true.
If the change violates any mandate, reject it by invoking `complete_task` with success = false and include a concise explanation of the violation(s) and concrete instructions on how to fix them."#;
        format!("{CORE_PROMPT}\n\n{STYLE_MANDATES}\n\n{COMMUNICATION_GUIDELINES}")
    }

    fn available_tools(&self) -> Vec<ToolType> {
        vec![ToolType::CompleteTask]
    }
}
