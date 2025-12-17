use std::sync::{Arc, Mutex};

use anyhow::Result;
use serde_json::{json, Value};

use crate::memory::MemoryLog;
use crate::tools::r#trait::{ToolCategory, ToolExecutor, ToolRequest, ValidatedToolCall};

pub struct AppendMemoryTool {
    memory_log: Arc<Mutex<MemoryLog>>,
}

impl AppendMemoryTool {
    pub fn new(memory_log: Arc<Mutex<MemoryLog>>) -> Self {
        Self { memory_log }
    }
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for AppendMemoryTool {
    fn name(&self) -> &str {
        "append_memory"
    }

    fn description(&self) -> &str {
        "Append a new memory to the persistent log. Use this to record learnings, user preferences, corrections, or important decisions."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "content": {
                    "type": "string",
                    "description": "The memory content to store - a concise description of what was learned"
                },
                "source": {
                    "type": "string",
                    "description": "Optional source context (e.g., project name). Omit for global memories."
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

        let mut log = self
            .memory_log
            .lock()
            .map_err(|e| anyhow::anyhow!("Failed to lock memory log: {e}"))?;

        let seq = log.append(content.clone(), source.clone())?;

        Ok(ValidatedToolCall::context_only(json!({
            "seq": seq,
            "content": content,
            "source": source,
            "success": true
        })))
    }
}
