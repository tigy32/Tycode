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
struct SpawnReconParams {
    /// Clear description of what the Recon agent should accomplish
    task: String,
}

pub struct SpawnRecon {
    catalog: Arc<AgentCatalog>,
}

impl SpawnRecon {
    pub fn new(catalog: Arc<AgentCatalog>) -> Self {
        Self { catalog }
    }
}

struct SpawnReconHandle {
    catalog: Arc<AgentCatalog>,
    task: String,
    tool_use_id: String,
}

#[async_trait::async_trait(?Send)]
impl ToolCallHandle for SpawnReconHandle {
    fn tool_request(&self) -> ToolRequestEvent {
        ToolRequestEvent {
            tool_call_id: self.tool_use_id.clone(),
            tool_name: "spawn_recon".to_string(),
            tool_type: ToolRequestType::Other {
                args: json!({ "task": self.task }),
            },
        }
    }

    async fn execute(self: Box<Self>) -> ToolOutput {
        match self.catalog.create_agent("recon") {
            Some(agent) => ToolOutput::PushAgent {
                agent,
                task: self.task,
            },
            None => ToolOutput::Result {
                content: "Recon agent not available".to_string(),
                is_error: true,
                continuation: ContinuationPreference::Continue,
                ui_result: ToolExecutionResult::Error {
                    short_message: "Recon unavailable".to_string(),
                    detailed_message: "Recon agent type not found in catalog".to_string(),
                },
            },
        }
    }
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for SpawnRecon {
    fn name(&self) -> &'static str {
        "spawn_recon"
    }

    fn description(&self) -> &'static str {
        "Spawn the Recon agent to gather specific information from project files. Provide a clear query, e.g., 'Find all files using BubbleSort' or 'Describe the public interface of DataRow'. The Recon agent will use file exploration tools and deliver results via CompleteTask. Use this for focused information retrieval tasks that require exploring and summarizing project data."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["task"],
            "properties": {
                "task": {
                    "type": "string",
                    "description": "Clear, specific description of the information to gather. Include any relevant context, constraints, or guidance."
                },
            }
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Meta
    }

    async fn process(&self, request: &ToolRequest) -> Result<Box<dyn ToolCallHandle>> {
        let params: SpawnReconParams = serde_json::from_value(request.arguments.clone())?;

        Ok(Box::new(SpawnReconHandle {
            catalog: self.catalog.clone(),
            task: params.task,
            tool_use_id: request.tool_use_id.clone(),
        }))
    }
}
