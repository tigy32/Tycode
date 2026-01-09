use std::path::Path;

use anyhow::{anyhow, Context, Result};
use serde::Deserialize;

use super::types::{SkillInstructions, SkillMetadata, SkillSource};

/// Raw frontmatter parsed from SKILL.md YAML header.
#[derive(Debug, Deserialize)]
struct RawFrontmatter {
    name: String,
    description: String,
}

/// Parses a SKILL.md file and extracts metadata and instructions.
///
/// The file format is:
/// ```markdown
/// ---
/// name: skill-name
/// description: What this skill does
/// ---
///
/// # Skill Instructions
/// ...
/// ```
pub fn parse_skill_file(
    path: &Path,
    source: SkillSource,
    enabled: bool,
) -> Result<SkillInstructions> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read skill file: {}", path.display()))?;

    parse_skill_content(&content, path, source, enabled)
}

/// Parses skill content from a string.
pub fn parse_skill_content(
    content: &str,
    path: &Path,
    source: SkillSource,
    enabled: bool,
) -> Result<SkillInstructions> {
    let (frontmatter, instructions) = extract_frontmatter(content)?;

    let raw: RawFrontmatter = serde_yaml::from_str(&frontmatter)
        .with_context(|| format!("Failed to parse YAML frontmatter in {}", path.display()))?;

    // Validate name format
    if !SkillMetadata::is_valid_name(&raw.name) {
        return Err(anyhow!(
            "Invalid skill name '{}': must be lowercase letters, numbers, and hyphens only (max {} chars)",
            raw.name,
            super::types::MAX_SKILL_NAME_LENGTH
        ));
    }

    // Validate description
    if !SkillMetadata::is_valid_description(&raw.description) {
        return Err(anyhow!(
            "Invalid skill description: must be non-empty and max {} chars",
            super::types::MAX_SKILL_DESCRIPTION_LENGTH
        ));
    }

    let skill_dir = path
        .parent()
        .ok_or_else(|| anyhow!("Skill file has no parent directory"))?;

    // Discover reference files (*.md files other than SKILL.md)
    let reference_files = discover_reference_files(skill_dir);

    // Discover scripts in scripts/ subdirectory
    let scripts = discover_scripts(skill_dir);

    Ok(SkillInstructions {
        metadata: SkillMetadata {
            name: raw.name,
            description: raw.description,
            source,
            path: path.to_path_buf(),
            enabled,
        },
        instructions,
        reference_files,
        scripts,
    })
}

/// Extracts YAML frontmatter and body from a markdown file.
///
/// Frontmatter is delimited by `---` at the start and end.
fn extract_frontmatter(content: &str) -> Result<(String, String)> {
    let content = content.trim();

    if !content.starts_with("---") {
        return Err(anyhow!(
            "SKILL.md must start with YAML frontmatter (---)"
        ));
    }

    // Find the closing ---
    let rest = &content[3..];
    let end_pos = rest
        .find("\n---")
        .ok_or_else(|| anyhow!("SKILL.md frontmatter not closed (missing ---)"))?;

    let frontmatter = rest[..end_pos].trim().to_string();
    let body = rest[end_pos + 4..].trim().to_string();

    if frontmatter.is_empty() {
        return Err(anyhow!("SKILL.md frontmatter is empty"));
    }

    Ok((frontmatter, body))
}

/// Discovers reference markdown files in the skill directory.
fn discover_reference_files(skill_dir: &Path) -> Vec<std::path::PathBuf> {
    let mut files = Vec::new();

    if let Ok(entries) = std::fs::read_dir(skill_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext == "md" {
                        // Exclude SKILL.md itself
                        if let Some(name) = path.file_name() {
                            if name.to_string_lossy().to_uppercase() != "SKILL.MD" {
                                files.push(path);
                            }
                        }
                    }
                }
            }
        }
    }

    files.sort();
    files
}

/// Discovers script files in the scripts/ subdirectory.
fn discover_scripts(skill_dir: &Path) -> Vec<std::path::PathBuf> {
    let scripts_dir = skill_dir.join("scripts");
    let mut files = Vec::new();

    if scripts_dir.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&scripts_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() {
                    files.push(path);
                }
            }
        }
    }

    files.sort();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_frontmatter_valid() {
        let content = r#"---
name: test-skill
description: A test skill
---

# Instructions

Some instructions here.
"#;
        let (frontmatter, body) = extract_frontmatter(content).unwrap();
        assert!(frontmatter.contains("name: test-skill"));
        assert!(frontmatter.contains("description: A test skill"));
        assert!(body.contains("# Instructions"));
        assert!(body.contains("Some instructions here."));
    }

    #[test]
    fn test_extract_frontmatter_no_start() {
        let content = "# No frontmatter\nJust content";
        assert!(extract_frontmatter(content).is_err());
    }

    #[test]
    fn test_extract_frontmatter_no_end() {
        let content = "---\nname: test\n# No closing delimiter";
        assert!(extract_frontmatter(content).is_err());
    }

    #[test]
    fn test_parse_skill_content_valid() {
        let content = r#"---
name: my-skill
description: Does something useful when you ask
---

# My Skill

Follow these instructions.
"#;
        let path = Path::new("/test/skills/my-skill/SKILL.md");
        let result = parse_skill_content(content, path, SkillSource::User, true).unwrap();

        assert_eq!(result.metadata.name, "my-skill");
        assert_eq!(result.metadata.description, "Does something useful when you ask");
        assert!(result.metadata.enabled);
        assert!(result.instructions.contains("# My Skill"));
    }

    #[test]
    fn test_parse_skill_content_invalid_name() {
        let content = r#"---
name: Invalid_Name
description: Has invalid name
---

Instructions
"#;
        let path = Path::new("/test/SKILL.md");
        let result = parse_skill_content(content, path, SkillSource::User, true);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Invalid skill name"));
    }

    #[test]
    fn test_parse_skill_content_empty_description() {
        let content = r#"---
name: valid-name
description: ""
---

Instructions
"#;
        let path = Path::new("/test/SKILL.md");
        let result = parse_skill_content(content, path, SkillSource::User, true);
        assert!(result.is_err());
    }
}
