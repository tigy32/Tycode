use crate::ai::{
    error::AiError, provider::AiProvider, Content, ContentBlock, ConversationRequest,
    ConversationResponse, Message, MessageRole, ModelSettings, StreamEvent, ToolUseData,
};
use crate::chat::events::{ChatEvent, ChatMessage, ModelInfo};
use crate::chat::request::{prepare_request, select_model_for_agent};
use crate::chat::tool_extraction::extract_all_tool_calls;
use crate::chat::tools::{self, current_agent_mut};

use crate::ai::tweaks::resolve_from_settings;
use crate::settings::config::ToolCallStyle;
use anyhow::{Context, Result};
use chrono::Utc;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tokio_stream::{Stream, StreamExt};
use tracing::{info, warn};

use super::actor::ActorState;

pub async fn send_ai_request(state: &mut ActorState) -> Result<()> {
    loop {
        let (agent, conversation) =
            tools::current_agent(state, |a| (a.agent.clone(), a.conversation.clone()));

        let provider = state.provider.read().unwrap().clone();
        let (request, model_settings, context_breakdown) = prepare_request(
            agent.as_ref(),
            &conversation,
            provider.as_ref(),
            state.settings.clone(),
            &state.steering,
            &state.prompt_builder,
            &state.context_builder,
            &state.modules,
        )
        .await?;

        state.pending_context_breakdown = Some(context_breakdown);

        state.transition_timing_state(crate::chat::actor::TimingState::ProcessingAI);

        let stream = match send_request_streaming_with_retry(state, request).await {
            Ok(stream) => stream,
            Err(e) => {
                state
                    .event_sender
                    .send_message(ChatMessage::error(format!("Error: {e:?}")));
                return Ok(());
            }
        };

        state.transition_timing_state(crate::chat::actor::TimingState::Idle);

        let model = model_settings.model;
        let tool_calls = consume_ai_stream(state, stream, model_settings).await?;

        if tool_calls.is_empty() {
            let is_sub_agent = state.spawn_module.stack_depth() > 1;
            if !is_sub_agent && !tools::current_agent(state, |a| a.agent.requires_tool_use()) {
                break;
            }
            tools::current_agent_mut(state, |a| {
                a.conversation.push(Message {
                role: MessageRole::User,
                content: Content::text_only("Tool use is required. Please use one of the available tools to complete your task.".to_string()),
            })
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

                current_agent_mut(state, |a| {
                    let _last = a.conversation.pop();
                    a.conversation.push(Message {
                        role: MessageRole::User,
                        content: Content::text_only(format!(
                            "You attempted to use tools incorrectly; the system has removed the incorrect tool calls from the conversation history. Please incorporate the following feedback feedback and retry. Here are the errors from the (removed) tool calls: {}",
                            e.to_string()
                        )),
                    });
                });
                continue;
            }
        }
    }

    Ok(())
}

fn finalize_ai_response(
    state: &mut ActorState,
    response: ConversationResponse,
    model_settings: ModelSettings,
) -> Result<(Vec<ToolUseData>, ChatMessage)> {
    let content = response.content.clone();

    info!(?response, "AI response");

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

    let provider = state.provider.read().unwrap().clone();
    let cost = provider.get_cost(&model_settings.model);
    let response_cost = cost.calculate_cost(&response.usage);
    state.session_cost += response_cost;

    let reasoning = content.reasoning().first().map(|r| (*r).clone());

    let extraction = extract_all_tool_calls(&content);
    let tool_calls = extraction.tool_calls;
    let display_text = extraction.display_text;
    let xml_parse_error = extraction.xml_parse_error;
    let json_parse_error = extraction.json_parse_error;

    if let Some(parse_error) = xml_parse_error {
        warn!("XML tool call parse error: {parse_error}");
        tools::current_agent_mut(state, |a| {
            a.conversation.push(Message {
                role: MessageRole::User,
                content: Content::text_only(format!(
                    "Error parsing XML tool calls: {}. Please check your XML format and retry.",
                    parse_error
                )),
            })
        });
    }
    if let Some(parse_error) = json_parse_error {
        warn!("JSON tool call parse error: {parse_error}");
        tools::current_agent_mut(state, |a| {
            a.conversation.push(Message {
                role: MessageRole::User,
                content: Content::text_only(format!(
                    "Error parsing JSON tool calls: {}. Please check your JSON format and retry.",
                    parse_error
                )),
            })
        });
    }

    let context_breakdown = if let Some(mut cb) = state.pending_context_breakdown.take() {
        cb.input_tokens = response.usage.input_tokens
            + response.usage.cached_prompt_tokens.unwrap_or(0)
            + response.usage.cache_creation_input_tokens.unwrap_or(0)
            + response.usage.output_tokens;
        for block in response.content.blocks() {
            let block_bytes = serde_json::to_string(block)
                .context("failed to serialize content block for context breakdown")?
                .len();
            if matches!(block, ContentBlock::ReasoningContent(_)) {
                cb.reasoning_bytes += block_bytes;
            } else {
                cb.conversation_history_bytes += block_bytes;
            }
        }
        Some(cb)
    } else {
        None
    };

    let message = ChatMessage::assistant(
        tools::current_agent(state, |a| a.agent.name().to_string()),
        display_text.clone(),
        tool_calls.clone(),
        ModelInfo {
            model: model_settings.model,
        },
        response.usage.clone(),
        reasoning,
        context_breakdown,
    );

    let settings_snapshot = state.settings.settings();
    let provider = state.provider.read().unwrap().clone();
    let resolved_tweaks =
        resolve_from_settings(&settings_snapshot, provider.as_ref(), model_settings.model);

    let mut blocks: Vec<ContentBlock> = Vec::new();

    for r in content.reasoning() {
        blocks.push(ContentBlock::ReasoningContent(r.clone()));
    }

    let trimmed_text = display_text.trim();
    if !trimmed_text.is_empty() {
        blocks.push(ContentBlock::Text(trimmed_text.to_string()));
    }

    if resolved_tweaks.tool_call_style != ToolCallStyle::Xml {
        for tool_use in &tool_calls {
            blocks.push(ContentBlock::ToolUse(tool_use.clone()));
        }
    }

    tools::current_agent_mut(state, |a| {
        a.conversation.push(Message {
            role: MessageRole::Assistant,
            content: Content::new(blocks),
        })
    });

    state.last_command_outputs.clear();

    Ok((tool_calls, message))
}

async fn consume_ai_stream(
    state: &mut ActorState,
    stream: Pin<Box<dyn Stream<Item = Result<StreamEvent, AiError>> + Send>>,
    model_settings: ModelSettings,
) -> Result<Vec<ToolUseData>> {
    let disable_streaming = state.settings.settings().disable_streaming;
    let message_id = format!("msg-{}", Utc::now().timestamp_millis());
    let agent_name = tools::current_agent(state, |a| a.agent.name().to_string());

    tokio::pin!(stream);

    let mut tool_calls = Vec::new();
    let mut received_text_deltas = false;
    let mut stream_started = false;

    while let Some(event) = stream.next().await {
        let event: StreamEvent = event.map_err(|e| anyhow::anyhow!("Stream error: {e:?}"))?;
        match event {
            StreamEvent::TextDelta { text } => {
                received_text_deltas = true;
                if !disable_streaming {
                    if !stream_started {
                        state.event_sender.stream_start(
                            message_id.clone(),
                            agent_name.clone(),
                            model_settings.model,
                        );
                        stream_started = true;
                    }
                    state.event_sender.stream_delta(message_id.clone(), text);
                }
            }
            StreamEvent::ReasoningDelta { text } => {
                if !disable_streaming {
                    if !stream_started {
                        state.event_sender.stream_start(
                            message_id.clone(),
                            agent_name.clone(),
                            model_settings.model,
                        );
                        stream_started = true;
                    }
                    state
                        .event_sender
                        .stream_reasoning_delta(message_id.clone(), text);
                }
            }
            StreamEvent::ContentBlockStart | StreamEvent::ContentBlockStop => {}
            StreamEvent::MessageComplete { response } => {
                if !disable_streaming && !received_text_deltas {
                    let full_text = response.content.text();
                    if !full_text.is_empty() {
                        if !stream_started {
                            state.event_sender.stream_start(
                                message_id.clone(),
                                agent_name.clone(),
                                model_settings.model,
                            );
                            stream_started = true;
                        }
                        state
                            .event_sender
                            .stream_delta(message_id.clone(), full_text);
                    }
                }
                let (calls, message) =
                    finalize_ai_response(state, response, model_settings.clone())?;
                tool_calls = calls;
                if disable_streaming {
                    state.event_sender.send_message(message);
                } else {
                    state.event_sender.stream_end(message);
                }
            }
        }
    }

    Ok(tool_calls)
}

async fn try_send_request_stream(
    provider: &Arc<dyn AiProvider>,
    request: &ConversationRequest,
) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, AiError>> + Send>>, AiError> {
    provider.converse_stream(request.clone()).await
}

fn truncate_recent_conversation(state: &mut ActorState) -> usize {
    tools::current_agent_mut(state, |agent| {
        let len = agent.conversation.len();
        if len >= 2 {
            agent.conversation.truncate(len - 2);
        }
        len
    })
}

async fn send_request_streaming_with_retry(
    state: &mut ActorState,
    mut request: ConversationRequest,
) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, AiError>> + Send>>> {
    const MAX_RETRIES: u32 = 1000;
    const MAX_TRANSIENT_RETRIES: u32 = 10;
    const INITIAL_BACKOFF_MS: u64 = 100;
    const MAX_BACKOFF_MS: u64 = 1000;
    const BACKOFF_MULTIPLIER: f64 = 2.0;

    let mut attempt = 0;

    loop {
        let provider = state.provider.read().unwrap().clone();
        let result = try_send_request_stream(&provider, &request).await;

        let max_retries = match &result {
            Err(AiError::Transient(_)) => MAX_TRANSIENT_RETRIES,
            _ => MAX_RETRIES,
        };

        match result {
            Ok(stream) => {
                if attempt > 0 {
                    info!("Streaming request succeeded after {} retries", attempt);
                }
                return Ok(stream);
            }
            Err(AiError::InputTooLong(_)) => {
                state.event_sender.send_message(ChatMessage::warning(
                    "Context overflow detected, auto-compacting conversation...".to_string(),
                ));
                warn!("Input too long, compacting context");

                let messages_before = truncate_recent_conversation(state);

                compact_context(state).await?;

                let messages_after = tools::current_agent(state, |a| a.conversation.len());
                state.event_sender.send_message(ChatMessage::system(format!(
                    "Compaction complete: {} messages → {} (summary). Tracked files cleared.",
                    messages_before, messages_after
                )));

                request.messages = state
                    .spawn_module
                    .with_current_agent(|a| a.conversation.clone())
                    .unwrap_or_default();

                continue;
            }
            Err(error) => {
                if !should_retry(&error, attempt, max_retries) {
                    warn!(
                        attempt,
                        max_retries,
                        "Streaming request failed after {} retries: {}",
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

                emit_retry_event(state, attempt + 1, max_retries, &error, backoff_ms);

                warn!(
                    attempt = attempt + 1,
                    max_retries = MAX_RETRIES,
                    backoff_ms,
                    error = %error,
                    "Streaming request failed, retrying after backoff"
                );

                sleep(Duration::from_millis(backoff_ms)).await;
                attempt += 1;
            }
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
    let (conversation, agent_name) = tools::current_agent(state, |a| {
        (a.conversation.clone(), a.agent.name().to_string())
    });

    let provider = state.provider.read().unwrap().clone();
    let settings_snapshot = state.settings.settings();
    let model_settings =
        select_model_for_agent(&settings_snapshot, provider.as_ref(), &agent_name)?;

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

    let summary_response = try_send_request(&provider, &summary_request).await?;
    let summary_text = summary_response.content.text();

    tools::current_agent_mut(state, |agent| {
        agent.conversation.clear();
        agent.conversation.push(Message {
            role: MessageRole::User,
            content: Content::text_only(format!(
                "Context summary from previous conversation:\n{}\n\nPlease continue assisting based on this context.",
                summary_text
            )),
        });
    });

    state.tracked_files.clear();

    Ok(())
}
