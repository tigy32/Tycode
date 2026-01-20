use std::sync::Arc;

use crate::module::{PromptComponent, PromptComponentId};
use crate::settings::config::Settings;
use crate::steering::{Builtin, SteeringDocuments};

pub const ID: PromptComponentId = PromptComponentId("communication");

/// Provides communication guidelines from steering documents.
pub struct CommunicationComponent {
    steering: Arc<SteeringDocuments>,
}

impl CommunicationComponent {
    pub fn new(steering: Arc<SteeringDocuments>) -> Self {
        Self { steering }
    }
}

impl PromptComponent for CommunicationComponent {
    fn id(&self) -> PromptComponentId {
        ID
    }

    fn build_prompt_section(&self, _settings: &Settings) -> Option<String> {
        Some(self.steering.get_builtin(Builtin::CommunicationGuidelines))
    }
}
