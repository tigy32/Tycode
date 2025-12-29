use crate::prompt::{PromptComponent, PromptComponentId};
use crate::settings::config::Settings;
use crate::steering::{Builtin, SteeringDocuments};
use std::sync::Arc;

pub const PROMPT_ID: PromptComponentId = PromptComponentId("tasks");

/// Provides task list management instructions from steering documents.
pub struct TaskListPromptComponent {
    steering: Arc<SteeringDocuments>,
}

impl TaskListPromptComponent {
    pub fn new(steering: Arc<SteeringDocuments>) -> Self {
        Self { steering }
    }
}

impl PromptComponent for TaskListPromptComponent {
    fn id(&self) -> PromptComponentId {
        PROMPT_ID
    }

    fn build_prompt_section(&self, _settings: &Settings) -> Option<String> {
        Some(self.steering.get_builtin(Builtin::TaskListManagement))
    }
}
