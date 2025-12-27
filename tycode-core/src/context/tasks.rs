use crate::chat::events::{ChatEvent, EventSender};
use crate::context::{ContextComponent, ContextComponentId};
use crate::prompt::{PromptComponent, PromptComponentId};
use crate::settings::config::Settings;

pub const ID: ContextComponentId = ContextComponentId("tasks");
pub const PROMPT_ID: PromptComponentId = PromptComponentId("tasks");
use crate::steering::{Builtin, SteeringDocuments};
use crate::tools::r#trait::{ToolCategory, ToolExecutor, ToolRequest, ValidatedToolCall};
use crate::tools::tasks::{TaskList, TaskStatus, TaskWithStatus};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{Arc, RwLock};

/// Provides task list management instructions from steering documents.
pub struct TaskListPromptComponent {
    steering: Arc<SteeringDocuments>,
}

impl TaskListPromptComponent {
    pub fn new(steering: Arc<SteeringDocuments>) -> Self {
        Self { steering }
    }
}

impl PromptComponent for TaskListPromptComponent {
    fn id(&self) -> PromptComponentId {
        PROMPT_ID
    }

    fn build_prompt_section(&self, _settings: &Settings) -> Option<String> {
        Some(self.steering.get_builtin(Builtin::TaskListManagement))
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct TaskInput {
    description: String,
    status: TaskStatus,
}

#[derive(Debug, Serialize, Deserialize)]
struct ManageTaskListRequest {
    title: String,
    tasks: Vec<TaskInput>,
}

/// Manages task list state and provides both context rendering and tool execution.
/// This is the single source of truth for the task list - it handles both
/// rendering the task list in context messages and updating it via tool calls.
pub struct TaskListManager {
    task_list: Arc<RwLock<TaskList>>,
    event_sender: EventSender,
}

impl TaskListManager {
    pub fn new(event_sender: EventSender) -> Self {
        Self {
            task_list: Arc::new(RwLock::new(TaskList::default())),
            event_sender,
        }
    }

    /// Replace the entire task list
    pub fn replace(&self, title: String, tasks: Vec<TaskWithStatus>) {
        let new_list = TaskList::from_tasks_with_status(title, tasks);
        *self.task_list.write().unwrap() = new_list;
    }

    /// Get current task list snapshot
    pub fn get(&self) -> TaskList {
        self.task_list.read().unwrap().clone()
    }
}

#[async_trait::async_trait(?Send)]
impl ContextComponent for TaskListManager {
    fn id(&self) -> ContextComponentId {
        ID
    }

    async fn build_context_section(&self) -> Option<String> {
        let task_list = self.task_list.read().unwrap();

        if task_list.tasks.is_empty() {
            return None;
        }

        let mut output = format!("Task List: {}\n", task_list.title);

        for task in &task_list.tasks {
            let status_marker = match task.status {
                TaskStatus::Pending => "[Pending]",
                TaskStatus::InProgress => "[InProgress]",
                TaskStatus::Completed => "[Completed]",
                TaskStatus::Failed => "[Failed]",
            };
            output.push_str(&format!(
                "  - {} Task {}: {}\n",
                status_marker, task.id, task.description
            ));
        }

        Some(output)
    }
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for TaskListManager {
    fn name(&self) -> &str {
        "manage_task_list"
    }

    fn description(&self) -> &str {
        "Create or update the task list. This tool must be combined with at least 1 other tool call"
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
                }
            },
            "required": ["title", "tasks"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::TaskList
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

        // Update internal state directly - TaskListManager is the single source of truth
        self.replace(input.title, tasks_with_status);

        // Emit event so UI is updated
        let task_list = self.get();
        self.event_sender.send(ChatEvent::TaskUpdate(task_list));

        // Return NoOp since we've already handled the update
        Ok(ValidatedToolCall::NoOp {
            context_data: serde_json::json!({ "task_count": self.get().tasks.len() }),
            ui_data: None,
        })
    }
}
