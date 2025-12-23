use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use tracing::{debug, info, warn};

use crate::agents::agent::{ActiveAgent, Agent};
use crate::agents::defaults::prepare_system_prompt_and_tools;
use crate::ai::provider::AiProvider;
use crate::ai::tweaks::resolve_from_settings;
use crate::ai::types::{
    Content, ContentBlock, ConversationRequest, Message, MessageRole, ToolDefinition,
    ToolResultData,
};
use crate::chat::ai::select_model_for_agent;
use crate::chat::tool_extraction::extract_all_tool_calls;
use crate::settings::manager::SettingsManager;
use crate::steering::SteeringDocuments;
use crate::tools::r#trait::{ToolExecutor, ToolRequest, ValidatedToolCall};

const MAX_ITERATIONS: usize = 10;

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
}

impl AgentRunner {
    pub fn new(
        ai_provider: Arc<dyn AiProvider>,
        settings: SettingsManager,
        tools: BTreeMap<String, Arc<dyn ToolExecutor + Send + Sync>>,
        steering: SteeringDocuments,
    ) -> Self {
        Self {
            ai_provider,
            settings,
            tools,
            steering,
        }
    }

    /// Run an agent until completion or max iterations.
    /// The ActiveAgent should already have its conversation populated.
    /// Returns the result string from complete_task on success.
    pub async fn run(&self, mut active_agent: ActiveAgent) -> Result<String> {
        let tools = self.build_tool_definitions(active_agent.agent.as_ref());

        for iteration in 0..MAX_ITERATIONS {
            debug!(iteration, "AgentRunner iteration");

            let settings_snapshot = self.settings.settings();
            let model = select_model_for_agent(
                &settings_snapshot,
                self.ai_provider.as_ref(),
                active_agent.agent.name(),
            )?;

            let resolved_tweaks =
                resolve_from_settings(&settings_snapshot, self.ai_provider.as_ref(), model.model);

            let system_prompt = self.steering.build_system_prompt(
                active_agent.agent.core_prompt(),
                active_agent.agent.requested_builtins(),
                !settings_snapshot.disable_custom_steering,
                settings_snapshot.autonomy_level,
            );

            let (final_system_prompt, final_tools) = prepare_system_prompt_and_tools(
                &system_prompt,
                tools.clone(),
                resolved_tweaks.tool_call_style.clone(),
            );

            let request = ConversationRequest {
                messages: active_agent.conversation.clone(),
                model,
                system_prompt: final_system_prompt,
                stop_sequences: Vec::new(),
                tools: final_tools,
            };

            let response = self.ai_provider.converse(request).await?;
            log_response_text(&response.content);

            let extraction = extract_all_tool_calls(&response.content);
            let tool_uses = extraction.tool_calls;

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

                if let Ok((_, validated)) = &result {
                    completion_result = completion_result.or(Self::extract_completion(validated));
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
            MAX_ITERATIONS
        ))
    }

    fn build_tool_definitions(&self, agent: &dyn Agent) -> Vec<ToolDefinition> {
        agent
            .available_tools()
            .iter()
            .filter_map(|tool_type| {
                let name = tool_type.name();
                let executor = self.tools.get(name)?;
                Some(ToolDefinition {
                    name: executor.name().to_string(),
                    description: executor.description().to_string(),
                    input_schema: executor.input_schema(),
                })
            })
            .collect()
    }

    fn extract_completion(validated: &ValidatedToolCall) -> Option<(bool, String)> {
        if let ValidatedToolCall::PopAgent { success, result } = validated {
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
    ) -> Result<(String, ValidatedToolCall)> {
        debug!(name, "Executing tool");

        let executor = self
            .tools
            .get(name)
            .ok_or_else(|| anyhow!("No executor for tool: {}", name))?;

        let request = ToolRequest::new(arguments.clone(), tool_use_id.to_string());
        let validated = executor.validate(&request).await?;

        let output = match &validated {
            ValidatedToolCall::NoOp { context_data, .. } => context_data.to_string(),
            ValidatedToolCall::PopAgent { success, result } => {
                format!("Task completed (success={}): {}", success, result)
            }
            ValidatedToolCall::Error(e) => return Err(anyhow!("{}", e)),
            ValidatedToolCall::FileModification { .. }
            | ValidatedToolCall::PushAgent { .. }
            | ValidatedToolCall::PromptUser { .. }
            | ValidatedToolCall::RunCommand { .. }
            | ValidatedToolCall::SetTrackedFiles { .. }
            | ValidatedToolCall::McpCall { .. }
            | ValidatedToolCall::SearchTypes { .. }
            | ValidatedToolCall::GetTypeDocs { .. }
            | ValidatedToolCall::PerformTaskListOp { .. } => {
                return Err(anyhow!(
                    "Tool '{}' returned unsupported action for AgentRunner context",
                    name
                ))
            }
        };

        Ok((output, validated))
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
