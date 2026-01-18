use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;

use super::{McpModuleInner, McpToolDef};
use crate::chat::events::{ToolExecutionResult, ToolRequest as ToolRequestEvent, ToolRequestType};
use crate::tools::r#trait::{
    ContinuationPreference, ToolCallHandle, ToolCategory, ToolExecutor, ToolOutput, ToolRequest,
};

fn format_mcp_content(content: &rmcp::model::Content) -> String {
    match &content.raw {
        rmcp::model::RawContent::Text(text) => text.text.clone(),
        rmcp::model::RawContent::Image(img) => {
            format!("[Image: {} bytes, type: {}]", img.data.len(), img.mime_type)
        }
        rmcp::model::RawContent::Resource(_) => "[Resource data]".to_string(),
        rmcp::model::RawContent::Audio(audio) => {
            format!(
                "[Audio: {} bytes, type: {}]",
                audio.data.len(),
                audio.mime_type
            )
        }
    }
}

pub struct McpTool {
    name: String,
    description: String,
    input_schema: Value,
    server_name: String,
    mcp_tool_name: String,
    inner: Arc<RwLock<McpModuleInner>>,
}

impl McpTool {
    pub(crate) fn new(
        def: &McpToolDef,
        inner: Arc<RwLock<McpModuleInner>>,
    ) -> anyhow::Result<Self> {
        let input_schema = serde_json::to_value(def.tool.input_schema.clone())
            .map_err(|e| anyhow::anyhow!("Failed to serialize MCP tool input schema: {e:?}"))?;

        Ok(Self {
            name: def.name.clone(),
            description: def.tool.description.as_deref().unwrap_or("").to_string(),
            input_schema,
            server_name: def.server_name.clone(),
            mcp_tool_name: def.tool.name.to_string(),
            inner,
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
    inner: Arc<RwLock<McpModuleInner>>,
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
        let mut inner = self.inner.write().await;
        let client = match inner.clients.get_mut(&self.server_name) {
            Some(c) => c,
            None => {
                return ToolOutput::Result {
                    content: format!("MCP server '{}' not found", self.server_name),
                    is_error: true,
                    continuation: ContinuationPreference::Continue,
                    ui_result: ToolExecutionResult::Error {
                        short_message: "Server not found".to_string(),
                        detailed_message: format!("MCP server '{}' not found", self.server_name),
                    },
                };
            }
        };

        match client.call_tool(&self.mcp_tool_name, self.arguments).await {
            Ok(result) => {
                let output = result
                    .content
                    .iter()
                    .map(format_mcp_content)
                    .collect::<Vec<_>>()
                    .join("\n");

                ToolOutput::Result {
                    content: output.clone(),
                    is_error: false,
                    continuation: ContinuationPreference::Continue,
                    ui_result: ToolExecutionResult::Other {
                        result: json!({ "mcp_result": output }),
                    },
                }
            }
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
            inner: self.inner.clone(),
        }))
    }
}

pub fn mcp_tool_definition(def: &McpToolDef) -> anyhow::Result<crate::ai::ToolDefinition> {
    let input_schema = serde_json::to_value(def.tool.input_schema.clone())
        .map_err(|e| anyhow::anyhow!("Failed to serialize MCP tool input schema: {e:?}"))?;

    Ok(crate::ai::ToolDefinition {
        name: def.name.clone(),
        description: def.tool.description.as_deref().unwrap_or("").to_string(),
        input_schema,
    })
}
