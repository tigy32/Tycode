use crate::security::types::RiskLevel;
use crate::tools::r#trait::{ToolExecutor, ToolRequest, ToolResult};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Serialize, Deserialize)]
struct SpawnReconParams {
    /// Clear description of what the Recon agent should accomplish
    task: String,
    /// Relevant context, constraints, or guidance for the Recon agent
    context: Option<String>,
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
                    "description": "Clear, specific description of the information to gather"
                },
            }
        })
    }

    fn evaluate_risk(&self, _arguments: &Value) -> RiskLevel {
        RiskLevel::ReadOnly
    }

    async fn validate(&self, request: &ToolRequest) -> Result<ToolResult> {
        let params: SpawnReconParams = serde_json::from_value(request.arguments.clone())?;
        let agent_type = "recon".to_string();
        Ok(ToolResult::PushAgent {
            agent_type,
            task: params.task,
            context: params.context,
        })
    }
}
