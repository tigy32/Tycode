//! Log-based memory system with JSON-backed storage.
//!
//! Memories are stored as a JSON log at ~/.tycode/memory/memories_log.json.
//! Each memory has a monotonic sequence number, content, timestamp, and optional source.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::agents::agent::ActiveAgent;
use crate::agents::memory_manager::MemoryManagerAgent;
use crate::agents::runner::AgentRunner;
use crate::ai::provider::AiProvider;
use crate::ai::types::{ContentBlock, Message, MessageRole};
use crate::settings::manager::SettingsManager;
use crate::tools::complete_task::CompleteTask;
use crate::tools::memory::append_memory::AppendMemoryTool;
use crate::tools::r#trait::ToolExecutor;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub seq: u64,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub source: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MemoryLog {
    memories: Vec<Memory>,
    next_seq: u64,
    #[serde(skip)]
    path: PathBuf,
}

impl MemoryLog {
    /// Create a new MemoryLog with a custom path.
    pub fn new(path: PathBuf) -> Self {
        Self {
            memories: Vec::new(),
            next_seq: 1,
            path,
        }
    }

    /// Get the default file path (~/.tycode/memory/memories_log.json).
    pub fn default_path() -> Result<PathBuf> {
        let dir = get_memory_dir(None)?;
        Ok(dir.join("memories_log.json"))
    }

    /// Load from the default location, creating empty if doesn't exist.
    pub fn default_location() -> Result<Self> {
        let path = Self::default_path()?;
        Self::load(&path)
    }

    /// Load from a file, creating empty log if file doesn't exist.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::new(path.to_path_buf()));
        }

        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read memory log: {}", path.display()))?;

        let mut log: MemoryLog = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse memory log: {}", path.display()))?;

        log.path = path.to_path_buf();
        Ok(log)
    }

    /// Save to the configured path.
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create memory directory: {}", parent.display())
            })?;
        }

        let content =
            serde_json::to_string_pretty(self).context("Failed to serialize memory log")?;

        fs::write(&self.path, content)
            .with_context(|| format!("Failed to write memory log: {}", self.path.display()))
    }

    /// Append a new memory, returning its sequence number.
    pub fn append(&mut self, content: String, source: Option<String>) -> Result<u64> {
        let seq = self.next_seq;
        self.next_seq += 1;

        let memory = Memory {
            seq,
            content,
            created_at: Utc::now(),
            source,
        };

        self.memories.push(memory);
        self.save()?;
        Ok(seq)
    }

    /// Read all memories.
    pub fn read_all(&self) -> &[Memory] {
        &self.memories
    }

    /// Get the file path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Get the memory directory, with optional override for testing.
pub fn get_memory_dir(override_dir: Option<&PathBuf>) -> Result<PathBuf> {
    if let Some(dir) = override_dir {
        return Ok(dir.clone());
    }

    let home = dirs::home_dir().ok_or_else(|| anyhow!("Could not determine home directory"))?;
    Ok(home.join(".tycode").join("memory"))
}

/// Spawn the memory manager agent as a background task.
/// This is fire-and-forget - errors are logged but not propagated.
///
/// # Arguments
/// * `ai_provider` - The AI provider to use
/// * `memory_log` - The memory log to store memories in
/// * `settings` - Settings manager
/// * `conversation` - The conversation messages to analyze (last N messages, pre-sliced by caller)
pub fn spawn_memory_manager(
    ai_provider: Arc<dyn AiProvider>,
    memory_log: Arc<Mutex<MemoryLog>>,
    settings: SettingsManager,
    conversation: Vec<Message>,
) {
    let mut tools: BTreeMap<String, Arc<dyn ToolExecutor + Send + Sync>> = BTreeMap::new();
    tools.insert(
        "append_memory".into(),
        Arc::new(AppendMemoryTool::new(memory_log)),
    );
    tools.insert("complete_task".into(), Arc::new(CompleteTask));

    tokio::task::spawn_local(async move {
        let msg_count = conversation.len();
        info!(messages = msg_count, "Memory manager starting");

        let mut active_agent = ActiveAgent::new(Box::new(MemoryManagerAgent));
        active_agent.conversation = conversation;
        active_agent.conversation.push(Message::user(
            "Based on the conversation above, extract any relevant learnings or preferences that should be remembered for future interactions. Use append_memory for each distinct learning, then call complete_task."
        ));

        let runner = AgentRunner::new(ai_provider, settings, tools);

        match runner.run(active_agent).await {
            Ok(_) => info!("Memory manager completed"),
            Err(e) => warn!(error = ?e, "Memory manager failed"),
        }
    });
}

/// Safely slice a conversation to get the last N messages without tearing tool call pairs.
/// Returns messages starting from a clean boundary (User message without orphaned ToolResults).
pub fn safe_conversation_slice(conversation: &[Message], max_messages: usize) -> Vec<Message> {
    if conversation.len() <= max_messages {
        return conversation.to_vec();
    }

    let start_idx = conversation.len().saturating_sub(max_messages);
    let mut slice = &conversation[start_idx..];

    // Tool results require matching tool uses from prior assistant messages.
    // Starting mid-pair would create invalid conversation structure for the AI model.
    while !slice.is_empty() {
        let first = &slice[0];
        if first.role == MessageRole::User {
            let has_tool_results = first
                .content
                .blocks()
                .iter()
                .any(|b| matches!(b, ContentBlock::ToolResult(_)));
            if !has_tool_results {
                break;
            }
        }
        slice = &slice[1..];
    }

    slice.to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_log() -> (TempDir, MemoryLog) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_memories.json");
        let log = MemoryLog::new(path);
        (dir, log)
    }

    #[test]
    fn test_append_and_read() {
        let (_dir, mut log) = temp_log();

        let seq1 = log.append("First memory".to_string(), None).unwrap();
        let seq2 = log
            .append("Second memory".to_string(), Some("project-x".to_string()))
            .unwrap();

        assert_eq!(seq1, 1);
        assert_eq!(seq2, 2);

        let memories = log.read_all();
        assert_eq!(memories.len(), 2);
        assert_eq!(memories[0].content, "First memory");
        assert_eq!(memories[0].source, None);
        assert_eq!(memories[1].content, "Second memory");
        assert_eq!(memories[1].source, Some("project-x".to_string()));
    }

    #[test]
    fn test_persistence() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("persist_test.json");

        // Create and save
        {
            let mut log = MemoryLog::new(path.clone());
            log.append("Persisted memory".to_string(), None).unwrap();
        }

        // Load and verify
        let loaded = MemoryLog::load(&path).unwrap();
        let memories = loaded.read_all();
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].content, "Persisted memory");
        assert_eq!(memories[0].seq, 1);
    }

    #[test]
    fn test_sequence_numbers_continue() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("seq_test.json");

        // Create with some memories
        {
            let mut log = MemoryLog::new(path.clone());
            log.append("First".to_string(), None).unwrap();
            log.append("Second".to_string(), None).unwrap();
        }

        // Load and append more
        {
            let mut log = MemoryLog::load(&path).unwrap();
            let seq = log.append("Third".to_string(), None).unwrap();
            assert_eq!(seq, 3);
        }
    }

    #[test]
    fn test_load_nonexistent_creates_empty() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.json");

        let log = MemoryLog::load(&path).unwrap();
        assert!(log.read_all().is_empty());
    }
}
