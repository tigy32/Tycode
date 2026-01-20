use std::sync::Arc;

use crate::module::{PromptComponent, PromptComponentId};
use crate::settings::config::Settings;
use crate::steering::{Builtin, SteeringDocuments};

pub const ID: PromptComponentId = PromptComponentId("style");

/// Provides style mandate instructions from steering documents.
pub struct StyleMandatesComponent {
    steering: Arc<SteeringDocuments>,
}

impl StyleMandatesComponent {
    pub fn new(steering: Arc<SteeringDocuments>) -> Self {
        Self { steering }
    }
}

impl PromptComponent for StyleMandatesComponent {
    fn id(&self) -> PromptComponentId {
        ID
    }

    fn build_prompt_section(&self, _settings: &Settings) -> Option<String> {
        Some(self.steering.get_builtin(Builtin::StyleMandates))
    }
}
