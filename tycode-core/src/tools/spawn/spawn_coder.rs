use crate::tools::r#trait::{ToolCategory, ToolExecutor, ToolRequest, ValidatedToolCall};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Serialize, Deserialize)]
struct SpawnCoderParams {
    task: String,
}

pub struct SpawnCoder;

#[async_trait::async_trait(?Send)]
impl ToolExecutor for SpawnCoder {
    fn name(&self) -> &'static str {
        "spawn_coder"
    }

    fn description(&self) -> &'static str {
        "Spawn a coder sub-agent to handle a specific coding task. The sub-agent starts with fresh context and runs to completion. Use this to break complex tasks into focused coding subtasks."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["task"],
            "properties": {
                "task": {
                    "type": "string",
                    "description": "Clear, specific description of what the coder should accomplish. Include any relevant context, constraints, or guidance."
                }
            }
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Meta
    }

    async fn validate(&self, request: &ToolRequest) -> Result<ValidatedToolCall> {
        let params: SpawnCoderParams = serde_json::from_value(request.arguments.clone())?;

        Ok(ValidatedToolCall::PushAgent {
            agent_type: "coder".to_string(),
            task: params.task,
        })
    }
}
