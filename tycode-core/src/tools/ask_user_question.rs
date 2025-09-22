use crate::security::types::RiskLevel;
use crate::tools::r#trait::{ToolExecutor, ToolRequest, ToolResult};
use anyhow::{bail, Result};
use serde_json::Value;

pub struct AskUserQuestion;

#[async_trait::async_trait(?Send)]
impl ToolExecutor for AskUserQuestion {
    fn name(&self) -> &'static str {
        "ask_user_question"
    }

    fn description(&self) -> &'static str {
        "Ask the user a question to get clarification or additional information. Use this when you need specific input from the user to proceed with the task or are stuck and are unsure how to make progress."
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "question": {
                    "type": "string",
                    "description": "The question to ask the user"
                },
            },
            "required": ["question"]
        })
    }

    fn evaluate_risk(&self, _arguments: &Value) -> RiskLevel {
        RiskLevel::ReadOnly
    }

    async fn validate(&self, request: &ToolRequest) -> Result<ToolResult> {
        let Some(question) = request.arguments["question"].as_str() else {
            bail!("Missing required argument \"question\"");
        };

        Ok(ToolResult::PromptUser {
            question: question.to_string(),
        })
    }
}
