//! Plugin system for Tycode with Claude Code compatibility.
//!
//! This module provides a plugin system that supports both Claude Code-compatible plugins
//! (using `.claude-plugin/plugin.json`) and native Rust plugins (using `tycode-plugin.toml`).
//!
//! ## Plugin Discovery
//!
//! Plugins are discovered from multiple directories in priority order:
//! 1. `~/.claude/plugins/` (Claude Code user-level, lowest priority)
//! 2. `~/.tycode/plugins/` (Tycode user-level)
//! 3. Additional configured directories
//! 4. `.claude/plugins/` in each workspace (Claude Code project-level)
//! 5. `.tycode/plugins/` in each workspace (Tycode project-level, highest priority)
//!
//! ## Plugin Types
//!
//! - **Claude Code Compatible**: Uses `.claude-plugin/plugin.json` manifest
//! - **Native Rust**: Uses `tycode-plugin.toml` manifest with dynamic library loading

pub mod agents;
pub mod commands;
pub mod discovery;
pub mod executor;
pub mod hooks;
pub mod installer;
pub mod manager;
pub mod manifest;
pub mod types;

#[cfg(feature = "native-plugins")]
pub mod native;

pub use discovery::PluginDiscovery;
pub use executor::HookExecutor;
pub use hooks::{HookDispatcher, HookEvent, HookInput, HookOutput, HookResult};
pub use installer::PluginInstaller;
pub use manager::PluginManager;
pub use manifest::{ClaudePluginManifest, NativePluginManifest};
pub use types::{LoadedPlugin, PluginAgent, PluginCommand, PluginSource, PluginType};
