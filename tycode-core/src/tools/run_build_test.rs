use crate::file::access::FileAccessManager;
use crate::tools::r#trait::{ToolCategory, ToolExecutor, ToolRequest, ValidatedToolCall};
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Clone)]
pub struct RunBuildTestTool {
    access: FileAccessManager,
}

impl RunBuildTestTool {
    pub fn new(workspace_roots: Vec<PathBuf>) -> Self {
        Self {
            access: FileAccessManager::new(workspace_roots),
        }
    }
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for RunBuildTestTool {
    fn name(&self) -> &'static str {
        "run_build_test"
    }

    fn description(&self) -> &'static str {
        "Run build, test, or execution commands (cargo build, npm test, python main.py) - NOT for file operations (no cat/ls/grep/find) or shell features (no pipes/redirects); use dedicated file tools instead."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The command to execute"
                },
                "working_directory": {
                    "type": "string",
                    "description": "The directory to run the command in. Must be within a workspace root. Must be an absolute path."
                },
                "timeout_seconds": {
                    "type": "integer",
                    "description": "Maximum seconds to wait for command completion",
                    "minimum": 1,
                    "maximum": 300
                }
            },
            "required": ["command", "timeout_seconds", "working_directory"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Execution
    }

    async fn validate(&self, request: &ToolRequest) -> Result<ValidatedToolCall> {
        let command_str = request
            .arguments
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'command' argument"))?;

        let timeout_seconds = request
            .arguments
            .get("timeout_seconds")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow!("Missing 'timeout_seconds' argument"))?;

        let working_directory = request
            .arguments
            .get("working_directory")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'working_directory' argument"))?;
        let resolved_working_directory = self.access.resolve(working_directory)?;

        let parts: Vec<&str> = command_str.split_whitespace().collect();
        if parts.is_empty() {
            return Ok(ValidatedToolCall::Error("Empty command".to_string()));
        }

        Ok(ValidatedToolCall::RunCommand {
            command: command_str.to_string(),
            working_directory: resolved_working_directory,
            timeout_seconds,
        })
    }
}
