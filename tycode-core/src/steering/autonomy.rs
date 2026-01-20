use crate::agents::defaults::get_autonomy_instructions;
use crate::module::{PromptComponent, PromptComponentId};
use crate::settings::config::{AutonomyLevel, Settings};

pub const ID: PromptComponentId = PromptComponentId("autonomy");

/// Provides autonomy-level instructions for the system prompt.
pub struct AutonomyComponent {
    level: AutonomyLevel,
}

impl AutonomyComponent {
    pub fn new(level: AutonomyLevel) -> Self {
        Self { level }
    }
}

impl PromptComponent for AutonomyComponent {
    fn id(&self) -> PromptComponentId {
        ID
    }

    fn build_prompt_section(&self, _settings: &Settings) -> Option<String> {
        Some(get_autonomy_instructions(self.level).to_string())
    }
}
