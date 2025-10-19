use serde::{Deserialize, Serialize};

pub mod propose_task_list;
pub mod update_task_list;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskListOp {
    Create(Vec<String>),
    UpdateStatus { task_id: usize, status: TaskStatus },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Failed,
}

impl TaskStatus {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Failed => "failed",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "in_progress" => Some(Self::InProgress),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: usize,
    pub description: String,
    pub status: TaskStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskList {
    pub tasks: Vec<Task>,
}

impl TaskList {
    pub fn new(descriptions: Vec<String>) -> Self {
        let tasks = descriptions
            .into_iter()
            .enumerate()
            .map(|(id, description)| Task {
                id,
                description,
                status: TaskStatus::Pending,
            })
            .collect();

        Self { tasks }
    }

    pub fn update_task_status(&mut self, task_id: usize, status: TaskStatus) -> Result<(), String> {
        let task = self
            .tasks
            .iter_mut()
            .find(|t| t.id == task_id)
            .ok_or_else(|| format!("Task {} not found", task_id))?;

        task.status = status;
        Ok(())
    }
}
