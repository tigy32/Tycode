//! Native plugin support for loading Rust plugins as dynamic libraries.
//!
//! This module provides the ability to load native Tycode plugins that are
//! compiled as dynamic libraries (.so, .dylib, .dll).
//!
//! **Security Note**: Native plugins have full access to the system and should
//! only be loaded from trusted sources. Native plugin loading is disabled by
//! default and must be explicitly enabled in settings.
//!
//! ## Plugin ABI
//!
//! Native plugins must implement a specific C ABI for compatibility:
//!
//! ```c
//! // Required: Plugin descriptor function
//! const PluginDescriptor* tycode_plugin_descriptor();
//!
//! // Required: Plugin create function
//! Plugin* tycode_plugin_create();
//!
//! // Required: Plugin destroy function
//! void tycode_plugin_destroy(Plugin* plugin);
//! ```
//!
//! ## Creating a Native Plugin
//!
//! Use the `tycode_plugin!` macro to create a native plugin:
//!
//! ```rust,ignore
//! use tycode_core::plugin::native::*;
//!
//! struct MyPlugin;
//!
//! impl NativePlugin for MyPlugin {
//!     fn name(&self) -> &str { "my-plugin" }
//!     fn version(&self) -> &str { "1.0.0" }
//!     fn description(&self) -> &str { "My custom plugin" }
//!     fn tools(&self) -> Vec<Arc<dyn ToolExecutor>> { vec![] }
//! }
//!
//! tycode_plugin!(MyPlugin);
//! ```

pub mod abi;
pub mod loader;

pub use abi::{NativePlugin, PluginDescriptor, TYCODE_PLUGIN_ABI_VERSION};
pub use loader::NativePluginLoader;
