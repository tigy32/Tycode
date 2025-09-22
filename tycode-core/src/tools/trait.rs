use crate::security::types::RiskLevel;
use anyhow::Result;
use serde_json::Value;
use std::path::PathBuf;

/// Request passed to tool execution
#[derive(Debug, Clone)]
pub struct ToolRequest {
    /// The arguments for the tool
    pub arguments: Value,
    /// The unique ID for this tool use
    pub tool_use_id: String,
    // Future fields can be added here without breaking compatibility
}

impl ToolRequest {
    /// Create a new tool request
    pub fn new(arguments: Value, tool_use_id: String) -> Self {
        Self {
            arguments,
            tool_use_id,
        }
    }
}

/// File modification operation type
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileOperation {
    Create,
    Update,
    Delete,
}

/// File modification details
#[derive(Debug, Clone)]
pub struct FileModification {
    pub path: PathBuf,
    pub operation: FileOperation,
    pub original_content: Option<String>,
    pub new_content: Option<String>,
}

/// Result from tool execution
#[derive(Debug)]
pub enum ToolResult {
    /// Standard success with context data and optional UI data
    Success {
        context_data: Value,
        ui_data: Option<Value>,
    },
    /// Error result
    Error(String),
    /// File modification that needs to be applied
    FileModification(FileModification),
    /// Push a new agent onto the stack
    PushAgent {
        agent_type: String,
        task: String,
        context: Option<String>,
    },
    /// Pop the current agent and return result to parent
    PopAgent {
        success: bool,
        summary: String,
        artifacts: Option<Value>,
    },
    /// Halt the AI loop after executing this tool and prompt the user with the
    /// provided question
    PromptUser { question: String },
    /// Command execution details for later processing
    RunCommand {
        command: String,
        working_directory: PathBuf,
        timeout_seconds: u64,
    },
}

impl ToolResult {
    /// Create a result with only context data
    pub fn context_only(data: Value) -> Self {
        Self::Success {
            context_data: data,
            ui_data: None,
        }
    }

    /// Create a result with both context and UI data
    pub fn with_ui(context_data: Value, ui_data: Value) -> Self {
        Self::Success {
            context_data,
            ui_data: Some(ui_data),
        }
    }
}

#[async_trait::async_trait(?Send)]
pub trait ToolExecutor {
    fn name(&self) -> &'static str;
    fn description(&self) -> &'static str;
    fn input_schema(&self) -> Value;

    /// Evaluate the risk level of executing this tool with given arguments
    fn evaluate_risk(&self, arguments: &Value) -> RiskLevel;

    async fn validate(&self, request: &ToolRequest) -> Result<ToolResult>;
}
