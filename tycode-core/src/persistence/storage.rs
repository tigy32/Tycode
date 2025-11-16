use crate::persistence::session::SessionData;
use anyhow::{Context, Result};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub id: String,
    pub created_at: u64,
    pub last_modified: u64,
    pub task_list_title: String,
    pub preview: String,
}

fn get_sessions_dir(override_dir: Option<&PathBuf>) -> Result<PathBuf> {
    let sessions_dir = if let Some(dir) = override_dir {
        dir.clone()
    } else {
        let home = dirs::home_dir().context("failed to get home directory")?;
        home.join(".tycode").join("sessions")
    };
    fs::create_dir_all(&sessions_dir).context("failed to create sessions directory")?;
    Ok(sessions_dir)
}

pub fn save_session(session: &SessionData, sessions_dir: Option<&PathBuf>) -> Result<()> {
    let sessions_dir = get_sessions_dir(sessions_dir)?;
    let file_path = sessions_dir.join(format!("{}.json", session.id));

    let mut session_to_save = session.clone();
    session_to_save.last_modified = Utc::now().timestamp_millis() as u64;

    let json =
        serde_json::to_string_pretty(&session_to_save).context("failed to serialize session")?;
    fs::write(&file_path, json).context("failed to write session file")?;

    Ok(())
}

pub fn load_session(id: &str, sessions_dir: Option<&PathBuf>) -> Result<SessionData> {
    let sessions_dir = get_sessions_dir(sessions_dir)?;
    let file_path = sessions_dir.join(format!("{}.json", id));

    let json = fs::read_to_string(&file_path).context("failed to read session file")?;
    let session: SessionData =
        serde_json::from_str(&json).context("failed to deserialize session")?;

    Ok(session)
}

pub fn list_sessions(sessions_dir: Option<&PathBuf>) -> Result<Vec<SessionMetadata>> {
    let sessions_dir = get_sessions_dir(sessions_dir)?;
    let mut sessions = Vec::new();

    let entries = fs::read_dir(&sessions_dir).context("failed to read sessions directory")?;

    for entry in entries {
        let entry = entry.context("failed to read directory entry")?;
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        let json = match fs::read_to_string(&path) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!("Skipping unreadable session file {:?}: {}", path, e);
                continue;
            }
        };

        let session: SessionData = match serde_json::from_str(&json) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("Skipping unparseable session file {:?}: {}", path, e);
                continue;
            }
        };

        let mut preview_text = String::new();
        for msg in session.messages.iter() {
            if msg.role == crate::ai::types::MessageRole::User {
                if !preview_text.is_empty() {
                    preview_text.push_str(" | ");
                }
                preview_text.push_str(&msg.content.text());
                if preview_text.len() >= 100 {
                    break;
                }
            }
        }

        if preview_text.is_empty() {
            preview_text = "New Session".to_string();
        }

        let preview_text = preview_text
            .replace("\r\n", " ")
            .replace('\r', " ")
            .replace('\n', " ");
        let truncated: String = preview_text.chars().take(100).collect();
        let preview = if preview_text.chars().count() > 100 {
            format!("{}...", truncated)
        } else {
            truncated
        };

        sessions.push(SessionMetadata {
            id: session.id,
            created_at: session.created_at,
            last_modified: session.last_modified,
            task_list_title: session.task_list.title,
            preview,
        });
    }

    sessions.sort_by(|a, b| a.last_modified.cmp(&b.last_modified));

    Ok(sessions)
}

pub fn delete_session(id: &str, sessions_dir: Option<&PathBuf>) -> Result<()> {
    let sessions_dir = get_sessions_dir(sessions_dir)?;
    let file_path = sessions_dir.join(format!("{}.json", id));

    fs::remove_file(&file_path).context("failed to delete session file")?;

    Ok(())
}

pub fn list_session_metadata(
    sessions_dir: &Path,
) -> Result<Vec<crate::persistence::session::SessionMetadata>, std::io::Error> {
    let mut metadata_list = Vec::new();

    let entries = fs::read_dir(sessions_dir)?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }

        let Some(id) = path
            .file_stem()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
        else {
            continue;
        };

        let sessions_dir_buf = sessions_dir.to_path_buf();
        let session_data = match load_session(&id, Some(&sessions_dir_buf)) {
            Ok(data) => data,
            Err(e) => {
                tracing::warn!("Skipping unparseable session {:?}: {}", id, e);
                continue;
            }
        };

        let metadata =
            crate::persistence::session::SessionMetadata::from_session_data(&session_data);
        metadata_list.push(metadata);
    }

    metadata_list.sort_by(|a, b| b.last_modified.cmp(&a.last_modified));

    Ok(metadata_list)
}
