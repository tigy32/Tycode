//! Plugin manager for lifecycle and access.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use anyhow::Result;
use tracing::{debug, info};

use crate::settings::config::{McpServerConfig, PluginsConfig};
use crate::tools::r#trait::ToolExecutor;

use super::discovery::PluginDiscovery;
use super::executor::HookExecutor;
use super::hooks::{HookDispatcher, HookEvent, HookInput, HookResult, PluginHooks};
use super::types::{LoadedPlugin, PluginAgent, PluginCommand};

/// Manages plugins throughout their lifecycle.
pub struct PluginManager {
    inner: Arc<PluginManagerInner>,
}

struct PluginManagerInner {
    /// All loaded plugins
    plugins: RwLock<HashMap<String, LoadedPlugin>>,
    /// Hook dispatcher for routing events
    hook_dispatcher: RwLock<HookDispatcher>,
    /// Configuration
    config: PluginsConfig,
    /// Workspace roots
    workspace_roots: Vec<PathBuf>,
    /// Home directory
    home_dir: PathBuf,
}

impl PluginManager {
    /// Creates a new PluginManager and discovers plugins.
    pub fn new(workspace_roots: &[PathBuf], home_dir: &Path, config: &PluginsConfig) -> Self {
        let inner = Arc::new(PluginManagerInner {
            plugins: RwLock::new(HashMap::new()),
            hook_dispatcher: RwLock::new(HookDispatcher::new()),
            config: config.clone(),
            workspace_roots: workspace_roots.to_vec(),
            home_dir: home_dir.to_path_buf(),
        });

        let manager = Self { inner };

        if config.enabled {
            manager.reload();
        }

        manager
    }

    /// Reloads all plugins from configured directories.
    pub fn reload(&self) {
        let discovery = PluginDiscovery::new(
            &self.inner.workspace_roots,
            &self.inner.home_dir,
            &self.inner.config,
        );

        let plugins = discovery.discover();
        let plugin_count = plugins.len();

        // Rebuild hook dispatcher
        let mut dispatcher = HookDispatcher::new();
        for plugin in plugins.values() {
            if plugin.is_enabled() {
                dispatcher.register_hooks(plugin.hooks.clone());
            }
        }

        // Update state
        *self.inner.plugins.write().unwrap() = plugins;
        *self.inner.hook_dispatcher.write().unwrap() = dispatcher;

        info!("Loaded {} plugins", plugin_count);
    }

    /// Returns all loaded plugin names.
    pub fn plugin_names(&self) -> Vec<String> {
        self.inner
            .plugins
            .read()
            .unwrap()
            .keys()
            .cloned()
            .collect()
    }

    /// Returns all enabled plugin names.
    pub fn enabled_plugin_names(&self) -> Vec<String> {
        self.inner
            .plugins
            .read()
            .unwrap()
            .iter()
            .filter(|(_, p)| p.is_enabled())
            .map(|(name, _)| name.clone())
            .collect()
    }

    /// Returns metadata for all plugins.
    pub fn get_all_metadata(&self) -> Vec<PluginInfo> {
        self.inner
            .plugins
            .read()
            .unwrap()
            .values()
            .map(|p| PluginInfo {
                name: p.metadata.name.clone(),
                version: p.metadata.version.clone(),
                description: p.metadata.description.clone(),
                enabled: p.metadata.enabled,
                plugin_type: format!("{:?}", p.metadata.plugin_type),
                source: format!("{:?}", p.metadata.source),
                commands_count: p.commands.len(),
                agents_count: p.agents.len(),
                skills_count: count_skills_in_dir(p.skills_dir.as_ref()),
                mcp_servers_count: p.mcp_servers.len(),
            })
            .collect()
    }

    /// Gets a specific plugin by name.
    pub fn get_plugin(&self, name: &str) -> Option<PluginInfo> {
        self.inner.plugins.read().unwrap().get(name).map(|p| PluginInfo {
            name: p.metadata.name.clone(),
            version: p.metadata.version.clone(),
            description: p.metadata.description.clone(),
            enabled: p.metadata.enabled,
            plugin_type: format!("{:?}", p.metadata.plugin_type),
            source: format!("{:?}", p.metadata.source),
            commands_count: p.commands.len(),
            agents_count: p.agents.len(),
            skills_count: count_skills_in_dir(p.skills_dir.as_ref()),
            mcp_servers_count: p.mcp_servers.len(),
        })
    }

    /// Enables a plugin.
    pub fn enable_plugin(&self, name: &str) -> Result<()> {
        let mut plugins = self.inner.plugins.write().unwrap();
        if let Some(plugin) = plugins.get_mut(name) {
            plugin.set_enabled(true);

            // Re-register hooks
            let mut dispatcher = self.inner.hook_dispatcher.write().unwrap();
            dispatcher.register_hooks(plugin.hooks.clone());

            debug!("Enabled plugin '{}'", name);
            Ok(())
        } else {
            Err(anyhow::anyhow!("Plugin '{}' not found", name))
        }
    }

    /// Disables a plugin.
    pub fn disable_plugin(&self, name: &str) -> Result<()> {
        let mut plugins = self.inner.plugins.write().unwrap();
        if let Some(plugin) = plugins.get_mut(name) {
            plugin.set_enabled(false);

            // Rebuild hook dispatcher without this plugin
            drop(plugins); // Release lock
            let plugins = self.inner.plugins.read().unwrap();
            let mut dispatcher = HookDispatcher::new();
            for p in plugins.values() {
                if p.is_enabled() {
                    dispatcher.register_hooks(p.hooks.clone());
                }
            }
            *self.inner.hook_dispatcher.write().unwrap() = dispatcher;

            debug!("Disabled plugin '{}'", name);
            Ok(())
        } else {
            Err(anyhow::anyhow!("Plugin '{}' not found", name))
        }
    }

    /// Returns all commands from enabled plugins.
    pub fn get_all_commands(&self) -> Vec<PluginCommand> {
        self.inner
            .plugins
            .read()
            .unwrap()
            .values()
            .filter(|p| p.is_enabled())
            .flat_map(|p| p.commands.clone())
            .collect()
    }

    /// Returns all agents from enabled plugins.
    pub fn get_all_agents(&self) -> Vec<PluginAgent> {
        self.inner
            .plugins
            .read()
            .unwrap()
            .values()
            .filter(|p| p.is_enabled())
            .flat_map(|p| p.agents.clone())
            .collect()
    }

    /// Returns all skill directories from enabled plugins.
    pub fn get_all_skill_dirs(&self) -> Vec<PathBuf> {
        self.inner
            .plugins
            .read()
            .unwrap()
            .values()
            .filter(|p| p.is_enabled())
            .filter_map(|p| p.skills_dir.clone())
            .collect()
    }

    /// Returns all MCP servers from enabled plugins.
    pub fn get_all_mcp_servers(&self) -> HashMap<String, McpServerConfig> {
        let mut servers = HashMap::new();

        for plugin in self.inner.plugins.read().unwrap().values() {
            if plugin.is_enabled() {
                for (name, config) in &plugin.mcp_servers {
                    // Prefix with plugin name to avoid conflicts
                    let full_name = format!("{}:{}", plugin.metadata.name, name);
                    servers.insert(full_name, config.clone());
                }
            }
        }

        servers
    }

    /// Returns all native tools from enabled plugins.
    pub fn get_all_native_tools(&self) -> Vec<Arc<dyn ToolExecutor>> {
        self.inner
            .plugins
            .read()
            .unwrap()
            .values()
            .filter(|p| p.is_enabled())
            .flat_map(|p| p.native_tools.clone())
            .collect()
    }

    /// Checks if there are any hooks for the given event.
    pub fn has_hooks(&self, event: HookEvent) -> bool {
        self.inner
            .hook_dispatcher
            .read()
            .unwrap()
            .has_hooks(event)
    }

    /// Creates a HookExecutor for dispatching events.
    pub fn create_hook_executor(&self) -> HookExecutor {
        let dispatcher = self.inner.hook_dispatcher.read().unwrap().clone();
        HookExecutor::new(dispatcher)
    }

    /// Dispatches a hook event and returns the result.
    pub async fn dispatch_hook(&self, event: HookEvent, input: HookInput) -> HookResult {
        let executor = self.create_hook_executor();
        executor.dispatch(event, input).await
    }

    /// Returns the total number of loaded plugins.
    pub fn count(&self) -> usize {
        self.inner.plugins.read().unwrap().len()
    }

    /// Returns the number of enabled plugins.
    pub fn enabled_count(&self) -> usize {
        self.inner
            .plugins
            .read()
            .unwrap()
            .values()
            .filter(|p| p.is_enabled())
            .count()
    }

    /// Gets a command by name from any enabled plugin.
    pub fn get_command(&self, name: &str) -> Option<PluginCommand> {
        self.inner
            .plugins
            .read()
            .unwrap()
            .values()
            .filter(|p| p.is_enabled())
            .flat_map(|p| p.commands.iter())
            .find(|c| c.name == name)
            .cloned()
    }

    /// Gets an agent by name from any enabled plugin.
    pub fn get_agent(&self, name: &str) -> Option<PluginAgent> {
        self.inner
            .plugins
            .read()
            .unwrap()
            .values()
            .filter(|p| p.is_enabled())
            .flat_map(|p| p.agents.iter())
            .find(|a| a.name == name)
            .cloned()
    }

    /// Returns summary information about all registered hooks.
    pub fn get_hooks_summary(&self) -> Vec<HookSummary> {
        let mut summaries = Vec::new();
        let plugins = self.inner.plugins.read().unwrap();

        for plugin in plugins.values() {
            if !plugin.is_enabled() {
                continue;
            }

            for (event, hooks) in &plugin.hooks.hooks {
                for hook in hooks {
                    let tool_filters: Vec<String> = hook
                        .definition
                        .matchers
                        .iter()
                        .filter(|m| m.matcher_type == "tool_name")
                        .flat_map(|m| m.tool_names.clone())
                        .collect();

                    summaries.push(HookSummary {
                        plugin_name: plugin.metadata.name.clone(),
                        event: *event,
                        command: hook.expanded_command(),
                        tool_filters,
                        timeout_ms: hook.definition.timeout,
                    });
                }
            }
        }

        // Sort by event, then plugin name
        summaries.sort_by(|a, b| {
            a.event
                .as_str()
                .cmp(b.event.as_str())
                .then_with(|| a.plugin_name.cmp(&b.plugin_name))
        });

        summaries
    }

    /// Returns the total number of registered hooks.
    pub fn hooks_count(&self) -> usize {
        self.inner
            .hook_dispatcher
            .read()
            .unwrap()
            .all_hooks
            .hooks
            .values()
            .map(|v| v.len())
            .sum()
    }
}

/// Summary information about a registered hook.
#[derive(Debug, Clone)]
pub struct HookSummary {
    pub plugin_name: String,
    pub event: HookEvent,
    pub command: String,
    pub tool_filters: Vec<String>,
    pub timeout_ms: u64,
}

impl Clone for PluginManager {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl Clone for HookDispatcher {
    fn clone(&self) -> Self {
        Self {
            all_hooks: PluginHooks {
                hooks: self.all_hooks.hooks.clone(),
            },
        }
    }
}

/// Summary information about a plugin.
#[derive(Debug, Clone)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    pub description: String,
    pub enabled: bool,
    pub plugin_type: String,
    pub source: String,
    pub commands_count: usize,
    pub agents_count: usize,
    pub skills_count: usize,
    pub mcp_servers_count: usize,
}

impl std::fmt::Display for PluginInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let status = if self.enabled { "enabled" } else { "disabled" };
        write!(
            f,
            "{} v{} ({}) - {} command(s), {} agent(s), {} skill(s)",
            self.name, self.version, status, self.commands_count, self.agents_count, self.skills_count
        )
    }
}

/// Counts skills in a directory by looking for SKILL.md files in subdirectories.
fn count_skills_in_dir(skills_dir: Option<&PathBuf>) -> usize {
    let Some(dir) = skills_dir else {
        return 0;
    };

    if !dir.is_dir() {
        return 0;
    }

    let mut count = 0;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Check if this skill directory has a SKILL.md file
                if path.join("SKILL.md").exists() {
                    count += 1;
                }
            }
        }
    }

    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_plugin_with_command(dir: &Path, name: &str) {
        let plugin_dir = dir.join(name);
        let manifest_dir = plugin_dir.join(".claude-plugin");
        let commands_dir = plugin_dir.join("commands");

        fs::create_dir_all(&manifest_dir).unwrap();
        fs::create_dir_all(&commands_dir).unwrap();

        // Create manifest
        let manifest = format!(
            r#"{{
                "name": "{}",
                "version": "1.0.0",
                "description": "Test plugin"
            }}"#,
            name
        );
        fs::write(manifest_dir.join("plugin.json"), manifest).unwrap();

        // Create command
        let command = r#"---
name: test-command
description: A test command
---

# Test Command

Instructions.
"#;
        fs::write(commands_dir.join("test-command.md"), command).unwrap();
    }

    #[test]
    fn test_plugin_manager_discovery() {
        let temp = TempDir::new().unwrap();
        let plugins_dir = temp.path().join(".tycode").join("plugins");
        fs::create_dir_all(&plugins_dir).unwrap();

        create_plugin_with_command(&plugins_dir, "plugin-a");
        create_plugin_with_command(&plugins_dir, "plugin-b");

        let config = PluginsConfig::default();
        let manager = PluginManager::new(&[], temp.path(), &config);

        assert_eq!(manager.count(), 2);
        assert_eq!(manager.enabled_count(), 2);
    }

    #[test]
    fn test_get_all_commands() {
        let temp = TempDir::new().unwrap();
        let plugins_dir = temp.path().join(".tycode").join("plugins");
        fs::create_dir_all(&plugins_dir).unwrap();

        create_plugin_with_command(&plugins_dir, "test-plugin");

        let config = PluginsConfig::default();
        let manager = PluginManager::new(&[], temp.path(), &config);

        let commands = manager.get_all_commands();
        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "test-command");
    }

    #[test]
    fn test_disable_plugin() {
        let temp = TempDir::new().unwrap();
        let plugins_dir = temp.path().join(".tycode").join("plugins");
        fs::create_dir_all(&plugins_dir).unwrap();

        create_plugin_with_command(&plugins_dir, "my-plugin");

        let config = PluginsConfig::default();
        let manager = PluginManager::new(&[], temp.path(), &config);

        assert_eq!(manager.enabled_count(), 1);

        manager.disable_plugin("my-plugin").unwrap();
        assert_eq!(manager.enabled_count(), 0);

        // Commands from disabled plugins should not be returned
        let commands = manager.get_all_commands();
        assert!(commands.is_empty());
    }

    #[test]
    fn test_enable_plugin() {
        let temp = TempDir::new().unwrap();
        let plugins_dir = temp.path().join(".tycode").join("plugins");
        fs::create_dir_all(&plugins_dir).unwrap();

        create_plugin_with_command(&plugins_dir, "my-plugin");

        let config = PluginsConfig::default();
        let manager = PluginManager::new(&[], temp.path(), &config);

        manager.disable_plugin("my-plugin").unwrap();
        assert_eq!(manager.enabled_count(), 0);

        manager.enable_plugin("my-plugin").unwrap();
        assert_eq!(manager.enabled_count(), 1);
    }
}
