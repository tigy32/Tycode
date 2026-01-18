//! ABI definitions for native plugin loading.
//!
//! This module defines the C ABI interface that native plugins must implement.

use std::sync::Arc;

use crate::module::Module;
use crate::tools::r#trait::ToolExecutor;

/// Current ABI version. Plugins must match this version to be loaded.
pub const TYCODE_PLUGIN_ABI_VERSION: u32 = 1;

/// Plugin descriptor returned by the plugin's `tycode_plugin_descriptor` function.
#[repr(C)]
pub struct PluginDescriptor {
    /// ABI version this plugin was compiled against
    pub abi_version: u32,
    /// Plugin name (null-terminated UTF-8 string)
    pub name: *const std::ffi::c_char,
    /// Plugin version (null-terminated UTF-8 string)
    pub version: *const std::ffi::c_char,
    /// Plugin description (null-terminated UTF-8 string, may be null)
    pub description: *const std::ffi::c_char,
}

/// Trait that native plugins must implement.
pub trait NativePlugin: Send + Sync {
    /// Returns the plugin name.
    fn name(&self) -> &str;

    /// Returns the plugin version.
    fn version(&self) -> &str;

    /// Returns the plugin description.
    fn description(&self) -> &str;

    /// Returns the tools provided by this plugin.
    fn tools(&self) -> Vec<Arc<dyn ToolExecutor>> {
        vec![]
    }

    /// Returns this plugin as a Module if it implements one.
    fn as_module(&self) -> Option<Arc<dyn Module>> {
        None
    }

    /// Called when the plugin is loaded.
    fn on_load(&self) {}

    /// Called when the plugin is unloaded.
    fn on_unload(&self) {}
}

/// Raw pointer type for plugin instances.
pub type RawPluginPtr = *mut std::ffi::c_void;

/// Function signature for `tycode_plugin_descriptor`.
pub type DescriptorFn = unsafe extern "C" fn() -> *const PluginDescriptor;

/// Function signature for `tycode_plugin_create`.
pub type CreateFn = unsafe extern "C" fn() -> RawPluginPtr;

/// Function signature for `tycode_plugin_destroy`.
pub type DestroyFn = unsafe extern "C" fn(RawPluginPtr);

/// Function signature for getting a trait object from the raw plugin pointer.
/// This is called after create to get a usable `dyn NativePlugin`.
pub type GetTraitObjectFn = unsafe extern "C" fn(RawPluginPtr) -> *mut dyn NativePlugin;

/// Macro to declare a native plugin.
///
/// This macro generates the necessary C ABI functions for a plugin.
///
/// # Example
///
/// ```rust,ignore
/// use tycode_core::plugin::native::*;
///
/// struct MyPlugin;
///
/// impl NativePlugin for MyPlugin {
///     fn name(&self) -> &str { "my-plugin" }
///     fn version(&self) -> &str { "1.0.0" }
///     fn description(&self) -> &str { "My plugin" }
/// }
///
/// tycode_plugin!(MyPlugin, MyPlugin::new);
/// ```
#[macro_export]
macro_rules! tycode_plugin {
    ($plugin_type:ty, $constructor:expr) => {
        static PLUGIN_NAME: &str = concat!(stringify!($plugin_type), "\0");
        static PLUGIN_VERSION: &str = concat!(env!("CARGO_PKG_VERSION"), "\0");

        static DESCRIPTOR: $crate::plugin::native::abi::PluginDescriptor =
            $crate::plugin::native::abi::PluginDescriptor {
                abi_version: $crate::plugin::native::abi::TYCODE_PLUGIN_ABI_VERSION,
                name: PLUGIN_NAME.as_ptr() as *const std::ffi::c_char,
                version: PLUGIN_VERSION.as_ptr() as *const std::ffi::c_char,
                description: std::ptr::null(),
            };

        #[no_mangle]
        pub unsafe extern "C" fn tycode_plugin_descriptor(
        ) -> *const $crate::plugin::native::abi::PluginDescriptor {
            &DESCRIPTOR
        }

        #[no_mangle]
        pub unsafe extern "C" fn tycode_plugin_create(
        ) -> $crate::plugin::native::abi::RawPluginPtr {
            let plugin = Box::new($constructor());
            Box::into_raw(plugin) as $crate::plugin::native::abi::RawPluginPtr
        }

        #[no_mangle]
        pub unsafe extern "C" fn tycode_plugin_destroy(
            ptr: $crate::plugin::native::abi::RawPluginPtr,
        ) {
            if !ptr.is_null() {
                let _ = Box::from_raw(ptr as *mut $plugin_type);
            }
        }
    };
}
