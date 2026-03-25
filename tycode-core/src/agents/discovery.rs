use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use tracing::warn;

use crate::skills::parser::extract_frontmatter;

use super::custom::CustomAgentConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSource {
    Project,
    User,
    ClaudeCode,
}

pub struct DiscoveredAgent {
    pub config: CustomAgentConfig,
    pub system_prompt: String,
    pub source: AgentSource,
    pub path: PathBuf,
}

pub struct CustomAgentManager {
    search_dirs: Vec<(PathBuf, AgentSource)>,
}

impl CustomAgentManager {
    /// Directories are ordered lowest-to-highest priority. When multiple
    /// agent files share the same `name`, the later (higher-priority)
    /// entry wins.
    pub fn new(workspace_roots: &[PathBuf], home_dir: &Path) -> Self {
        let mut search_dirs = Vec::new();

        search_dirs.push((
            home_dir.join(".claude").join("agents"),
            AgentSource::ClaudeCode,
        ));
        search_dirs.push((home_dir.join(".tycode").join("agents"), AgentSource::User));

        for root in workspace_roots {
            search_dirs.push((root.join(".claude").join("agents"), AgentSource::ClaudeCode));
            search_dirs.push((root.join(".tycode").join("agents"), AgentSource::Project));
        }

        Self { search_dirs }
    }

    pub fn discover(&self) -> Vec<DiscoveredAgent> {
        let mut agents_by_name: HashMap<String, DiscoveredAgent> = HashMap::new();

        for (dir, source) in &self.search_dirs {
            if !dir.exists() {
                continue;
            }

            let entries = match std::fs::read_dir(dir) {
                Ok(entries) => entries,
                Err(e) => {
                    warn!("Failed to read agents directory {}: {e:?}", dir.display());
                    continue;
                }
            };

            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "md") {
                    match parse_agent_file(&path) {
                        Ok((config, system_prompt)) => {
                            let name = config.name.clone();
                            agents_by_name.insert(
                                name,
                                DiscoveredAgent {
                                    config,
                                    system_prompt,
                                    source: source.clone(),
                                    path,
                                },
                            );
                        }
                        Err(e) => {
                            warn!("Failed to parse agent file {}: {e:?}", path.display());
                        }
                    }
                }
            }
        }

        agents_by_name.into_values().collect()
    }
}

fn parse_agent_file(path: &Path) -> Result<(CustomAgentConfig, String)> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("reading agent file {}", path.display()))?;

    let (yaml_str, body) = extract_frontmatter(&content)
        .with_context(|| format!("extracting frontmatter from {}", path.display()))?;

    if body.is_empty() {
        return Err(anyhow!(
            "agent file {} has no system prompt body",
            path.display()
        ));
    }

    let config: CustomAgentConfig = serde_yaml::from_str(&yaml_str)
        .with_context(|| format!("parsing frontmatter YAML in {}", path.display()))?;

    Ok((config, body))
}
