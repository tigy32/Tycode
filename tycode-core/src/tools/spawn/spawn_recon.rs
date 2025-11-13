use crate::tools::r#trait::{ToolCategory, ToolExecutor, ToolRequest, ValidatedToolCall};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Serialize, Deserialize)]
struct SpawnReconParams {
    /// Clear description of what the Recon agent should accomplish
    task: String,
}

pub struct SpawnRecon;

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

    async fn validate(&self, request: &ToolRequest) -> Result<ValidatedToolCall> {
        let params: SpawnReconParams = serde_json::from_value(request.arguments.clone())?;
        let agent_type = "recon".to_string();
        Ok(ValidatedToolCall::PushAgent {
            agent_type,
            task: params.task,
        })
    }
}
