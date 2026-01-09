use crate::prompt::{PromptComponent, PromptComponentId};
use crate::settings::config::Settings;

use super::discovery::SkillsManager;

/// Prompt component ID for skills.
pub const SKILLS_PROMPT_ID: PromptComponentId = PromptComponentId("skills");

/// Prompt component that lists available skills in the system prompt.
///
/// This provides "Level 1" loading of skills - just metadata (name + description)
/// that helps the AI decide when to invoke skills.
pub struct SkillsPromptComponent {
    manager: SkillsManager,
}

impl SkillsPromptComponent {
    pub fn new(manager: SkillsManager) -> Self {
        Self { manager }
    }
}

impl PromptComponent for SkillsPromptComponent {
    fn id(&self) -> PromptComponentId {
        SKILLS_PROMPT_ID
    }

    fn build_prompt_section(&self, _settings: &Settings) -> Option<String> {
        let skills = self.manager.get_enabled_metadata();

        if skills.is_empty() {
            return None;
        }

        let mut output = String::new();
        output.push_str("## Available Skills\n\n");
        output.push_str("You have access to skills that provide specialized capabilities. ");
        output.push_str("When a user's request matches a skill's description, ");
        output.push_str("you MUST use the `invoke_skill` tool to load the skill instructions.\n\n");

        output.push_str("| Skill | When to Use |\n");
        output.push_str("|-------|-------------|\n");

        for skill in &skills {
            // Truncate description for table display
            let desc = if skill.description.len() > 80 {
                format!("{}...", &skill.description[..77])
            } else {
                skill.description.clone()
            };
            output.push_str(&format!("| {} | {} |\n", skill.name, desc));
        }

        output.push_str("\n**CRITICAL**: You MUST call `invoke_skill` tool with the skill name to load instructions. ");
        output.push_str("Do NOT attempt to read SKILL.md files directly via file tools or set_tracked_files. ");
        output.push_str("The `invoke_skill` tool is the ONLY correct way to activate a skill.\n");

        Some(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::config::SkillsConfig;
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
"#,
            name, description, name
        );

        fs::write(skill_dir.join("SKILL.md"), content).unwrap();
    }

    #[test]
    fn test_prompt_with_skills() {
        let temp = TempDir::new().unwrap();
        let skills_dir = temp.path().join(".tycode").join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        create_test_skill(&skills_dir, "commit", "When committing changes to git");
        create_test_skill(&skills_dir, "pdf", "When working with PDF documents");

        let config = SkillsConfig::default();
        let manager = SkillsManager::discover(&[], temp.path(), &config);
        let component = SkillsPromptComponent::new(manager);

        let settings = Settings::default();
        let prompt = component.build_prompt_section(&settings).unwrap();

        assert!(prompt.contains("## Available Skills"));
        assert!(prompt.contains("| commit |"));
        assert!(prompt.contains("| pdf |"));
        assert!(prompt.contains("invoke_skill"));
        assert!(prompt.contains("CRITICAL"));
    }

    #[test]
    fn test_prompt_without_skills() {
        let temp = TempDir::new().unwrap();

        let config = SkillsConfig::default();
        let manager = SkillsManager::discover(&[], temp.path(), &config);
        let component = SkillsPromptComponent::new(manager);

        let settings = Settings::default();
        let prompt = component.build_prompt_section(&settings);

        assert!(prompt.is_none());
    }
}
