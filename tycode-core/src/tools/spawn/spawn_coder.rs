use std::sync::Arc;

use crate::agents::catalog::AgentCatalog;
use crate::chat::events::{ToolExecutionResult, ToolRequest as ToolRequestEvent, ToolRequestType};
use crate::tools::r#trait::{
    ContinuationPreference, ToolCallHandle, ToolCategory, ToolExecutor, ToolOutput, ToolRequest,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Serialize, Deserialize)]
struct SpawnCoderParams {
    task: String,
}

pub struct SpawnCoder {
    catalog: Arc<AgentCatalog>,
}

impl SpawnCoder {
    pub fn new(catalog: Arc<AgentCatalog>) -> Self {
        Self { catalog }
    }
}

struct SpawnCoderHandle {
    catalog: Arc<AgentCatalog>,
    task: String,
    tool_use_id: String,
}

#[async_trait::async_trait(?Send)]
impl ToolCallHandle for SpawnCoderHandle {
    fn tool_request(&self) -> ToolRequestEvent {
        ToolRequestEvent {
            tool_call_id: self.tool_use_id.clone(),
            tool_name: "spawn_coder".to_string(),
            tool_type: ToolRequestType::Other {
                args: json!({ "task": self.task }),
            },
        }
    }

    async fn execute(self: Box<Self>) -> ToolOutput {
        match self.catalog.create_agent("coder") {
            Some(agent) => ToolOutput::PushAgent {
                agent,
                task: self.task,
            },
            None => ToolOutput::Result {
                content: "Failed to create coder agent".to_string(),
                is_error: true,
                continuation: ContinuationPreference::Continue,
                ui_result: ToolExecutionResult::Error {
                    short_message: "Agent creation failed".to_string(),
                    detailed_message: "Failed to create coder agent".to_string(),
                },
            },
        }
    }
}

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

    async fn process(&self, request: &ToolRequest) -> Result<Box<dyn ToolCallHandle>> {
        let params: SpawnCoderParams = serde_json::from_value(request.arguments.clone())?;

        Ok(Box::new(SpawnCoderHandle {
            catalog: self.catalog.clone(),
            task: params.task,
            tool_use_id: request.tool_use_id.clone(),
        }))
    }
}
