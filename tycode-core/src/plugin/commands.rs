//! Plugin command loading from markdown files.

use anyhow::Result;
use std::path::Path;
use tracing::{debug, warn};

use super::manifest::CommandFrontmatter;
use super::types::PluginCommand;

/// Loads all commands from a directory.
pub fn load_commands_from_directory(
    dir: &Path,
    plugin_name: &str,
) -> Result<Vec<PluginCommand>> {
    let mut commands = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            warn!(
                "Failed to read commands directory {:?}: {}",
                dir, e
            );
            return Ok(commands);
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();

        // Only process .md files
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }

        match load_command_from_file(&path, plugin_name) {
            Ok(command) => {
                debug!(
                    "Loaded command '{}' from {:?}",
                    command.name, path
                );
                commands.push(command);
            }
            Err(e) => {
                warn!(
                    "Failed to load command from {:?}: {}",
                    path, e
                );
            }
        }
    }

    Ok(commands)
}

/// Loads a single command from a markdown file.
pub fn load_command_from_file(path: &Path, plugin_name: &str) -> Result<PluginCommand> {
    let content = std::fs::read_to_string(path)?;
    let (frontmatter, _body) = CommandFrontmatter::parse(&content)?;

    // Get allowed_tools first before we move description
    let allowed_tools = frontmatter.all_tools();

    // Get name from frontmatter or filename
    let name = frontmatter
        .name
        .or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        })
        .ok_or_else(|| anyhow::anyhow!("Command has no name"))?;

    let description = frontmatter
        .description
        .unwrap_or_else(|| format!("Command from plugin {}", plugin_name));

    Ok(PluginCommand {
        name,
        description,
        path: path.to_path_buf(),
        plugin_name: plugin_name.to_string(),
        allowed_tools,
    })
}

/// Loads command instructions from a markdown file.
pub fn load_command_instructions(path: &Path) -> Result<String> {
    let content = std::fs::read_to_string(path)?;
    let (_frontmatter, body) = CommandFrontmatter::parse(&content)?;
    Ok(body)
}

/// Represents loaded command instructions with metadata.
#[derive(Debug, Clone)]
pub struct CommandInstructions {
    /// The command metadata
    pub command: PluginCommand,
    /// The markdown body/instructions
    pub instructions: String,
}

impl CommandInstructions {
    /// Loads full command instructions from a PluginCommand.
    pub fn load(command: &PluginCommand) -> Result<Self> {
        let content = std::fs::read_to_string(&command.path)?;
        let (_frontmatter, instructions) = CommandFrontmatter::parse(&content)?;

        Ok(Self {
            command: command.clone(),
            instructions,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_command_file(dir: &Path, name: &str, content: &str) {
        let path = dir.join(format!("{}.md", name));
        fs::write(&path, content).unwrap();
    }

    #[test]
    fn test_load_command_with_frontmatter() {
        let temp = TempDir::new().unwrap();

        let content = r#"---
name: my-command
description: A custom command
allowed_tools:
  - read_file
  - write_file
---

# My Command

Instructions for the command.
"#;

        create_command_file(temp.path(), "my-command", content);

        let commands = load_commands_from_directory(temp.path(), "test-plugin").unwrap();

        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "my-command");
        assert_eq!(commands[0].description, "A custom command");
        assert_eq!(
            commands[0].allowed_tools,
            vec!["read_file", "write_file"]
        );
    }

    #[test]
    fn test_load_command_without_frontmatter() {
        let temp = TempDir::new().unwrap();

        let content = r#"# Simple Command

Just instructions, no frontmatter.
"#;

        create_command_file(temp.path(), "simple-cmd", content);

        let commands = load_commands_from_directory(temp.path(), "test-plugin").unwrap();

        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "simple-cmd"); // From filename
    }

    #[test]
    fn test_load_command_instructions() {
        let temp = TempDir::new().unwrap();

        let content = r#"---
name: test
---

# Instructions

These are the instructions.
"#;

        let path = temp.path().join("test.md");
        fs::write(&path, content).unwrap();

        let instructions = load_command_instructions(&path).unwrap();
        assert!(instructions.contains("# Instructions"));
        assert!(instructions.contains("These are the instructions."));
    }

    #[test]
    fn test_skip_non_markdown_files() {
        let temp = TempDir::new().unwrap();

        // Create a markdown file
        create_command_file(temp.path(), "valid-cmd", "# Valid");

        // Create a non-markdown file
        fs::write(temp.path().join("script.sh"), "#!/bin/sh").unwrap();

        let commands = load_commands_from_directory(temp.path(), "test-plugin").unwrap();

        assert_eq!(commands.len(), 1);
        assert_eq!(commands[0].name, "valid-cmd");
    }
}
