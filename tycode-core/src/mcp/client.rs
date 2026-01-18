use std::process::Stdio;

use crate::settings::config::McpServerConfig;
use rmcp::{
    model::{CallToolRequestParam, CallToolResult, Tool},
    service::{RunningService, ServiceExt},
    transport::{ConfigureCommandExt, TokioChildProcess},
    ClientHandler,
};
use tokio::process::Command;
use tracing::{debug, info};

/// MCP is pretty basic - we aren't parsing any out of bound messages from MCP
/// servers so we use this simple handler that no-ops all messages.
#[derive(Clone, Debug, Default)]
struct SimpleClientHandler;

impl ClientHandler for SimpleClientHandler {}

pub struct McpClient {
    name: String,
    client_handle: RunningService<rmcp::RoleClient, SimpleClientHandler>,
}

impl McpClient {
    pub async fn new(name: String, config: McpServerConfig) -> anyhow::Result<Self> {
        info!(client_name = %name, "Initializing MCP client");

        let cmd = Command::new(&config.command).configure(|c| {
            c.args(&config.args);
            c.envs(config.env.iter());
            c.stderr(Stdio::null());
            #[cfg(unix)]
            c.process_group(0);
            #[cfg(windows)]
            {
                const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
                c.creation_flags(CREATE_NEW_PROCESS_GROUP);
            }
        });

        let transport = TokioChildProcess::new(cmd)
            .map_err(|e| anyhow::anyhow!("Failed to create MCP transport: {e:?}"))?;

        let client: RunningService<rmcp::RoleClient, SimpleClientHandler> = SimpleClientHandler
            .serve(transport)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to serve MCP client: {e:?}"))?;

        debug!(client_name = %name, "MCP client initialized successfully");

        Ok(Self {
            name,
            client_handle: client,
        })
    }

    pub async fn list_tools(&mut self) -> anyhow::Result<Vec<Tool>> {
        let tools_response = self
            .client_handle
            .list_tools(Default::default())
            .await
            .map_err(|e| anyhow::anyhow!("Failed to list MCP tools: {e:?}"))?;

        Ok(tools_response.tools)
    }

    pub async fn call_tool(
        &mut self,
        name: &str,
        arguments: Option<serde_json::Value>,
    ) -> anyhow::Result<CallToolResult> {
        let request = CallToolRequestParam {
            name: name.to_string().into(),
            arguments: arguments.as_ref().and_then(|v| v.as_object().cloned()),
        };

        self.client_handle
            .call_tool(request)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to call MCP tool '{name}': {e:?}"))
    }

    pub async fn close(self) -> anyhow::Result<()> {
        info!(client_name = %self.name, "Closing MCP client");

        self.client_handle
            .cancel()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to cancel MCP client: {e:?}"))?;

        debug!(client_name = %self.name, "MCP client closed successfully");

        Ok(())
    }
}
