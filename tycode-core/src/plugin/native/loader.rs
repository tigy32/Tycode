//! Native plugin loading using libloading.

use std::ffi::CStr;
use std::path::Path;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use tracing::{debug, warn};

use super::abi::{CreateFn, DescriptorFn, DestroyFn, PluginDescriptor, TYCODE_PLUGIN_ABI_VERSION};
use crate::tools::r#trait::ToolExecutor;

/// Handles loading native plugins from dynamic libraries.
pub struct NativePluginLoader;

impl NativePluginLoader {
    /// Loads a native plugin from a dynamic library.
    ///
    /// # Safety
    ///
    /// This function loads and executes code from external dynamic libraries.
    /// Only load plugins from trusted sources.
    #[cfg(feature = "native-plugins")]
    pub fn load(library_path: &Path) -> Result<LoadedNativePlugin> {
        use libloading::Library;

        debug!("Loading native plugin from {:?}", library_path);

        // Load the library
        let library = unsafe {
            Library::new(library_path)
                .with_context(|| format!("Failed to load library: {}", library_path.display()))?
        };

        // Get the descriptor function
        let descriptor_fn: libloading::Symbol<DescriptorFn> = unsafe {
            library
                .get(b"tycode_plugin_descriptor\0")
                .context("Plugin missing tycode_plugin_descriptor function")?
        };

        // Get and validate the descriptor
        let descriptor = unsafe { &*descriptor_fn() };
        Self::validate_descriptor(descriptor)?;

        let name = unsafe { CStr::from_ptr(descriptor.name) }
            .to_str()
            .context("Invalid plugin name")?
            .to_string();

        let version = unsafe { CStr::from_ptr(descriptor.version) }
            .to_str()
            .context("Invalid plugin version")?
            .to_string();

        let description = if descriptor.description.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(descriptor.description) }
                .to_str()
                .unwrap_or("")
                .to_string()
        };

        // Get the create and destroy functions
        let create_fn: libloading::Symbol<CreateFn> = unsafe {
            library
                .get(b"tycode_plugin_create\0")
                .context("Plugin missing tycode_plugin_create function")?
        };

        let destroy_fn: libloading::Symbol<DestroyFn> = unsafe {
            library
                .get(b"tycode_plugin_destroy\0")
                .context("Plugin missing tycode_plugin_destroy function")?
        };

        // Create the plugin instance
        let raw_ptr = unsafe { create_fn() };
        if raw_ptr.is_null() {
            bail!("Plugin create function returned null");
        }

        debug!(
            "Loaded native plugin: {} v{} ({})",
            name, version, description
        );

        Ok(LoadedNativePlugin {
            name,
            version,
            description,
            _library: Arc::new(library),
            raw_ptr,
            destroy_fn: *destroy_fn,
        })
    }

    #[cfg(not(feature = "native-plugins"))]
    pub fn load(_library_path: &Path) -> Result<LoadedNativePlugin> {
        bail!("Native plugin support is not enabled. Compile with the 'native-plugins' feature.")
    }

    fn validate_descriptor(descriptor: &PluginDescriptor) -> Result<()> {
        if descriptor.abi_version != TYCODE_PLUGIN_ABI_VERSION {
            bail!(
                "ABI version mismatch: plugin uses v{}, tycode uses v{}",
                descriptor.abi_version,
                TYCODE_PLUGIN_ABI_VERSION
            );
        }

        if descriptor.name.is_null() {
            bail!("Plugin descriptor has null name");
        }

        if descriptor.version.is_null() {
            bail!("Plugin descriptor has null version");
        }

        Ok(())
    }
}

/// A loaded native plugin.
pub struct LoadedNativePlugin {
    /// Plugin name
    pub name: String,
    /// Plugin version
    pub version: String,
    /// Plugin description
    pub description: String,
    /// Library handle (kept alive to prevent unloading)
    #[cfg(feature = "native-plugins")]
    _library: Arc<libloading::Library>,
    #[cfg(not(feature = "native-plugins"))]
    _library: Arc<()>,
    /// Raw plugin pointer
    raw_ptr: super::abi::RawPluginPtr,
    /// Destroy function
    destroy_fn: DestroyFn,
}

impl LoadedNativePlugin {
    /// Returns the tools provided by this plugin.
    ///
    /// Note: In this simplified implementation, we return an empty list.
    /// Full implementation would require a more complex FFI interface.
    pub fn tools(&self) -> Vec<Arc<dyn ToolExecutor>> {
        // TODO: Implement tool extraction from native plugins
        // This would require additional FFI functions in the plugin ABI
        warn!("Native plugin tool extraction not yet implemented");
        vec![]
    }
}

impl Drop for LoadedNativePlugin {
    fn drop(&mut self) {
        if !self.raw_ptr.is_null() {
            unsafe {
                (self.destroy_fn)(self.raw_ptr);
            }
        }
    }
}

// Ensure LoadedNativePlugin is Send + Sync
// The raw pointer is managed exclusively by us and the library,
// and we ensure proper destruction on drop.
unsafe impl Send for LoadedNativePlugin {}
unsafe impl Sync for LoadedNativePlugin {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_abi_version() {
        assert_eq!(TYCODE_PLUGIN_ABI_VERSION, 1);
    }

    #[test]
    #[cfg(not(feature = "native-plugins"))]
    fn test_load_without_feature() {
        let result = NativePluginLoader::load(Path::new("nonexistent.dylib"));
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("not enabled"));
    }
}
