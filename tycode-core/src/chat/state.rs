use serde::{Deserialize, Serialize};

/// Configuration for chat behavior - moved to ActorState
#[derive(Debug, Clone)]
pub struct ChatConfig {
    pub file_modification_api: FileModificationApi,
    pub trace: bool,
}

impl Default for ChatConfig {
    fn default() -> Self {
        Self {
            file_modification_api: FileModificationApi::FindReplace,
            trace: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[derive(Default)]
pub enum FileModificationApi {
    Patch,
    #[default]
    FindReplace,
}

