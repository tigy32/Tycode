use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::module::PromptComponentSelection;
use crate::tools::ToolName;

use super::agent::Agent;

/// Supports both comma-separated string ("Read, Grep") and YAML list format.
/// Claude Code uses comma-separated strings in frontmatter; YAML lists are
/// also common. This enum handles both via untagged deserialization.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum StringOrVec {
    String(String),
    Vec(Vec<String>),
}

impl StringOrVec {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::String(s) => s
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            Self::Vec(v) => v,
        }
    }
}

/// Frontmatter configuration parsed from a custom agent markdown file.
/// Field names use camelCase to match Claude Code's wire format.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomAgentConfig {
    pub name: String,
    pub description: String,
    #[serde(default)]
    tools: Option<StringOrVec>,
    #[serde(default)]
    disallowed_tools: Option<StringOrVec>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub max_turns: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomAgentSpec {
    pub name: String,
    pub description: String,
    pub system_prompt: String,
    #[serde(default)]
    pub tools: Option<Vec<String>>,
    #[serde(default)]
    pub disallowed_tools: Option<Vec<String>>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub max_turns: Option<u32>,
}

pub struct CustomAgent {
    name: String,
    description: String,
    system_prompt: String,
    resolved_tools: Vec<ToolName>,
}

impl CustomAgent {
    pub fn from_config(
        config: CustomAgentConfig,
        system_prompt: String,
        default_tools: &[ToolName],
    ) -> Self {
        let resolved_tools = resolve_tools(&config, default_tools);
        Self {
            name: config.name,
            description: config.description,
            system_prompt,
            resolved_tools,
        }
    }

    pub fn from_spec(spec: CustomAgentSpec, default_tools: &[ToolName]) -> Self {
        let base: Vec<ToolName> = match spec.tools {
            Some(tools) => tools.into_iter().map(ToolName::new).collect(),
            None => default_tools.to_vec(),
        };

        let resolved_tools = match spec.disallowed_tools {
            Some(disallowed) => {
                let blocked: HashSet<String> = disallowed.into_iter().collect();
                base.into_iter()
                    .filter(|t| !blocked.contains(t.as_str()))
                    .collect()
            }
            None => base,
        };

        Self {
            name: spec.name,
            description: spec.description,
            system_prompt: spec.system_prompt,
            resolved_tools,
        }
    }
}

fn resolve_tools(config: &CustomAgentConfig, default_tools: &[ToolName]) -> Vec<ToolName> {
    let base: Vec<ToolName> = match &config.tools {
        Some(specified) => specified
            .clone()
            .into_vec()
            .into_iter()
            .map(ToolName::new)
            .collect(),
        None => default_tools.to_vec(),
    };

    let Some(disallowed) = &config.disallowed_tools else {
        return base;
    };

    let blocked: HashSet<String> = disallowed.clone().into_vec().into_iter().collect();
    base.into_iter()
        .filter(|t| !blocked.contains(t.as_str()))
        .collect()
}

impl Agent for CustomAgent {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn core_prompt(&self) -> &str {
        &self.system_prompt
    }

    fn available_tools(&self) -> Vec<ToolName> {
        self.resolved_tools.clone()
    }

    /// Custom agents supply their own complete system prompt via the markdown
    /// body, so we exclude the standard prompt components (style mandates,
    /// communication guidelines, etc.).
    fn requested_prompt_components(&self) -> PromptComponentSelection {
        PromptComponentSelection::None
    }

    fn requires_tool_use(&self) -> bool {
        true
    }
}
