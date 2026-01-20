use std::sync::{Arc, RwLock};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::chat::events::{
    ChatEvent, EventSender, ToolExecutionResult, ToolRequest as ToolRequestEvent, ToolRequestType,
};
use crate::context::{ContextComponent, ContextComponentId};
use crate::module::{Module, SessionStateComponent};
use crate::module::{PromptComponent, PromptComponentId};
use crate::settings::config::Settings;
use crate::tools::r#trait::{
    ContinuationPreference, ToolCallHandle, ToolCategory, ToolExecutor, ToolOutput, ToolRequest,
};
use crate::tools::ToolName;

/// Module that owns task list state and provides tools + context component + prompt.
pub struct TaskListModule {
    inner: Arc<TaskListModuleInner>,
}

pub(crate) struct TaskListModuleInner {
    pub(crate) task_list: RwLock<TaskList>,
    pub(crate) event_sender: EventSender,
}

impl TaskListModule {
    pub fn new(event_sender: EventSender) -> Self {
        let inner = Arc::new(TaskListModuleInner {
            task_list: RwLock::new(TaskList::default()),
            event_sender,
        });
        inner.emit_update();
        Self { inner }
    }

    pub fn manage_tool(&self) -> Arc<dyn ToolExecutor> {
        Arc::new(ManageTaskListTool {
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

impl Module for TaskListModule {
    fn prompt_components(&self) -> Vec<Arc<dyn PromptComponent>> {
        vec![Arc::new(TaskListPromptComponent)]
    }

    fn context_components(&self) -> Vec<Arc<dyn ContextComponent>> {
        vec![self.context_component()]
    }

    fn tools(&self) -> Vec<Arc<dyn ToolExecutor>> {
        vec![self.manage_tool()]
    }

    fn session_state(&self) -> Option<Arc<dyn SessionStateComponent>> {
        Some(Arc::new(TaskListSessionState {
            inner: self.inner.clone(),
        }))
    }
}

struct TaskListSessionState {
    inner: Arc<TaskListModuleInner>,
}

impl SessionStateComponent for TaskListSessionState {
    fn key(&self) -> &str {
        "task_list"
    }

    fn save(&self) -> Value {
        serde_json::to_value(self.inner.get()).expect("TaskList serialization cannot fail")
    }

    fn load(&self, state: Value) -> Result<()> {
        let task_list: TaskList = serde_json::from_value(state)?;
        let tasks = task_list
            .tasks
            .iter()
            .map(|t| TaskWithStatus {
                description: t.description.clone(),
                status: t.status,
            })
            .collect();
        self.inner.replace(task_list.title, tasks);
        Ok(())
    }
}

impl TaskListModuleInner {
    pub(crate) fn replace(&self, title: String, tasks: Vec<TaskWithStatus>) {
        let new_list = TaskList::from_tasks_with_status(title, tasks);
        *self.task_list.write().unwrap() = new_list;
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

pub const TASK_LIST_PROMPT_ID: PromptComponentId = PromptComponentId("tasks");

const TASK_LIST_MANAGEMENT: &str = r#"## Task List Management
• The 'context' will always include a task list. The task list is designed to help you break down large tasks in to smaller chunks of work and to provide feedback to the user about what you are working on.
• When possible, design each step so that it can be validated (compile and pass tests). Some tasks may require multiple steps before validation is feasible. 
• The task list can be updated with a special tool called "manage_task_list". Ensure the task list is always up to date.
• The "manage_task_list" is neither an "Execution" nor a "Meta" tool and may be combined with either type of response. "manage_task_list" may never be the only tool request; "manage_task_list" must always be combined with at least 1 other tool call. 

## When to Update the Task List
• Set the task list once a plan has been presented to the user and approved. A new task list created with "manage_task_list" must be combined with "Exection" tools beginning work on the first task.
• Update the task list when a task has been completed. If there are additional tasks, "manage_task_list" must be combined with "Execution" tools beginning work on the next task. When completing the last task, "manage_task_list" must be combined with "complete_task".
• Before marking a task complete ensure changes: 1/ comply with style mandates 2/ compile and build (when possible) 3/ tests pass (when possible)
• "complete_task" should only be used when completing the final task in the task list.
"#;

/// Provides task list management instructions.
pub struct TaskListPromptComponent;

impl PromptComponent for TaskListPromptComponent {
    fn id(&self) -> PromptComponentId {
        TASK_LIST_PROMPT_ID
    }

    fn build_prompt_section(&self, _settings: &Settings) -> Option<String> {
        Some(TASK_LIST_MANAGEMENT.to_string())
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
// ManageTaskListTool - replaces entire task list
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
struct TaskInput {
    description: String,
    status: TaskStatus,
}

#[derive(Debug, Serialize, Deserialize)]
struct ManageTaskListInput {
    title: String,
    tasks: Vec<TaskInput>,
}

pub struct ManageTaskListTool {
    pub(crate) inner: Arc<TaskListModuleInner>,
}

impl ManageTaskListTool {
    pub fn tool_name() -> ToolName {
        ToolName::new("manage_task_list")
    }
}

struct ManageTaskListHandle {
    title: String,
    tasks: Vec<TaskWithStatus>,
    tool_use_id: String,
    inner: Arc<TaskListModuleInner>,
}

#[async_trait::async_trait(?Send)]
impl ToolCallHandle for ManageTaskListHandle {
    fn tool_request(&self) -> ToolRequestEvent {
        ToolRequestEvent {
            tool_call_id: self.tool_use_id.clone(),
            tool_name: "manage_task_list".to_string(),
            tool_type: ToolRequestType::Other {
                args: json!({ "title": self.title, "task_count": self.tasks.len() }),
            },
        }
    }

    async fn execute(self: Box<Self>) -> ToolOutput {
        self.inner.replace(self.title.clone(), self.tasks);
        ToolOutput::Result {
            content: format!("Task list updated: {}", self.title),
            is_error: false,
            continuation: ContinuationPreference::Continue,
            ui_result: ToolExecutionResult::Other {
                result: json!({ "title": self.title }),
            },
        }
    }
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for ManageTaskListTool {
    fn name(&self) -> &str {
        "manage_task_list"
    }

    fn description(&self) -> &str {
        "Create or update the task list. This tool must be combined with at least 1 other tool call."
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

    async fn process(&self, request: &ToolRequest) -> Result<Box<dyn ToolCallHandle>> {
        let input: ManageTaskListInput = serde_json::from_value(request.arguments.clone())?;

        if input.tasks.is_empty() {
            return Err(anyhow::anyhow!("Task list cannot be empty"));
        }

        let tasks: Vec<TaskWithStatus> = input
            .tasks
            .into_iter()
            .map(|t| TaskWithStatus {
                description: t.description,
                status: t.status,
            })
            .collect();

        Ok(Box::new(ManageTaskListHandle {
            title: input.title,
            tasks,
            tool_use_id: request.tool_use_id.clone(),
            inner: self.inner.clone(),
        }))
    }
}
