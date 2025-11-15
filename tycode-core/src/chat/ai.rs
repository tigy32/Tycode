use crate::agents::tool_type::ToolType;
use crate::ai::model::Model;
use crate::ai::{
    error::AiError, model::ModelCost, provider::AiProvider, Content, ContentBlock,
    ConversationRequest, ConversationResponse, Message, MessageRole, ModelSettings, ToolUseData,
};
use crate::chat::context::build_context;
use crate::chat::events::{ChatEvent, ChatMessage, ContextInfo, ModelInfo};
use crate::chat::tools::{self, current_agent_mut};

use crate::settings::config::Settings;
use crate::tools::registry::{resolve_file_modification_api, ToolRegistry};
use anyhow::{bail, Result};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, info, warn};

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

        // Transition to AI processing state
        state.transition_timing_state(crate::chat::actor::TimingState::ProcessingAI);

        // Send request and get response
        let response = match send_request_with_retry(state, request).await {
            Ok(response) => response,
            Err(e) => {
                state
                    .event_sender
                    .send_message(ChatMessage::error(format!("Error: {e:?}")));
                return Ok(());
            }
        };

        // Transition back to idle after AI processing
        state.transition_timing_state(crate::chat::actor::TimingState::Idle);

        // Process the response and update conversation
        let model = model_settings.model;
        let tool_calls = process_ai_response(state, response, model_settings, context_info);

        if tool_calls.is_empty() {
            if !tools::current_agent(state).agent.requires_tool_use() {
                break;
            }
            tools::current_agent_mut(state).conversation.push(Message {
                role: MessageRole::User,
                content: Content::text_only("Tool use is required. Please use one of the available tools to complete your task.".to_string()),
            });
            continue;
        }

        match tools::execute_tool_calls(state, tool_calls, model).await {
            Ok(tool_results) => {
                if tool_results.continue_conversation {
                    continue;
                } else {
                    break;
                }
            }
            Err(e) => {
                state.event_sender.send(ChatEvent::RetryAttempt {
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
                continue;
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
    let system_prompt = current.agent.system_prompt().to_string();
    let model_settings =
        select_model_for_agent(&settings_snapshot, state.provider.as_ref(), agent_name)?;

    // Prepare tools
    let allowed_tool_types: Vec<ToolType> = current.agent.available_tools().into_iter().collect();
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
    let (context_text, context_info) =
        build_context(state, settings_snapshot.auto_context_bytes).await?;

    let include_file_list =
        context_info.directory_list_bytes <= settings_snapshot.auto_context_bytes;

    if !include_file_list {
        state.event_sender.send_message(ChatMessage::warning(
            format!(
                "Warning: The project contains a very large number of files ({} KB in file list). \
                The file list has been omitted from context to prevent overflow. \
                Consider adding a .gitignore file to exclude unnecessary files (e.g., node_modules, target, build artifacts).",
                context_info.directory_list_bytes / 1000
            )
        ));
    }

    let mut conversation = tools::current_agent(state).conversation.clone();
    if conversation.is_empty() {
        bail!("No messages to send to AI. Conversation is empty!")
    }

    conversation
        .last_mut()
        .unwrap()
        .content
        .push(ContentBlock::Text(context_text));

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
    state.session_token_usage.cached_prompt_tokens = Some(
        state.session_token_usage.cached_prompt_tokens.unwrap_or(0)
            + response.usage.cached_prompt_tokens.unwrap_or(0),
    );
    state.session_token_usage.cache_creation_input_tokens = Some(
        state
            .session_token_usage
            .cache_creation_input_tokens
            .unwrap_or(0)
            + response.usage.cache_creation_input_tokens.unwrap_or(0),
    );
    state.session_token_usage.reasoning_tokens = Some(
        state.session_token_usage.reasoning_tokens.unwrap_or(0)
            + response.usage.reasoning_tokens.unwrap_or(0),
    );

    // Calculate and accumulate cost using the actual model used for this response
    let cost = state.provider.get_cost(&model_settings.model);
    let response_cost = cost.calculate_cost(&response.usage);
    state.session_cost += response_cost;

    let reasoning = content.reasoning().first().map(|r| (*r).clone());
    let tool_calls: Vec<_> = content.tool_uses().iter().map(|t| (*t).clone()).collect();

    // Add assistant message to UI and capture for session replay
    state.event_sender.send_message(ChatMessage::assistant(
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

    state.last_command_output = None;

    tool_calls
}

async fn send_request_with_retry(
    state: &mut ActorState,
    mut request: ConversationRequest,
) -> Result<ConversationResponse> {
    const MAX_RETRIES: u32 = 1000;
    const MAX_TRANSIENT_RETRIES: u32 = 10;
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
            Err(error) => match &error {
                AiError::InputTooLong(_) => {
                    warn!("Input too long, compacting context");
                    state.event_sender.send_message(ChatMessage::warning("Input too long. Compacting conversation history to continue...".to_string()));

                    let agent = tools::current_agent_mut(state);
                    if agent.conversation.len() >= 2 {
                        agent.conversation.truncate(agent.conversation.len() - 2);
                    }

                    compact_context(state).await.map_err(|e| {
                        state.event_sender.send_message(ChatMessage::error(format!("Failed to compact conversation history: {e:?}")));
                        e
                    })?;
                    state.event_sender.send_message(ChatMessage::system("Conversation history compacted successfully. Continuing...".to_string()));

                    request.messages = tools::current_agent(state).conversation.clone();

                    continue;
                }
                _ => {
                    let max_retries = if matches!(error, AiError::Transient(_)) {
                        MAX_TRANSIENT_RETRIES
                    } else {
                        MAX_RETRIES
                    };

                    if !should_retry(&error, attempt, max_retries) {
                        warn!(
                            attempt,
                            max_retries, "Request failed after {} retries: {}", attempt, error
                        );
                        return Err(error.into());
                    }

                    let backoff_ms = calculate_backoff(
                        attempt,
                        INITIAL_BACKOFF_MS,
                        MAX_BACKOFF_MS,
                        BACKOFF_MULTIPLIER,
                    );

                    emit_retry_event(state, attempt + 1, max_retries, &error, backoff_ms);

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
            },
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
    (matches!(error, AiError::Retryable(_)) || matches!(error, AiError::Transient(_)))
        && attempt < max_retries
}

fn calculate_backoff(attempt: u32, initial_ms: u64, max_ms: u64, multiplier: f64) -> u64 {
    let base_backoff = initial_ms as f64 * multiplier.powi(attempt as i32);
    base_backoff.min(max_ms as f64) as u64
}

fn emit_retry_event(
    state: &mut ActorState,
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

    state.event_sender.send(retry_event);
}

async fn compact_context(state: &mut ActorState) -> Result<()> {
    let conversation = tools::current_agent(state).conversation.clone();

    let settings_snapshot = state.settings.settings();
    let agent_name = tools::current_agent(state).agent.name();
    let model_settings =
        select_model_for_agent(&settings_snapshot, state.provider.as_ref(), agent_name)?;

    let summarization_prompt = "Please provide a concise summary of the conversation so far, preserving all critical context, decisions, and important details. The summary will be used to continue the conversation efficiently. Focus on:
1. Key decisions made
2. Important context about the task
3. Current state of work and remaining work
4. Any critical information needed to continue effectively";

    // Remove ToolUse and ToolResult blocks to prevent Bedrock API validation error
    // "The toolConfig field must be defined when using toolUse and toolResult content blocks"
    let filtered_messages: Vec<Message> = conversation
        .iter()
        .map(|msg| {
            let filtered_content = Content::new(
                msg.content
                    .blocks()
                    .iter()
                    .filter(|block| !matches!(block, ContentBlock::ToolUse(_) | ContentBlock::ToolResult(_)))
                    .cloned()
                    .collect()
            );
            Message {
                role: msg.role.clone(),
                content: filtered_content,
            }
        })
        .collect();

    let mut summary_request = ConversationRequest {
        messages: filtered_messages,
        model: model_settings.clone(),
        system_prompt: "You are a conversation summarizer. Create concise, comprehensive summaries that preserve critical context.".to_string(),
        stop_sequences: vec![],
        tools: vec![],
    };

    summary_request.messages.push(Message {
        role: MessageRole::User,
        content: Content::text_only(summarization_prompt.to_string()),
    });

    let summary_response = try_send_request(&state.provider, &summary_request).await?;
    let summary_text = summary_response.content.text();

    let agent = tools::current_agent_mut(state);
    agent.conversation.clear();
    agent.conversation.push(Message {
        role: MessageRole::User,
        content: Content::text_only(format!(
            "Context summary from previous conversation:\n{}\n\nPlease continue assisting based on this context.",
            summary_text
        )),
    });

    state.tracked_files.clear();

    Ok(())
}
