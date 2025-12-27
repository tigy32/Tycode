use std::sync::Arc;

use crate::prompt::{PromptComponent, PromptComponentId};
use crate::settings::config::Settings;
use crate::steering::{Builtin, SteeringDocuments};

pub const ID: PromptComponentId = PromptComponentId("tools");

/// Provides tool usage instructions from steering documents.
pub struct ToolInstructionsComponent {
    steering: Arc<SteeringDocuments>,
}

impl ToolInstructionsComponent {
    pub fn new(steering: Arc<SteeringDocuments>) -> Self {
        Self { steering }
    }
}

impl PromptComponent for ToolInstructionsComponent {
    fn id(&self) -> PromptComponentId {
        ID
    }

    fn build_prompt_section(&self, _settings: &Settings) -> Option<String> {
        Some(self.steering.get_builtin(Builtin::UnderstandingTools))
    }
}
