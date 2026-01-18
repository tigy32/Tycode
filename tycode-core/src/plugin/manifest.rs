//! Plugin manifest parsing for both Claude Code and native plugin formats.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Claude Code plugin manifest (`.claude-plugin/plugin.json`).
///
/// This follows the Claude Code plugin specification for compatibility.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudePluginManifest {
    /// Plugin name (required)
    pub name: String,

    /// Plugin version (required)
    pub version: String,

    /// Plugin description (optional)
    pub description: Option<String>,

    /// Author information (optional)
    pub author: Option<ClaudePluginAuthor>,

    /// Path to commands directory (relative to plugin root)
    pub commands: Option<String>,

    /// Path to agents directory (relative to plugin root)
    pub agents: Option<String>,

    /// Path to skills directory (relative to plugin root)
    pub skills: Option<String>,

    /// Path to hooks configuration file (relative to plugin root)
    pub hooks: Option<String>,

    /// Path to MCP servers configuration file (relative to plugin root)
    #[serde(rename = "mcpServers")]
    pub mcp_servers: Option<String>,

    /// Path to LSP servers configuration file (relative to plugin root)
    #[serde(rename = "lspServers")]
    pub lsp_servers: Option<String>,
}

/// Author information in Claude Code plugin manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudePluginAuthor {
    pub name: String,
    pub email: Option<String>,
}

impl ClaudePluginManifest {
    /// Loads a Claude Code plugin manifest from a JSON file.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read plugin manifest: {}", path.display()))?;

        let manifest: Self = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse plugin manifest: {}", path.display()))?;

        Ok(manifest)
    }

    /// Returns the path to the manifest file given a plugin root directory.
    pub fn manifest_path(plugin_root: &Path) -> std::path::PathBuf {
        plugin_root.join(".claude-plugin").join("plugin.json")
    }
}

/// Native Tycode plugin manifest (`tycode-plugin.toml`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativePluginManifest {
    /// Plugin name (required)
    pub name: String,

    /// Plugin version (required)
    pub version: String,

    /// Plugin description (optional)
    pub description: Option<String>,

    /// Author information (optional)
    pub author: Option<NativePluginAuthor>,

    /// Path to the native library (relative to plugin root)
    pub library: String,

    /// Minimum ABI version required
    #[serde(default = "default_abi_version")]
    pub abi_version: u32,

    /// Optional path to commands directory
    pub commands: Option<String>,

    /// Optional path to agents directory
    pub agents: Option<String>,

    /// Optional path to hooks configuration
    pub hooks: Option<String>,
}

/// Author information in native plugin manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativePluginAuthor {
    pub name: String,
    pub email: Option<String>,
}

fn default_abi_version() -> u32 {
    1
}

impl NativePluginManifest {
    /// Loads a native plugin manifest from a TOML file.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read native plugin manifest: {}", path.display()))?;

        let manifest: Self = toml::from_str(&content)
            .with_context(|| format!("Failed to parse native plugin manifest: {}", path.display()))?;

        Ok(manifest)
    }

    /// Returns the path to the manifest file given a plugin root directory.
    pub fn manifest_path(plugin_root: &Path) -> std::path::PathBuf {
        plugin_root.join("tycode-plugin.toml")
    }
}

/// Hook configuration file format (Claude Code compatible).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HooksConfig {
    /// List of hook definitions
    #[serde(default)]
    pub hooks: Vec<HookDefinition>,
}

impl HooksConfig {
    /// Loads hooks configuration from a JSON file.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read hooks config: {}", path.display()))?;

        let config: Self = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse hooks config: {}", path.display()))?;

        Ok(config)
    }
}

/// Individual hook definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookDefinition {
    /// Hook event to trigger on
    pub event: String,

    /// Matchers for filtering when this hook should fire
    #[serde(default)]
    pub matchers: Vec<HookMatcher>,

    /// Shell command to execute
    pub command: String,

    /// Optional timeout in milliseconds
    #[serde(default = "default_hook_timeout")]
    pub timeout: u64,
}

fn default_hook_timeout() -> u64 {
    30000 // 30 seconds
}

/// Matcher for filtering hook execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookMatcher {
    /// Type of matcher
    #[serde(rename = "type")]
    pub matcher_type: String,

    /// Pattern to match against
    pub pattern: Option<String>,

    /// Tool names to match (for tool-related hooks)
    #[serde(default)]
    pub tool_names: Vec<String>,
}

/// MCP servers configuration file format.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpServersConfig {
    /// Map of server name to configuration
    #[serde(default, rename = "mcpServers")]
    pub mcp_servers: std::collections::HashMap<String, McpServerDefinition>,
}

impl McpServersConfig {
    /// Loads MCP servers configuration from a JSON file.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read MCP servers config: {}", path.display()))?;

        let config: Self = serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse MCP servers config: {}", path.display()))?;

        Ok(config)
    }
}

/// Individual MCP server definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerDefinition {
    /// Command to execute
    pub command: String,

    /// Arguments to pass to the command
    #[serde(default)]
    pub args: Vec<String>,

    /// Environment variables
    #[serde(default)]
    pub env: std::collections::HashMap<String, String>,
}

/// Command/Agent frontmatter metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CommandFrontmatter {
    /// Command/agent name
    pub name: Option<String>,

    /// Description
    pub description: Option<String>,

    /// Allowed tools for this command/agent
    #[serde(default)]
    pub allowed_tools: Vec<String>,

    /// Available tools (alias for allowed_tools)
    #[serde(default)]
    pub available_tools: Vec<String>,

    /// Prompt component selection
    pub prompt_selection: Option<String>,
}

impl CommandFrontmatter {
    /// Parses frontmatter from a markdown file.
    pub fn parse(content: &str) -> Result<(Self, String)> {
        // Check for YAML frontmatter delimiters
        if !content.starts_with("---") {
            return Ok((Self::default(), content.to_string()));
        }

        // Find the closing delimiter
        let rest = &content[3..];
        let end_pos = rest.find("\n---");

        if let Some(pos) = end_pos {
            let yaml_content = &rest[..pos];
            let markdown_content = &rest[pos + 4..];

            let frontmatter: Self = serde_yaml::from_str(yaml_content.trim())
                .with_context(|| "Failed to parse frontmatter YAML")?;

            Ok((frontmatter, markdown_content.trim_start().to_string()))
        } else {
            // No closing delimiter found, treat entire content as markdown
            Ok((Self::default(), content.to_string()))
        }
    }

    /// Merges allowed_tools and available_tools into a single list.
    pub fn all_tools(&self) -> Vec<String> {
        let mut tools = self.allowed_tools.clone();
        tools.extend(self.available_tools.clone());
        tools.sort();
        tools.dedup();
        tools
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_claude_plugin_manifest_parsing() {
        let temp = TempDir::new().unwrap();
        let plugin_dir = temp.path().join("test-plugin");
        let manifest_dir = plugin_dir.join(".claude-plugin");
        fs::create_dir_all(&manifest_dir).unwrap();

        let manifest_content = r#"{
            "name": "test-plugin",
            "version": "1.0.0",
            "description": "A test plugin",
            "author": {
                "name": "Test Author",
                "email": "test@example.com"
            },
            "commands": "./commands/",
            "agents": "./agents/",
            "hooks": "./hooks/hooks.json",
            "mcpServers": "./.mcp.json"
        }"#;

        let manifest_path = manifest_dir.join("plugin.json");
        fs::write(&manifest_path, manifest_content).unwrap();

        let manifest = ClaudePluginManifest::load(&manifest_path).unwrap();
        assert_eq!(manifest.name, "test-plugin");
        assert_eq!(manifest.version, "1.0.0");
        assert_eq!(manifest.description.as_deref(), Some("A test plugin"));
        assert!(manifest.author.is_some());
        assert_eq!(manifest.commands.as_deref(), Some("./commands/"));
    }

    #[test]
    fn test_native_plugin_manifest_parsing() {
        let temp = TempDir::new().unwrap();
        let manifest_path = temp.path().join("tycode-plugin.toml");

        let manifest_content = r#"
name = "native-plugin"
version = "0.1.0"
description = "A native test plugin"
library = "libplugin.dylib"
abi_version = 1

[author]
name = "Test Author"
"#;

        fs::write(&manifest_path, manifest_content).unwrap();

        let manifest = NativePluginManifest::load(&manifest_path).unwrap();
        assert_eq!(manifest.name, "native-plugin");
        assert_eq!(manifest.version, "0.1.0");
        assert_eq!(manifest.library, "libplugin.dylib");
        assert_eq!(manifest.abi_version, 1);
    }

    #[test]
    fn test_command_frontmatter_parsing() {
        let content = r#"---
name: test-command
description: A test command
allowed_tools:
  - read_file
  - write_file
---

# Test Command

Instructions for the command.
"#;

        let (frontmatter, markdown) = CommandFrontmatter::parse(content).unwrap();
        assert_eq!(frontmatter.name.as_deref(), Some("test-command"));
        assert_eq!(frontmatter.description.as_deref(), Some("A test command"));
        assert_eq!(frontmatter.allowed_tools, vec!["read_file", "write_file"]);
        assert!(markdown.starts_with("# Test Command"));
    }

    #[test]
    fn test_frontmatter_without_delimiters() {
        let content = "# Just Markdown\n\nNo frontmatter here.";
        let (frontmatter, markdown) = CommandFrontmatter::parse(content).unwrap();
        assert!(frontmatter.name.is_none());
        assert_eq!(markdown, content);
    }

    #[test]
    fn test_hooks_config_parsing() {
        let temp = TempDir::new().unwrap();
        let hooks_path = temp.path().join("hooks.json");

        let hooks_content = r#"{
            "hooks": [
                {
                    "event": "PreToolUse",
                    "matchers": [
                        {
                            "type": "tool_name",
                            "tool_names": ["write_file", "delete_file"]
                        }
                    ],
                    "command": "${CLAUDE_PLUGIN_ROOT}/scripts/validate.sh",
                    "timeout": 5000
                }
            ]
        }"#;

        fs::write(&hooks_path, hooks_content).unwrap();

        let config = HooksConfig::load(&hooks_path).unwrap();
        assert_eq!(config.hooks.len(), 1);
        assert_eq!(config.hooks[0].event, "PreToolUse");
        assert_eq!(config.hooks[0].timeout, 5000);
    }
}
