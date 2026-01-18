//! Core types for the plugin system.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::settings::config::McpServerConfig;
use crate::tools::r#trait::ToolExecutor;

use super::hooks::PluginHooks;
use super::manifest::{ClaudePluginManifest, NativePluginManifest};

/// Represents the type/origin of a plugin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginType {
    /// Claude Code compatible plugin (`.claude-plugin/plugin.json`)
    ClaudeCodeCompatible,
    /// Native Rust plugin (`tycode-plugin.toml` + dynamic library)
    TycodeNative,
}

/// Represents the source/location where a plugin was discovered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginSource {
    /// User-level plugin from Claude Code compatibility path (`~/.claude/plugins/`)
    ClaudeCodeUser,
    /// User-level plugin from Tycode path (`~/.tycode/plugins/`)
    TycodeUser,
    /// Project-level plugin from Claude Code compatibility path (`.claude/plugins/`)
    ClaudeCodeProject(PathBuf),
    /// Project-level plugin from Tycode path (`.tycode/plugins/`)
    TycodeProject(PathBuf),
    /// Additional configured directory
    Additional(PathBuf),
}

impl PluginSource {
    /// Returns true if this is a project-level plugin source.
    pub fn is_project_level(&self) -> bool {
        matches!(
            self,
            PluginSource::ClaudeCodeProject(_) | PluginSource::TycodeProject(_)
        )
    }

    /// Returns the workspace root if this is a project-level source.
    pub fn workspace_root(&self) -> Option<&PathBuf> {
        match self {
            PluginSource::ClaudeCodeProject(root) | PluginSource::TycodeProject(root) => Some(root),
            _ => None,
        }
    }
}

/// A slash command defined by a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginCommand {
    /// The command name (without leading slash)
    pub name: String,
    /// Short description of the command
    pub description: String,
    /// Path to the markdown file containing the command definition
    pub path: PathBuf,
    /// The plugin that provides this command
    pub plugin_name: String,
    /// Whether the command is allowed to run
    pub allowed_tools: Vec<String>,
}

/// A subagent defined by a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginAgent {
    /// The agent name/identifier
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// Path to the markdown file containing agent definition
    pub path: PathBuf,
    /// The plugin that provides this agent
    pub plugin_name: String,
    /// Tools available to this agent
    pub available_tools: Vec<String>,
    /// Prompt selection
    pub prompt_selection: Option<String>,
}

/// Metadata about a loaded plugin.
#[derive(Debug, Clone)]
pub struct PluginMetadata {
    /// Plugin name
    pub name: String,
    /// Plugin version
    pub version: String,
    /// Plugin description
    pub description: String,
    /// Author information
    pub author: Option<PluginAuthor>,
    /// Plugin type
    pub plugin_type: PluginType,
    /// Where the plugin was loaded from
    pub source: PluginSource,
    /// Root directory of the plugin
    pub root_path: PathBuf,
    /// Whether the plugin is enabled
    pub enabled: bool,
}

/// Author information for a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginAuthor {
    pub name: String,
    pub email: Option<String>,
}

/// A fully loaded plugin with all its components.
pub struct LoadedPlugin {
    /// Plugin metadata
    pub metadata: PluginMetadata,
    /// Slash commands provided by this plugin
    pub commands: Vec<PluginCommand>,
    /// Subagents provided by this plugin
    pub agents: Vec<PluginAgent>,
    /// Path to skills directory (if any)
    pub skills_dir: Option<PathBuf>,
    /// MCP server configurations
    pub mcp_servers: HashMap<String, McpServerConfig>,
    /// Hook configurations
    pub hooks: PluginHooks,
    /// Native tools (only for native plugins)
    pub native_tools: Vec<Arc<dyn ToolExecutor>>,
}

impl LoadedPlugin {
    /// Creates a new LoadedPlugin from a Claude Code compatible manifest.
    pub fn from_claude_manifest(
        manifest: ClaudePluginManifest,
        source: PluginSource,
        root_path: PathBuf,
    ) -> Self {
        let metadata = PluginMetadata {
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            description: manifest.description.clone().unwrap_or_default(),
            author: manifest.author.map(|a| PluginAuthor {
                name: a.name,
                email: a.email,
            }),
            plugin_type: PluginType::ClaudeCodeCompatible,
            source,
            root_path: root_path.clone(),
            enabled: true,
        };

        Self {
            metadata,
            commands: Vec::new(),
            agents: Vec::new(),
            skills_dir: manifest.skills.map(|p| root_path.join(p)),
            mcp_servers: HashMap::new(),
            hooks: PluginHooks::default(),
            native_tools: Vec::new(),
        }
    }

    /// Creates a new LoadedPlugin from a native Tycode manifest.
    pub fn from_native_manifest(
        manifest: NativePluginManifest,
        source: PluginSource,
        root_path: PathBuf,
    ) -> Self {
        let metadata = PluginMetadata {
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            description: manifest.description.clone().unwrap_or_default(),
            author: manifest.author.map(|a| PluginAuthor {
                name: a.name,
                email: a.email,
            }),
            plugin_type: PluginType::TycodeNative,
            source,
            root_path,
            enabled: true,
        };

        Self {
            metadata,
            commands: Vec::new(),
            agents: Vec::new(),
            skills_dir: None,
            mcp_servers: HashMap::new(),
            hooks: PluginHooks::default(),
            native_tools: Vec::new(),
        }
    }

    /// Returns true if this plugin is enabled.
    pub fn is_enabled(&self) -> bool {
        self.metadata.enabled
    }

    /// Sets the enabled state of this plugin.
    pub fn set_enabled(&mut self, enabled: bool) {
        self.metadata.enabled = enabled;
    }
}

impl std::fmt::Debug for LoadedPlugin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedPlugin")
            .field("metadata", &self.metadata)
            .field("commands", &self.commands)
            .field("agents", &self.agents)
            .field("skills_dir", &self.skills_dir)
            .field("mcp_servers", &self.mcp_servers)
            .field("hooks", &self.hooks)
            .field("native_tools_count", &self.native_tools.len())
            .finish()
    }
}
