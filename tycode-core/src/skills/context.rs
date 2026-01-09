use std::sync::{Arc, RwLock};

use crate::context::{ContextComponent, ContextComponentId};

/// Context component ID for skills.
pub const SKILLS_CONTEXT_ID: ContextComponentId = ContextComponentId("skills");

/// Tracks which skills have been invoked in the current session.
pub struct InvokedSkillsState {
    /// Skills that have been invoked, with their instructions
    invoked: RwLock<Vec<InvokedSkill>>,
}

/// Represents a skill that has been invoked.
#[derive(Clone)]
pub struct InvokedSkill {
    pub name: String,
    pub instructions: String,
}

impl InvokedSkillsState {
    pub fn new() -> Self {
        Self {
            invoked: RwLock::new(Vec::new()),
        }
    }

    /// Records that a skill has been invoked.
    pub fn add_invoked(&self, name: String, instructions: String) {
        let mut invoked = self.invoked.write().unwrap();
        // Check if already invoked (don't duplicate)
        if !invoked.iter().any(|s| s.name == name) {
            invoked.push(InvokedSkill { name, instructions });
        }
    }

    /// Clears all invoked skills (e.g., when starting a new conversation).
    pub fn clear(&self) {
        self.invoked.write().unwrap().clear();
    }

    /// Returns the list of invoked skills.
    pub fn get_invoked(&self) -> Vec<InvokedSkill> {
        self.invoked.read().unwrap().clone()
    }

    /// Checks if a skill has been invoked.
    pub fn is_invoked(&self, name: &str) -> bool {
        self.invoked.read().unwrap().iter().any(|s| s.name == name)
    }
}

impl Default for InvokedSkillsState {
    fn default() -> Self {
        Self::new()
    }
}

/// Context component that shows currently active skills.
///
/// This shows which skills have been invoked in the current session,
/// including their full instructions.
pub struct SkillsContextComponent {
    state: Arc<InvokedSkillsState>,
}

impl SkillsContextComponent {
    pub fn new(state: Arc<InvokedSkillsState>) -> Self {
        Self { state }
    }
}

#[async_trait::async_trait(?Send)]
impl ContextComponent for SkillsContextComponent {
    fn id(&self) -> ContextComponentId {
        SKILLS_CONTEXT_ID
    }

    async fn build_context_section(&self) -> Option<String> {
        let invoked = self.state.get_invoked();

        if invoked.is_empty() {
            return None;
        }

        let mut output = String::new();
        output.push_str("## Active Skills\n\n");
        output.push_str("The following skills have been loaded for this task:\n\n");

        for skill in &invoked {
            output.push_str(&format!("### Skill: {}\n\n", skill.name));
            output.push_str(&skill.instructions);
            output.push_str("\n\n---\n\n");
        }

        Some(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_context_with_invoked_skills() {
        let state = Arc::new(InvokedSkillsState::new());
        state.add_invoked(
            "commit".to_string(),
            "# Commit Skill\n\nInstructions for committing.".to_string(),
        );

        let component = SkillsContextComponent::new(state);
        let context = component.build_context_section().await.unwrap();

        assert!(context.contains("## Active Skills"));
        assert!(context.contains("### Skill: commit"));
        assert!(context.contains("Instructions for committing"));
    }

    #[tokio::test]
    async fn test_context_without_invoked_skills() {
        let state = Arc::new(InvokedSkillsState::new());
        let component = SkillsContextComponent::new(state);
        let context = component.build_context_section().await;

        assert!(context.is_none());
    }

    #[test]
    fn test_no_duplicate_invocations() {
        let state = InvokedSkillsState::new();
        state.add_invoked("commit".to_string(), "Instructions 1".to_string());
        state.add_invoked("commit".to_string(), "Instructions 2".to_string());

        assert_eq!(state.get_invoked().len(), 1);
    }
}
