use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;

use crate::agents::agent::Agent;
use crate::chat::events::{ToolExecutionResult, ToolRequest as ToolRequestEvent};

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

/// Preference for whether the conversation should continue after tool execution
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContinuationPreference {
    Continue,
    Stop,
}

/// Output from tool execution - either a direct result or an action for the orchestrator
pub enum ToolOutput {
    /// Standard result - add to conversation
    Result {
        content: String,
        is_error: bool,
        continuation: ContinuationPreference,
        ui_result: ToolExecutionResult,
    },
    /// Push agent onto stack (spawn_coder, spawn_agent, spawn_recon)
    PushAgent { agent: Arc<dyn Agent>, task: String },
    /// Pop agent from stack (complete_task)
    PopAgent { success: bool, result: String },
    /// Stop and prompt user (ask_user_question)
    PromptUser { question: String },
}

/// Handle for a validated tool call, encapsulating request generation and execution
#[async_trait::async_trait(?Send)]
pub trait ToolCallHandle: Send {
    fn tool_request(&self) -> ToolRequestEvent;
    async fn execute(self: Box<Self>) -> ToolOutput;
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

#[async_trait::async_trait(?Send)]
pub trait ToolExecutor {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn input_schema(&self) -> Value;
    fn category(&self) -> ToolCategory;
    async fn process(&self, request: &ToolRequest) -> Result<Box<dyn ToolCallHandle>>;
}
