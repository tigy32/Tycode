use serde_json::Value;

use crate::tools::r#trait::{ToolCategory, ToolExecutor, ToolRequest, ValidatedToolCall};

pub struct McpTool {
    name: String,
    description: String,
    input_schema: Value,
    server_name: String,
    mcp_tool_name: String,
}

impl McpTool {
    pub fn new(mcp_tool: &rmcp::model::Tool, server_name: String) -> anyhow::Result<Self> {
        let input_schema = serde_json::to_value(mcp_tool.input_schema.clone())
            .map_err(|e| anyhow::anyhow!("Failed to serialize MCP tool input schema: {e:?}"))?;

        Ok(Self {
            name: format!("mcp_{}", mcp_tool.name),
            description: mcp_tool.description.as_deref().unwrap_or("").to_string(),
            input_schema,
            server_name,
            mcp_tool_name: mcp_tool.name.to_string(),
        })
    }

    pub fn get_server_name(&self) -> &str {
        &self.server_name
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

    async fn validate(&self, request: &ToolRequest) -> Result<ValidatedToolCall, anyhow::Error> {
        Ok(ValidatedToolCall::McpCall {
            server_name: self.server_name.clone(),
            tool_name: self.mcp_tool_name.clone(),
            arguments: Some(request.arguments.clone()),
        })
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
