use std::sync::Arc;

use anyhow::Result;
use serde_json::{json, Value};

use crate::memory::MemoryLog;
use crate::tools::r#trait::{ToolCategory, ToolExecutor, ToolRequest, ValidatedToolCall};

pub struct AppendMemoryTool {
    memory_log: Arc<MemoryLog>,
}

impl AppendMemoryTool {
    pub fn new(memory_log: Arc<MemoryLog>) -> Self {
        Self { memory_log }
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
