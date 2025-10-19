use super::TaskListOp;
use crate::tools::r#trait::{ToolCategory, ToolExecutor, ToolRequest, ValidatedToolCall};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Serialize, Deserialize)]
pub struct ProposeTaskListRequest {
    pub tasks: Vec<String>,
}

pub struct ProposeTaskListTool;

#[async_trait::async_trait(?Send)]
impl ToolExecutor for ProposeTaskListTool {
    fn name(&self) -> &str {
        "propose_task_list"
    }

    fn description(&self) -> &str {
        "Create a new task list for this session. Replaces any existing task list."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "tasks": {
                    "type": "array",
                    "items": {
                        "type": "string"
                    },
                    "description": "List of task descriptions"
                }
            },
            "required": ["tasks"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::ControlFlow
    }

    async fn validate(&self, request: &ToolRequest) -> Result<ValidatedToolCall> {
        let input: ProposeTaskListRequest = serde_json::from_value(request.arguments.clone())?;

        if input.tasks.is_empty() {
            return Ok(ValidatedToolCall::Error(
                "Task list cannot be empty".to_string(),
            ));
        }

        Ok(ValidatedToolCall::PerformTaskListOp(TaskListOp::Create(
            input.tasks,
        )))
    }
}
