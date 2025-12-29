use crate::chat::events::{ToolRequest as ToolRequestEvent, ToolRequestType};
use crate::tools::r#trait::{ToolCallHandle, ToolCategory, ToolExecutor, ToolOutput, ToolRequest};
use anyhow::{bail, Result};
use serde_json::{json, Value};

pub struct AskUserQuestion;

struct AskUserQuestionHandle {
    question: String,
    tool_use_id: String,
}

#[async_trait::async_trait(?Send)]
impl ToolCallHandle for AskUserQuestionHandle {
    fn tool_request(&self) -> ToolRequestEvent {
        ToolRequestEvent {
            tool_call_id: self.tool_use_id.clone(),
            tool_name: "ask_user_question".to_string(),
            tool_type: ToolRequestType::Other {
                args: json!({ "question": self.question }),
            },
        }
    }

    async fn execute(self: Box<Self>) -> ToolOutput {
        ToolOutput::PromptUser {
            question: self.question,
        }
    }
}

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

    fn category(&self) -> ToolCategory {
        ToolCategory::Meta
    }

    async fn process(&self, request: &ToolRequest) -> Result<Box<dyn ToolCallHandle>> {
        let Some(question) = request.arguments["question"].as_str() else {
            bail!("Missing required argument \"question\"");
        };

        Ok(Box::new(AskUserQuestionHandle {
            question: question.to_string(),
            tool_use_id: request.tool_use_id.clone(),
        }))
    }
}
