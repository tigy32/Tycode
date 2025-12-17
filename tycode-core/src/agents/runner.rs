use std::sync::{Arc, Mutex};

use anyhow::{anyhow, Result};
use tracing::{debug, info, warn};

use crate::agents::agent::Agent;
use crate::agents::tool_type::ToolType;
use crate::ai::provider::AiProvider;
use crate::ai::types::{
    Content, ContentBlock, ConversationRequest, Message, MessageRole, ToolDefinition,
    ToolResultData,
};
use crate::chat::ai::select_model_for_agent;
use crate::chat::tool_extraction::extract_all_tool_calls;
use crate::memory::MemoryLog;
use crate::settings::manager::SettingsManager;
use crate::tools::complete_task::CompleteTask;
use crate::tools::memory::append_memory::AppendMemoryTool;
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
    memory_log: Arc<Mutex<MemoryLog>>,
    settings: SettingsManager,
}

impl AgentRunner {
    pub fn new(
        ai_provider: Arc<dyn AiProvider>,
        memory_log: Arc<Mutex<MemoryLog>>,
        settings: SettingsManager,
    ) -> Self {
        Self {
            ai_provider,
            memory_log,
            settings,
        }
    }

    /// Run an agent with the given input until completion or max iterations.
    pub async fn run(&self, agent: &dyn Agent, input: &str) -> Result<()> {
        let tools = self.build_tool_definitions(agent);
        let mut messages = vec![Message::user(input)];

        for iteration in 0..MAX_ITERATIONS {
            debug!(iteration, "AgentRunner iteration");

            let settings_snapshot = self.settings.settings();
            let model = select_model_for_agent(
                &settings_snapshot,
                self.ai_provider.as_ref(),
                agent.name(),
            )?;

            let request = ConversationRequest {
                messages: messages.clone(),
                model,
                system_prompt: agent.core_prompt().to_string(),
                stop_sequences: Vec::new(),
                tools: tools.clone(),
            };

            let response = self.ai_provider.converse(request).await?;
            log_response_text(&response.content);

            let extraction = extract_all_tool_calls(&response.content);
            let tool_uses = extraction.tool_calls;

            // Surface parse errors by adding to conversation for AI to retry
            if let Some(parse_error) = extraction.xml_parse_error {
                warn!("XML tool call parse error: {parse_error}");
                messages.push(Message {
                    role: MessageRole::User,
                    content: Content::text_only(format!(
                        "Error parsing XML tool calls: {}. Please check your XML format and retry.",
                        parse_error
                    )),
                });
            }
            if let Some(parse_error) = extraction.json_parse_error {
                warn!("JSON tool call parse error: {parse_error}");
                messages.push(Message {
                    role: MessageRole::User,
                    content: Content::text_only(format!(
                        "Error parsing JSON tool calls: {}. Please check your JSON format and retry.",
                        parse_error
                    )),
                });
            }

            messages.push(Message::assistant(response.content.clone()));

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

            messages.push(Message::user(Content::new(tool_results)));

            if let Some((success, result)) = completion_result {
                if success {
                    debug!("AgentRunner completed via complete_task");
                    return Ok(());
                }
                return Err(anyhow!("Task failed: {}", result));
            }
        }

        Ok(())
    }

    fn build_tool_definitions(&self, agent: &dyn Agent) -> Vec<ToolDefinition> {
        agent
            .available_tools()
            .iter()
            .filter_map(|tool_type| self.tool_definition(tool_type))
            .collect()
    }

    fn create_executor(&self, tool_type: &ToolType) -> Option<Box<dyn ToolExecutor + Send + Sync>> {
        match tool_type {
            ToolType::AppendMemory => {
                Some(Box::new(AppendMemoryTool::new(self.memory_log.clone())))
            }
            ToolType::CompleteTask => Some(Box::new(CompleteTask)),
            other => {
                warn!(?other, "AgentRunner does not support this tool type");
                None
            }
        }
    }

    fn tool_definition(&self, tool_type: &ToolType) -> Option<ToolDefinition> {
        let executor = self.create_executor(tool_type)?;
        Some(ToolDefinition {
            name: executor.name().to_string(),
            description: executor.description().to_string(),
            input_schema: executor.input_schema(),
        })
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

        let tool_type =
            ToolType::from_name(name).ok_or_else(|| anyhow!("Unknown tool: {}", name))?;

        let executor = self
            .create_executor(&tool_type)
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
        info!(response = %preview, "Memory agent reasoning");
    }
}

/// Spawn the memory manager agent as a background task.
/// This is fire-and-forget - errors are logged but not propagated.
pub fn spawn_memory_manager(
    ai_provider: Arc<dyn AiProvider>,
    memory_log: Arc<Mutex<MemoryLog>>,
    settings: SettingsManager,
    user_message: String,
) {
    tokio::task::spawn_local(async move {
        let input_preview: String = user_message.chars().take(500).collect();
        info!(input = %input_preview, "Memory manager starting");
        let runner = AgentRunner::new(ai_provider, memory_log, settings);
        let agent = crate::agents::memory_manager::MemoryManagerAgent;

        match runner.run(&agent, &user_message).await {
            Ok(()) => info!("Memory manager completed"),
            Err(e) => warn!(error = ?e, "Memory manager failed"),
        }
    });
}
