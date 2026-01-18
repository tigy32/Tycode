//! Plugin agent loading from markdown files.

use anyhow::Result;
use std::path::Path;
use tracing::{debug, warn};

use super::manifest::CommandFrontmatter;
use super::types::PluginAgent;

/// Loads all agents from a directory.
pub fn load_agents_from_directory(
    dir: &Path,
    plugin_name: &str,
) -> Result<Vec<PluginAgent>> {
    let mut agents = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            warn!("Failed to read agents directory {:?}: {}", dir, e);
            return Ok(agents);
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();

        // Only process .md files
        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }

        match load_agent_from_file(&path, plugin_name) {
            Ok(agent) => {
                debug!("Loaded agent '{}' from {:?}", agent.name, path);
                agents.push(agent);
            }
            Err(e) => {
                warn!("Failed to load agent from {:?}: {}", path, e);
            }
        }
    }

    Ok(agents)
}

/// Loads a single agent from a markdown file.
pub fn load_agent_from_file(path: &Path, plugin_name: &str) -> Result<PluginAgent> {
    let content = std::fs::read_to_string(path)?;
    let (frontmatter, _body) = CommandFrontmatter::parse(&content)?;

    // Get available_tools first before we move description
    let available_tools = frontmatter.all_tools();
    let prompt_selection = frontmatter.prompt_selection.clone();

    // Get name from frontmatter or filename
    let name = frontmatter
        .name
        .or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
        })
        .ok_or_else(|| anyhow::anyhow!("Agent has no name"))?;

    let description = frontmatter
        .description
        .unwrap_or_else(|| format!("Agent from plugin {}", plugin_name));

    Ok(PluginAgent {
        name,
        description,
        path: path.to_path_buf(),
        plugin_name: plugin_name.to_string(),
        available_tools,
        prompt_selection,
    })
}

/// Loads agent instructions from a markdown file.
pub fn load_agent_instructions(path: &Path) -> Result<String> {
    let content = std::fs::read_to_string(path)?;
    let (_frontmatter, body) = CommandFrontmatter::parse(&content)?;
    Ok(body)
}

/// Represents loaded agent instructions with metadata.
#[derive(Debug, Clone)]
pub struct AgentInstructions {
    /// The agent metadata
    pub agent: PluginAgent,
    /// The markdown body/instructions
    pub instructions: String,
}

impl AgentInstructions {
    /// Loads full agent instructions from a PluginAgent.
    pub fn load(agent: &PluginAgent) -> Result<Self> {
        let content = std::fs::read_to_string(&agent.path)?;
        let (_frontmatter, instructions) = CommandFrontmatter::parse(&content)?;

        Ok(Self {
            agent: agent.clone(),
            instructions,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_agent_file(dir: &Path, name: &str, content: &str) {
        let path = dir.join(format!("{}.md", name));
        fs::write(&path, content).unwrap();
    }

    #[test]
    fn test_load_agent_with_frontmatter() {
        let temp = TempDir::new().unwrap();

        let content = r#"---
name: code-reviewer
description: Reviews code for issues
available_tools:
  - read_file
  - search_files
prompt_selection: minimal
---

# Code Reviewer Agent

You are a code review agent. Review the code for bugs and issues.
"#;

        create_agent_file(temp.path(), "code-reviewer", content);

        let agents = load_agents_from_directory(temp.path(), "test-plugin").unwrap();

        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "code-reviewer");
        assert_eq!(agents[0].description, "Reviews code for issues");
        assert_eq!(
            agents[0].available_tools,
            vec!["read_file", "search_files"]
        );
        assert_eq!(
            agents[0].prompt_selection.as_deref(),
            Some("minimal")
        );
    }

    #[test]
    fn test_load_agent_without_frontmatter() {
        let temp = TempDir::new().unwrap();

        let content = r#"# Simple Agent

Just a simple agent with no frontmatter.
"#;

        create_agent_file(temp.path(), "simple-agent", content);

        let agents = load_agents_from_directory(temp.path(), "test-plugin").unwrap();

        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].name, "simple-agent"); // From filename
    }

    #[test]
    fn test_load_agent_instructions() {
        let temp = TempDir::new().unwrap();

        let content = r#"---
name: test-agent
---

# Agent Instructions

These are the agent instructions.

## Rules

1. Do this
2. Do that
"#;

        let path = temp.path().join("test-agent.md");
        fs::write(&path, content).unwrap();

        let instructions = load_agent_instructions(&path).unwrap();
        assert!(instructions.contains("# Agent Instructions"));
        assert!(instructions.contains("## Rules"));
    }

    #[test]
    fn test_agent_instructions_struct() {
        let temp = TempDir::new().unwrap();

        let content = r#"---
name: my-agent
description: Test agent
---

Custom instructions here.
"#;

        let path = temp.path().join("my-agent.md");
        fs::write(&path, content).unwrap();

        let agent = load_agent_from_file(&path, "test-plugin").unwrap();
        let instructions = AgentInstructions::load(&agent).unwrap();

        assert_eq!(instructions.agent.name, "my-agent");
        assert!(instructions.instructions.contains("Custom instructions"));
    }
}
