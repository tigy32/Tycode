use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use tracing::{debug, info, warn};

use crate::agents::agent::ActiveAgent;
use crate::ai::provider::AiProvider;
use crate::ai::types::{Content, ContentBlock, Message, MessageRole, ToolResultData};
use crate::chat::request::prepare_request;
use crate::chat::tool_extraction::extract_all_tool_calls;
use crate::context::ContextBuilder;
use crate::memory::MemoryLog;
use crate::prompt::PromptBuilder;
use crate::settings::SettingsManager;
use crate::steering::SteeringDocuments;
use crate::tools::r#trait::{ToolExecutor, ToolOutput, ToolRequest};

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
    tools: BTreeMap<String, Arc<dyn ToolExecutor + Send + Sync>>,
    steering: SteeringDocuments,
    workspace_roots: Vec<PathBuf>,
    memory_log: Arc<MemoryLog>,
    prompt_builder: PromptBuilder,
    context_builder: ContextBuilder,
}

impl AgentRunner {
    pub fn new(
        ai_provider: Arc<dyn AiProvider>,
        settings: SettingsManager,
        tools: BTreeMap<String, Arc<dyn ToolExecutor + Send + Sync>>,
        steering: SteeringDocuments,
        workspace_roots: Vec<PathBuf>,
        memory_log: Arc<MemoryLog>,
        prompt_builder: PromptBuilder,
        context_builder: ContextBuilder,
    ) -> Self {
        Self {
            ai_provider,
            settings,
            tools,
            steering,
            workspace_roots,
            memory_log,
            prompt_builder,
            context_builder,
        }
    }

    /// Run an agent until completion or max iterations.
    /// The ActiveAgent should already have its conversation populated.
    /// Returns the result string from complete_task on success.
    pub async fn run(
        &self,
        mut active_agent: ActiveAgent,
        max_iterations: usize,
    ) -> Result<String> {
        for iteration in 0..max_iterations {
            debug!(iteration, "AgentRunner iteration");

            let (request, _model_settings) = prepare_request(
                active_agent.agent.as_ref(),
                &active_agent.conversation,
                self.ai_provider.as_ref(),
                self.settings.clone(),
                &self.steering,
                self.workspace_roots.clone(),
                self.memory_log.clone(),
                Vec::new(),
                &self.prompt_builder,
                &self.context_builder,
            )
            .await?;

            let response = self.ai_provider.converse(request).await?;
            log_response_text(&response.content);

            let extraction = extract_all_tool_calls(&response.content);
            let tool_uses = extraction.tool_calls;

            info!(
                tool_count = tool_uses.len(),
                tools = ?tool_uses.iter().map(|t| &t.name).collect::<Vec<_>>(),
                "Extracted tools from response"
            );

            // Surface parse errors by adding to conversation for AI to retry
            if let Some(parse_error) = extraction.xml_parse_error {
                warn!("XML tool call parse error: {parse_error}");
                active_agent.conversation.push(Message {
                    role: MessageRole::User,
                    content: Content::text_only(format!(
                        "Error parsing XML tool calls: {}. Please check your XML format and retry.",
                        parse_error
                    )),
                });
            }
            if let Some(parse_error) = extraction.json_parse_error {
                warn!("JSON tool call parse error: {parse_error}");
                active_agent.conversation.push(Message {
                    role: MessageRole::User,
                    content: Content::text_only(format!(
                        "Error parsing JSON tool calls: {}. Please check your JSON format and retry.",
                        parse_error
                    )),
                });
            }

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
                    .execute_tool(&tool_use.name, &tool_use.id, &tool_use.arguments)
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
                    return Ok(result);
                }
                return Err(anyhow!("Task failed: {}", result));
            }
        }

        Err(anyhow!(
            "Agent did not complete task within {} iterations",
            max_iterations
        ))
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
        name: &str,
        tool_use_id: &str,
        arguments: &serde_json::Value,
    ) -> Result<(String, ToolOutput)> {
        debug!(name, "Executing tool");

        let executor = self
            .tools
            .get(name)
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

fn log_response_text(content: &Content) {
    for block in content.blocks() {
        let ContentBlock::Text(text) = block else {
            continue;
        };
        let preview: String = text.chars().take(500).collect();
        info!(response = %preview, "Agent reasoning");
    }
}
