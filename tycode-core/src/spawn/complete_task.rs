//! Complete task tool - signals task completion and pops the agent stack.

use std::sync::Arc;

use crate::chat::events::{ToolRequest as ToolRequestEvent, ToolRequestType};
use crate::tools::r#trait::{ToolCallHandle, ToolCategory, ToolExecutor, ToolOutput, ToolRequest};
use crate::tools::ToolName;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::AgentStack;

#[derive(Debug, Serialize, Deserialize)]
struct CompleteTaskParams {
    result: String,
    success: bool,
}

pub struct CompleteTask {
    agent_stack: AgentStack,
}

impl CompleteTask {
    pub fn tool_name() -> ToolName {
        ToolName::new("complete_task")
    }

    pub fn new(agent_stack: AgentStack) -> Self {
        Self { agent_stack }
    }

    /// Create a standalone CompleteTask that doesn't track agent stack.
    /// Use this for contexts that don't participate in spawn tracking
    /// (e.g., memory manager agent, background tasks).
    pub fn standalone() -> Self {
        Self {
            agent_stack: Arc::new(std::sync::RwLock::new(Vec::new())),
        }
    }
}

struct CompleteTaskHandle {
    agent_stack: AgentStack,
    success: bool,
    result: String,
    tool_use_id: String,
}

#[async_trait::async_trait(?Send)]
impl ToolCallHandle for CompleteTaskHandle {
    fn tool_request(&self) -> ToolRequestEvent {
        ToolRequestEvent {
            tool_call_id: self.tool_use_id.clone(),
            tool_name: "complete_task".to_string(),
            tool_type: ToolRequestType::Other {
                args: json!({
                    "success": self.success,
                    "result": self.result,
                }),
            },
        }
    }

    async fn execute(self: Box<Self>) -> ToolOutput {
        // Pop the current agent from the stack (we're completing)
        if let Ok(mut stack) = self.agent_stack.write() {
            stack.pop();
        }

        ToolOutput::PopAgent {
            success: self.success,
            result: self.result,
        }
    }
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for CompleteTask {
    fn name(&self) -> String {
        "complete_task".to_string()
    }

    fn description(&self) -> String {
        "Signal task completion (success or failure) and return control to parent agent. \
         FAIL a task when: \
         • Required resources/files don't exist \
         • The task requirements are unclear or contradictory \
         • You encounter errors you cannot resolve \
         • The requested change would break existing functionality \
         • You lack necessary permissions or access \
         SUCCEED when: \
         • All requested changes are implemented \
         • The task objectives are met \
         NOTE: Sub-agents must use this with failure instead of spawning more agents when stuck. \
         Parent agents have more context to handle failures properly."
            .to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["result", "success"],
            "properties": {
                "result": {
                    "type": "string",
                    "description": "Result of the task - summary of what was accomplished, failure details, code outline, or any other output"
                },
                "success": {
                    "type": "boolean",
                    "description": "Whether the task completed successfully"
                }
            }
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Meta
    }

    async fn process(&self, request: &ToolRequest) -> Result<Box<dyn ToolCallHandle>> {
        let params: CompleteTaskParams = serde_json::from_value(request.arguments.clone())?;

        Ok(Box::new(CompleteTaskHandle {
            agent_stack: self.agent_stack.clone(),
            success: params.success,
            result: params.result,
            tool_use_id: request.tool_use_id.clone(),
        }))
    }
}
