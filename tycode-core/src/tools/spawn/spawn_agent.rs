use std::sync::Arc;

use crate::agents::catalog::AgentCatalog;
use crate::chat::events::{ToolExecutionResult, ToolRequest as ToolRequestEvent, ToolRequestType};
use crate::tools::r#trait::{
    ContinuationPreference, ToolCallHandle, ToolCategory, ToolExecutor, ToolOutput, ToolRequest,
};
use crate::tools::ToolName;
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

pub struct SpawnAgent {
    catalog: Arc<AgentCatalog>,
}

impl SpawnAgent {
    pub fn tool_name() -> ToolName {
        ToolName::new("spawn_agent")
    }

    pub fn new(catalog: Arc<AgentCatalog>) -> Self {
        Self { catalog }
    }
}

struct SpawnAgentHandle {
    catalog: Arc<AgentCatalog>,
    agent_type: String,
    task: String,
    tool_use_id: String,
}

#[async_trait::async_trait(?Send)]
impl ToolCallHandle for SpawnAgentHandle {
    fn tool_request(&self) -> ToolRequestEvent {
        ToolRequestEvent {
            tool_call_id: self.tool_use_id.clone(),
            tool_name: "spawn_agent".to_string(),
            tool_type: ToolRequestType::Other {
                args: json!({
                    "agent_type": self.agent_type,
                    "task": self.task
                }),
            },
        }
    }

    async fn execute(self: Box<Self>) -> ToolOutput {
        match self.catalog.create_agent(&self.agent_type) {
            Some(agent) => ToolOutput::PushAgent {
                agent,
                task: self.task,
            },
            None => ToolOutput::Result {
                content: format!("Unknown agent type: {}", self.agent_type),
                is_error: true,
                continuation: ContinuationPreference::Continue,
                ui_result: ToolExecutionResult::Error {
                    short_message: format!("Unknown agent: {}", self.agent_type),
                    detailed_message: format!(
                        "Agent type '{}' not found in catalog. Available: {:?}",
                        self.agent_type,
                        self.catalog.get_agent_names()
                    ),
                },
            },
        }
    }
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for SpawnAgent {
    fn name(&self) -> &'static str {
        "spawn_agent"
    }

    fn description(&self) -> &'static str {
        "Spawn a sub-agent to handle a specific task. The sub-agent starts with fresh context and runs to completion. Use this to break complex tasks into focused subtasks. WARNING: Never use this to work around failures - if you're a sub-agent and get stuck, use complete_task with failure instead to let the parent handle it."
    }

    fn input_schema(&self) -> Value {
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
                    "description": "Type of agent to spawn"
                }
            }
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Meta
    }

    async fn process(&self, request: &ToolRequest) -> Result<Box<dyn ToolCallHandle>> {
        let params: SpawnAgentParams = serde_json::from_value(request.arguments.clone())?;

        Ok(Box::new(SpawnAgentHandle {
            catalog: self.catalog.clone(),
            agent_type: params.agent_type,
            task: params.task,
            tool_use_id: request.tool_use_id.clone(),
        }))
    }
}
