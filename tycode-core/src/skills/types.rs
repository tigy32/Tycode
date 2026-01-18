use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Maximum length for skill names (matches Claude Code spec).
pub const MAX_SKILL_NAME_LENGTH: usize = 64;

/// Maximum length for skill descriptions (matches Claude Code spec).
pub const MAX_SKILL_DESCRIPTION_LENGTH: usize = 1024;

/// Where a skill was discovered from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkillSource {
    /// Project-level skill from .tycode/skills/ or .claude/skills/ in a workspace.
    /// The PathBuf contains the workspace root path.
    Project(PathBuf),
    /// User-level skill from ~/.tycode/skills/
    User,
    /// User-level Claude Code compatibility from ~/.claude/skills/
    ClaudeCode,
    /// Plugin-provided skill. The PathBuf contains the plugin's skills directory.
    Plugin(PathBuf),
}

impl std::fmt::Display for SkillSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkillSource::Project(path) => write!(f, "project ({})", path.display()),
            SkillSource::User => write!(f, "user"),
            SkillSource::ClaudeCode => write!(f, "claude-code"),
            SkillSource::Plugin(path) => write!(f, "plugin ({})", path.display()),
        }
    }
}

/// Metadata parsed from a skill's YAML frontmatter.
///
/// This is the "Level 1" content that is always loaded at startup
/// and included in the system prompt (~100 tokens per skill).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMetadata {
    /// Unique identifier for the skill.
    /// Must be lowercase letters, numbers, and hyphens only.
    /// Maximum 64 characters.
    pub name: String,

    /// Description of what the skill does and when to use it.
    /// This helps the AI decide when to invoke the skill.
    /// Maximum 1024 characters.
    pub description: String,

    /// Where the skill was discovered from.
    pub source: SkillSource,

    /// Absolute path to the skill's SKILL.md file.
    pub path: PathBuf,

    /// Whether the skill is enabled (can be disabled in settings).
    pub enabled: bool,
}

impl SkillMetadata {
    /// Validates the skill name format.
    /// Names must be lowercase letters, numbers, and hyphens only.
    pub fn is_valid_name(name: &str) -> bool {
        !name.is_empty()
            && name.len() <= MAX_SKILL_NAME_LENGTH
            && name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && !name.starts_with('-')
            && !name.ends_with('-')
    }

    /// Validates the skill description.
    pub fn is_valid_description(description: &str) -> bool {
        !description.is_empty() && description.len() <= MAX_SKILL_DESCRIPTION_LENGTH
    }
}

/// Full skill instructions loaded on demand.
///
/// This is the "Level 2" content that is loaded when a skill is invoked.
/// Contains the full markdown instructions from SKILL.md.
#[derive(Debug, Clone)]
pub struct SkillInstructions {
    /// The skill's metadata.
    pub metadata: SkillMetadata,

    /// Full markdown instructions from SKILL.md (after frontmatter).
    pub instructions: String,

    /// Paths to additional reference files (REFERENCE.md, etc.).
    pub reference_files: Vec<PathBuf>,

    /// Paths to script files in the scripts/ directory.
    pub scripts: Vec<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_skill_names() {
        assert!(SkillMetadata::is_valid_name("commit"));
        assert!(SkillMetadata::is_valid_name("pdf-processing"));
        assert!(SkillMetadata::is_valid_name("skill123"));
        assert!(SkillMetadata::is_valid_name("my-skill-2"));
    }

    #[test]
    fn test_invalid_skill_names() {
        assert!(!SkillMetadata::is_valid_name("")); // empty
        assert!(!SkillMetadata::is_valid_name("My-Skill")); // uppercase
        assert!(!SkillMetadata::is_valid_name("skill_name")); // underscore
        assert!(!SkillMetadata::is_valid_name("-skill")); // starts with hyphen
        assert!(!SkillMetadata::is_valid_name("skill-")); // ends with hyphen
        assert!(!SkillMetadata::is_valid_name(&"a".repeat(65))); // too long
    }

    #[test]
    fn test_skill_source_display() {
        assert_eq!(SkillSource::User.to_string(), "user");
        assert_eq!(SkillSource::ClaudeCode.to_string(), "claude-code");
        assert_eq!(
            SkillSource::Project(PathBuf::from("/project")).to_string(),
            "project (/project)"
        );
    }
}
