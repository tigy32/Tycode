use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::module::ContextComponent;
use crate::module::Module;
use crate::module::PromptComponent;
use crate::module::SlashCommand;
use crate::settings::config::{McpServerConfig, Settings};
use crate::tools::r#trait::ToolExecutor;
use tracing::{debug, error, info, warn};

pub mod client;
pub mod command;
pub mod tool;

#[cfg(test)]
mod tests;

use client::McpClient;
use command::McpSlashCommand;
use tool::McpTool;

#[derive(Clone)]
pub struct McpToolDef {
    pub name: String,
    pub tool: rmcp::model::Tool,
    pub server_name: String,
}

pub(crate) struct McpModuleInner {
    pub(crate) clients: HashMap<String, McpClient>,
    pub(crate) tool_defs: Vec<McpToolDef>,
}

pub struct McpModule {
    inner: Arc<RwLock<McpModuleInner>>,
}

impl McpModule {
    pub fn empty() -> Arc<Self> {
        Arc::new(Self {
            inner: Arc::new(RwLock::new(McpModuleInner {
                clients: HashMap::new(),
                tool_defs: Vec::new(),
            })),
        })
    }

    pub async fn from_settings(settings: &Settings) -> anyhow::Result<Arc<Self>> {
        let module = Self::empty();

        for (name, config) in &settings.mcp_servers {
            info!(server_name = %name, command = %config.command, "Initializing MCP server");
            if let Err(e) = module.add_server(name.clone(), config.clone()).await {
                error!(error = ?e, server_name = %name, "Failed to initialize MCP server");
            }
        }

        {
            let inner = module.inner.read().await;
            info!(
                servers = inner.clients.len(),
                tools = inner.tool_defs.len(),
                "MCP module initialized"
            );
        }

        Ok(module)
    }

    pub async fn add_server(&self, name: String, config: McpServerConfig) -> anyhow::Result<()> {
        debug!(server_name = %name, "Adding MCP server");

        let mut client = McpClient::new(name.clone(), config).await.map_err(|e| {
            error!(error = ?e, server_name = %name, "Failed to initialize MCP client");
            anyhow::anyhow!("Failed to initialize MCP server '{name}': {e:?}")
        })?;

        let mcp_tools = client.list_tools().await.map_err(|e| {
            error!(error = ?e, server_name = %name, "Failed to list MCP tools");
            anyhow::anyhow!("Failed to list tools from MCP server '{name}': {e:?}")
        })?;

        debug!(server_name = %name, tool_count = mcp_tools.len(), "Found MCP tools");

        let mut inner = self.inner.write().await;
        for mcp_tool in mcp_tools {
            inner.tool_defs.push(McpToolDef {
                name: format!("mcp_{}", mcp_tool.name),
                tool: mcp_tool,
                server_name: name.clone(),
            });
        }
        inner.clients.insert(name, client);
        Ok(())
    }

    pub async fn remove_server(&self, name: &str) -> anyhow::Result<()> {
        debug!(server_name = %name, "Removing MCP server");
        let mut inner = self.inner.write().await;

        if inner.clients.remove(name).is_some() {
            inner.tool_defs.retain(|def| def.server_name != name);
        }
        Ok(())
    }

    pub fn get_tool_definitions(&self) -> Vec<McpToolDef> {
        match self.inner.try_read() {
            Ok(inner) => inner.tool_defs.clone(),
            Err(_) => {
                warn!("Failed to acquire read lock for MCP tool definitions");
                Vec::new()
            }
        }
    }
}

impl Module for McpModule {
    fn prompt_components(&self) -> Vec<Arc<dyn PromptComponent>> {
        Vec::new()
    }

    fn context_components(&self) -> Vec<Arc<dyn ContextComponent>> {
        Vec::new()
    }

    fn tools(&self) -> Vec<Arc<dyn ToolExecutor>> {
        match self.inner.try_read() {
            Ok(inner) => inner
                .tool_defs
                .iter()
                .filter_map(|def| McpTool::new(def, self.inner.clone()).ok())
                .map(|tool| Arc::new(tool) as Arc<dyn ToolExecutor>)
                .collect(),
            Err(_) => {
                warn!("Failed to acquire read lock for MCP tools");
                Vec::new()
            }
        }
    }

    fn slash_commands(&self) -> Vec<Arc<dyn SlashCommand>> {
        vec![Arc::new(McpSlashCommand)]
    }
}
