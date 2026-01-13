use std::sync::Arc;

use anyhow::{bail, Result};
use serde_json::{json, Value};

use crate::chat::events::{ToolExecutionResult, ToolRequest as ToolRequestEvent, ToolRequestType};
use crate::tools::r#trait::{
    ContinuationPreference, ToolCallHandle, ToolCategory, ToolExecutor, ToolOutput, ToolRequest,
};
use crate::tools::ToolName;

use super::context::InvokedSkillsState;
use super::discovery::SkillsManager;

/// Tool for invoking skills and loading their instructions.
pub struct InvokeSkillTool {
    manager: SkillsManager,
    state: Arc<InvokedSkillsState>,
}

impl InvokeSkillTool {
    pub fn new(manager: SkillsManager, state: Arc<InvokedSkillsState>) -> Self {
        Self { manager, state }
    }

    pub fn tool_name() -> ToolName {
        ToolName::new("invoke_skill")
    }
}

struct InvokeSkillHandle {
    skill_name: String,
    tool_use_id: String,
    manager: SkillsManager,
    state: Arc<InvokedSkillsState>,
}

#[async_trait::async_trait(?Send)]
impl ToolCallHandle for InvokeSkillHandle {
    fn tool_request(&self) -> ToolRequestEvent {
        ToolRequestEvent {
            tool_call_id: self.tool_use_id.clone(),
            tool_name: "invoke_skill".to_string(),
            tool_type: ToolRequestType::Other {
                args: json!({ "skill_name": self.skill_name }),
            },
        }
    }

    async fn execute(self: Box<Self>) -> ToolOutput {
        // Load the skill instructions
        match self.manager.load_instructions(&self.skill_name) {
            Ok(skill) => {
                // Record that this skill has been invoked
                self.state
                    .add_invoked(skill.metadata.name.clone(), skill.instructions.clone());

                // Build the response
                let mut response = format!(
                    "Skill '{}' loaded successfully.\n\n## Instructions\n\n{}",
                    skill.metadata.name, skill.instructions
                );

                // Include reference files if any
                if !skill.reference_files.is_empty() {
                    response.push_str("\n\n## Reference Files\n\n");
                    response.push_str(
                        "The following reference files are available. Use the read_file tool to access them:\n",
                    );
                    for file in &skill.reference_files {
                        response.push_str(&format!("- {}\n", file.display()));
                    }
                }

                // Include scripts if any
                if !skill.scripts.is_empty() {
                    response.push_str("\n\n## Scripts\n\n");
                    response
                        .push_str("The following scripts are available for use with this skill:\n");
                    for script in &skill.scripts {
                        response.push_str(&format!("- {}\n", script.display()));
                    }
                }

                ToolOutput::Result {
                    content: response,
                    is_error: false,
                    continuation: ContinuationPreference::Continue,
                    ui_result: ToolExecutionResult::Other {
                        result: json!({
                            "skill_name": skill.metadata.name,
                            "source": format!("{}", skill.metadata.source),
                        }),
                    },
                }
            }
            Err(e) => ToolOutput::Result {
                content: format!("Failed to load skill '{}': {}", self.skill_name, e),
                is_error: true,
                continuation: ContinuationPreference::Continue,
                ui_result: ToolExecutionResult::Error {
                    short_message: format!("Skill '{}' not found", self.skill_name),
                    detailed_message: e.to_string(),
                },
            },
        }
    }
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for InvokeSkillTool {
    fn name(&self) -> &str {
        "invoke_skill"
    }

    fn description(&self) -> &str {
        "Load and activate a skill's instructions. Use this when a user's request matches \
         a skill's description from the Available Skills list. The skill will provide \
         detailed instructions for how to proceed with the task."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "skill_name": {
                    "type": "string",
                    "description": "The name of the skill to invoke (from the Available Skills list)"
                }
            },
            "required": ["skill_name"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Meta
    }

    async fn process(&self, request: &ToolRequest) -> Result<Box<dyn ToolCallHandle>> {
        let Some(skill_name) = request.arguments["skill_name"].as_str() else {
            bail!("Missing required argument \"skill_name\"");
        };

        // Check if skill exists and is enabled
        if !self.manager.is_enabled(skill_name) {
            if self.manager.get_skill(skill_name).is_some() {
                bail!("Skill '{}' is disabled", skill_name);
            } else {
                bail!(
                    "Skill '{}' not found. Use /skills to list available skills.",
                    skill_name
                );
            }
        }

        Ok(Box::new(InvokeSkillHandle {
            skill_name: skill_name.to_string(),
            tool_use_id: request.tool_use_id.clone(),
            manager: self.manager.clone(),
            state: self.state.clone(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::config::SkillsConfig;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_skill(dir: &std::path::Path, name: &str, description: &str, instructions: &str) {
        let skill_dir = dir.join(name);
        fs::create_dir_all(&skill_dir).unwrap();

        let content = format!(
            r#"---
name: {}
description: {}
---

{}
"#,
            name, description, instructions
        );

        fs::write(skill_dir.join("SKILL.md"), content).unwrap();
    }

    #[tokio::test]
    async fn test_invoke_skill_success() {
        let temp = TempDir::new().unwrap();
        let skills_dir = temp.path().join(".tycode").join("skills");
        fs::create_dir_all(&skills_dir).unwrap();

        create_test_skill(
            &skills_dir,
            "test-skill",
            "A test skill",
            "# Test Instructions\n\nFollow these steps.",
        );

        let config = SkillsConfig::default();
        let manager = SkillsManager::discover(&[], temp.path(), &config);
        let state = Arc::new(InvokedSkillsState::new());
        let tool = InvokeSkillTool::new(manager, state.clone());

        let request = ToolRequest::new(json!({"skill_name": "test-skill"}), "test-id".to_string());

        let handle = tool.process(&request).await.unwrap();
        let output = handle.execute().await;

        if let ToolOutput::Result {
            content, is_error, ..
        } = output
        {
            assert!(!is_error);
            assert!(content.contains("Test Instructions"));
            assert!(state.is_invoked("test-skill"));
        } else {
            panic!("Expected ToolOutput::Result");
        }
    }

    #[tokio::test]
    async fn test_invoke_skill_not_found() {
        let temp = TempDir::new().unwrap();

        let config = SkillsConfig::default();
        let manager = SkillsManager::discover(&[], temp.path(), &config);
        let state = Arc::new(InvokedSkillsState::new());
        let tool = InvokeSkillTool::new(manager, state);

        let request = ToolRequest::new(json!({"skill_name": "nonexistent"}), "test-id".to_string());

        let result = tool.process(&request).await;
        assert!(result.is_err());
    }
}
