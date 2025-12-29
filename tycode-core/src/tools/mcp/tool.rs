use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::chat::events::{ToolExecutionResult, ToolRequest as ToolRequestEvent, ToolRequestType};
use crate::tools::mcp::manager::McpManager;
use crate::tools::r#trait::{
    ContinuationPreference, ToolCallHandle, ToolCategory, ToolExecutor, ToolOutput, ToolRequest,
};

pub struct McpTool {
    name: String,
    description: String,
    input_schema: Value,
    server_name: String,
    mcp_tool_name: String,
    manager: Arc<Mutex<McpManager>>,
}

impl McpTool {
    pub fn new(
        mcp_tool: &rmcp::model::Tool,
        server_name: String,
        manager: Arc<Mutex<McpManager>>,
    ) -> anyhow::Result<Self> {
        let input_schema = serde_json::to_value(mcp_tool.input_schema.clone())
            .map_err(|e| anyhow::anyhow!("Failed to serialize MCP tool input schema: {e:?}"))?;

        Ok(Self {
            name: format!("mcp_{}", mcp_tool.name),
            description: mcp_tool.description.as_deref().unwrap_or("").to_string(),
            input_schema,
            server_name,
            mcp_tool_name: mcp_tool.name.to_string(),
            manager,
        })
    }

    pub fn get_server_name(&self) -> &str {
        &self.server_name
    }
}

struct McpToolHandle {
    server_name: String,
    tool_name: String,
    mcp_tool_name: String,
    arguments: Option<Value>,
    tool_use_id: String,
    manager: Arc<Mutex<McpManager>>,
}

#[async_trait::async_trait(?Send)]
impl ToolCallHandle for McpToolHandle {
    fn tool_request(&self) -> ToolRequestEvent {
        ToolRequestEvent {
            tool_call_id: self.tool_use_id.clone(),
            tool_name: self.tool_name.clone(),
            tool_type: ToolRequestType::Other {
                args: json!({
                    "server": self.server_name,
                    "tool": self.mcp_tool_name,
                    "arguments": self.arguments
                }),
            },
        }
    }

    async fn execute(self: Box<Self>) -> ToolOutput {
        let mut manager = self.manager.lock().await;

        match manager
            .execute_tool(&self.server_name, &self.mcp_tool_name, self.arguments)
            .await
        {
            Ok(result) => ToolOutput::Result {
                content: result.clone(),
                is_error: false,
                continuation: ContinuationPreference::Continue,
                ui_result: ToolExecutionResult::Other {
                    result: json!({ "mcp_result": result }),
                },
            },
            Err(e) => ToolOutput::Result {
                content: format!("MCP tool call failed: {e:?}"),
                is_error: true,
                continuation: ContinuationPreference::Continue,
                ui_result: ToolExecutionResult::Error {
                    short_message: "MCP call failed".to_string(),
                    detailed_message: format!("MCP tool call failed: {e:?}"),
                },
            },
        }
    }
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for McpTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn input_schema(&self) -> Value {
        self.input_schema.clone()
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Execution
    }

    async fn process(
        &self,
        request: &ToolRequest,
    ) -> Result<Box<dyn ToolCallHandle>, anyhow::Error> {
        Ok(Box::new(McpToolHandle {
            server_name: self.server_name.clone(),
            tool_name: self.name.clone(),
            mcp_tool_name: self.mcp_tool_name.clone(),
            arguments: Some(request.arguments.clone()),
            tool_use_id: request.tool_use_id.clone(),
            manager: self.manager.clone(),
        }))
    }
}

pub fn mcp_tool_definition(
    mcp_tool: &rmcp::model::Tool,
) -> anyhow::Result<crate::ai::ToolDefinition> {
    let input_schema = serde_json::to_value(mcp_tool.input_schema.clone())
        .map_err(|e| anyhow::anyhow!("Failed to serialize MCP tool input schema: {e:?}"))?;

    Ok(crate::ai::ToolDefinition {
        name: format!("mcp_{}", mcp_tool.name),
        description: mcp_tool.description.as_deref().unwrap_or("").to_string(),
        input_schema,
    })
}
