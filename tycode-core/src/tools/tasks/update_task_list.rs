use super::{TaskListOp, TaskStatus};
use crate::tools::r#trait::{ToolCategory, ToolExecutor, ToolRequest, ValidatedToolCall};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Serialize, Deserialize)]
pub struct UpdateTaskListRequest {
    pub task_number: usize,
    pub status: String,
}

pub struct UpdateTaskListTool;

#[async_trait::async_trait(?Send)]
impl ToolExecutor for UpdateTaskListTool {
    fn name(&self) -> &str {
        "update_task_list"
    }

    fn description(&self) -> &str {
        "Update the status of a task. Provide the task number and new status (pending, in_progress, completed, or failed)."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "task_number": {
                    "type": "integer",
                    "description": "The task number (0-based index)"
                },
                "status": {
                    "type": "string",
                    "enum": ["pending", "in_progress", "completed", "failed"],
                    "description": "The new status for the task"
                }
            },
            "required": ["task_number", "status"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::ControlFlow
    }

    async fn validate(&self, request: &ToolRequest) -> Result<ValidatedToolCall> {
        let input: UpdateTaskListRequest = serde_json::from_value(request.arguments.clone())?;

        let status = TaskStatus::from_str(&input.status)
            .ok_or_else(|| anyhow::anyhow!("Invalid status: {}", input.status))?;

        Ok(ValidatedToolCall::PerformTaskListOp(
            TaskListOp::UpdateStatus {
                task_id: input.task_number,
                status,
            },
        ))
    }
}
