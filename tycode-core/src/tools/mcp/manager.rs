use crate::settings::config::{McpServerConfig, Settings};
use crate::tools::r#trait::ToolExecutor;
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error, info};

use super::client::McpClient;
use super::tool::McpTool;

pub struct McpManager {
    clients: HashMap<String, McpClient>,
    tools: Vec<Arc<McpTool>>,
}

impl McpManager {
    pub async fn from_settings(settings: &Settings) -> anyhow::Result<Self> {
        let mut manager = Self {
            clients: HashMap::new(),
            tools: Vec::new(),
        };

        for (name, config) in &settings.mcp_servers {
            info!(server_name = %name, command = %config.command, "Initializing MCP server");

            if let Err(e) = manager.add_server(name.clone(), config.clone()).await {
                error!(error = %e, server_name = %name, "Failed to initialize MCP server");
            }
        }

        let server_count = manager.clients.len();
        let tool_count = manager.tools.len();
        info!(
            servers = server_count,
            tools = tool_count,
            "MCP manager initialized"
        );

        Ok(manager)
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
            let tool = Arc::new(McpTool::new(&mcp_tool, name.clone())?);
            self.tools.push(tool);
        }

        self.clients.insert(name, client);

        Ok(())
    }

    pub fn get_tools(&self) -> &[Arc<McpTool>] {
        &self.tools
    }

    pub fn get_tools_as_executors(&self) -> Vec<Arc<dyn ToolExecutor>> {
        self.tools
            .iter()
            .map(|tool| tool.clone() as Arc<dyn ToolExecutor>)
            .collect()
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

        let original_count = self.tools.len();
        self.tools.retain(|tool| tool.get_server_name() != name);
        let removed_count = original_count - self.tools.len();

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

    /// Get server statistics
    pub fn get_stats(&self) -> McpManagerStats {
        McpManagerStats {
            server_count: self.clients.len(),
            tool_count: self.tools.len(),
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
