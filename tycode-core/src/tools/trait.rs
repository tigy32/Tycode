use anyhow::Result;
use serde_json::Value;
use std::path::PathBuf;

/// Tool category that determines the type of operation
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum ToolCategory {
    TaskList,
    Execution,
    Meta,
}

/// Request passed to tool execution
#[derive(Debug, Clone)]
pub struct ToolRequest {
    /// The arguments for the tool
    pub arguments: Value,
    /// The unique ID for this tool use
    pub tool_use_id: String,
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
    pub warning: Option<String>,
}

/// A "validated" tool call. A tool is responsible for taking json from the AI
/// model, validating that its a coherent request, and returning it as one of
/// these known tool types.
#[derive(Debug)]
pub enum ValidatedToolCall {
    /// A response from a tool that doesn't need additional processing (either
    /// the tool does nothing, or the validation also executed the tool)
    NoOp {
        context_data: Value,
        ui_data: Option<Value>,
    },
    /// File modification that needs to be applied
    FileModification(FileModification),
    /// Push a new agent onto the stack
    PushAgent { agent_type: String, task: String },
    /// Pop the current agent and return result to parent
    PopAgent { success: bool, result: String },
    /// Halt the AI loop after executing this tool and prompt the user with the
    /// provided question
    PromptUser { question: String },
    /// Command execution details for later processing
    RunCommand {
        command: String,
        working_directory: PathBuf,
        timeout_seconds: u64,
    },
    /// Set tracked files for the session
    SetTrackedFiles { file_paths: Vec<String> },
    /// MCP tool call to be executed by the manager
    McpCall {
        server_name: String,
        tool_name: String,
        arguments: Option<serde_json::Value>,
    },
    /// Search for types by name in a workspace
    SearchTypes {
        language: String,
        workspace_root: PathBuf,
        type_name: String,
    },
    /// Get documentation for a specific type
    GetTypeDocs {
        language: String,
        workspace_root: PathBuf,
        type_path: String,
    },
    /// Error result
    Error(String),
}

impl ValidatedToolCall {
    /// Create a result with only context data
    pub fn context_only(data: Value) -> Self {
        Self::NoOp {
            context_data: data,
            ui_data: None,
        }
    }

    /// Create a result with both context and UI data
    pub fn with_ui(context_data: Value, ui_data: Value) -> Self {
        Self::NoOp {
            context_data,
            ui_data: Some(ui_data),
        }
    }
}

#[async_trait::async_trait(?Send)]
pub trait ToolExecutor {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> Value;
    fn category(&self) -> ToolCategory;
    async fn validate(&self, request: &ToolRequest) -> Result<ValidatedToolCall>;
}
