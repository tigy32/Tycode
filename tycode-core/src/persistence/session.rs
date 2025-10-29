use crate::ai::types::{Message, MessageRole};
use crate::chat::events::ChatEvent;
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
    pub events: Vec<ChatEvent>,
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
            events: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub id: String,
    pub title: String,
    pub last_modified: u64,
}

impl SessionMetadata {
    pub fn from_session_data(data: &SessionData) -> Self {
        let title = data
            .messages
            .iter()
            .find(|msg| msg.role == MessageRole::User)
            .map(|msg| Self::truncate_text(&msg.content.text()))
            .unwrap_or_else(|| "New Session".to_string());

        Self {
            id: data.id.clone(),
            title,
            last_modified: data.last_modified,
        }
    }

    fn truncate_text(text: &str) -> String {
        let truncated: String = text.chars().take(50).collect();
        if text.chars().count() > 50 {
            format!("{}...", truncated)
        } else {
            truncated
        }
    }
}
