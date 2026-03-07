use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::agents::catalog::AgentCatalog;
use crate::chat::events::{ToolExecutionResult, ToolRequest as ToolRequestEvent, ToolRequestType};
use crate::module::SpawnParameter;
use crate::tools::r#trait::{
    ContinuationPreference, ToolCallHandle, ToolCategory, ToolExecutor, ToolOutput, ToolRequest,
};
use crate::tools::ToolName;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Serialize, Deserialize)]
struct SpawnAgentParams {
    task: String,
    agent_type: String,
}

pub struct SpawnAgent {
    catalog: Arc<AgentCatalog>,
    allowed_agents: HashSet<String>,
    current_agent: String,
    spawn_params: Vec<SpawnParameter>,
}

impl SpawnAgent {
    pub fn tool_name() -> ToolName {
        ToolName::new("spawn_agent")
    }

    pub fn new(
        catalog: Arc<AgentCatalog>,
        allowed_agents: HashSet<String>,
        current_agent: String,
        spawn_params: Vec<SpawnParameter>,
    ) -> Self {
        Self {
            catalog,
            allowed_agents,
            current_agent,
            spawn_params,
        }
    }
}

struct SpawnAgentHandle {
    catalog: Arc<AgentCatalog>,
    allowed_agents: HashSet<String>,
    current_agent: String,
    agent_type: String,
    task: String,
    tool_use_id: String,
    spawn_params: HashMap<String, Value>,
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
        if self.current_agent == self.agent_type {
            return ToolOutput::Result {
                content: format!(
                    "Cannot spawn agent of type '{}' from the same agent type. Use complete_task with failure instead.",
                    self.agent_type
                ),
                is_error: true,
                continuation: ContinuationPreference::Continue,
                ui_result: ToolExecutionResult::Error {
                    short_message: format!("Cannot spawn self ({})", self.agent_type),
                    detailed_message: format!(
                        "Agent '{}' cannot spawn another '{}'. Use complete_task with failure instead.",
                        self.agent_type, self.agent_type
                    ),
                },
            };
        }

        if !self.allowed_agents.contains(&self.agent_type) {
            return ToolOutput::Result {
                content: format!(
                    "Agent type '{}' not allowed. Allowed types: {:?}",
                    self.agent_type, self.allowed_agents
                ),
                is_error: true,
                continuation: ContinuationPreference::Continue,
                ui_result: ToolExecutionResult::Error {
                    short_message: format!("Agent type '{}' not allowed", self.agent_type),
                    detailed_message: format!(
                        "Cannot spawn '{}'. Allowed agent types: {:?}",
                        self.agent_type, self.allowed_agents
                    ),
                },
            };
        }

        match self.catalog.create_agent(&self.agent_type) {
            Some(agent) => ToolOutput::PushAgent {
                agent,
                task: self.task,
                spawn_params: self.spawn_params,
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
    fn name(&self) -> String {
        "spawn_agent".to_string()
    }

    fn description(&self) -> String {
        let mut agents: Vec<&str> = self.allowed_agents.iter().map(|s| s.as_str()).collect();
        agents.sort();
        format!(
            "Spawn a sub-agent to handle a specific task. Available agent types: {}. The sub-agent starts with fresh context and runs to completion. Use this to break complex tasks into focused subtasks. WARNING: Never use this to work around failures - if you're a sub-agent and get stuck, use complete_task with failure instead to let the parent handle it.",
            agents.join(", ")
        )
    }

    fn input_schema(&self) -> Value {
        let mut properties = json!({
            "task": {
                "type": "string",
                "description": "Clear, specific description of what the sub-agent should accomplish. Include any relevant context, constraints, or guidance."
            },
            "agent_type": {
                "type": "string",
                "description": "Type of agent to spawn"
            }
        });

        let mut required = vec!["task", "agent_type"];

        for param in &self.spawn_params {
            if let serde_json::Value::Object(ref mut props) = properties {
                props.insert(param.name.to_string(), param.schema.clone());
            }
            if param.required {
                required.push(param.name);
            }
        }

        json!({
            "type": "object",
            "required": required,
            "properties": properties
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Meta
    }

    async fn process(&self, request: &ToolRequest) -> Result<Box<dyn ToolCallHandle>> {
        let params: SpawnAgentParams = serde_json::from_value(request.arguments.clone())?;

        Ok(Box::new(SpawnAgentHandle {
            catalog: self.catalog.clone(),
            allowed_agents: self.allowed_agents.clone(),
            current_agent: self.current_agent.clone(),
            agent_type: params.agent_type,
            task: params.task,
            tool_use_id: request.tool_use_id.clone(),
            spawn_params: SpawnAgentHandle::extract_spawn_params(
                &request.arguments,
                &self.spawn_params,
            ),
        }))
    }
}

impl SpawnAgentHandle {
    fn extract_spawn_params(
        arguments: &Value,
        spawn_params: &[SpawnParameter],
    ) -> HashMap<String, Value> {
        let mut extracted = HashMap::new();

        for param in spawn_params {
            if let Some(value) = arguments.get(param.name) {
                extracted.insert(param.name.to_string(), value.clone());
            }
        }

        extracted
    }
}
