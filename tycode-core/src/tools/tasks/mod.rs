use serde::{Deserialize, Serialize};

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
