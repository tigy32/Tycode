use crate::ai::{
    error::AiError, provider::AiProvider, Content, ContentBlock, ConversationRequest,
    ConversationResponse, Message, MessageRole, ModelSettings, ToolUseData,
};
use crate::chat::context::ContextInputs;
use crate::chat::events::{ChatEvent, ChatMessage, ContextInfo, ModelInfo};
use crate::chat::request::{prepare_request, select_model_for_agent};
use crate::chat::tool_extraction::extract_all_tool_calls;
use crate::chat::tools::{self, current_agent_mut};

use crate::ai::tweaks::resolve_from_settings;
use crate::settings::config::ToolCallStyle;
use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, warn};

use super::actor::ActorState;

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
    let settings_snapshot = state.settings.settings();

    let context_inputs = ContextInputs {
        workspace_roots: state.workspace_roots.clone(),
        tracked_files: state.tracked_files.iter().cloned().collect(),
        task_list: state.task_list.clone(),
        command_outputs: state.last_command_outputs.clone(),
        memory_log: state.memory_log.clone(),
        additional_tools: state.additional_tools.clone(),
    };

    let (request, context_info, model_settings) = prepare_request(
        current.agent.as_ref(),
        &current.conversation,
        state.provider.as_ref(),
        &settings_snapshot,
        &state.steering,
        &context_inputs,
        state.mcp_manager.as_ref(),
    )
    .await?;

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

    let extraction = extract_all_tool_calls(&content);
    let tool_calls = extraction.tool_calls;
    let display_text = extraction.display_text;
    let xml_parse_error = extraction.xml_parse_error;
    let json_parse_error = extraction.json_parse_error;

    // Surface parse errors by adding to conversation for AI to retry
    if let Some(parse_error) = xml_parse_error {
        warn!("XML tool call parse error: {parse_error}");
        tools::current_agent_mut(state).conversation.push(Message {
            role: MessageRole::User,
            content: Content::text_only(format!(
                "Error parsing XML tool calls: {}. Please check your XML format and retry.",
                parse_error
            )),
        });
    }
    if let Some(parse_error) = json_parse_error {
        warn!("JSON tool call parse error: {parse_error}");
        tools::current_agent_mut(state).conversation.push(Message {
            role: MessageRole::User,
            content: Content::text_only(format!(
                "Error parsing JSON tool calls: {}. Please check your JSON format and retry.",
                parse_error
            )),
        });
    }

    state.event_sender.send_message(ChatMessage::assistant(
        tools::current_agent(state).agent.name().to_string(),
        display_text.clone(),
        tool_calls.clone(),
        ModelInfo {
            model: model_settings.model,
        },
        response.usage,
        context_info,
        reasoning,
    ));

    // Determine tool call style to decide normalization behavior
    let settings_snapshot = state.settings.settings();
    let resolved_tweaks = resolve_from_settings(
        &settings_snapshot,
        state.provider.as_ref(),
        model_settings.model,
    );

    // XML mode: Keep text-only to avoid Bedrock's toolConfig requirement
    // Native mode: Normalize to ToolUse blocks for provider compatibility
    let mut blocks: Vec<ContentBlock> = Vec::new();

    for r in content.reasoning() {
        blocks.push(ContentBlock::ReasoningContent(r.clone()));
    }

    let trimmed_text = display_text.trim();
    if !trimmed_text.is_empty() {
        blocks.push(ContentBlock::Text(trimmed_text.to_string()));
    }

    // Only add ToolUse blocks in native mode
    if resolved_tweaks.tool_call_style != ToolCallStyle::Xml {
        for tool_use in &tool_calls {
            blocks.push(ContentBlock::ToolUse(tool_use.clone()));
        }
    }

    tools::current_agent_mut(state).conversation.push(Message {
        role: MessageRole::Assistant,
        content: Content::new(blocks),
    });

    state.last_command_outputs.clear();

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
                    state.event_sender.send_message(ChatMessage::warning(
                        "Context overflow detected, auto-compacting conversation...".to_string(),
                    ));
                    warn!("Input too long, compacting context");

                    let agent = tools::current_agent_mut(state);
                    let messages_before = agent.conversation.len();
                    if agent.conversation.len() >= 2 {
                        agent.conversation.truncate(agent.conversation.len() - 2);
                    }

                    compact_context(state).await?;

                    let messages_after = tools::current_agent(state).conversation.len();
                    state.event_sender.send_message(ChatMessage::system(format!(
                        "Compaction complete: {} messages → {} (summary). Tracked files cleared.",
                        messages_before, messages_after
                    )));

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
    provider: &Arc<dyn AiProvider>,
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

    // Filter ToolUse/ToolResult blocks before summarization to avoid Bedrock's
    // toolConfig validation error. Bedrock requires toolConfig when messages contain
    // these blocks, but summarization requests don't offer tools (tools: vec![]).
    // Only conversational content (Text, ReasoningContent) is needed for summarization.
    let filtered_messages: Vec<Message> = conversation
        .clone()
        .into_iter()
        .map(|mut msg| {
            let filtered_blocks: Vec<ContentBlock> = msg
                .content
                .into_blocks()
                .into_iter()
                .filter(|block| {
                    !matches!(block, ContentBlock::ToolUse { .. })
                        && !matches!(block, ContentBlock::ToolResult { .. })
                })
                .collect();
            msg.content = Content::new(filtered_blocks);
            msg
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
