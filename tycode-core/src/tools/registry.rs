use crate::ai::{ToolDefinition, ToolUseData};
use crate::tools::mcp::manager::McpManager;
use crate::tools::r#trait::{ToolCallHandle, ToolCategory, ToolExecutor, ToolRequest};
use crate::tools::ToolName;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error};

pub struct ToolRegistry {
    tools: BTreeMap<String, Arc<dyn ToolExecutor>>,
    mcp_manager: Arc<Mutex<McpManager>>,
}

impl ToolRegistry {
    pub fn new(tools: Vec<Arc<dyn ToolExecutor>>, mcp_manager: Arc<Mutex<McpManager>>) -> Self {
        let mut registry = Self {
            tools: BTreeMap::new(),
            mcp_manager,
        };

        for tool in tools {
            registry.register_tool(tool);
        }

        registry
    }

    pub fn register_tool(&mut self, tool: Arc<dyn ToolExecutor>) {
        let name = tool.name().to_string();
        debug!(tool_name = %name, "Registering tool");
        self.tools.insert(name, tool);
    }

    /// Gets tool definitions for a specific set of tool types, automatically including MCP tools
    pub fn get_tool_definitions(&self, tool_names: &[ToolName]) -> Vec<ToolDefinition> {
        tool_names
            .iter()
            .map(|name| name.to_string())
            .filter_map(|tool_name| self.tools.get(&tool_name))
            .map(|tool| ToolDefinition {
                name: tool.name().to_string(),
                description: tool.description().to_string(),
                input_schema: tool.input_schema(),
            })
            .chain(self.get_mcp_definitions())
            .collect()
    }

    /// Get tool definitions from MCP manager (automatically included in all agents)
    pub fn get_mcp_definitions(&self) -> Vec<ToolDefinition> {
        let manager = self.mcp_manager.try_lock().expect(
            "Failed to acquire MCP manager lock - this indicates a deadlock or incorrect usage",
        );

        manager
            .get_tool_definitions()
            .iter()
            .map(|def| ToolDefinition {
                name: def.tool.name.to_string(),
                description: def
                    .tool
                    .description
                    .as_ref()
                    .map(|d| d.to_string())
                    .unwrap_or_default(),
                input_schema: serde_json::to_value(&def.tool.input_schema)
                    .unwrap_or(serde_json::json!({"type": "object", "properties": {}})),
            })
            .collect()
    }

    pub async fn process_tools(
        &self,
        tool_use: &ToolUseData,
        allowed_tools: &[ToolName],
    ) -> Result<Box<dyn ToolCallHandle>, String> {
        let mcp_tool_names: Vec<String> = {
            let manager = self.mcp_manager.try_lock().expect(
                "Failed to acquire MCP manager lock - this indicates a deadlock or incorrect usage",
            );
            manager
                .get_tool_definitions()
                .iter()
                .map(|def| def.tool.name.to_string())
                .collect()
        };

        let allowed_names: Vec<&str> = allowed_tools
            .iter()
            .map(|tool| tool.as_str())
            .filter(|name| self.tools.contains_key(*name))
            .chain(mcp_tool_names.iter().map(|s| s.as_str()))
            .collect();

        let tool = match self.tools.get(&tool_use.name) {
            Some(tool) => tool,
            None => {
                let available = allowed_names.join(", ");
                error!(tool_name = %tool_use.name, "Unknown tool");
                return Err(format!(
                    "Unknown tool: {}. Available tools: {}",
                    tool_use.name, available
                ));
            }
        };

        if !allowed_names.contains(&tool_use.name.as_str()) {
            debug!(
                tool_name = %tool_use.name,
                allowed_tools = ?allowed_names,
                "Tool not in allowed list for current agent"
            );
            return Err(format!(
                "Tool not available for current agent: {}",
                tool_use.name
            ));
        }

        let schema = tool.input_schema();
        let coerced_arguments =
            match crate::tools::fuzzy_json::coerce_to_schema(&tool_use.arguments, &schema) {
                Ok(args) => args,
                Err(e) => {
                    error!(?e, tool_name = %tool_use.name, "Failed to coerce tool arguments");
                    return Err(format!("Failed to coerce arguments: {e:?}"));
                }
            };

        let request = ToolRequest::new(coerced_arguments, tool_use.id.clone());
        tool.process(&request).await.map_err(|e| {
            error!(?e, tool_name = %tool_use.name, "Tool processing failed");
            format!("Error: {e:?}")
        })
    }

    pub fn list_tools(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    /// Get tool executor by name
    pub fn get_tool_executor_by_name(&self, name: &str) -> Option<&Arc<dyn ToolExecutor>> {
        self.tools.get(name)
    }

    /// Get tool category by name
    pub fn get_tool_category_by_name(&self, name: &str) -> Option<ToolCategory> {
        self.tools.get(name).map(|executor| executor.category())
    }
}
