//! Log-based memory system with JSON-backed storage.
//!
//! Memories are stored as a JSON log at ~/.tycode/memory/memories_log.json.
//! Each memory has a monotonic sequence number, content, timestamp, and optional source.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::agents::agent::ActiveAgent;
use crate::agents::memory_manager::MemoryManagerAgent;
use crate::agents::runner::AgentRunner;
use crate::ai::provider::AiProvider;
use crate::ai::types::{ContentBlock, Message, MessageRole};
use crate::chat::events::{ToolExecutionResult, ToolRequest as ToolRequestEvent, ToolRequestType};
use crate::context::ContextBuilder;
use crate::prompt::PromptBuilder;
use crate::settings::manager::SettingsManager;
use crate::steering::SteeringDocuments;
use crate::tools::complete_task::CompleteTask;
use crate::tools::r#trait::{
    ContinuationPreference, ToolCallHandle, ToolCategory, ToolExecutor, ToolOutput, ToolRequest,
};
use crate::tools::ToolName;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub seq: u64,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub source: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct MemoryLogInner {
    memories: Vec<Memory>,
    next_seq: u64,
}

/// Memory log that loads from disk on every operation.
#[derive(Debug)]
pub struct MemoryLog {
    path: PathBuf,
}

impl MemoryLog {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Load current state from disk. Returns empty if file doesn't exist.
    fn load_inner(&self) -> Result<MemoryLogInner> {
        if !self.path.exists() {
            return Ok(MemoryLogInner {
                memories: Vec::new(),
                next_seq: 1,
            });
        }

        let content = fs::read_to_string(&self.path)
            .with_context(|| format!("Failed to read memory log: {}", self.path.display()))?;

        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse memory log: {}", self.path.display()))
    }

    /// Save state to disk, creating directories as needed.
    fn save_inner(&self, inner: &MemoryLogInner) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create memory directory: {}", parent.display())
            })?;
        }

        let content =
            serde_json::to_string_pretty(inner).context("Failed to serialize memory log")?;

        fs::write(&self.path, content)
            .with_context(|| format!("Failed to write memory log: {}", self.path.display()))
    }

    /// Append a new memory. Loads from disk, adds memory, saves back.
    /// Race condition: if two processes append simultaneously, one may lose.
    /// This is acceptable - we lose a few memories, not the entire log.
    pub fn append(&self, content: String, source: Option<String>) -> Result<u64> {
        let mut inner = self.load_inner()?;

        let seq = inner.next_seq;
        inner.next_seq += 1;

        inner.memories.push(Memory {
            seq,
            content,
            created_at: Utc::now(),
            source,
        });

        self.save_inner(&inner)?;
        Ok(seq)
    }

    /// Read all memories from disk.
    pub fn read_all(&self) -> Result<Vec<Memory>> {
        self.load_inner().map(|inner| inner.memories)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Spawn the memory manager agent as a background task.
/// This is fire-and-forget - errors are logged but not propagated.
///
/// # Arguments
/// * `ai_provider` - The AI provider to use
/// * `memory_log` - The memory log to store memories in
/// * `settings` - Settings manager
/// * `conversation` - The conversation messages to analyze (last N messages, pre-sliced by caller)
/// * `steering` - Steering documents
/// * `mcp_manager` - MCP manager for tool access
pub fn spawn_memory_manager(
    ai_provider: Arc<dyn AiProvider>,
    memory_log: Arc<MemoryLog>,
    settings: SettingsManager,
    conversation: Vec<Message>,
    steering: SteeringDocuments,
    mcp_manager: Arc<tokio::sync::Mutex<crate::tools::mcp::manager::McpManager>>,
    prompt_builder: PromptBuilder,
    context_builder: ContextBuilder,
) {
    let mut tools: BTreeMap<String, Arc<dyn ToolExecutor + Send + Sync>> = BTreeMap::new();
    tools.insert(
        "append_memory".into(),
        Arc::new(AppendMemoryTool::new(memory_log.clone())),
    );
    tools.insert("complete_task".into(), Arc::new(CompleteTask));

    tokio::task::spawn_local(async move {
        let msg_count = conversation.len();
        info!(messages = msg_count, "Memory manager starting");

        let mut active_agent = ActiveAgent::new(Arc::new(MemoryManagerAgent));
        active_agent.conversation = conversation;
        active_agent.conversation.push(Message::user(
            "=== MEMORY MANAGER AGENT ===\n\n\
            You are now the Memory Manager agent. Your conversation history contains the interaction \
            between the user and a coding agent that just concluded. Your task is to analyze that conversation \
            history (all messages before this one) and extract any learnings worth remembering.\n\n\
            Look for:\n\
            - User preferences or corrections\n\
            - Project-specific decisions\n\
            - Coding style preferences\n\
            - Technical constraints mentioned\n\n\
            Use append_memory for each distinct learning, then call complete_task. \
            If the conversation contains no extractable learnings, call complete_task immediately."
        ));

        let runner = AgentRunner::new(
            ai_provider,
            settings,
            tools,
            steering,
            prompt_builder,
            context_builder,
            mcp_manager,
        );

        match runner.run(active_agent, 2).await {
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

// === AppendMemoryTool ===

pub struct AppendMemoryTool {
    memory_log: Arc<MemoryLog>,
}

impl AppendMemoryTool {
    pub fn new(memory_log: Arc<MemoryLog>) -> Self {
        Self { memory_log }
    }

    pub fn tool_name() -> ToolName {
        ToolName::new("append_memory")
    }
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for AppendMemoryTool {
    fn name(&self) -> &str {
        "append_memory"
    }

    fn description(&self) -> &str {
        "Appends text to the memory log. Stored memories appear in the model's context in future conversations, helping avoid repeated corrections and follow user preferences. Store when corrected repeatedly or when the user expresses frustration."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "A concise description of what was learned"
                },
                "source": {
                    "type": "string",
                    "description": "Optional project name this memory applies to. Omit for global memories."
                }
            },
            "required": ["content"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Execution
    }

    async fn process(&self, request: &ToolRequest) -> anyhow::Result<Box<dyn ToolCallHandle>> {
        let content = request.arguments["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("content is required"))?
            .to_string();

        let source = request
            .arguments
            .get("source")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Ok(Box::new(AppendMemoryHandle {
            content,
            source,
            tool_use_id: request.tool_use_id.clone(),
            memory_log: self.memory_log.clone(),
        }))
    }
}

struct AppendMemoryHandle {
    content: String,
    source: Option<String>,
    tool_use_id: String,
    memory_log: Arc<MemoryLog>,
}

#[async_trait::async_trait(?Send)]
impl ToolCallHandle for AppendMemoryHandle {
    fn tool_request(&self) -> ToolRequestEvent {
        ToolRequestEvent {
            tool_call_id: self.tool_use_id.clone(),
            tool_name: "append_memory".to_string(),
            tool_type: ToolRequestType::Other {
                args: serde_json::json!({
                    "content": self.content,
                    "source": self.source
                }),
            },
        }
    }

    async fn execute(self: Box<Self>) -> ToolOutput {
        match self
            .memory_log
            .append(self.content.clone(), self.source.clone())
        {
            Ok(seq) => ToolOutput::Result {
                content: serde_json::json!({
                    "seq": seq,
                    "content": self.content,
                    "source": self.source,
                    "success": true
                })
                .to_string(),
                is_error: false,
                continuation: ContinuationPreference::Continue,
                ui_result: ToolExecutionResult::Other {
                    result: serde_json::json!({
                        "seq": seq,
                        "success": true
                    }),
                },
            },
            Err(e) => ToolOutput::Result {
                content: format!("Failed to append memory: {e:?}"),
                is_error: true,
                continuation: ContinuationPreference::Continue,
                ui_result: ToolExecutionResult::Error {
                    short_message: "Memory append failed".to_string(),
                    detailed_message: format!("{e:?}"),
                },
            },
        }
    }
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
        let (_dir, log) = temp_log();

        let seq1 = log.append("First memory".to_string(), None).unwrap();
        let seq2 = log
            .append("Second memory".to_string(), Some("project-x".to_string()))
            .unwrap();

        assert_eq!(seq1, 1);
        assert_eq!(seq2, 2);

        let memories = log.read_all().unwrap();
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
            let log = MemoryLog::new(path.clone());
            log.append("Persisted memory".to_string(), None).unwrap();
        }

        // New instance loads from disk automatically
        let loaded = MemoryLog::new(path);
        let memories = loaded.read_all().unwrap();
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
            let log = MemoryLog::new(path.clone());
            log.append("First".to_string(), None).unwrap();
            log.append("Second".to_string(), None).unwrap();
        }

        // New instance picks up from disk
        {
            let log = MemoryLog::new(path);
            let seq = log.append("Third".to_string(), None).unwrap();
            assert_eq!(seq, 3);
        }
    }

    #[test]
    fn test_read_nonexistent_returns_empty() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("nonexistent.json");

        let log = MemoryLog::new(path);
        assert!(log.read_all().unwrap().is_empty());
    }
}
