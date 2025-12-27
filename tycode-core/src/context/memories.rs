use std::sync::Arc;

use anyhow::Result;
use serde_json::{json, Value};

use crate::context::{ContextComponent, ContextComponentId};

pub const ID: ContextComponentId = ContextComponentId("memories");
use crate::memory::MemoryLog;
use crate::settings::manager::SettingsManager;
use crate::tools::r#trait::{ToolCategory, ToolExecutor, ToolRequest, ValidatedToolCall};

/// Manages memory log and provides both context rendering and tool execution.
pub struct MemoriesManager {
    memory_log: Arc<MemoryLog>,
    settings: SettingsManager,
}

impl MemoriesManager {
    pub fn new(memory_log: Arc<MemoryLog>, settings: SettingsManager) -> Self {
        Self {
            memory_log,
            settings,
        }
    }

    pub fn memory_log(&self) -> &Arc<MemoryLog> {
        &self.memory_log
    }
}

#[async_trait::async_trait(?Send)]
impl ContextComponent for MemoriesManager {
    fn id(&self) -> ContextComponentId {
        ID
    }

    async fn build_context_section(&self) -> Option<String> {
        let memories = self.memory_log.read_all().ok()?;
        if memories.is_empty() {
            return None;
        }

        let max_recent = self.settings.settings().memory.recent_memories_count;
        let recent: Vec<_> = memories.iter().rev().take(max_recent).collect();

        if recent.is_empty() {
            return None;
        }

        let mut output = String::from("Recent Memories:\n");
        for memory in recent.iter().rev() {
            let source_info = memory
                .source
                .as_ref()
                .map(|s| format!(" [{}]", s))
                .unwrap_or_default();
            output.push_str(&format!("- {}{}", memory.content, source_info));
            output.push('\n');
        }
        Some(output)
    }
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for MemoriesManager {
    fn name(&self) -> &str {
        "append_memory"
    }

    fn description(&self) -> &str {
        "Appends text to the memory log. Stored memories appear in the model's context in future conversations, helping avoid repeated corrections and follow user preferences. Store when corrected repeatedly or when the user expresses frustration."
    }

    fn input_schema(&self) -> Value {
        json!({
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

    async fn validate(&self, request: &ToolRequest) -> Result<ValidatedToolCall> {
        let content = request.arguments["content"]
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("content is required"))?
            .to_string();

        let source = request
            .arguments
            .get("source")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let seq = self.memory_log.append(content.clone(), source.clone())?;

        Ok(ValidatedToolCall::context_only(json!({
            "seq": seq,
            "content": content,
            "source": source,
            "success": true
        })))
    }
}
