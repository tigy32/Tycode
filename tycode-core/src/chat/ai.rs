use crate::agents::tool_type::ToolType;
use crate::ai::model::Model;
use crate::ai::{
    error::AiError, model::ModelCost, provider::AiProvider, Content, ContentBlock,
    ConversationRequest, ConversationResponse, Message, MessageRole, ModelSettings, ToolUseData,
};
use crate::chat::events::{ChatEvent, ChatMessage, ContextInfo, ModelInfo};
use crate::chat::tools::{self, current_agent_mut};
use crate::file::context::{build_message_context, create_context_info};
use crate::settings::config::Settings;
use crate::tools::registry::{resolve_file_modification_api, ToolRegistry};
use anyhow::{bail, Result};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use super::actor::ActorState;

pub(crate) fn select_model_for_agent(
    settings: &Settings,
    provider: &dyn AiProvider,
    agent_name: &str,
) -> Result<ModelSettings, AiError> {
    if let Some(override_model) = settings.get_agent_model(agent_name) {
        return Ok(override_model.clone());
    }

    let quality = settings.model_quality.unwrap_or(ModelCost::Unlimited);

    let Some(model) = Model::select_for_cost(provider, quality) else {
        return Err(AiError::Terminal(anyhow::anyhow!(
            "No model available for {quality:?} in provider {}",
            provider.name()
        )));
    };
    Ok(model)
}

pub async fn send_ai_request(state: &mut ActorState) -> Result<()> {
    loop {
        // Prepare the AI request with all necessary context
        let (request, context_info, model_settings) = prepare_ai_request(state).await?;

        // Send request and get response
        let response = match send_request_with_retry(state, request).await {
            Ok(response) => response,
            Err(e) => {
                state
                    .event_sender
                    .add_message(ChatMessage::error(format!("Error: {e:?}")));
                return Ok(());
            }
        };

        // Process the response and update conversation
        let model = model_settings.model;
        let tool_calls = process_ai_response(state, response, model_settings, context_info);
        if tool_calls.is_empty() {
            break;
        }

        match tools::execute_tool_calls(state, tool_calls, model).await {
            Ok(tool_results) => {
                // Add all tool results as a single message to satisfy Bedrock's expectations
                if !tool_results.results.is_empty() {
                    tools::current_agent_mut(state).conversation.push(Message {
                        role: MessageRole::User,
                        content: Content::from(tool_results.results),
                    });
                }

                if tool_results.continue_conversation {
                    continue;
                } else {
                    break;
                }
            }
            Err(e) => {
                // Remove bad tool calls from conversation history and provide immediate feedback
                let _ = state.event_sender.event_tx.send(ChatEvent::RetryAttempt {
                    attempt: 1,
                    max_retries: 1000,
                    error: e.to_string(),
                    backoff_ms: 0,
                });

                let _last = current_agent_mut(state).conversation.pop();
                current_agent_mut(state).conversation.push(Message {
                    role: MessageRole::User,
                    content: Content::text_only(format!(
                        "You attempted to use tools incorrectly; the system has removed the incorrect tool calls from the conversation history. Please incorporate the following feedback feedback and retry. Here are the errors from the (removed) tool calls: {}",
                        e.to_string()
                    )),
                });
            }
        }
    }

    Ok(())
}

async fn prepare_ai_request(
    state: &mut ActorState,
) -> Result<(ConversationRequest, ContextInfo, ModelSettings)> {
    let current = tools::current_agent(state);

    // Select model early for tool registry resolution
    let settings_snapshot = state.settings.settings();
    let agent_name = current.agent.name();
    let model_settings =
        select_model_for_agent(&settings_snapshot, state.provider.as_ref(), agent_name)?;

    // Prepare tools
    let allowed_tools: HashSet<ToolType> = current.agent.available_tools().into_iter().collect();
    let allowed_tool_types: Vec<ToolType> = allowed_tools.into_iter().collect();

    let file_modification_api = settings_snapshot.file_modification_api;
    let resolved_api = resolve_file_modification_api(file_modification_api, model_settings.model);
    let tool_registry = ToolRegistry::new(
        state.workspace_roots.clone(),
        resolved_api,
        state.mcp_manager.as_ref(),
    )
    .await?;
    let available_tools = tool_registry.get_tool_definitions_for_types(&allowed_tool_types);

    // Build message context
    let tracked_files: Vec<PathBuf> = state.tracked_files.iter().cloned().collect();
    let message_context = build_message_context(
        &state.workspace_roots,
        &tracked_files,
        state.task_list.clone(),
    )
    .await;
    let context_info = create_context_info(&message_context);
    let context_string = message_context.to_formatted_string();
    let context_text = format!("Current Context:\n{context_string}");

    let mut conversation = tools::current_agent(state).conversation.clone();
    if conversation.is_empty() {
        bail!("No messages to send to AI. Conversation is empty!")
    }

    conversation
        .last_mut()
        .unwrap()
        .content
        .push(ContentBlock::Text(context_text));

    let system_prompt = current.agent.system_prompt().to_string();

    let request = ConversationRequest {
        messages: conversation,
        model: model_settings.clone(),
        system_prompt,
        stop_sequences: vec![],
        tools: available_tools,
    };

    debug!(?request, "AI request");

    Ok((request, context_info, model_settings))
}

fn process_ai_response(
    state: &mut ActorState,
    response: ConversationResponse,
    model_settings: ModelSettings,
    context_info: ContextInfo,
) -> Vec<ToolUseData> {
    let content = response.content.clone();

    info!(?response, "AI response");

    // Accumulate token usage for session tracking
    state.session_token_usage.input_tokens += response.usage.input_tokens;
    state.session_token_usage.output_tokens += response.usage.output_tokens;
    state.session_token_usage.total_tokens += response.usage.total_tokens;

    // Calculate and accumulate cost using the actual model used for this response
    let cost = state.provider.get_cost(&model_settings.model);
    let response_cost = cost.calculate_cost(&response.usage);
    state.session_cost += response_cost;

    let reasoning = content.reasoning().first().map(|r| (*r).clone());
    let tool_calls: Vec<_> = content.tool_uses().iter().map(|t| (*t).clone()).collect();

    // Add assistant message to UI
    state.event_sender.add_message(ChatMessage::assistant(
        tools::current_agent(state).agent.name().to_string(),
        content.text(),
        tool_calls.clone(),
        ModelInfo {
            model: model_settings.model,
        },
        response.usage,
        context_info,
        reasoning,
    ));

    // Add to conversation history
    tools::current_agent_mut(state).conversation.push(Message {
        role: MessageRole::Assistant,
        content,
    });

    tool_calls
}

async fn send_request_with_retry(
    state: &mut ActorState,
    request: ConversationRequest,
) -> Result<ConversationResponse> {
    const MAX_RETRIES: u32 = 1000;
    const INITIAL_BACKOFF_MS: u64 = 100;
    const MAX_BACKOFF_MS: u64 = 1000;
    const BACKOFF_MULTIPLIER: f64 = 2.0;

    let mut attempt = 0;

    loop {
        match try_send_request(&state.provider, &request).await {
            Ok(response) => {
                if attempt > 0 {
                    info!("Request succeeded after {} retries", attempt);
                }
                return Ok(response);
            }
            Err(error) => {
                if !should_retry(&error, attempt, MAX_RETRIES) {
                    warn!(
                        attempt,
                        max_retries = MAX_RETRIES,
                        "Request failed after {} retries: {}",
                        attempt,
                        error
                    );
                    return Err(error.into());
                }

                let backoff_ms = calculate_backoff(
                    attempt,
                    INITIAL_BACKOFF_MS,
                    MAX_BACKOFF_MS,
                    BACKOFF_MULTIPLIER,
                );

                emit_retry_event(state, attempt + 1, MAX_RETRIES, &error, backoff_ms);

                warn!(
                    attempt = attempt + 1,
                    max_retries = MAX_RETRIES,
                    backoff_ms,
                    error = %error,
                    "Request failed, retrying after backoff"
                );

                sleep(Duration::from_millis(backoff_ms)).await;
                attempt += 1;
            }
        }
    }
}

async fn try_send_request(
    provider: &Box<dyn AiProvider>,
    request: &ConversationRequest,
) -> Result<ConversationResponse, AiError> {
    provider.converse(request.clone()).await
}

fn should_retry(error: &AiError, attempt: u32, max_retries: u32) -> bool {
    matches!(error, AiError::Retryable(_)) && attempt < max_retries
}

fn calculate_backoff(attempt: u32, initial_ms: u64, max_ms: u64, multiplier: f64) -> u64 {
    let base_backoff = initial_ms as f64 * multiplier.powi(attempt as i32);
    base_backoff.min(max_ms as f64) as u64
}

fn emit_retry_event(
    state: &ActorState,
    attempt: u32,
    max_retries: u32,
    error: &AiError,
    backoff_ms: u64,
) {
    let retry_event = ChatEvent::RetryAttempt {
        attempt,
        max_retries,
        error: error.to_string(),
        backoff_ms,
    };

    if let Err(e) = state.event_sender.event_tx.send(retry_event) {
        error!("Failed to send retry event: {:?}", e);
    }
}
