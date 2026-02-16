//! Skills system for extending agent capabilities.
//!
//! This module provides support for Claude Code Agent Skills - modular capabilities
//! that extend the agent's functionality. Skills are discovered from (in priority order):
//!
//! 1. `~/.claude/skills/` (user-level Claude Code compatibility)
//! 2. `~/.tycode/skills/` (user-level)
//! 3. `.claude/skills/` in each workspace (project-level Claude Code compatibility)
//! 4. `.tycode/skills/` in each workspace (project-level, highest priority)
//!
//! Later sources override earlier ones if the same skill name is found.
//!
//! Each skill is a directory containing a `SKILL.md` file with YAML frontmatter
//! defining the skill's name and description, followed by markdown instructions.

pub mod command;
pub mod context;
pub mod discovery;
pub mod parser;
pub mod prompt;
pub mod tool;
pub mod types;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use serde_json::Value;

use crate::module::ContextComponent;
use crate::module::PromptComponent;
use crate::module::{Module, SessionStateComponent, SlashCommand};
use crate::settings::config::SkillsConfig;
use crate::tools::r#trait::SharedTool;

use command::{SkillInvokeCommand, SkillsListCommand};

use context::{InvokedSkillsState, SkillsContextComponent};
use discovery::SkillsManager;
use prompt::SkillsPromptComponent;
use tool::InvokeSkillTool;

pub use context::InvokedSkill;
pub use discovery::SkillsManager as Manager;
pub use types::{SkillInstructions, SkillMetadata, SkillSource};

/// Module that provides skills functionality.
///
/// SkillsModule bundles:
/// - `SkillsPromptComponent` - Lists available skills in system prompt
/// - `SkillsContextComponent` - Shows currently active/invoked skills
/// - `InvokeSkillTool` - Tool for loading skill instructions
pub struct SkillsModule {
    manager: SkillsManager,
    state: Arc<InvokedSkillsState>,
}

impl SkillsModule {
    /// Creates a new SkillsModule by discovering skills from configured directories.
    pub fn new(
        workspace_roots: &[PathBuf],
        home_dir: &std::path::Path,
        config: &SkillsConfig,
    ) -> Self {
        let manager = SkillsManager::discover(workspace_roots, home_dir, config);
        let state = Arc::new(InvokedSkillsState::new());
        Self { manager, state }
    }

    /// Creates a SkillsModule with an existing manager (for testing).
    pub fn with_manager(manager: SkillsManager) -> Self {
        let state = Arc::new(InvokedSkillsState::new());
        Self { manager, state }
    }

    /// Returns a reference to the skills manager.
    pub fn manager(&self) -> &SkillsManager {
        &self.manager
    }

    /// Returns a reference to the invoked skills state.
    pub fn state(&self) -> &Arc<InvokedSkillsState> {
        &self.state
    }

    /// Reloads skills from all directories.
    pub fn reload(&self) {
        self.manager.reload();
    }

    /// Returns metadata for all discovered skills.
    pub fn get_all_skills(&self) -> Vec<SkillMetadata> {
        self.manager.get_all_metadata()
    }

    /// Returns metadata for enabled skills only.
    pub fn get_enabled_skills(&self) -> Vec<SkillMetadata> {
        self.manager.get_enabled_metadata()
    }

    /// Gets a skill by name.
    pub fn get_skill(&self, name: &str) -> Option<types::SkillInstructions> {
        self.manager.get_skill(name)
    }
}

impl Module for SkillsModule {
    fn prompt_components(&self) -> Vec<Arc<dyn PromptComponent>> {
        vec![Arc::new(SkillsPromptComponent::new(self.manager.clone()))]
    }

    fn context_components(&self) -> Vec<Arc<dyn ContextComponent>> {
        vec![Arc::new(SkillsContextComponent::new(self.state.clone()))]
    }

    fn tools(&self) -> Vec<SharedTool> {
        vec![Arc::new(InvokeSkillTool::new(
            self.manager.clone(),
            self.state.clone(),
        ))]
    }

    fn session_state(&self) -> Option<Arc<dyn SessionStateComponent>> {
        Some(Arc::new(SkillsSessionState {
            state: self.state.clone(),
        }))
    }

    fn slash_commands(&self) -> Vec<Arc<dyn SlashCommand>> {
        vec![
            Arc::new(SkillsListCommand::new(self.manager.clone())),
            Arc::new(SkillInvokeCommand::new(self.manager.clone())),
        ]
    }
}

/// Session state component for persisting invoked skills.
struct SkillsSessionState {
    state: Arc<InvokedSkillsState>,
}

impl SessionStateComponent for SkillsSessionState {
    fn key(&self) -> &str {
        "skills"
    }

    fn save(&self) -> Value {
        let invoked = self.state.get_invoked();
        serde_json::json!({
            "invoked": invoked.iter().map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "instructions": s.instructions,
                })
            }).collect::<Vec<_>>()
        })
    }

    fn load(&self, state: Value) -> Result<()> {
        self.state.clear();

        if let Some(invoked) = state.get("invoked").and_then(|v| v.as_array()) {
            for skill in invoked {
                if let (Some(name), Some(instructions)) = (
                    skill.get("name").and_then(|v| v.as_str()),
                    skill.get("instructions").and_then(|v| v.as_str()),
                ) {
                    self.state
                        .add_invoked(name.to_string(), instructions.to_string());
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_skill(dir: &std::path::Path, name: &str, description: &str) {
        let skill_dir = dir.join(name);
        fs::create_dir_all(&skill_dir).unwrap();

        let content = format!(
            r#"---
name: {}
description: {}
---

# {} Instructions

Follow these steps.
"#,
            name, description, name
        );

        fs::write(skill_dir.join("SKILL.md"), content).unwrap();
    }

    #[test]
    fn test_skills_module_creation() {
        let temp = TempDir::new().unwrap();
        let skills_dir = temp.path().join(".tycode").join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        create_test_skill(&skills_dir, "test-skill", "A test skill");

        let config = SkillsConfig::default();
        let module = SkillsModule::new(&[], temp.path(), &config);

        assert_eq!(module.get_all_skills().len(), 1);
    }

    #[test]
    fn test_module_provides_components() {
        let temp = TempDir::new().unwrap();
        let config = SkillsConfig::default();
        let module = SkillsModule::new(&[], temp.path(), &config);

        assert_eq!(module.prompt_components().len(), 1);
        assert_eq!(module.context_components().len(), 1);
        assert_eq!(module.tools().len(), 1);
        assert!(module.session_state().is_some());
    }

    #[test]
    fn test_session_state_save_load() {
        let state = Arc::new(InvokedSkillsState::new());
        state.add_invoked("test".to_string(), "instructions".to_string());

        let session = SkillsSessionState {
            state: state.clone(),
        };

        let saved = session.save();

        // Clear and reload
        state.clear();
        assert_eq!(state.get_invoked().len(), 0);

        session.load(saved).unwrap();
        assert_eq!(state.get_invoked().len(), 1);
        assert!(state.is_invoked("test"));
    }
}
