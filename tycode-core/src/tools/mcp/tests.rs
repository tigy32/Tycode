#![allow(dead_code)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tempfile::TempDir;
use tokio::process::Child;
use tokio::time::sleep;

use crate::settings::config::{McpServerConfig, Settings};
use crate::tools::mcp::manager::McpManager;
use crate::tools::r#trait::{ToolExecutor, ToolOutput, ToolRequest};

/// Test harness for MCP integration tests
pub struct McpTestHarness {
    temp_dir: TempDir,
    server_process: Option<Child>,
    tools: Vec<Arc<dyn ToolExecutor>>,
}

impl McpTestHarness {
    /// Create a new test harness with a running MCP fetch server
    pub async fn new() -> anyhow::Result<Self> {
        // Create temporary directory that might be needed by some tools
        let temp_dir =
            tempfile::tempdir().map_err(|e| anyhow::anyhow!("Failed to create temp dir: {e}"))?;

        // Create settings that point to our fetch server
        let settings = Self::create_test_settings();

        // Initialize MCP tools directly
        let tools = McpManager::from_settings(&settings).await?;

        // Give the server time to start (if needed for fetch server)
        sleep(Duration::from_secs(2)).await;

        Ok(Self {
            temp_dir,
            server_process: None,
            tools,
        })
    }

    fn create_test_settings() -> Settings {
        let mut mcp_servers = HashMap::new();

        mcp_servers.insert(
            "fetch".to_string(),
            McpServerConfig {
                command: "uvx".to_string(),
                args: vec!["mcp-server-fetch".to_string()],
                env: HashMap::new(),
            },
        );

        Settings {
            mcp_servers,
            ..Default::default()
        }
    }

    /// Get the MCP tools for testing
    pub fn tools(&self) -> &[Arc<dyn ToolExecutor>] {
        &self.tools
    }

    /// Get the temporary directory path
    pub fn temp_dir_path(&self) -> &std::path::Path {
        self.temp_dir.path()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore] // Ignore by default since it requires external dependencies
    async fn test_mcp_integration() -> anyhow::Result<()> {
        let harness = McpTestHarness::new().await?;

        for tool in harness.tools() {
            println!(
                "Found mcp server: {}\n{}",
                tool.description(),
                tool.input_schema()
            );
        }

        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_mcp_fetch_google() -> anyhow::Result<()> {
        let harness = McpTestHarness::new().await?;

        // Get the fetch tool
        let fetch_tool = harness
            .tools()
            .iter()
            .find(|t| t.name() == "mcp_fetch")
            .ok_or(anyhow::anyhow!("mcp_fetch tool not found"))?;

        // Create a request to fetch example.com
        let request = ToolRequest {
            arguments: serde_json::json!({
                "url": "https://example.com",
                "max_length": 1000
            }),
            tool_use_id: "test-example-fetch".to_string(),
        };

        // Process the request using handle API
        let handle = fetch_tool.process(&request).await?;

        // Verify handle was created
        let _ = handle.tool_request();

        // Execute via the tool's handle
        let output = handle.execute().await;

        match output {
            ToolOutput::Result {
                content, is_error, ..
            } => {
                assert!(!is_error, "Tool execution should not error");
                println!("Successfully fetched content from example.com:");
                println!("Content length: {} characters", content.len());
                println!("First 200 characters:");
                println!("{}", content.chars().take(200).collect::<String>());
                assert!(!content.is_empty(), "Should have received some content");
            }
            _ => panic!("Expected ToolOutput::Result, got different variant"),
        }

        Ok(())
    }
}
