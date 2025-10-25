use serde::{Deserialize, Serialize};

pub mod manage_task_list;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TaskListOp {
    Replace {
        title: String,
        tasks: Vec<TaskWithStatus>,
    },
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
