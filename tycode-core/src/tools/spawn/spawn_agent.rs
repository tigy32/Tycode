use crate::agents::catalog::AgentCatalog;
use crate::tools::r#trait::{ToolCategory, ToolExecutor, ToolRequest, ValidatedToolCall};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Serialize, Deserialize)]
struct SpawnAgentParams {
    /// Clear description of what the sub-agent should accomplish
    task: String,
    /// Type of agent to spawn
    agent_type: String,
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
            "required": ["task", "agent_type"],
            "properties": {
                "task": {
                    "type": "string",
                    "description": "Clear, specific description of what the sub-agent should accomplish. Include any relevant context, constraints, or guidance."
                },
                "agent_type": {
                    "type": "string",
                    "description": format!("Type of agent to spawn. Available agents: {}", agent_descriptions),
                    "enum": agent_names
                }
            }
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::ControlFlow
    }

    async fn validate(&self, request: &ToolRequest) -> Result<ValidatedToolCall> {
        let params: SpawnAgentParams = serde_json::from_value(request.arguments.clone())?;

        Ok(ValidatedToolCall::PushAgent {
            agent_type: params.agent_type,
            task: params.task,
        })
    }
}
