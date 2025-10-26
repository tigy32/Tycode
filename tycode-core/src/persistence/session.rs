use crate::ai::types::Message;
use crate::tools::tasks::TaskList;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionData {
    pub id: String,
    pub created_at: u64,
    pub last_modified: u64,
    pub messages: Vec<Message>,
    pub task_list: TaskList,
    pub tracked_files: Vec<PathBuf>,
}

impl SessionData {
    pub fn new(
        id: String,
        messages: Vec<Message>,
        task_list: TaskList,
        tracked_files: Vec<PathBuf>,
    ) -> Self {
        let now = Utc::now().timestamp_millis() as u64;
        Self {
            id,
            created_at: now,
            last_modified: now,
            messages,
            task_list,
            tracked_files,
        }
    }
}
