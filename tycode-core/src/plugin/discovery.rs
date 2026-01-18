//! Plugin discovery from filesystem.

use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, warn};

use crate::settings::config::PluginsConfig;

use super::manifest::{ClaudePluginManifest, NativePluginManifest};
use super::types::{LoadedPlugin, PluginSource};

/// Handles plugin discovery from multiple directories.
pub struct PluginDiscovery {
    config: PluginsConfig,
    workspace_roots: Vec<PathBuf>,
    home_dir: PathBuf,
}

impl PluginDiscovery {
    /// Creates a new PluginDiscovery instance.
    pub fn new(workspace_roots: &[PathBuf], home_dir: &Path, config: &PluginsConfig) -> Self {
        Self {
            config: config.clone(),
            workspace_roots: workspace_roots.to_vec(),
            home_dir: home_dir.to_path_buf(),
        }
    }

    /// Discovers all plugins from configured directories.
    ///
    /// Discovery order (later overrides earlier):
    /// 1. `~/.claude/plugins/` (Claude Code user-level, lowest priority)
    /// 2. `~/.tycode/plugins/` (Tycode user-level)
    /// 3. Additional configured directories
    /// 4. `.claude/plugins/` in each workspace (Claude Code project-level)
    /// 5. `.tycode/plugins/` in each workspace (Tycode project-level, highest priority)
    pub fn discover(&self) -> HashMap<String, LoadedPlugin> {
        let mut plugins = HashMap::new();

        if !self.config.enabled {
            debug!("Plugin system is disabled in config");
            return plugins;
        }

        // 1. Claude Code user-level plugins (lowest priority)
        if self.config.enable_claude_code_compat {
            let claude_plugins_dir = self.home_dir.join(".claude").join("plugins");
            if claude_plugins_dir.is_dir() {
                debug!("Discovering plugins from {:?}", claude_plugins_dir);
                self.discover_from_directory(
                    &claude_plugins_dir,
                    PluginSource::ClaudeCodeUser,
                    &mut plugins,
                );
            }
        }

        // 2. Tycode user-level plugins
        let tycode_plugins_dir = self.home_dir.join(".tycode").join("plugins");
        if tycode_plugins_dir.is_dir() {
            debug!("Discovering plugins from {:?}", tycode_plugins_dir);
            self.discover_from_directory(
                &tycode_plugins_dir,
                PluginSource::TycodeUser,
                &mut plugins,
            );
        }

        // 3. Additional configured directories
        for dir in &self.config.additional_dirs {
            if dir.is_dir() {
                debug!("Discovering plugins from additional dir {:?}", dir);
                self.discover_from_directory(
                    dir,
                    PluginSource::Additional(dir.clone()),
                    &mut plugins,
                );
            }
        }

        // 4 & 5. Project-level plugins in each workspace
        for workspace_root in &self.workspace_roots {
            // Claude Code project-level
            if self.config.enable_claude_code_compat {
                let claude_plugins_dir = workspace_root.join(".claude").join("plugins");
                if claude_plugins_dir.is_dir() {
                    debug!(
                        "Discovering plugins from {:?} (workspace: {:?})",
                        claude_plugins_dir, workspace_root
                    );
                    self.discover_from_directory(
                        &claude_plugins_dir,
                        PluginSource::ClaudeCodeProject(workspace_root.clone()),
                        &mut plugins,
                    );
                }
            }

            // Tycode project-level (highest priority)
            let tycode_plugins_dir = workspace_root.join(".tycode").join("plugins");
            if tycode_plugins_dir.is_dir() {
                debug!(
                    "Discovering plugins from {:?} (workspace: {:?})",
                    tycode_plugins_dir, workspace_root
                );
                self.discover_from_directory(
                    &tycode_plugins_dir,
                    PluginSource::TycodeProject(workspace_root.clone()),
                    &mut plugins,
                );
            }
        }

        // Filter out disabled plugins
        for name in &self.config.disabled_plugins {
            if let Some(plugin) = plugins.get_mut(name) {
                plugin.set_enabled(false);
                debug!("Plugin '{}' is disabled via config", name);
            }
        }

        debug!("Discovered {} plugins", plugins.len());
        plugins
    }

    /// Discovers plugins from a single directory.
    fn discover_from_directory(
        &self,
        dir: &Path,
        source: PluginSource,
        plugins: &mut HashMap<String, LoadedPlugin>,
    ) {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                warn!("Failed to read plugins directory {:?}: {}", dir, e);
                return;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            // Try to load as Claude Code plugin first
            if let Some(plugin) = self.try_load_claude_plugin(&path, &source) {
                let name = plugin.metadata.name.clone();
                debug!(
                    "Discovered Claude Code plugin '{}' at {:?}",
                    name, path
                );
                plugins.insert(name, plugin);
                continue;
            }

            // Try to load as native plugin
            if self.config.allow_native {
                if let Some(plugin) = self.try_load_native_plugin(&path, &source) {
                    let name = plugin.metadata.name.clone();
                    debug!("Discovered native plugin '{}' at {:?}", name, path);
                    plugins.insert(name, plugin);
                }
            }
        }
    }

    /// Tries to load a Claude Code compatible plugin from a directory.
    fn try_load_claude_plugin(&self, path: &Path, source: &PluginSource) -> Option<LoadedPlugin> {
        let manifest_path = ClaudePluginManifest::manifest_path(path);
        if !manifest_path.exists() {
            return None;
        }

        match ClaudePluginManifest::load(&manifest_path) {
            Ok(manifest) => {
                let mut plugin = LoadedPlugin::from_claude_manifest(manifest.clone(), source.clone(), path.to_path_buf());

                // Load additional components using manifest paths
                if let Err(e) = self.load_plugin_components_from_manifest(&mut plugin, &manifest) {
                    warn!(
                        "Failed to load some components for plugin '{}': {}",
                        plugin.metadata.name, e
                    );
                }

                Some(plugin)
            }
            Err(e) => {
                warn!(
                    "Failed to load Claude Code plugin manifest at {:?}: {}",
                    manifest_path, e
                );
                None
            }
        }
    }

    /// Tries to load a native Tycode plugin from a directory.
    fn try_load_native_plugin(&self, path: &Path, source: &PluginSource) -> Option<LoadedPlugin> {
        let manifest_path = NativePluginManifest::manifest_path(path);
        if !manifest_path.exists() {
            return None;
        }

        match NativePluginManifest::load(&manifest_path) {
            Ok(manifest) => {
                let plugin = LoadedPlugin::from_native_manifest(manifest, source.clone(), path.to_path_buf());
                Some(plugin)
            }
            Err(e) => {
                warn!(
                    "Failed to load native plugin manifest at {:?}: {}",
                    manifest_path, e
                );
                None
            }
        }
    }

    /// Loads additional components for a Claude Code plugin using manifest paths.
    fn load_plugin_components_from_manifest(
        &self,
        plugin: &mut LoadedPlugin,
        manifest: &ClaudePluginManifest,
    ) -> Result<()> {
        let root = &plugin.metadata.root_path;

        // Load commands from manifest path or fall back to default
        let commands_path = manifest
            .commands
            .as_ref()
            .map(|p| root.join(p.trim_start_matches("./")))
            .unwrap_or_else(|| root.join("commands"));
        if commands_path.is_dir() {
            debug!("Loading commands from {:?}", commands_path);
            plugin.commands = super::commands::load_commands_from_directory(
                &commands_path,
                &plugin.metadata.name,
            )?;
        }

        // Load agents from manifest path or fall back to default
        let agents_path = manifest
            .agents
            .as_ref()
            .map(|p| root.join(p.trim_start_matches("./")))
            .unwrap_or_else(|| root.join("agents"));
        if agents_path.is_dir() {
            debug!("Loading agents from {:?}", agents_path);
            plugin.agents = super::agents::load_agents_from_directory(
                &agents_path,
                &plugin.metadata.name,
            )?;
        }

        // Load hooks from manifest path or fall back to default
        let hooks_path = manifest
            .hooks
            .as_ref()
            .map(|p| root.join(p.trim_start_matches("./")))
            .unwrap_or_else(|| root.join("hooks").join("hooks.json"));
        if hooks_path.is_file() {
            debug!("Loading hooks from {:?}", hooks_path);
            plugin.hooks = super::executor::load_hooks_from_file(
                &hooks_path,
                root.clone(),
                &plugin.metadata.name,
            )?;
        }

        // Load MCP servers from manifest path or fall back to default
        let mcp_path = manifest
            .mcp_servers
            .as_ref()
            .map(|p| root.join(p.trim_start_matches("./")))
            .unwrap_or_else(|| root.join(".mcp.json"));
        if mcp_path.is_file() {
            debug!("Loading MCP servers from {:?}", mcp_path);
            plugin.mcp_servers = load_mcp_servers_from_file(&mcp_path, root)?;
        }

        // Skills directory is already set from manifest in from_claude_manifest
        // Auto-detect if not set by manifest
        if plugin.skills_dir.is_none() {
            let skills_dir = root.join("skills");
            if skills_dir.is_dir() {
                debug!("Auto-detected skills directory at {:?}", skills_dir);
                plugin.skills_dir = Some(skills_dir);
            }
        }

        Ok(())
    }
}

/// Loads MCP server configurations from a file.
fn load_mcp_servers_from_file(
    path: &Path,
    plugin_root: &Path,
) -> Result<HashMap<String, crate::settings::config::McpServerConfig>> {
    use super::manifest::McpServersConfig;
    use crate::settings::config::McpServerConfig;

    let config = McpServersConfig::load(path)?;
    let mut servers = HashMap::new();

    for (name, def) in config.mcp_servers {
        // Expand plugin root in command and args
        let command = def
            .command
            .replace("${CLAUDE_PLUGIN_ROOT}", &plugin_root.display().to_string());
        let args: Vec<String> = def
            .args
            .iter()
            .map(|a| a.replace("${CLAUDE_PLUGIN_ROOT}", &plugin_root.display().to_string()))
            .collect();

        servers.insert(
            name,
            McpServerConfig {
                command,
                args,
                env: def.env,
            },
        );
    }

    Ok(servers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_claude_plugin(dir: &Path, name: &str, version: &str) {
        let plugin_dir = dir.join(name);
        let manifest_dir = plugin_dir.join(".claude-plugin");
        fs::create_dir_all(&manifest_dir).unwrap();

        let manifest = format!(
            r#"{{
                "name": "{}",
                "version": "{}",
                "description": "Test plugin"
            }}"#,
            name, version
        );

        fs::write(manifest_dir.join("plugin.json"), manifest).unwrap();
    }

    fn create_native_plugin(dir: &Path, name: &str, version: &str) {
        let plugin_dir = dir.join(name);
        fs::create_dir_all(&plugin_dir).unwrap();

        let manifest = format!(
            r#"
name = "{}"
version = "{}"
library = "libplugin.dylib"
"#,
            name, version
        );

        fs::write(plugin_dir.join("tycode-plugin.toml"), manifest).unwrap();
    }

    #[test]
    fn test_discover_claude_plugins() {
        let temp = TempDir::new().unwrap();
        let plugins_dir = temp.path().join(".tycode").join("plugins");
        fs::create_dir_all(&plugins_dir).unwrap();

        create_claude_plugin(&plugins_dir, "test-plugin", "1.0.0");
        create_claude_plugin(&plugins_dir, "another-plugin", "2.0.0");

        let config = PluginsConfig::default();
        let discovery = PluginDiscovery::new(&[], temp.path(), &config);
        let plugins = discovery.discover();

        assert_eq!(plugins.len(), 2);
        assert!(plugins.contains_key("test-plugin"));
        assert!(plugins.contains_key("another-plugin"));
    }

    #[test]
    fn test_discover_native_plugins() {
        let temp = TempDir::new().unwrap();
        let plugins_dir = temp.path().join(".tycode").join("plugins");
        fs::create_dir_all(&plugins_dir).unwrap();

        create_native_plugin(&plugins_dir, "native-plugin", "0.1.0");

        let config = PluginsConfig {
            allow_native: true,
            ..Default::default()
        };
        let discovery = PluginDiscovery::new(&[], temp.path(), &config);
        let plugins = discovery.discover();

        assert_eq!(plugins.len(), 1);
        assert!(plugins.contains_key("native-plugin"));
    }

    #[test]
    fn test_project_overrides_user() {
        let temp = TempDir::new().unwrap();

        // Create user-level plugin
        let user_plugins = temp.path().join(".tycode").join("plugins");
        fs::create_dir_all(&user_plugins).unwrap();
        create_claude_plugin(&user_plugins, "my-plugin", "1.0.0");

        // Create project-level plugin with same name
        let project_plugins = temp
            .path()
            .join("project")
            .join(".tycode")
            .join("plugins");
        fs::create_dir_all(&project_plugins).unwrap();
        create_claude_plugin(&project_plugins, "my-plugin", "2.0.0");

        let config = PluginsConfig::default();
        let workspace_roots = vec![temp.path().join("project")];
        let discovery = PluginDiscovery::new(&workspace_roots, temp.path(), &config);
        let plugins = discovery.discover();

        // Should have 1 plugin (project overrides user)
        assert_eq!(plugins.len(), 1);

        let plugin = plugins.get("my-plugin").unwrap();
        assert_eq!(plugin.metadata.version, "2.0.0");
        assert!(matches!(
            plugin.metadata.source,
            PluginSource::TycodeProject(_)
        ));
    }

    #[test]
    fn test_disabled_plugins() {
        let temp = TempDir::new().unwrap();
        let plugins_dir = temp.path().join(".tycode").join("plugins");
        fs::create_dir_all(&plugins_dir).unwrap();

        create_claude_plugin(&plugins_dir, "enabled-plugin", "1.0.0");
        create_claude_plugin(&plugins_dir, "disabled-plugin", "1.0.0");

        let mut config = PluginsConfig::default();
        config
            .disabled_plugins
            .insert("disabled-plugin".to_string());

        let discovery = PluginDiscovery::new(&[], temp.path(), &config);
        let plugins = discovery.discover();

        assert_eq!(plugins.len(), 2);
        assert!(plugins.get("enabled-plugin").unwrap().is_enabled());
        assert!(!plugins.get("disabled-plugin").unwrap().is_enabled());
    }

    #[test]
    fn test_plugins_disabled_in_config() {
        let temp = TempDir::new().unwrap();
        let plugins_dir = temp.path().join(".tycode").join("plugins");
        fs::create_dir_all(&plugins_dir).unwrap();
        create_claude_plugin(&plugins_dir, "test-plugin", "1.0.0");

        let mut config = PluginsConfig::default();
        config.enabled = false;

        let discovery = PluginDiscovery::new(&[], temp.path(), &config);
        let plugins = discovery.discover();

        // Plugin discovery is disabled, so no plugins should be found
        assert_eq!(plugins.len(), 0);
    }
}
