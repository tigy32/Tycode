use std::sync::{Arc, RwLock};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::chat::events::{
    ChatEvent, EventSender, ToolExecutionResult, ToolRequest as ToolRequestEvent, ToolRequestType,
};
use crate::context::{ContextComponent, ContextComponentId};
use crate::tools::r#trait::{
    ContinuationPreference, ToolCallHandle, ToolCategory, ToolExecutor, ToolOutput, ToolRequest,
};

/// Module that owns task list state and provides tools + context component
pub struct TaskListModule {
    inner: Arc<TaskListModuleInner>,
}

pub(crate) struct TaskListModuleInner {
    pub(crate) task_list: RwLock<TaskList>,
    pub(crate) event_sender: EventSender,
}

impl TaskListModule {
    pub fn new(event_sender: EventSender) -> Self {
        Self {
            inner: Arc::new(TaskListModuleInner {
                task_list: RwLock::new(TaskList::default()),
                event_sender,
            }),
        }
    }

    pub fn propose_tool(&self) -> Arc<dyn ToolExecutor> {
        Arc::new(ProposeTaskListTool {
            inner: self.inner.clone(),
        })
    }

    pub fn update_tool(&self) -> Arc<dyn ToolExecutor> {
        Arc::new(UpdateTaskListTool {
            inner: self.inner.clone(),
        })
    }

    pub fn context_component(&self) -> Arc<dyn ContextComponent + Send + Sync> {
        Arc::new(TaskListContextComponent {
            inner: self.inner.clone(),
        })
    }

    pub fn get(&self) -> TaskList {
        self.inner.get()
    }

    pub fn replace(&self, title: String, tasks: Vec<TaskWithStatus>) {
        self.inner.replace(title, tasks);
    }
}

impl TaskListModuleInner {
    pub(crate) fn replace(&self, title: String, tasks: Vec<TaskWithStatus>) {
        let new_list = TaskList::from_tasks_with_status(title, tasks);
        *self.task_list.write().unwrap() = new_list;
        self.emit_update();
    }

    pub(crate) fn update_status(&self, task_id: usize, status: TaskStatus) {
        {
            let mut task_list = self.task_list.write().unwrap();
            if let Some(task) = task_list.tasks.iter_mut().find(|t| t.id == task_id) {
                task.status = status;
            }
        }
        self.emit_update();
    }

    pub(crate) fn get(&self) -> TaskList {
        self.task_list.read().unwrap().clone()
    }

    fn emit_update(&self) {
        self.event_sender.send(ChatEvent::TaskUpdate(self.get()));
    }
}

pub const TASK_LIST_CONTEXT_ID: ContextComponentId = ContextComponentId("tasks");

struct TaskListContextComponent {
    inner: Arc<TaskListModuleInner>,
}

#[async_trait::async_trait(?Send)]
impl ContextComponent for TaskListContextComponent {
    fn id(&self) -> ContextComponentId {
        TASK_LIST_CONTEXT_ID
    }

    async fn build_context_section(&self) -> Option<String> {
        let task_list = self.inner.task_list.read().unwrap();

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskWithStatus {
    pub description: String,
    pub status: TaskStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: usize,
    pub description: String,
    pub status: TaskStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskList {
    pub title: String,
    pub tasks: Vec<Task>,
}

impl TaskList {
    pub fn from_tasks_with_status(title: String, tasks_with_status: Vec<TaskWithStatus>) -> Self {
        let tasks = tasks_with_status
            .into_iter()
            .enumerate()
            .map(|(id, task)| Task {
                id,
                description: task.description,
                status: task.status,
            })
            .collect();

        Self { title, tasks }
    }
}

impl Default for TaskList {
    fn default() -> Self {
        Self {
            title: "Understand user requirements".to_string(),
            tasks: vec![
                Task {
                    id: 0,
                    description: "Await user request".to_string(),
                    status: TaskStatus::InProgress,
                },
                Task {
                    id: 1,
                    description:
                        "Understand/Explore the code base and propose a comprehensive plan"
                            .to_string(),
                    status: TaskStatus::Pending,
                },
            ],
        }
    }
}

// ============================================================================
// ProposeTaskListTool
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
struct ProposeTaskListRequest {
    title: String,
    tasks: Vec<String>,
}

pub struct ProposeTaskListTool {
    pub(crate) inner: Arc<TaskListModuleInner>,
}

struct ProposeTaskListHandle {
    title: String,
    tasks: Vec<TaskWithStatus>,
    tool_use_id: String,
    inner: Arc<TaskListModuleInner>,
}

#[async_trait::async_trait(?Send)]
impl ToolCallHandle for ProposeTaskListHandle {
    fn tool_request(&self) -> ToolRequestEvent {
        ToolRequestEvent {
            tool_call_id: self.tool_use_id.clone(),
            tool_name: "propose_task_list".to_string(),
            tool_type: ToolRequestType::Other {
                args: json!({ "title": self.title, "tasks": self.tasks }),
            },
        }
    }

    async fn execute(self: Box<Self>) -> ToolOutput {
        let task_count = self.tasks.len();
        self.inner.replace(self.title, self.tasks);

        ToolOutput::Result {
            content: json!({ "task_count": task_count }).to_string(),
            is_error: false,
            continuation: ContinuationPreference::Continue,
            ui_result: ToolExecutionResult::Other {
                result: json!({ "task_count": task_count }),
            },
        }
    }
}

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
                "title": {
                    "type": "string",
                    "description": "Title for the task list (≤50 characters) describing the current work. The title will be displayed prominently in the UI"
                },
                "tasks": {
                    "type": "array",
                    "items": {
                        "type": "string"
                    },
                    "description": "List of task descriptions"
                }
            },
            "required": ["title", "tasks"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::TaskList
    }

    async fn process(&self, request: &ToolRequest) -> Result<Box<dyn ToolCallHandle>> {
        let input: ProposeTaskListRequest = serde_json::from_value(request.arguments.clone())?;

        if input.tasks.is_empty() {
            anyhow::bail!("Task list cannot be empty");
        }

        let tasks_with_status: Vec<TaskWithStatus> = input
            .tasks
            .into_iter()
            .map(|description| TaskWithStatus {
                description,
                status: TaskStatus::Pending,
            })
            .collect();

        Ok(Box::new(ProposeTaskListHandle {
            title: input.title,
            tasks: tasks_with_status,
            tool_use_id: request.tool_use_id.clone(),
            inner: self.inner.clone(),
        }))
    }
}

// ============================================================================
// UpdateTaskListTool
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
struct UpdateTaskListRequest {
    task_number: usize,
    status: TaskStatus,
}

pub struct UpdateTaskListTool {
    pub(crate) inner: Arc<TaskListModuleInner>,
}

struct UpdateTaskListHandle {
    task_id: usize,
    status: TaskStatus,
    tool_use_id: String,
    inner: Arc<TaskListModuleInner>,
}

#[async_trait::async_trait(?Send)]
impl ToolCallHandle for UpdateTaskListHandle {
    fn tool_request(&self) -> ToolRequestEvent {
        ToolRequestEvent {
            tool_call_id: self.tool_use_id.clone(),
            tool_name: "update_task_list".to_string(),
            tool_type: ToolRequestType::Other {
                args: json!({ "task_number": self.task_id, "status": self.status }),
            },
        }
    }

    async fn execute(self: Box<Self>) -> ToolOutput {
        self.inner.update_status(self.task_id, self.status);

        ToolOutput::Result {
            content: json!({ "task_count": self.inner.get().tasks.len() }).to_string(),
            is_error: false,
            continuation: ContinuationPreference::Continue,
            ui_result: ToolExecutionResult::Other {
                result: json!({ "task_id": self.task_id, "status": self.status }),
            },
        }
    }
}

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
        ToolCategory::TaskList
    }

    async fn process(&self, request: &ToolRequest) -> Result<Box<dyn ToolCallHandle>> {
        let input: UpdateTaskListRequest = serde_json::from_value(request.arguments.clone())?;

        Ok(Box::new(UpdateTaskListHandle {
            task_id: input.task_number,
            status: input.status,
            tool_use_id: request.tool_use_id.clone(),
            inner: self.inner.clone(),
        }))
    }
}
