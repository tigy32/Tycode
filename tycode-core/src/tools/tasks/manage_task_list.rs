use super::{TaskListOp, TaskWithStatus};
use crate::tools::r#trait::{ToolCategory, ToolExecutor, ToolRequest, ValidatedToolCall};
use crate::tools::tasks::TaskStatus;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Serialize, Deserialize)]
pub struct TaskInput {
    pub description: String,
    pub status: TaskStatus,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ManageTaskListRequest {
    pub title: String,
    pub tasks: Vec<TaskInput>,
}

pub struct ManageTaskListTool;

#[async_trait::async_trait(?Send)]
impl ToolExecutor for ManageTaskListTool {
    fn name(&self) -> &str {
        "manage_task_list"
    }

    fn description(&self) -> &str {
        "Create or update the task list. This tool must be combined with meaningful work in the same turn - either presenting a plan, making progress with other tools, or providing a substantial summary."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "title": {
                    "type": "string",
                    "description": "Title for the task list (≤50 characters) describing the current work"
                },
                "tasks": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "description": {
                                "type": "string",
                                "description": "Task description"
                            },
                            "status": {
                                "type": "string",
                                "enum": ["pending", "in_progress", "completed", "failed"],
                                "description": "Current task status"
                            }
                        },
                        "required": ["description", "status"],
                        "additionalProperties": false
                    },
                    "description": "Complete list of tasks with current status"
                },
            },
            "required": ["title", "tasks"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::AlwaysAllowed
    }

    async fn validate(&self, request: &ToolRequest) -> Result<ValidatedToolCall> {
        let input: ManageTaskListRequest = serde_json::from_value(request.arguments.clone())?;

        if input.tasks.is_empty() {
            return Ok(ValidatedToolCall::Error(
                "Task list cannot be empty".to_string(),
            ));
        }

        let tasks_with_status: Vec<TaskWithStatus> = input
            .tasks
            .into_iter()
            .map(|task| TaskWithStatus {
                description: task.description,
                status: task.status,
            })
            .collect();

        Ok(ValidatedToolCall::PerformTaskListOp(TaskListOp::Replace {
            title: input.title,
            tasks: tasks_with_status,
        }))
    }
}
