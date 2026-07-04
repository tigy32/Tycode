use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{anyhow, Result};
use tracing::{debug, info, warn};

use crate::agents::agent::ActiveAgent;
use crate::agents::catalog::AgentCatalog;
use crate::ai::provider::AiProvider;
use crate::ai::types::{Content, ContentBlock, Message, ToolResultData};
use crate::chat::request::prepare_request;
use crate::module::ContextBuilder;
use crate::module::Module;
use crate::module::PromptBuilder;
use crate::settings::SettingsManager;
use crate::steering::SteeringDocuments;
use crate::tools::r#trait::{ToolOutput, ToolRequest};
use crate::tools::registry::ToolRegistry;

/// Tools that mutate files; subject to write_allowlist enforcement during
/// fan-out so a worker cannot edit outside its assignment.
const WRITE_TOOL_NAMES: &[&str] = &["write_file", "modify_file", "delete_file"];

/// A sub-agent runner.
///
/// This runs autonomous agents that do not require user input. Agents are run
/// until "complete_task" is called. This currently has some duplicated logic
/// with chat/ai.rs & chat/tools.rs. We need to refactor to better abstract out
/// execution. This currently exists separately from ChatActor for background
/// tasks like memory management.
pub struct AgentRunner {
    ai_provider: Arc<dyn AiProvider>,
    settings: SettingsManager,
    modules: Vec<Arc<dyn Module>>,
    steering: SteeringDocuments,
    prompt_builder: PromptBuilder,
    context_builder: ContextBuilder,
    catalog: Arc<AgentCatalog>,
}

impl AgentRunner {
    pub fn new(
        ai_provider: Arc<dyn AiProvider>,
        settings: SettingsManager,
        modules: Vec<Arc<dyn Module>>,
        steering: SteeringDocuments,
        prompt_builder: PromptBuilder,
        context_builder: ContextBuilder,
        catalog: Arc<AgentCatalog>,
    ) -> Self {
        Self {
            ai_provider,
            settings,
            modules,
            steering,
            prompt_builder,
            context_builder,
            catalog,
        }
    }

    /// Run an agent until completion or max iterations.
    /// The ActiveAgent should already have its conversation populated.
    /// Returns the result string from complete_task on success.
    pub async fn run(&self, active_agent: ActiveAgent, max_iterations: usize) -> Result<String> {
        let (_, result) = self.run_returning(active_agent, max_iterations).await;
        result
    }

    /// Like [`Self::run`], but also returns the agent with its final
    /// conversation so callers can fork it (e.g. pairing a reviewer with a
    /// worker during fan-out).
    pub async fn run_returning(
        &self,
        mut active_agent: ActiveAgent,
        max_iterations: usize,
    ) -> (ActiveAgent, Result<String>) {
        for iteration in 0..max_iterations {
            debug!(iteration, "AgentRunner iteration");

            let prepared = prepare_request(
                active_agent.agent.as_ref(),
                &active_agent.conversation,
                self.ai_provider.as_ref(),
                self.settings.clone(),
                &self.steering,
                &self.prompt_builder,
                &self.context_builder,
                &self.modules,
                &self.catalog,
            )
            .await;
            let (request, _model_settings, _context_breakdown, tools) = match prepared {
                Ok(prepared) => prepared,
                Err(error) => return (active_agent, Err(error)),
            };

            let tool_registry = ToolRegistry::new(tools);

            let response = match self.ai_provider.converse(request).await {
                Ok(response) => response,
                Err(error) => return (active_agent, Err(error.into())),
            };
            log_response_text(&response.content);

            let tool_uses: Vec<_> = response.content.tool_uses().into_iter().cloned().collect();

            info!(
                tool_count = tool_uses.len(),
                tools = ?tool_uses.iter().map(|t| &t.name).collect::<Vec<_>>(),
                "Extracted tools from response"
            );

            active_agent
                .conversation
                .push(Message::assistant(response.content.clone()));

            if tool_uses.is_empty() {
                warn!("AgentRunner completed - no more tool calls, but never got complete_task. This is likely a model error.");
                break;
            }

            let mut tool_results = Vec::new();
            let mut completion_result: Option<(bool, String)> = None;
            for tool_use in &tool_uses {
                info!(tool = %tool_use.name, args = %tool_use.arguments, "Runner calling tool");
                let result = self
                    .execute_tool(
                        &tool_registry,
                        &tool_use.name,
                        &tool_use.id,
                        &tool_use.arguments,
                        active_agent.write_allowlist.as_ref(),
                    )
                    .await;

                let (content, is_error) = match &result {
                    Ok((output, _)) => (output.clone(), false),
                    Err(e) => (format!("Error: {e:?}"), true),
                };

                if let Ok((_, tool_output)) = &result {
                    completion_result = completion_result.or(Self::extract_completion(tool_output));
                }

                let result_preview: String = content.chars().take(300).collect();
                info!(tool = %tool_use.name, result = %result_preview, is_error, "Tool result");

                tool_results.push(ContentBlock::ToolResult(ToolResultData {
                    tool_use_id: tool_use.id.to_string(),
                    content,
                    is_error,
                }));
            }

            active_agent
                .conversation
                .push(Message::user(Content::new(tool_results)));

            if let Some((success, result)) = completion_result {
                if success {
                    debug!("AgentRunner completed via complete_task");
                    return (active_agent, Ok(result));
                }
                return (active_agent, Err(anyhow!("Task failed: {}", result)));
            }
        }

        (
            active_agent,
            Err(anyhow!(
                "Agent did not complete task within {} iterations",
                max_iterations
            )),
        )
    }

    fn extract_completion(output: &ToolOutput) -> Option<(bool, String)> {
        if let ToolOutput::PopAgent { success, result } = output {
            Some((*success, result.clone()))
        } else {
            None
        }
    }

    async fn execute_tool(
        &self,
        tool_registry: &ToolRegistry,
        name: &str,
        tool_use_id: &str,
        arguments: &serde_json::Value,
        write_allowlist: Option<&HashSet<PathBuf>>,
    ) -> Result<(String, ToolOutput)> {
        debug!(name, "Executing tool");

        enforce_write_allowlist(name, arguments, write_allowlist)?;

        let executor = tool_registry
            .get_tool_executor_by_name(name)
            .ok_or_else(|| anyhow!("No executor for tool: {}", name))?;

        let schema = executor.input_schema();
        let coerced_arguments = crate::tools::fuzzy_json::coerce_to_schema(arguments, &schema)
            .map_err(|e| anyhow!("Failed to coerce tool arguments: {e:?}"))?;
        let request = ToolRequest::new(coerced_arguments, tool_use_id.to_string());
        let handle = executor.process(&request).await?;

        let tool_output = handle.execute().await;

        let output_string = match &tool_output {
            ToolOutput::Result {
                content, is_error, ..
            } => {
                if *is_error {
                    return Err(anyhow!("{}", content));
                }
                content.clone()
            }
            ToolOutput::PopAgent { success, result } => {
                format!("Task completed (success={}): {}", success, result)
            }
            ToolOutput::ImageResult { content, .. } => content.clone(),
            ToolOutput::PushAgent { .. } | ToolOutput::PromptUser { .. } => {
                return Err(anyhow!(
                    "Tool '{}' returned unsupported action for AgentRunner context",
                    name
                ))
            }
        };

        Ok((output_string, tool_output))
    }
}

/// Rejects file-modification tool calls targeting files outside the agent's
/// assignment. Read tools and bash are unrestricted; the orchestrator's
/// integration gate audits anything that slips past prompt-level guidance.
fn enforce_write_allowlist(
    tool_name: &str,
    arguments: &serde_json::Value,
    write_allowlist: Option<&HashSet<PathBuf>>,
) -> Result<()> {
    let Some(allowlist) = write_allowlist else {
        return Ok(());
    };
    if !WRITE_TOOL_NAMES.contains(&tool_name) {
        return Ok(());
    }

    let file_path = arguments
        .get("file_path")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    if allowlist.contains(Path::new(file_path)) {
        return Ok(());
    }

    Err(anyhow!(
        "File '{}' is outside this agent's assignment; allowed files: {:?}",
        file_path,
        allowlist
    ))
}

fn log_response_text(content: &Content) {
    for block in content.blocks() {
        let ContentBlock::Text(text) = block else {
            continue;
        };
        let preview: String = text.chars().take(500).collect();
        info!(response = %preview, "Agent reasoning");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn allowlist(paths: &[&str]) -> HashSet<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn write_outside_allowlist_is_rejected() {
        let allow = allowlist(&["src/a.rs"]);
        for tool in ["write_file", "modify_file", "delete_file"] {
            let result =
                enforce_write_allowlist(tool, &json!({"file_path": "src/b.rs"}), Some(&allow));
            assert!(result.is_err(), "{tool} should be rejected");
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("outside this agent's assignment"));
        }
    }

    #[test]
    fn write_inside_allowlist_is_permitted() {
        let allow = allowlist(&["src/a.rs"]);
        assert!(enforce_write_allowlist(
            "write_file",
            &json!({"file_path": "src/a.rs"}),
            Some(&allow)
        )
        .is_ok());
    }

    #[test]
    fn read_tools_and_no_allowlist_are_unrestricted() {
        let allow = allowlist(&["src/a.rs"]);
        assert!(
            enforce_write_allowlist("bash", &json!({"command": "cat src/b.rs"}), Some(&allow))
                .is_ok()
        );
        assert!(
            enforce_write_allowlist("write_file", &json!({"file_path": "src/b.rs"}), None).is_ok()
        );
    }
}
