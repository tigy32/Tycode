use std::process::Stdio;

use crate::settings::config::McpServerConfig;
use rmcp::{
    model::{CallToolRequestParam, CallToolResult, Tool},
    service::{RunningService, ServiceExt},
    transport::{
        streamable_http_client::{
            StreamableHttpClientTransport, StreamableHttpClientTransportConfig,
        },
        ConfigureCommandExt, TokioChildProcess,
    },
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
        info!(client_name = %name, endpoint = %config.display_label(), "Initializing MCP client");

        let client_handle = match &config {
            McpServerConfig::Stdio { command, args, env } => {
                let cmd = Command::new(command).configure(|c| {
                    c.args(args);
                    c.envs(env.iter());
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
                    .map_err(|e| anyhow::anyhow!("Failed to create stdio MCP transport: {e:?}"))?;

                SimpleClientHandler
                    .serve(transport)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to serve stdio MCP client: {e:?}"))?
            }
            McpServerConfig::Http { url, headers } => {
                let http_config = StreamableHttpClientTransportConfig::with_uri(url.as_str());

                let transport = if headers.is_empty() {
                    StreamableHttpClientTransport::with_client(reqwest::Client::new(), http_config)
                } else {
                    let mut header_map = reqwest::header::HeaderMap::new();
                    for (key, value) in headers {
                        let header_name: reqwest::header::HeaderName = key
                            .parse()
                            .map_err(|e| anyhow::anyhow!("Invalid header name '{key}': {e}"))?;
                        let header_value: reqwest::header::HeaderValue =
                            value.parse().map_err(|e| {
                                anyhow::anyhow!("Invalid header value for '{key}': {e}")
                            })?;
                        header_map.insert(header_name, header_value);
                    }

                    let reqwest_client = reqwest::Client::builder()
                        .default_headers(header_map)
                        .build()
                        .map_err(|e| anyhow::anyhow!("Failed to build HTTP client: {e:?}"))?;

                    StreamableHttpClientTransport::with_client(reqwest_client, http_config)
                };

                SimpleClientHandler
                    .serve(transport)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to serve HTTP MCP client: {e:?}"))?
            }
        };

        debug!(client_name = %name, "MCP client initialized successfully");

        Ok(Self {
            name,
            client_handle,
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
