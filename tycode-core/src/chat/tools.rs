use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use base64::engine::general_purpose;
use base64::Engine;
use serde_json::{json, Value};
use tracing::{info, warn};

use crate::agents::agent::{ActiveAgent, Agent};
use crate::agents::code_review::CodeReviewAgent;
use crate::agents::coder::CoderAgent;
use crate::ai::model::Model;
use crate::ai::types::ImageData;
use crate::ai::{Content, ContentBlock, Message, MessageRole, ToolResultData, ToolUseData};
use crate::chat::actor::ActorState;
use crate::chat::events::{ChatMessage, ToolExecutionResult, ToolRequest};
use crate::chat::protocol::TurnProtocol;
use crate::modules::execution::config::ExecutionConfig;
use crate::modules::execution::{compact_output, truncate_and_persist};
use crate::settings::config::{ReviewLevel, SpawnContextMode};

use crate::tools::r#trait::{ContinuationPreference, ToolCallHandle, ToolCategory, ToolOutput};
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

/// Find the minimum category from a list of tool calls
fn find_minimum_category(
    tool_calls: &[ToolUseData],
    tool_registry: &ToolRegistry,
) -> Option<ToolCategory> {
    tool_calls
        .iter()
        .filter_map(|tool_call| {
            // Look up the tool executor and get its category
            tool_registry
                .get_tool_executor_by_name(&tool_call.name)
                .map(|executor| executor.category())
        })
        .min()
}

fn filter_tool_calls_by_minimum_category(
    state: &mut ActorState,
    protocol: &mut TurnProtocol,
    tool_calls: Vec<ToolUseData>,
    tool_registry: &ToolRegistry,
) -> (Vec<ToolUseData>, Vec<ContentBlock>) {
    // Separate AlwaysAllowed tools from other tool calls
    let mut always_allowed_calls = Vec::new();
    let mut other_calls = Vec::new();

    for tool_call in tool_calls {
        let category = tool_registry
            .get_tool_executor_by_name(&tool_call.name)
            .map(|executor| executor.category());

        if category == Some(ToolCategory::TaskList) {
            always_allowed_calls.push(tool_call);
        } else {
            other_calls.push(tool_call);
        }
    }

    // If there are no other calls, just return the AlwaysAllowed ones
    if other_calls.is_empty() {
        return (always_allowed_calls, vec![]);
    }

    // Find minimum category among non-AlwaysAllowed tools
    let minimum_category = match find_minimum_category(&other_calls, tool_registry) {
        Some(cat) => cat,
        None => {
            // If we can't find a minimum, return all calls
            let mut all_calls = always_allowed_calls;
            all_calls.extend(other_calls);
            return (all_calls, vec![]);
        }
    };

    // Store the original calls before filtering
    let original_other_calls = other_calls.clone();

    let filtered_calls: Vec<ToolUseData> = other_calls
        .into_iter()
        .filter(|tool_call| {
            tool_registry
                .get_tool_executor_by_name(&tool_call.name)
                .map(|executor| executor.category() == minimum_category)
                .unwrap_or(false)
        })
        .collect();

    let mut error_responses = vec![];

    if filtered_calls.len() != original_other_calls.len() {
        let dropped_count = original_other_calls.len() - filtered_calls.len();
        let min_cat_clone = minimum_category.clone();
        warn!(
            "Filtered out {} tool calls from higher categories than {:?}",
            dropped_count, min_cat_clone
        );

        // Generate error responses for dropped calls using handle_tool_error
        for tool_call in original_other_calls.iter() {
            let category = tool_registry
                .get_tool_executor_by_name(&tool_call.name)
                .map(|executor| executor.category());

            if category != Some(min_cat_clone.clone()) {
                warn!(
                    tool_name = %tool_call.name,
                    category = ?category,
                    min_category = ?min_cat_clone,
                    "Dropping tool call due to higher priority category"
                );

                let error_msg = format!(
                    "Tool call '{}' from category {:?} was dropped because there are tool calls in a lower priority category ({:?}). Only the lowest priority category tools are executed.",
                    tool_call.name, category, min_cat_clone
                );

                let error_result = handle_tool_error(state, protocol, tool_call, error_msg);
                error_responses.push(error_result.content_block);
            }
        }
    }

    // Combine AlwaysAllowed tools with filtered tools
    let mut result = always_allowed_calls;
    result.extend(filtered_calls);

    (result, error_responses)
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

    // Filter tool calls by minimum category
    let (tool_calls, error_responses) =
        filter_tool_calls_by_minimum_category(state, protocol, tool_calls, &tool_registry);
    let mut all_results = error_responses;

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
    let continue_conversation = if preferences
        .iter()
        .any(|p| *p == ContinuationPreference::Stop)
    {
        false
    } else {
        preferences
            .iter()
            .any(|p| *p == ContinuationPreference::Continue)
    };

    // Combine invalid tool error responses with valid tool execution results
    all_results.extend(invalid_tool_results);
    all_results.extend(results);

    protocol.append_tool_results_to_conversation(all_results);

    // Execute deferred actions after conversation update
    for action in deferred_actions {
        execute_deferred_action(state, protocol, action).await;
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

async fn execute_deferred_action(
    state: &mut ActorState,
    protocol: &mut TurnProtocol,
    action: DeferredAction,
) {
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
            .await;
        }
        DeferredAction::PopAgent {
            success,
            result,
            tool_call_id,
            tool_name,
        } => {
            execute_pop_agent(state, protocol, success, result, tool_call_id, tool_name).await;
        }
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
) {
    info!("Pushing new agent: task={}", task);

    let initial_message = task.clone();

    let mut new_agent = ActiveAgent::new(agent);

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

    state.event_sender.send_message(ChatMessage::system(format!(
        "🔄 Spawning agent for task: {task}"
    )));

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
}

async fn execute_pop_agent(
    state: &mut ActorState,
    protocol: &mut TurnProtocol,
    success: bool,
    result: String,
    tool_call_id: String,
    tool_name: String,
) {
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
        return;
    }

    let current_agent_name = current_agent(state, |a| a.agent.name().to_string());
    let review_enabled = state.settings.settings().review_level == ReviewLevel::Task;

    if current_agent_name == CoderAgent::NAME && review_enabled && success {
        info!("Intercepting coder completion to spawn review agent");

        current_agent_mut(state, |a| a.completion_result = Some(result.clone()));

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

        let review_agent: Arc<dyn Agent> = Arc::new(CodeReviewAgent::new());
        let review_task = format!(
            "Review the code changes for the following completed task: {}",
            result
        );

        let mut review_active = ActiveAgent::new(review_agent);

        // Fork the coder's full conversation so the reviewer can see all modifications
        if let Some(coder_conv) = state
            .spawn_module
            .with_current_agent(|a| a.conversation.clone())
        {
            review_active.conversation = coder_conv;
        }

        let orientation = "\
            --- AGENT TRANSITION ---\n\
            You are a code review agent. The conversation above is from the parent coder agent. \
            Review all of the file modifications the parent coder agent made. \
            Evaluate correctness, style compliance, and whether the changes satisfy the task requirements. \
            When done, use complete_task to return your verdict to the parent.";
        review_active.conversation.push(Message {
            role: MessageRole::User,
            content: Content::text_only(orientation.to_string()),
        });

        review_active.conversation.push(Message {
            role: MessageRole::User,
            content: Content::text_only(review_task.clone()),
        });

        state.spawn_module.push_agent(review_active);
        let review_spawn_params = HashMap::new();
        run_on_agent_pushed_hooks(
            state,
            &review_task,
            CodeReviewAgent::NAME,
            &review_spawn_params,
        );

        state.event_sender.add_message(ChatMessage::system(
            "🔍 Spawning review agent to validate code changes".to_string(),
        ));
        return;
    }

    if current_agent_name == CodeReviewAgent::NAME {
        info!("Review agent completing: success={}", success);

        run_on_agent_popped_hooks(state);
        state.spawn_module.pop_agent();

        if success {
            info!("Review approved, popping coder agent");

            let coder_result = current_agent(state, |a| a.completion_result.clone())
                .expect("completion_result must be set before review agent spawns");

            run_on_agent_popped_hooks(state);
            state.spawn_module.pop_agent();

            current_agent_mut(state, |a| {
                a.conversation.push(Message {
                    role: MessageRole::User,
                    content: Content::text_only(format!(
                        "Code review feedback from the review agent: {}",
                        result
                    )),
                })
            });

            state.event_sender.add_message(ChatMessage::system(format!(
                "✅ Code review approved. Task completed: {}",
                coder_result
            )));
        } else {
            info!("Review rejected, sending feedback to coder");

            current_agent_mut(state, |a| {
                a.conversation.push(Message {
                    role: MessageRole::User,
                    content: Content::text_only(format!(
                        "Code review feedback from the review agent: {}",
                        result
                    )),
                })
            });

            state.event_sender.add_message(ChatMessage::system(format!(
                "❌ Code review rejected. Feedback sent to coder: {}",
                result
            )));
        }

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

        return;
    }

    run_on_agent_popped_hooks(state);
    state.spawn_module.pop_agent();

    current_agent_mut(state, |a| {
        a.conversation.push(Message {
            role: MessageRole::User,
            content: Content::text_only(format!(
                "Sub-agent completed [success={}]: {}",
                success, result
            )),
        })
    });

    let result_message = if success {
        format!("✅ Sub-agent completed successfully:\n{result}")
    } else {
        format!("❌ Sub-agent failed:\n{result}")
    };

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

    state
        .event_sender
        .send_message(ChatMessage::system(result_message));
}
