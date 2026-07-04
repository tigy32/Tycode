use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use base64::engine::general_purpose;
use base64::Engine;
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::agents::agent::{ActiveAgent, Agent};
use crate::agents::catalog::AgentCatalog;
use crate::agents::code_review::CodeReviewAgent;
use crate::agents::runner::AgentRunner;
use crate::ai::model::Model;
use crate::ai::types::ImageData;
use crate::ai::{Content, ContentBlock, Message, MessageRole, ToolResultData, ToolUseData};
use crate::chat::actor::ActorState;
use crate::chat::events::{ChatEvent, ChatMessage, ToolExecutionResult, ToolRequest};
use crate::chat::protocol::TurnProtocol;
use crate::chat::request::pinned_model_settings;
use crate::modules::execution::config::ExecutionConfig;
use crate::modules::execution::{compact_output, truncate_and_persist};
use crate::orchestration::events::{
    next_orchestration_id, task_preview, AgentId, AgentOrigin, OrchestrationEvent,
    OrchestrationPayload, OutcomeStatus, WorkerInfo, WorkflowPhase,
};
use crate::orchestration::{
    ChildAction, ChildOutcome, CompletionAction, ConversationSeed, FanOutSpec, SpawnSpec,
    TaskAction, WorkerResult, WorkerSpec, FANOUT_AGENT,
};
use crate::settings::config::SpawnContextMode;
use futures_util::stream::{FuturesUnordered, StreamExt};

use crate::tools::r#trait::{ContinuationPreference, ToolCallHandle, ToolOutput};
use crate::tools::registry::ToolRegistry;
use crate::tools::ToolName;

use crate::chat::events::ToolRequestType;

#[derive(Debug)]
pub struct ToolResults {
    pub continue_conversation: bool,
}

struct ToolCallResult {
    content_block: ContentBlock,
    continuation_preference: ContinuationPreference,
}

impl ToolCallResult {
    fn immediate(
        content_block: ContentBlock,
        continuation_preference: ContinuationPreference,
    ) -> Self {
        Self {
            content_block,
            continuation_preference,
        }
    }
}

enum DeferredAction {
    PushAgent {
        agent: Arc<dyn Agent>,
        task: String,
        agent_type: String,
        spawn_params: HashMap<String, Value>,
        tool_call_id: String,
        tool_name: String,
    },
    PopAgent {
        success: bool,
        result: String,
        tool_call_id: String,
        tool_name: String,
    },
}

// Helper functions for ActorState - delegate to spawn_module (closure-only API)
pub fn current_agent<F, R>(state: &ActorState, f: F) -> R
where
    F: FnOnce(&ActiveAgent) -> R,
{
    state
        .spawn_module
        .with_current_agent(f)
        .expect("No active agent")
}

pub fn current_agent_mut<F, R>(state: &ActorState, f: F) -> R
where
    F: FnOnce(&mut ActiveAgent) -> R,
{
    state
        .spawn_module
        .with_current_agent_mut(f)
        .expect("No active agent")
}

/// Emit a structured orchestration event on the chat event stream.
pub(crate) fn send_orchestration(
    state: &ActorState,
    agent_id: &str,
    agent_type: &str,
    payload: OrchestrationPayload,
) {
    state
        .event_sender
        .send(ChatEvent::Orchestration(OrchestrationEvent {
            agent_id: agent_id.to_string(),
            agent_type: agent_type.to_string(),
            payload,
        }));
}

/// Announce an agent that predates structured events — the root of the
/// interactive stack, including one freshly created by a session restore or
/// agent switch — so children's parent ids always resolve.
fn ensure_current_agent_announced(state: &ActorState) {
    let unannounced = current_agent(state, |a| {
        (!a.announced).then(|| (a.id.clone(), a.agent.name().to_string()))
    });
    let Some((agent_id, agent_type)) = unannounced else {
        return;
    };
    current_agent_mut(state, |a| a.announced = true);
    send_orchestration(
        state,
        &agent_id,
        &agent_type,
        OrchestrationPayload::AgentStarted {
            parent_agent_id: None,
            task_preview: String::new(),
            origin: AgentOrigin::Root,
            depth: state.spawn_module.stack_depth(),
            interactive: true,
            model: None,
        },
    );
}

/// Human-readable progress strings duplicate the structured orchestration
/// events; UIs consuming the structured stream can turn them off with the
/// `orchestration_progress_messages` setting.
fn send_progress_message(state: &ActorState, text: String) {
    if state.settings.settings().orchestration_progress_messages {
        state.event_sender.send_message(ChatMessage::system(text));
    }
}

/// Emit PhaseChanged when a hook moved the workflow to a different phase.
fn emit_phase_change(
    state: &ActorState,
    agent_id: &str,
    agent_type: &str,
    before: Option<WorkflowPhase>,
) {
    let after = current_agent(state, |a| a.workflow.phase());
    if after != before {
        if let Some(phase) = after {
            send_orchestration(
                state,
                agent_id,
                agent_type,
                OrchestrationPayload::PhaseChanged { phase },
            );
        }
    }
}

fn send_tool_completion(
    protocol: &mut TurnProtocol,
    tool_call_id: &str,
    tool_name: &str,
    tool_result: ToolExecutionResult,
    success: bool,
    error: Option<String>,
) {
    protocol.tool_completed(tool_call_id, tool_name, tool_result, success, error);
}

pub async fn execute_tool_calls(
    state: &mut ActorState,
    tool_calls: Vec<ToolUseData>,
    protocol: &mut TurnProtocol,
) -> Result<ToolResults> {
    state.transition_timing_state(crate::chat::actor::TimingState::ExecutingTools);

    let execution_config: ExecutionConfig = state.settings.get_module_config("execution");
    let max_output_bytes = execution_config.max_output_bytes.unwrap_or(200_000);
    let tool_calls_dir = state.tool_calls_dir.clone();

    info!(
        tool_count = tool_calls.len(),
        tools = ?tool_calls.iter().map(|t| &t.name).collect::<Vec<_>>(),
        "Executing tool calls"
    );

    // Get allowed tools for security checks
    let allowed_tool_names: Vec<ToolName> = current_agent(state, |a| a.agent.available_tools());

    let current_agent_name = state.spawn_module.current_agent_name().unwrap_or_default();
    let all_tools = crate::spawn::build_tools(
        &state.modules,
        state.spawn_module.catalog().clone(),
        &current_agent_name,
    )
    .await;

    let tool_registry = ToolRegistry::new(all_tools);

    let mut all_results = vec![];

    // Initialize preferences vector early to track all error and success preferences
    let mut preferences = vec![];

    let mut validated: Vec<(ToolUseData, Box<dyn ToolCallHandle>)> = vec![];
    let mut invalid_tool_results = vec![];
    for tool_use in tool_calls {
        match tool_registry
            .process_tools(&tool_use, &allowed_tool_names)
            .await
        {
            Ok(handle) => validated.push((tool_use, handle)),
            Err(error) => {
                warn!(
                    tool_name = %tool_use.name,
                    error = %error,
                    "Tool call validation failed, will return error response"
                );
                let error_result = handle_tool_error(state, protocol, &tool_use, error);
                invalid_tool_results.push(error_result.content_block);
                preferences.push(error_result.continuation_preference);
            }
        }
    }

    let mut results = Vec::new();
    let mut deferred_actions = Vec::new();
    for (raw, handle) in validated {
        let request = handle.tool_request();
        let tool_call_id = request.tool_call_id.clone();
        let tool_name = request.tool_name.clone();
        protocol.tool_request(request);

        let output = handle.execute().await;

        match output {
            ToolOutput::Result {
                content,
                is_error,
                continuation,
                ui_result,
            } => {
                let content =
                    truncate_tool_result(content, &raw.id, max_output_bytes, &tool_calls_dir).await;

                let result = ToolResultData {
                    tool_use_id: raw.id.clone(),
                    content,
                    is_error,
                };

                send_tool_completion(
                    protocol,
                    &tool_call_id,
                    &tool_name,
                    ui_result,
                    !is_error,
                    None,
                );

                let result_block = ContentBlock::ToolResult(result);
                protocol.stage_tool_result(result_block.clone());
                results.push(result_block);
                preferences.push(continuation);
            }
            ToolOutput::ImageResult {
                content,
                images,
                continuation,
                ui_result,
            } => {
                let content =
                    truncate_tool_result(content, &raw.id, max_output_bytes, &tool_calls_dir).await;

                let result = ToolResultData {
                    tool_use_id: raw.id.clone(),
                    content,
                    is_error: false,
                };

                send_tool_completion(protocol, &tool_call_id, &tool_name, ui_result, true, None);

                let result_block = ContentBlock::ToolResult(result);
                protocol.stage_tool_result(result_block.clone());
                results.push(result_block);
                for (image_data, media_type) in images {
                    results.push(ContentBlock::Image(ImageData {
                        media_type,
                        data: general_purpose::STANDARD.encode(&image_data),
                    }));
                }
                preferences.push(continuation);
            }
            ToolOutput::PushAgent {
                agent,
                task,
                spawn_params,
            } => {
                let agent_type = agent.name().to_string();
                let acknowledgment = ContentBlock::ToolResult(ToolResultData {
                    tool_use_id: raw.id.clone(),
                    content: json!({
                        "status": "spawned",
                        "agent_type": agent_type,
                        "task": task
                    })
                    .to_string(),
                    is_error: false,
                });
                protocol.stage_tool_result(acknowledgment.clone());
                results.push(acknowledgment);
                deferred_actions.push(DeferredAction::PushAgent {
                    agent,
                    task,
                    agent_type,
                    spawn_params,
                    tool_call_id,
                    tool_name,
                });
                preferences.push(ContinuationPreference::Continue);
            }
            ToolOutput::PopAgent { success, result } => {
                let is_root = state.spawn_module.stack_depth() <= 1;
                let preference = if is_root {
                    ContinuationPreference::Stop
                } else {
                    ContinuationPreference::Continue
                };

                let acknowledgment = ContentBlock::ToolResult(ToolResultData {
                    tool_use_id: raw.id.clone(),
                    content: json!({
                        "status": "completing",
                        "success": success,
                        "result": result
                    })
                    .to_string(),
                    is_error: false,
                });
                protocol.stage_tool_result(acknowledgment.clone());
                results.push(acknowledgment);
                deferred_actions.push(DeferredAction::PopAgent {
                    success,
                    result,
                    tool_call_id,
                    tool_name,
                });
                preferences.push(preference);
            }
            ToolOutput::PromptUser { question } => {
                let result = ToolResultData {
                    tool_use_id: raw.id.clone(),
                    content: json!({}).to_string(),
                    is_error: false,
                };

                let agent_name = current_agent(state, |a| a.agent.name().to_string());
                state.event_sender.send_message(ChatMessage::assistant(
                    agent_name,
                    question,
                    vec![],
                    crate::chat::events::ModelInfo { model: Model::None },
                    crate::ai::types::TokenUsage::empty(),
                    None,
                    None,
                ));

                send_tool_completion(
                    protocol,
                    &tool_call_id,
                    &tool_name,
                    ToolExecutionResult::Other {
                        result: json!({ "status": "waiting_for_user" }),
                    },
                    true,
                    None,
                );

                let result_block = ContentBlock::ToolResult(result);
                protocol.stage_tool_result(result_block.clone());
                results.push(result_block);
                preferences.push(ContinuationPreference::Stop);
            }
        }
    }

    // Implement truth table for continuation preferences:
    // - Any Stop → stop conversation
    // - Otherwise, any Continue → continue conversation
    let mut continue_conversation = if preferences.contains(&ContinuationPreference::Stop) {
        false
    } else {
        preferences.contains(&ContinuationPreference::Continue)
    };

    // Combine invalid tool error responses with valid tool execution results
    all_results.extend(invalid_tool_results);
    all_results.extend(results);

    protocol.append_tool_results_to_conversation(all_results);

    // Execute deferred actions after conversation update. A completion that
    // cascades all the way to the root agent must stop the conversation even
    // though the tool's continuation preference was computed before the
    // cascade ran.
    for action in deferred_actions {
        if execute_deferred_action(state, protocol, action).await {
            continue_conversation = false;
        }
    }

    state.transition_timing_state(crate::chat::actor::TimingState::Idle);

    if let Err(e) = state.save_session() {
        tracing::warn!("Failed to auto-save session after tool execution: {}", e);
    }

    Ok(ToolResults {
        continue_conversation,
    })
}

async fn truncate_tool_result(
    content: String,
    tool_call_id: &str,
    max_bytes: usize,
    tool_calls_dir: &Path,
) -> String {
    if content.len() <= max_bytes {
        return content;
    }

    let display_path = tool_calls_dir.join(tool_call_id).display().to_string();

    match truncate_and_persist(
        &content,
        tool_call_id,
        max_bytes,
        tool_calls_dir,
        &display_path,
    )
    .await
    {
        Ok((truncated, _)) => truncated,
        Err(e) => {
            warn!(
                ?e,
                "Failed to truncate/persist tool result, using compact_output fallback"
            );
            compact_output(&content, max_bytes)
        }
    }
}

fn create_short_message(detailed: &str) -> String {
    let first_line = detailed.lines().next().unwrap_or(detailed);
    if first_line.chars().count() > 100 {
        format!("{}...", first_line.chars().take(100).collect::<String>())
    } else {
        first_line.to_string()
    }
}

fn handle_tool_error(
    _state: &mut ActorState,
    protocol: &mut TurnProtocol,
    tool_use: &ToolUseData,
    error: String,
) -> ToolCallResult {
    let short_message = create_short_message(&error);

    let result = ToolResultData {
        tool_use_id: tool_use.id.clone(),
        content: error.clone(),
        is_error: true,
    };

    info!(
        tool_name = %tool_use.name,
        ?result,
        "Tool execution failed"
    );

    protocol.tool_request(tool_request_from_model_tool_use(tool_use));
    send_tool_completion(
        protocol,
        &tool_use.id,
        &tool_use.name,
        ToolExecutionResult::Error {
            short_message,
            detailed_message: error.clone(),
        },
        false,
        Some(error),
    );

    let result_block = ContentBlock::ToolResult(result);
    protocol.stage_tool_result(result_block.clone());

    ToolCallResult::immediate(result_block, ContinuationPreference::Continue)
}

fn tool_request_from_model_tool_use(tool_use: &ToolUseData) -> ToolRequest {
    ToolRequest {
        tool_call_id: tool_use.id.clone(),
        tool_name: tool_use.name.clone(),
        tool_type: ToolRequestType::Other {
            args: tool_use.arguments.clone(),
        },
    }
}

/// Returns true when the action stopped the conversation (a completion
/// cascaded to the root agent).
async fn execute_deferred_action(
    state: &mut ActorState,
    protocol: &mut TurnProtocol,
    action: DeferredAction,
) -> bool {
    match action {
        DeferredAction::PushAgent {
            agent,
            task,
            agent_type,
            spawn_params,
            tool_call_id,
            tool_name,
        } => {
            execute_push_agent(
                state,
                protocol,
                agent,
                task,
                agent_type,
                spawn_params,
                tool_call_id,
                tool_name,
            )
            .await
        }
        DeferredAction::PopAgent {
            success,
            result,
            tool_call_id,
            tool_name,
        } => execute_pop_agent(state, protocol, success, result, tool_call_id, tool_name).await,
    }
}

fn run_on_agent_pushed_hooks(
    state: &mut ActorState,
    task: &str,
    agent_type: &str,
    spawn_params: &HashMap<String, Value>,
) {
    for module in &state.modules {
        let mut module_params: HashMap<String, Value> = module
            .spawn_parameters()
            .into_iter()
            .filter_map(|param| {
                spawn_params
                    .get(param.name)
                    .cloned()
                    .map(|v| (param.name.to_string(), v))
            })
            .collect();

        module_params.insert("task".to_string(), Value::String(task.to_string()));
        module_params.insert(
            "agent_type".to_string(),
            Value::String(agent_type.to_string()),
        );

        state.spawn_module.with_current_agent(|agent| {
            module.on_agent_pushed(agent, module_params);
        });
    }
}

fn run_on_agent_popped_hooks(state: &mut ActorState) {
    for module in &state.modules {
        state.spawn_module.with_current_agent(|agent| {
            module.on_agent_popped(agent);
        });
    }
}

async fn execute_push_agent(
    state: &mut ActorState,
    protocol: &mut TurnProtocol,
    agent: Arc<dyn Agent>,
    task: String,
    agent_type: String,
    spawn_params: HashMap<String, Value>,
    tool_call_id: String,
    tool_name: String,
) -> bool {
    info!("Pushing new agent: task={}", task);

    let initial_message = task.clone();

    ensure_current_agent_announced(state);
    let parent_agent_id = current_agent(state, |a| a.id.clone());
    let mut new_agent = ActiveAgent::new(agent);
    new_agent.announced = true;
    let new_agent_id = new_agent.id.clone();

    // Why: Fork mode copies parent conversation for continuity; Fresh mode starts clean
    let spawn_mode = state.settings.settings().spawn_context_mode.clone();
    if spawn_mode == SpawnContextMode::Fork {
        if let Some(parent_conv) = state
            .spawn_module
            .with_current_agent(|a| a.conversation.clone())
        {
            new_agent.conversation = parent_conv;
        }

        // Orientation message helps spawned agent understand its context
        let orientation = format!(
            "--- AGENT TRANSITION ---\n\
            You are now a {} sub-agent spawned to handle a specific task. \
            The conversation above is from the parent agent - use it for context only. \
            Focus on completing your assigned task below. \
            When done, use complete_task to return control to the parent.",
            agent_type
        );
        new_agent.conversation.push(Message {
            role: MessageRole::User,
            content: Content::text_only(orientation),
        });
    }

    new_agent.conversation.push(Message {
        role: MessageRole::User,
        content: Content::text_only(initial_message.clone()),
    });

    state.spawn_module.push_agent(new_agent);

    run_on_agent_pushed_hooks(state, &task, &agent_type, &spawn_params);

    send_orchestration(
        state,
        &new_agent_id,
        &agent_type,
        OrchestrationPayload::AgentStarted {
            parent_agent_id: Some(parent_agent_id),
            task_preview: task_preview(&task),
            origin: AgentOrigin::Tool {
                tool_call_id: tool_call_id.clone(),
            },
            depth: state.spawn_module.stack_depth(),
            interactive: true,
            model: None,
        },
    );

    send_progress_message(state, format!("🔄 Spawning agent for task: {task}"));

    send_tool_completion(
        protocol,
        &tool_call_id,
        &tool_name,
        ToolExecutionResult::Other {
            result: json!({ "agent_type": agent_type, "task": task }),
        },
        true,
        None,
    );

    run_orchestration(state, OrchestrationStep::Task(initial_message)).await
}

/// One step of mechanical orchestration: an agent receiving its task, or a
/// child outcome awaiting the parent's decision.
pub enum OrchestrationStep {
    Task(String),
    Outcome(ChildOutcome),
}

/// Drive mechanical orchestration to quiescence: chain on_task delegations,
/// execute fan-outs, and cascade completions until an agent that actually
/// converses is on top of the stack (returns false) or a completion reaches
/// the root agent (returns true; the conversation should stop).
pub async fn run_orchestration(state: &mut ActorState, step: OrchestrationStep) -> bool {
    let mut step = step;
    loop {
        match step {
            OrchestrationStep::Task(task) => {
                let settings = state.settings.settings();
                let (agent_id, agent_type, phase_before) = current_agent(state, |a| {
                    (a.id.clone(), a.agent.name().to_string(), a.workflow.phase())
                });
                let action = current_agent_mut(state, |a| {
                    let agent = a.agent.clone();
                    agent.on_task(&mut a.workflow, &settings, &task)
                });
                // A plain conversational turn emits no orchestration events;
                // anything else announces this agent first so every event
                // references a known id.
                let phase_changed = current_agent(state, |a| a.workflow.phase()) != phase_before;
                if phase_changed || !matches!(action, TaskAction::Converse) {
                    ensure_current_agent_announced(state);
                }
                emit_phase_change(state, &agent_id, &agent_type, phase_before);

                match action {
                    TaskAction::Converse => return false,
                    TaskAction::Spawn(spec) => {
                        send_progress_message(
                            state,
                            format!(
                                "🔄 {agent_type} → spawning {} for task: {}",
                                spec.agent, spec.task
                            ),
                        );
                        let next_task = spec.task.clone();
                        if !push_from_spec(state, spec, None) {
                            return false;
                        }
                        step = OrchestrationStep::Task(next_task);
                    }
                    TaskAction::FanOut(spec) => {
                        let parent_conversation = current_agent(state, |a| a.conversation.clone());
                        let outcome = run_fanout(state, spec, &parent_conversation).await;
                        step = OrchestrationStep::Outcome(outcome);
                    }
                }
            }
            OrchestrationStep::Outcome(outcome) => {
                let settings = state.settings.settings();
                let (agent_id, agent_type, phase_before) = current_agent(state, |a| {
                    (a.id.clone(), a.agent.name().to_string(), a.workflow.phase())
                });
                let mut hook_events: Vec<OrchestrationPayload> = Vec::new();
                let action = current_agent_mut(state, |a| {
                    let agent = a.agent.clone();
                    agent.on_child_complete(&mut a.workflow, &settings, &outcome, &mut hook_events)
                });
                for payload in hook_events {
                    send_orchestration(state, &agent_id, &agent_type, payload);
                }
                emit_phase_change(state, &agent_id, &agent_type, phase_before);

                match action {
                    ChildAction::Resume { message } => {
                        current_agent_mut(state, |a| {
                            a.conversation.push(Message {
                                role: MessageRole::User,
                                content: Content::text_only(message),
                            })
                        });

                        let result_message = if outcome.success {
                            format!("✅ Sub-agent completed successfully:\n{}", outcome.result)
                        } else {
                            format!("❌ Sub-agent failed:\n{}", outcome.result)
                        };
                        send_progress_message(state, result_message);
                        return false;
                    }
                    ChildAction::Spawn(spec) => {
                        send_progress_message(
                            state,
                            format!("🔄 Spawning {} for task: {}", spec.agent, spec.task),
                        );
                        let next_task = spec.task.clone();
                        if !push_from_spec(state, spec, Some(&outcome.conversation)) {
                            return false;
                        }
                        step = OrchestrationStep::Task(next_task);
                    }
                    ChildAction::FanOut(spec) => {
                        let next = run_fanout(state, spec, &outcome.conversation).await;
                        step = OrchestrationStep::Outcome(next);
                    }
                    ChildAction::Complete {
                        success: cascaded_success,
                        result: cascaded_result,
                    } => {
                        if state.spawn_module.stack_depth() <= 1 {
                            state.event_sender.send_message(ChatMessage::system(format!(
                                "Task completed [success={cascaded_success}]: {cascaded_result}"
                            )));
                            return true;
                        }

                        run_on_agent_popped_hooks(state);
                        let popped = state
                            .spawn_module
                            .pop_agent()
                            .expect("stack depth checked above");
                        send_orchestration(
                            state,
                            &popped.id,
                            popped.agent.name(),
                            OrchestrationPayload::AgentCompleted {
                                status: OutcomeStatus::from(cascaded_success),
                                result: cascaded_result.clone(),
                            },
                        );
                        step = OrchestrationStep::Outcome(ChildOutcome {
                            agent_name: popped.agent.name().to_string(),
                            success: cascaded_success,
                            result: cascaded_result,
                            conversation: popped.conversation,
                            reports: Vec::new(),
                        });
                    }
                }
            }
        }
    }
}

/// Push an agent described by an orchestration SpawnSpec. Returns false when
/// the agent type is unknown (reported to the user, stack unchanged).
fn push_from_spec(
    state: &mut ActorState,
    spec: SpawnSpec,
    forked_child: Option<&[Message]>,
) -> bool {
    let Some(agent) = state.spawn_module.catalog().create_agent(&spec.agent) else {
        state.event_sender.send_message(ChatMessage::system(format!(
            "Orchestration error: agent type '{}' not found in catalog",
            spec.agent
        )));
        return false;
    };

    ensure_current_agent_announced(state);
    let parent_agent_id = current_agent(state, |a| a.id.clone());
    let mut new_agent = ActiveAgent::new(agent);
    new_agent.announced = true;
    let new_agent_id = new_agent.id.clone();
    match spec.seed {
        ConversationSeed::Fresh => {}
        ConversationSeed::ForkSelf => {
            if let Some(conv) = state
                .spawn_module
                .with_current_agent(|a| a.conversation.clone())
            {
                new_agent.conversation = conv;
            }
        }
        ConversationSeed::ForkChild => {
            if let Some(conv) = forked_child {
                new_agent.conversation = conv.to_vec();
            }
        }
    }

    if let Some(orientation) = &spec.orientation {
        new_agent.conversation.push(Message {
            role: MessageRole::User,
            content: Content::text_only(orientation.clone()),
        });
    }
    new_agent.conversation.push(Message {
        role: MessageRole::User,
        content: Content::text_only(spec.task.clone()),
    });

    if let Some(model) = spec.model {
        let settings = state.settings.settings();
        new_agent.model_override = Some(pinned_model_settings(model, &settings));
    }

    state.spawn_module.push_agent(new_agent);
    run_on_agent_pushed_hooks(state, &spec.task, &spec.agent, &HashMap::new());
    send_orchestration(
        state,
        &new_agent_id,
        &spec.agent,
        OrchestrationPayload::AgentStarted {
            parent_agent_id: Some(parent_agent_id),
            task_preview: task_preview(&spec.task),
            origin: AgentOrigin::Workflow,
            depth: state.spawn_module.stack_depth(),
            interactive: true,
            model: spec.model,
        },
    );
    true
}

/// Apply an agent's complete_task through the orchestration hooks, cascading
/// completions up the stack. Returns true when a completion reached the root
/// agent and the conversation should stop.
async fn execute_pop_agent(
    state: &mut ActorState,
    protocol: &mut TurnProtocol,
    success: bool,
    result: String,
    tool_call_id: String,
    tool_name: String,
) -> bool {
    info!("Popping agent: success={}, result={}", success, result);

    // Don't pop if we're at the root agent
    if state.spawn_module.stack_depth() <= 1 {
        send_tool_completion(
            protocol,
            &tool_call_id,
            &tool_name,
            ToolExecutionResult::Other {
                result: json!(result),
            },
            true,
            None,
        );

        state.event_sender.send_message(ChatMessage::system(format!(
            "Task completed [success={success}]: {result}"
        )));
        return false;
    }

    let settings = state.settings.settings();
    let (agent_id, agent_type, phase_before) = current_agent(state, |a| {
        (a.id.clone(), a.agent.name().to_string(), a.workflow.phase())
    });
    let action = current_agent_mut(state, |a| {
        let agent = a.agent.clone();
        agent.on_complete(&mut a.workflow, &settings, success, &result)
    });
    emit_phase_change(state, &agent_id, &agent_type, phase_before);

    send_tool_completion(
        protocol,
        &tool_call_id,
        &tool_name,
        ToolExecutionResult::Other {
            result: json!(result),
        },
        success,
        None,
    );

    if let CompletionAction::Spawn(spec) = action {
        info!(
            "Intercepting {agent_type} completion to spawn {}",
            spec.agent
        );
        send_progress_message(
            state,
            format!(
                "🔍 Spawning {} to validate {agent_type} completion",
                spec.agent
            ),
        );
        let next_task = spec.task.clone();
        if !push_from_spec(state, spec, None) {
            return false;
        }
        return run_orchestration(state, OrchestrationStep::Task(next_task)).await;
    }

    // Pop the completing agent and cascade through parent hooks.
    run_on_agent_popped_hooks(state);
    let popped = state
        .spawn_module
        .pop_agent()
        .expect("stack depth checked above");
    send_orchestration(
        state,
        &popped.id,
        popped.agent.name(),
        OrchestrationPayload::AgentCompleted {
            status: OutcomeStatus::from(success),
            result: result.clone(),
        },
    );
    let outcome = ChildOutcome {
        agent_name: popped.agent.name().to_string(),
        success,
        result,
        conversation: popped.conversation,
        reports: Vec::new(),
    };

    run_orchestration(state, OrchestrationStep::Outcome(outcome)).await
}

const PAIR_REVIEW_ORIENTATION: &str = "\
    --- AGENT TRANSITION ---\n\
    You are a code review agent. The conversation above is from a worker agent \
    that implemented one assignment of a larger plan. Review only the changes \
    the worker made to its assigned file. Do not run builds or tests: the file \
    may depend on sibling assignments that are still in progress. \
    When done, use complete_task to return your verdict.";

/// Execute fan-out workers concurrently off-stack, pairing each with a
/// reviewer when requested, and join the results into a synthetic
/// [`FANOUT_AGENT`] outcome for the orchestrating agent.
async fn run_fanout(
    state: &mut ActorState,
    spec: FanOutSpec,
    parent_conversation: &[Message],
) -> ChildOutcome {
    let settings = state.settings.settings();
    let cap = settings.fanout_concurrency.max(1);
    let max_rounds = settings.max_review_rounds.max(1);
    let total = spec.workers.len();

    ensure_current_agent_announced(state);
    let (orchestrator_id, orchestrator_type) =
        current_agent(state, |a| (a.id.clone(), a.agent.name().to_string()));
    let fanout_id = next_orchestration_id();
    let worker_ids: Vec<AgentId> = spec
        .workers
        .iter()
        .map(|_| next_orchestration_id())
        .collect();
    let worker_infos: Vec<WorkerInfo> = spec
        .workers
        .iter()
        .zip(&worker_ids)
        .map(|(worker, worker_id)| WorkerInfo {
            worker_id: worker_id.clone(),
            label: worker.label.clone(),
            agent_type: worker.agent.clone(),
            model: worker.model,
            reviewed: worker.reviewed,
            task_preview: task_preview(&worker.task),
        })
        .collect();
    send_orchestration(
        state,
        &orchestrator_id,
        &orchestrator_type,
        OrchestrationPayload::FanOutStarted {
            fanout_id: fanout_id.clone(),
            total,
            concurrency: cap,
            workers: worker_infos,
        },
    );

    send_progress_message(
        state,
        format!("⚡ Fan-out: launching {total} worker(s), concurrency {cap}"),
    );

    let provider = state.provider.read().unwrap().clone();
    let supported_models = provider.supported_models();
    let catalog = state.spawn_module.catalog().clone();
    let semaphore = Arc::new(tokio::sync::Semaphore::new(cap));
    let event_sender = state.event_sender.clone();

    let futures: FuturesUnordered<_> = spec
        .workers
        .into_iter()
        .enumerate()
        .map(|(index, worker)| {
            let runner = AgentRunner::new(
                provider.clone(),
                state.settings.clone(),
                state.modules.clone(),
                state.steering.clone(),
                state.prompt_builder.clone(),
                state.context_builder.clone(),
                Arc::new(AgentCatalog::new()),
            );
            let catalog = catalog.clone();
            let semaphore = semaphore.clone();
            let event_sender = event_sender.clone();
            let orchestrator_id = orchestrator_id.clone();
            let orchestrator_type = orchestrator_type.clone();
            let fanout_id = fanout_id.clone();
            let worker_id = worker_ids[index].clone();
            let base_conversation: Vec<Message> = match worker.seed {
                ConversationSeed::ForkChild | ConversationSeed::ForkSelf => {
                    parent_conversation.to_vec()
                }
                ConversationSeed::Fresh => Vec::new(),
            };
            let model_override = worker
                .model
                .map(|model| pinned_model_settings(model, &settings));
            let unsupported_model = worker
                .model
                .filter(|model| !supported_models.contains(model));

            async move {
                let send_worker_started = |label: String| {
                    event_sender.send(ChatEvent::Orchestration(OrchestrationEvent {
                        agent_id: orchestrator_id.clone(),
                        agent_type: orchestrator_type.clone(),
                        payload: OrchestrationPayload::WorkerStarted {
                            fanout_id: fanout_id.clone(),
                            worker_id: worker_id.clone(),
                            label,
                        },
                    }));
                };

                // Every worker emits WorkerStarted before its WorkerCompleted,
                // including preflight failures that never acquire a slot.
                if let Some(model) = unsupported_model {
                    send_worker_started(worker.label.clone());
                    return (
                        index,
                        WorkerResult {
                            label: worker.label,
                            success: false,
                            summary: format!(
                                "model '{}' is not available on the active provider",
                                model.name()
                            ),
                        },
                    );
                }

                let _permit = semaphore
                    .acquire()
                    .await
                    .expect("fan-out semaphore is never closed");
                send_worker_started(worker.label.clone());
                let report = run_impl_review_pair(
                    &runner,
                    &catalog,
                    worker,
                    base_conversation,
                    max_rounds,
                    model_override,
                )
                .await;
                (index, report)
            }
        })
        .collect();

    let mut futures = futures;
    let mut completed = 0usize;
    let mut reports: Vec<(usize, WorkerResult)> = Vec::with_capacity(total);
    while let Some((index, report)) = futures.next().await {
        completed += 1;
        send_orchestration(
            state,
            &orchestrator_id,
            &orchestrator_type,
            OrchestrationPayload::WorkerCompleted {
                fanout_id: fanout_id.clone(),
                worker_id: worker_ids[index].clone(),
                label: report.label.clone(),
                status: OutcomeStatus::from(report.success),
                summary: report.summary.clone(),
            },
        );
        let status = if report.success { "✅" } else { "❌" };
        send_progress_message(
            state,
            format!("{status} [{completed}/{total}] {}", report.label),
        );
        reports.push((index, report));
    }

    reports.sort_by_key(|(index, _)| *index);
    let reports: Vec<WorkerResult> = reports.into_iter().map(|(_, report)| report).collect();
    let all_ok = reports.iter().all(|report| report.success);
    send_orchestration(
        state,
        &orchestrator_id,
        &orchestrator_type,
        OrchestrationPayload::FanOutCompleted {
            fanout_id,
            status: OutcomeStatus::from(all_ok),
        },
    );
    let joined = reports
        .iter()
        .map(|report| {
            let status = if report.success { "ok" } else { "FAILED" };
            format!("### {} [{status}]\n{}", report.label, report.summary)
        })
        .collect::<Vec<_>>()
        .join("\n\n");

    ChildOutcome {
        agent_name: FANOUT_AGENT.to_string(),
        success: all_ok,
        result: joined,
        conversation: Vec::new(),
        reports,
    }
}

/// Run one worker to completion, then a reviewer forked from the worker's
/// conversation; rejected feedback loops back to the worker up to max_rounds.
/// A pinned model applies to both the worker and its pair reviewer.
async fn run_impl_review_pair(
    runner: &AgentRunner,
    catalog: &Arc<AgentCatalog>,
    worker: WorkerSpec,
    base_conversation: Vec<Message>,
    max_rounds: u32,
    model_override: Option<crate::ai::ModelSettings>,
) -> WorkerResult {
    let Some(agent) = catalog.create_agent(&worker.agent) else {
        return WorkerResult {
            label: worker.label,
            success: false,
            summary: format!("worker agent type '{}' not found in catalog", worker.agent),
        };
    };

    let mut conversation = base_conversation;
    if let Some(orientation) = &worker.orientation {
        conversation.push(Message::user(orientation.clone()));
    }
    conversation.push(Message::user(worker.task.clone()));

    let mut last_feedback = String::new();
    for _ in 0..max_rounds {
        let mut active = ActiveAgent::new(agent.clone());
        active.conversation = conversation;
        active.write_allowlist = worker.write_allowlist.clone();
        active.model_override = model_override.clone();

        let (worker_state, run_result) = runner.run_returning(active, 40).await;
        let impl_result = match run_result {
            Ok(result) => result,
            Err(error) => {
                return WorkerResult {
                    label: worker.label,
                    success: false,
                    summary: format!("implementation failed: {error:?}"),
                }
            }
        };

        if !worker.reviewed {
            return WorkerResult {
                label: worker.label,
                success: true,
                summary: impl_result,
            };
        }

        let mut review = ActiveAgent::new(Arc::new(CodeReviewAgent::new()));
        review.conversation = worker_state.conversation.clone();
        review.model_override = model_override.clone();
        review
            .conversation
            .push(Message::user(PAIR_REVIEW_ORIENTATION.to_string()));
        review.conversation.push(Message::user(format!(
            "Review the changes for this assignment: {}",
            worker.task
        )));

        match runner.run(review, 15).await {
            Ok(verdict) => {
                return WorkerResult {
                    label: worker.label,
                    success: true,
                    summary: format!("{impl_result}\nReview: {verdict}"),
                }
            }
            Err(error) => {
                last_feedback = error.to_string();
                conversation = worker_state.conversation;
                conversation.push(Message::user(format!(
                    "Code review feedback (address it, then complete_task again): {last_feedback}"
                )));
            }
        }
    }

    WorkerResult {
        label: worker.label,
        success: false,
        summary: format!("unresolved after {max_rounds} review round(s): {last_feedback}"),
    }
}
