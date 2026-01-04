//! Memory append tool for storing learnings.

use std::sync::Arc;

use crate::chat::events::{ToolExecutionResult, ToolRequest as ToolRequestEvent, ToolRequestType};
use crate::tools::r#trait::{
    ContinuationPreference, ToolCallHandle, ToolCategory, ToolExecutor, ToolOutput, ToolRequest,
};
use crate::tools::ToolName;

use super::log::MemoryLog;

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
