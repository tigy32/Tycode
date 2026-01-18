use crate::settings::config::{McpServerConfig, Settings};
use crate::tools::r#trait::ToolExecutor;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{debug, error, info};

use super::client::McpClient;
use super::tool::McpTool;

/// Tool definition metadata stored before wrapping in Arc<Mutex<>>
#[derive(Clone)]
pub struct McpToolDef {
    /// The prefixed tool name (e.g., "mcp_read_file")
    pub name: String,
    pub tool: rmcp::model::Tool,
    pub server_name: String,
}

pub struct McpManager {
    clients: HashMap<String, McpClient>,
    tool_defs: Vec<McpToolDef>,
}

impl McpManager {
    /// Creates an empty MCP manager with no servers configured
    pub fn empty() -> Self {
        Self {
            clients: HashMap::new(),
            tool_defs: Vec::new(),
        }
    }

    /// Creates MCP manager from settings and returns both the manager and ready-to-use tools.
    /// Returns (manager, tools) tuple where manager is wrapped in Arc<Mutex<>> and each tool holds a reference.
    /// Always returns a valid manager - empty manager when no MCP servers configured.
    pub async fn from_settings(
        settings: &Settings,
    ) -> anyhow::Result<(Arc<Mutex<Self>>, Vec<Arc<dyn ToolExecutor>>)> {
        // Early return with empty manager if no MCP servers configured
        if settings.mcp_servers.is_empty() {
            let empty_manager = Self {
                clients: HashMap::new(),
                tool_defs: Vec::new(),
            };
            return Ok((Arc::new(Mutex::new(empty_manager)), Vec::new()));
        }

        let mut manager = Self {
            clients: HashMap::new(),
            tool_defs: Vec::new(),
        };

        for (name, config) in &settings.mcp_servers {
            info!(server_name = %name, command = %config.command, "Initializing MCP server");

            if let Err(e) = manager.add_server(name.clone(), config.clone()).await {
                error!(error = %e, server_name = %name, "Failed to initialize MCP server");
            }
        }

        let server_count = manager.clients.len();
        let tool_count = manager.tool_defs.len();
        info!(
            servers = server_count,
            tools = tool_count,
            "MCP manager initialized"
        );

        let tool_defs = manager.tool_defs.clone();
        let wrapped = Arc::new(Mutex::new(manager));

        let tools = tool_defs
            .into_iter()
            .filter_map(|def| McpTool::new(&def, wrapped.clone()).ok())
            .map(|tool| Arc::new(tool) as Arc<dyn ToolExecutor>)
            .collect();

        Ok((wrapped, tools))
    }

    pub async fn add_server(
        &mut self,
        name: String,
        config: McpServerConfig,
    ) -> anyhow::Result<()> {
        debug!(server_name = %name, "Adding MCP server");

        let mut client = McpClient::new(name.clone(), config).await.map_err(|e| {
            error!(error = %e, server_name = %name, "Failed to initialize MCP client");
            anyhow::anyhow!("Failed to initialize MCP server '{name}': {e}")
        })?;

        let mcp_tools = client.list_tools().await.map_err(|e| {
            error!(error = %e, server_name = %name, "Failed to list MCP tools");
            anyhow::anyhow!("Failed to list tools from MCP server '{name}': {e}")
        })?;

        debug!(server_name = %name, tool_count = mcp_tools.len(), "Found MCP tools");

        for mcp_tool in mcp_tools {
            self.tool_defs.push(McpToolDef {
                name: format!("mcp_{}", mcp_tool.name),
                tool: mcp_tool,
                server_name: name.clone(),
            });
        }

        self.clients.insert(name, client);

        Ok(())
    }

    pub async fn execute_tool(
        &mut self,
        server_name: &str,
        tool_name: &str,
        arguments: Option<serde_json::Value>,
    ) -> anyhow::Result<String> {
        let client = self
            .clients
            .get_mut(server_name)
            .ok_or(anyhow::anyhow!("MCP server '{server_name}' not found"))?;

        let result = client.call_tool(tool_name, arguments).await?;

        let output = result
            .content
            .into_iter()
            .map(|content| match content.raw {
                rmcp::model::RawContent::Text(text) => text.text.clone(),
                rmcp::model::RawContent::Image(image_data) => {
                    format!(
                        "[Image: {} bytes, type: {}]",
                        image_data.data.len(),
                        image_data.mime_type
                    )
                }
                rmcp::model::RawContent::Resource(_resource_data) => "[Resource data]".to_string(),
                rmcp::model::RawContent::Audio(audio_data) => {
                    format!(
                        "[Audio: {} bytes, type: {}]",
                        audio_data.data.len(),
                        audio_data.mime_type
                    )
                }
            })
            .collect::<Vec<_>>()
            .join("\n");

        Ok(output)
    }

    pub async fn remove_server(&mut self, name: &str) -> anyhow::Result<()> {
        debug!(server_name = %name, "Removing MCP server");

        if let Some(client) = self.clients.remove(name) {
            if let Err(e) = client.close().await {
                error!(error = %e, server_name = %name, "Failed to close MCP client");
            }
        }

        let original_count = self.tool_defs.len();
        self.tool_defs.retain(|def| def.server_name != name);
        let removed_count = original_count - self.tool_defs.len();

        info!(server_name = %name, removed_tools = removed_count, "Removed MCP server");

        Ok(())
    }

    pub async fn shutdown(self) -> anyhow::Result<()> {
        info!("Shutting down MCP manager");

        for (name, client) in self.clients {
            debug!(server_name = %name, "Closing MCP server connection");
            if let Err(e) = client.close().await {
                error!(error = %e, server_name = %name, "Failed to close MCP client during shutdown");
            }
        }

        info!("MCP manager shutdown complete");

        Ok(())
    }

    /// Get tool definitions for all registered MCP tools
    pub fn get_tool_definitions(&self) -> &[McpToolDef] {
        &self.tool_defs
    }

    /// Get server statistics
    pub fn get_stats(&self) -> McpManagerStats {
        McpManagerStats {
            server_count: self.clients.len(),
            tool_count: self.tool_defs.len(),
            servers: self.clients.keys().cloned().collect(),
        }
    }
}

#[derive(Debug)]
pub struct McpManagerStats {
    pub server_count: usize,
    pub tool_count: usize,
    pub servers: Vec<String>,
}
