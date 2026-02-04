use crate::settings::config::FileModificationApi;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn is_default_file_modification_api(api: &FileModificationApi) -> bool {
    api == &FileModificationApi::Default
}

fn default_auto_context_bytes() -> usize {
    80_000
}

/// Settings for tools that interact with the file system.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct File {
    /// Controls which tool is used for editing files. Different models work best with
    /// different tools (or leave default for what we think works best).
    #[serde(default, skip_serializing_if = "is_default_file_modification_api")]
    pub file_modification_api: FileModificationApi,

    /// Maximum amount of bytes the file listing can be. Beyond that a directory listing
    /// is not shown to the model and a warning is shown on every message - this generally
    /// means that build artifacts are included in the directory listing and a gitignore
    /// needs to be configured.
    #[serde(default = "default_auto_context_bytes")]
    pub auto_context_bytes: usize,
}

impl File {
    pub const NAMESPACE: &str = "file";
}

impl Default for File {
    fn default() -> Self {
        Self {
            file_modification_api: FileModificationApi::Default,
            auto_context_bytes: default_auto_context_bytes(),
        }
    }
}
