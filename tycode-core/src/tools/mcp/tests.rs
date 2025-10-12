#![allow(dead_code)]

use std::collections::HashMap;
use std::time::Duration;
use tempfile::TempDir;
use tokio::process::Child;
use tokio::time::sleep;

use crate::settings::config::{McpServerConfig, Settings};
use crate::tools::mcp::manager::McpManager;
use crate::tools::r#trait::{ToolRequest, ValidatedToolCall};

/// Test harness for MCP integration tests
pub struct McpTestHarness {
    temp_dir: TempDir,
    server_process: Option<Child>,
    manager: McpManager,
}

impl McpTestHarness {
    /// Create a new test harness with a running MCP fetch server
    pub async fn new() -> anyhow::Result<Self> {
        // Create temporary directory that might be needed by some tools
        let temp_dir =
            tempfile::tempdir().map_err(|e| anyhow::anyhow!("Failed to create temp dir: {e}"))?;

        // Create settings that point to our fetch server
        let settings = Self::create_test_settings();

        // Initialize MCP manager
        let manager = McpManager::from_settings(&settings).await?;

        // Give the server time to start (if needed for fetch server)
        sleep(Duration::from_secs(2)).await;

        Ok(Self {
            temp_dir,
            server_process: None,
            manager,
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

    /// Get the MCP manager for testing
    pub fn manager(&self) -> &McpManager {
        &self.manager
    }

    /// Get mutable access to the MCP manager
    pub fn manager_mut(&mut self) -> &mut McpManager {
        &mut self.manager
    }

    /// Get the temporary directory path
    pub fn temp_dir_path(&self) -> &std::path::Path {
        self.temp_dir.path()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::r#trait::ToolExecutor;

    #[tokio::test]
    #[ignore] // Ignore by default since it requires external dependencies
    async fn test_mcp_integration() -> anyhow::Result<()> {
        let harness = McpTestHarness::new().await?;

        // Test basic manager access - this ensures the manager is functioning
        let manager_ref = harness.manager();

        for tool in manager_ref.get_tools() {
            println!(
                "Found mcp server: {}: {}\n{}",
                tool.get_server_name(),
                tool.description(),
                tool.input_schema()
            );
        }

        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_mcp_fetch_google() -> anyhow::Result<()> {
        let mut harness = McpTestHarness::new().await?;

        // Get the fetch tool from the manager
        let tools = harness.manager().get_tools();
        let fetch_tool = tools
            .iter()
            .find(|t| t.name() == "mcp_fetch")
            .ok_or(anyhow::anyhow!("mcp_fetch tool not found"))?;

        // Create a request to fetch google.com
        let request = ToolRequest {
            arguments: serde_json::json!({
                "url": "https://example.com",
                "max_length": 1000
            }),
            tool_use_id: "test-example-fetch".to_string(),
        };

        // Validate the request
        let validated = fetch_tool.validate(&request).await?;

        // Execute the request
        match validated {
            ValidatedToolCall::McpCall {
                server_name,
                tool_name,
                arguments,
            } => {
                println!("Calling MCP tool: {}::{}", server_name, tool_name);

                let result = harness
                    .manager_mut()
                    .execute_tool(&server_name, &tool_name, arguments)
                    .await?;

                println!("Successfully fetched content from google.com:");
                println!("Content length: {} characters", result.len());
                println!("First 200 characters:");
                println!("{}", result.chars().take(200).collect::<String>());

                assert!(result.len() > 0, "Should have received some content");
            }
            _ => panic!("Expected McpCall validation result"),
        }

        Ok(())
    }
}
