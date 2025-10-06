use crate::agents::catalog::AgentCatalog;
use crate::tools::r#trait::{ToolExecutor, ToolRequest, ValidatedToolCall};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Serialize, Deserialize)]
struct SpawnAgentParams {
    /// Clear description of what the sub-agent should accomplish
    task: String,
    /// Relevant context, constraints, or guidance for the sub-agent
    context: Option<String>,
    /// Type of agent to spawn (optional - defaults to appropriate type based on task)
    agent_type: Option<String>,
}

pub struct SpawnAgent;

#[async_trait::async_trait(?Send)]
impl ToolExecutor for SpawnAgent {
    fn name(&self) -> &'static str {
        "spawn_agent"
    }

    fn description(&self) -> &'static str {
        "Spawn a sub-agent to handle a specific task. The sub-agent starts with fresh context and runs to completion. Use this to break complex tasks into focused subtasks. WARNING: Never use this to work around failures - if you're a sub-agent and get stuck, use complete_task with failure instead to let the parent handle it."
    }

    fn input_schema(&self) -> Value {
        let agent_names = AgentCatalog::get_agent_names();
        let agent_descriptions = AgentCatalog::get_agent_descriptions();

        json!({
            "type": "object",
            "required": ["task"],
            "properties": {
                "task": {
                    "type": "string",
                    "description": "Clear, specific description of what the sub-agent should accomplish"
                },
                "context": {
                    "type": "string",
                    "description": "Any relevant context, constraints, or guidance for the sub-agent"
                },
                "agent_type": {
                    "type": "string",
                    "description": format!("Type of agent to spawn. Available agents: {}", agent_descriptions),
                    "enum": agent_names
                }
            }
        })
    }

    async fn validate(&self, request: &ToolRequest) -> Result<ValidatedToolCall> {
        let params: SpawnAgentParams = serde_json::from_value(request.arguments.clone())?;

        // Determine agent type, default to software_engineer
        let Some(agent_type) = params.agent_type else {
            return Ok(ValidatedToolCall::Error(
                "Missing requied parameter agent_type".to_string(),
            ));
        };

        // Return PushAgent variant - actor will handle the actual push
        Ok(ValidatedToolCall::PushAgent {
            agent_type,
            task: params.task,
            context: params.context,
        })
    }
}
