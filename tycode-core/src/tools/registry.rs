use crate::ai::{ToolDefinition, ToolUseData};
use crate::tools::r#trait::{SharedTool, ToolCallHandle, ToolCategory, ToolRequest};
use crate::tools::ToolName;
use std::collections::BTreeMap;
use tracing::{debug, error};

pub struct ToolRegistry {
    tools: BTreeMap<String, SharedTool>,
}

impl ToolRegistry {
    pub fn new(tools: Vec<SharedTool>) -> Self {
        let mut registry = Self {
            tools: BTreeMap::new(),
        };

        for tool in tools {
            registry.register_tool(tool);
        }

        registry
    }

    pub fn register_tool(&mut self, tool: SharedTool) {
        let name = tool.name().to_string();
        debug!(tool_name = %name, "Registering tool");
        self.tools.insert(name, tool);
    }

    pub fn get_tool_definitions(&self, tool_names: &[ToolName]) -> Vec<ToolDefinition> {
        let allowed_names: Vec<String> = tool_names.iter().map(|name| name.to_string()).collect();

        self.tools
            .iter()
            .filter(|(name, _)| allowed_names.contains(name) || name.starts_with("mcp_"))
            .map(|(_, tool)| ToolDefinition {
                name: tool.name().to_string(),
                description: tool.description().to_string(),
                input_schema: tool.input_schema(),
            })
            .collect()
    }

    pub async fn process_tools(
        &self,
        tool_use: &ToolUseData,
        allowed_tools: &[ToolName],
    ) -> Result<Box<dyn ToolCallHandle>, String> {
        let mut allowed_names: Vec<&str> = allowed_tools
            .iter()
            .map(|tool| tool.as_str())
            .filter(|name| self.tools.contains_key(*name))
            .collect();

        // MCP tools are dynamically discovered and should be allowed for all agents
        for name in self.tools.keys() {
            if name.starts_with("mcp_") && !allowed_names.contains(&name.as_str()) {
                allowed_names.push(name.as_str());
            }
        }

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
    pub fn get_tool_executor_by_name(&self, name: &str) -> Option<&SharedTool> {
        self.tools.get(name)
    }

    /// Get tool category by name
    pub fn get_tool_category_by_name(&self, name: &str) -> Option<ToolCategory> {
        self.tools.get(name).map(|executor| executor.category())
    }
}
