use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use anyhow::Result;
use tracing::{debug, warn};

use super::parser::parse_skill_file;
use super::types::{SkillInstructions, SkillMetadata, SkillSource};
use crate::settings::config::SkillsConfig;

const SKILL_FILE_NAME: &str = "SKILL.md";

/// Manages skill discovery and loading.
///
/// SkillsManager discovers skills from multiple directories (in priority order):
/// 1. `~/.claude/skills/` (user-level Claude Code compatibility, lowest priority)
/// 2. `~/.tycode/skills/` (user-level)
/// 3. `.claude/skills/` in each workspace (project-level Claude Code compatibility)
/// 4. `.tycode/skills/` in each workspace (project-level, highest priority)
///
/// Later sources override earlier ones if the same skill name is found.
pub struct SkillsManager {
    inner: Arc<SkillsManagerInner>,
}

struct SkillsManagerInner {
    /// All discovered skills indexed by name
    skills: RwLock<HashMap<String, SkillInstructions>>,
    /// Configuration
    config: SkillsConfig,
    /// Workspace roots for project-level skill discovery
    workspace_roots: Vec<PathBuf>,
    /// Home directory
    home_dir: PathBuf,
}

impl SkillsManager {
    /// Discovers skills from all configured directories.
    pub fn discover(workspace_roots: &[PathBuf], home_dir: &Path, config: &SkillsConfig) -> Self {
        let inner = Arc::new(SkillsManagerInner {
            skills: RwLock::new(HashMap::new()),
            config: config.clone(),
            workspace_roots: workspace_roots.to_vec(),
            home_dir: home_dir.to_path_buf(),
        });

        let manager = Self { inner };

        if config.enabled {
            manager.reload();
        }

        manager
    }

    /// Reloads skills from all directories.
    pub fn reload(&self) {
        let mut skills = HashMap::new();

        // 1. Load from ~/.claude/skills/ (Claude Code compatibility, lowest priority)
        if self.inner.config.enable_claude_code_compat {
            let claude_skills_dir = self.inner.home_dir.join(".claude").join("skills");
            if claude_skills_dir.is_dir() {
                debug!("Discovering skills from {:?}", claude_skills_dir);
                self.discover_from_directory(
                    &claude_skills_dir,
                    SkillSource::ClaudeCode,
                    &mut skills,
                );
            }
        }

        // 2. Load from ~/.tycode/skills/ (user-level)
        let user_skills_dir = self.inner.home_dir.join(".tycode").join("skills");
        if user_skills_dir.is_dir() {
            debug!("Discovering skills from {:?}", user_skills_dir);
            self.discover_from_directory(&user_skills_dir, SkillSource::User, &mut skills);
        }

        // 3. Load from additional directories configured in settings
        for dir in &self.inner.config.additional_dirs {
            if dir.is_dir() {
                debug!("Discovering skills from additional dir {:?}", dir);
                self.discover_from_directory(dir, SkillSource::User, &mut skills);
            }
        }

        // 4. Load from .tycode/skills/ and .claude/skills/ in each workspace root (project-level, highest priority)
        for workspace_root in &self.inner.workspace_roots {
            // Check .tycode/skills/
            let tycode_skills_dir = workspace_root.join(".tycode").join("skills");
            if tycode_skills_dir.is_dir() {
                debug!("Discovering skills from {:?}", tycode_skills_dir);
                self.discover_from_directory(
                    &tycode_skills_dir,
                    SkillSource::Project(workspace_root.clone()),
                    &mut skills,
                );
            }

            // Check .claude/skills/ (Claude Code project-level compatibility)
            if self.inner.config.enable_claude_code_compat {
                let claude_skills_dir = workspace_root.join(".claude").join("skills");
                if claude_skills_dir.is_dir() {
                    debug!("Discovering skills from {:?}", claude_skills_dir);
                    self.discover_from_directory(
                        &claude_skills_dir,
                        SkillSource::Project(workspace_root.clone()),
                        &mut skills,
                    );
                }
            }
        }

        let count = skills.len();
        *self.inner.skills.write().unwrap() = skills;
        debug!("Discovered {} skills", count);
    }

    /// Discovers skills from a single directory.
    fn discover_from_directory(
        &self,
        dir: &Path,
        source: SkillSource,
        skills: &mut HashMap<String, SkillInstructions>,
    ) {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                warn!("Failed to read skills directory {:?}: {}", dir, e);
                return;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let skill_file = path.join(SKILL_FILE_NAME);
            if !skill_file.is_file() {
                continue;
            }

            let enabled = !self.inner.config.disabled_skills.contains(
                &path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
            );

            match parse_skill_file(&skill_file, source.clone(), enabled) {
                Ok(skill) => {
                    debug!(
                        "Discovered skill '{}' from {:?} (enabled: {})",
                        skill.metadata.name, skill_file, enabled
                    );
                    skills.insert(skill.metadata.name.clone(), skill);
                }
                Err(e) => {
                    warn!("Failed to parse skill at {:?}: {}", skill_file, e);
                }
            }
        }
    }

    /// Returns metadata for all discovered skills.
    pub fn get_all_metadata(&self) -> Vec<SkillMetadata> {
        self.inner
            .skills
            .read()
            .unwrap()
            .values()
            .map(|s| s.metadata.clone())
            .collect()
    }

    /// Returns metadata for enabled skills only.
    pub fn get_enabled_metadata(&self) -> Vec<SkillMetadata> {
        self.inner
            .skills
            .read()
            .unwrap()
            .values()
            .filter(|s| s.metadata.enabled)
            .map(|s| s.metadata.clone())
            .collect()
    }

    /// Gets a skill by name.
    pub fn get_skill(&self, name: &str) -> Option<SkillInstructions> {
        self.inner.skills.read().unwrap().get(name).cloned()
    }

    /// Loads full instructions for a skill.
    pub fn load_instructions(&self, name: &str) -> Result<SkillInstructions> {
        self.inner
            .skills
            .read()
            .unwrap()
            .get(name)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("Skill '{}' not found", name))
    }

    /// Checks if a skill exists and is enabled.
    pub fn is_enabled(&self, name: &str) -> bool {
        self.inner
            .skills
            .read()
            .unwrap()
            .get(name)
            .map(|s| s.metadata.enabled)
            .unwrap_or(false)
    }

    /// Returns the number of discovered skills.
    pub fn count(&self) -> usize {
        self.inner.skills.read().unwrap().len()
    }

    /// Returns the number of enabled skills.
    pub fn enabled_count(&self) -> usize {
        self.inner
            .skills
            .read()
            .unwrap()
            .values()
            .filter(|s| s.metadata.enabled)
            .count()
    }

    /// Adds skills from additional directories (e.g., from plugins).
    /// These are added with Plugin source and will not override existing skills.
    pub fn add_plugin_skill_dirs(&self, dirs: &[PathBuf]) {
        let mut skills = self.inner.skills.write().unwrap();

        for dir in dirs {
            if !dir.is_dir() {
                continue;
            }

            debug!("Discovering skills from plugin directory {:?}", dir);

            let entries = match std::fs::read_dir(dir) {
                Ok(entries) => entries,
                Err(e) => {
                    warn!("Failed to read plugin skills directory {:?}: {}", dir, e);
                    continue;
                }
            };

            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }

                let skill_file = path.join(SKILL_FILE_NAME);
                if !skill_file.is_file() {
                    continue;
                }

                let skill_name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();

                // Don't override existing skills (plugins have lower priority)
                if skills.contains_key(&skill_name) {
                    debug!(
                        "Skipping plugin skill '{}' - already exists from higher priority source",
                        skill_name
                    );
                    continue;
                }

                let enabled = !self
                    .inner
                    .config
                    .disabled_skills
                    .contains(&skill_name);

                match parse_skill_file(&skill_file, SkillSource::Plugin(dir.clone()), enabled) {
                    Ok(skill) => {
                        debug!(
                            "Discovered plugin skill '{}' from {:?} (enabled: {})",
                            skill.metadata.name, skill_file, enabled
                        );
                        skills.insert(skill.metadata.name.clone(), skill);
                    }
                    Err(e) => {
                        warn!("Failed to parse plugin skill at {:?}: {}", skill_file, e);
                    }
                }
            }
        }

        debug!("Total skills after adding plugin dirs: {}", skills.len());
    }
}

impl Clone for SkillsManager {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_skill(dir: &Path, name: &str, description: &str) {
        let skill_dir = dir.join(name);
        fs::create_dir_all(&skill_dir).unwrap();

        let content = format!(
            r#"---
name: {}
description: {}
---

# {} Skill

Instructions for the skill.
"#,
            name, description, name
        );

        fs::write(skill_dir.join("SKILL.md"), content).unwrap();
    }

    #[test]
    fn test_discover_skills() {
        let temp = TempDir::new().unwrap();
        let skills_dir = temp.path().join(".tycode").join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        create_test_skill(&skills_dir, "test-skill", "A test skill");
        create_test_skill(&skills_dir, "another-skill", "Another skill");

        let config = SkillsConfig::default();
        let manager = SkillsManager::discover(&[], temp.path(), &config);

        assert_eq!(manager.count(), 2);
        assert!(manager.get_skill("test-skill").is_some());
        assert!(manager.get_skill("another-skill").is_some());
    }

    #[test]
    fn test_project_overrides_user() {
        let temp = TempDir::new().unwrap();

        // Create user-level skill
        let user_skills = temp.path().join(".tycode").join("skills");
        fs::create_dir_all(&user_skills).unwrap();
        create_test_skill(&user_skills, "my-skill", "User version");

        // Create project-level skill with same name
        let project_skills = temp.path().join("project").join(".tycode").join("skills");
        fs::create_dir_all(&project_skills).unwrap();
        create_test_skill(&project_skills, "my-skill", "Project version");

        let config = SkillsConfig::default();
        let workspace_roots = vec![temp.path().join("project")];
        let manager = SkillsManager::discover(&workspace_roots, temp.path(), &config);

        // Should have only 1 skill (project overrides user)
        assert_eq!(manager.count(), 1);

        let skill = manager.get_skill("my-skill").unwrap();
        assert_eq!(skill.metadata.description, "Project version");
    }

    #[test]
    fn test_disabled_skills() {
        let temp = TempDir::new().unwrap();
        let skills_dir = temp.path().join(".tycode").join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        create_test_skill(&skills_dir, "enabled-skill", "Enabled");
        create_test_skill(&skills_dir, "disabled-skill", "Disabled");

        let mut config = SkillsConfig::default();
        config.disabled_skills.insert("disabled-skill".to_string());

        let manager = SkillsManager::discover(&[], temp.path(), &config);

        assert_eq!(manager.count(), 2);
        assert_eq!(manager.enabled_count(), 1);
        assert!(manager.is_enabled("enabled-skill"));
        assert!(!manager.is_enabled("disabled-skill"));
    }

    #[test]
    fn test_skills_disabled_in_config() {
        let temp = TempDir::new().unwrap();
        let skills_dir = temp.path().join(".tycode").join("skills");
        fs::create_dir_all(&skills_dir).unwrap();
        create_test_skill(&skills_dir, "test-skill", "Test");

        let mut config = SkillsConfig::default();
        config.enabled = false;

        let manager = SkillsManager::discover(&[], temp.path(), &config);

        // Skills discovery is disabled, so no skills should be found
        assert_eq!(manager.count(), 0);
    }
}
