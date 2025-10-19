use serde::{Deserialize, Serialize};

pub mod propose_task_list;
pub mod update_task_list;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskListOp {
    Create { title: String, tasks: Vec<String> },
    UpdateStatus { task_id: usize, status: TaskStatus },
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
    pub fn new(title: String, descriptions: Vec<String>) -> Self {
        let tasks = descriptions
            .into_iter()
            .enumerate()
            .map(|(id, description)| Task {
                id,
                description,
                status: TaskStatus::Pending,
            })
            .collect();

        Self { title, tasks }
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
