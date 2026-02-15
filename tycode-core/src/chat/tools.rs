use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::agents::agent::{ActiveAgent, Agent};
use crate::agents::code_review::CodeReviewAgent;
use crate::agents::coder::CoderAgent;
use crate::ai::model::Model;
use crate::ai::tweaks::resolve_from_settings;
use crate::ai::{Content, ContentBlock, Message, MessageRole, ToolResultData, ToolUseData};
use crate::chat::actor::ActorState;
use crate::chat::events::{ChatEvent, ChatMessage, ToolExecutionResult, ToolRequest};
use crate::file::resolver::Resolver;
use crate::modules::execution::config::ExecutionConfig;
use crate::modules::execution::{compact_output, truncate_and_persist};
use crate::settings::config::{ReviewLevel, SpawnContextMode, ToolCallStyle};
use crate::tools::r#trait::{
    ContinuationPreference, ToolCallHandle, ToolCategory, ToolExecutor, ToolOutput,
};
use crate::tools::registry::ToolRegistry;
use crate::tools::ToolName;
use anyhow::Result;
use serde_json::json;
use tracing::{info, warn};

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
        tool_use_id: String,
        agent_type: String,
    },
    PopAgent {
        success: bool,
        result: String,
        tool_use_id: String,
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

                if let Ok(error_result) = handle_tool_error(state, tool_call, error_msg) {
                    error_responses.push(error_result.content_block);
                }
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
    model: Model,
) -> Result<ToolResults> {
    state.transition_timing_state(crate::chat::actor::TimingState::ExecutingTools);

    let execution_config: ExecutionConfig = state.settings.get_module_config("execution");
    let max_output_bytes = execution_config.max_output_bytes.unwrap_or(200_000);
    let workspace_root = state
        .workspace_roots
        .first()
        .cloned()
        .unwrap_or_else(|| PathBuf::from("."));

    info!(
        tool_count = tool_calls.len(),
        tools = ?tool_calls.iter().map(|t| &t.name).collect::<Vec<_>>(),
        "Executing tool calls"
    );

    // Get allowed tools for security checks
    let allowed_tool_names: Vec<ToolName> = current_agent(state, |a| a.agent.available_tools());

    let module_tools: Vec<Arc<dyn ToolExecutor>> =
        state.modules.iter().flat_map(|m| m.tools()).collect();
    let all_tools: Vec<Arc<dyn ToolExecutor>> =
        state.tools.iter().cloned().chain(module_tools).collect();
    let tool_registry = ToolRegistry::new(all_tools);

    // Filter tool calls by minimum category
    let (tool_calls, error_responses) =
        filter_tool_calls_by_minimum_category(state, tool_calls, &tool_registry);
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
                if let Ok(error_result) = handle_tool_error(state, &tool_use, error) {
                    invalid_tool_results.push(error_result.content_block);
                    preferences.push(error_result.continuation_preference);
                }
            }
        }
    }

    let mut results = Vec::new();
    let mut deferred_actions = Vec::new();
    for (raw, handle) in validated {
        state
            .event_sender
            .send(ChatEvent::ToolRequest(handle.tool_request()));

        let output = handle.execute().await;

        match output {
            ToolOutput::Result {
                content,
                is_error,
                continuation,
                ui_result,
            } => {
                let resolver = Resolver::new(state.workspace_roots.clone())
                    .expect("workspace roots already validated");
                let content = truncate_tool_result(
                    content,
                    &raw.id,
                    max_output_bytes,
                    &workspace_root,
                    &resolver,
                )
                .await;

                let result = ToolResultData {
                    tool_use_id: raw.id.clone(),
                    content,
                    is_error,
                };

                let event = ChatEvent::ToolExecutionCompleted {
                    tool_call_id: raw.id.clone(),
                    tool_name: raw.name.clone(),
                    tool_result: ui_result,
                    success: !is_error,
                    error: None,
                };
                state.event_sender.send(event);

                results.push(ContentBlock::ToolResult(result));
                preferences.push(continuation);
            }
            ToolOutput::PushAgent { agent, task } => {
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
                results.push(acknowledgment);
                deferred_actions.push(DeferredAction::PushAgent {
                    agent,
                    task,
                    tool_use_id: raw.id.clone(),
                    agent_type,
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
                results.push(acknowledgment);
                deferred_actions.push(DeferredAction::PopAgent {
                    success,
                    result,
                    tool_use_id: raw.id.clone(),
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
                ));

                results.push(ContentBlock::ToolResult(result));
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

    // Add all tool results as a single message
    if !all_results.is_empty() {
        let settings_snapshot = state.settings.settings();
        let provider = state.provider.read().unwrap().clone();
        let resolved_tweaks = resolve_from_settings(&settings_snapshot, provider.as_ref(), model);

        // XML mode: Convert ToolResult blocks to XML text to avoid Bedrock's toolConfig requirement
        let content = if resolved_tweaks.tool_call_style == ToolCallStyle::Xml {
            let xml_results: Vec<ContentBlock> = all_results
                .into_iter()
                .map(convert_tool_result_to_xml)
                .collect();
            Content::from(xml_results)
        } else {
            Content::from(all_results)
        };

        current_agent_mut(state, |a| {
            a.conversation.push(Message {
                role: MessageRole::User,
                content,
            })
        });
    }

    // Execute deferred actions after conversation update
    for action in deferred_actions {
        execute_deferred_action(state, action).await;
    }

    state.transition_timing_state(crate::chat::actor::TimingState::Idle);

    if let Err(e) = state.save_session() {
        tracing::warn!("Failed to auto-save session after tool execution: {}", e);
    }

    Ok(ToolResults {
        continue_conversation,
    })
}

fn convert_tool_result_to_xml(block: ContentBlock) -> ContentBlock {
    let ContentBlock::ToolResult(result) = block else {
        return block;
    };
    let error_attr = if result.is_error {
        " is_error=\"true\""
    } else {
        ""
    };
    let xml = format!(
        "<tool_result tool_use_id=\"{}\"{}>{}</tool_result>",
        result.tool_use_id, error_attr, result.content
    );
    ContentBlock::Text(xml)
}

async fn truncate_tool_result(
    content: String,
    tool_call_id: &str,
    max_bytes: usize,
    workspace_root: &Path,
    resolver: &Resolver,
) -> String {
    if content.len() <= max_bytes {
        return content;
    }

    let vfs_workspace = resolver
        .canonicalize(workspace_root)
        .map(|r| r.virtual_path.display().to_string())
        .unwrap_or_else(|_| ".".to_string());
    let vfs_display_path = format!("{}/.tycode/tool-calls/{}", vfs_workspace, tool_call_id);

    match truncate_and_persist(
        &content,
        tool_call_id,
        max_bytes,
        workspace_root,
        &vfs_display_path,
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
    state: &mut ActorState,
    tool_use: &ToolUseData,
    error: String,
) -> Result<ToolCallResult> {
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

    let event = ChatEvent::ToolExecutionCompleted {
        tool_call_id: tool_use.id.clone(),
        tool_name: tool_use.name.clone(),
        tool_result: ToolExecutionResult::Error {
            short_message,
            detailed_message: error.clone(),
        },
        success: false,
        error: Some(error),
    };

    state.event_sender.send(event);

    Ok(ToolCallResult::immediate(
        ContentBlock::ToolResult(result),
        ContinuationPreference::Continue,
    ))
}

async fn execute_deferred_action(state: &mut ActorState, action: DeferredAction) {
    match action {
        DeferredAction::PushAgent {
            agent,
            task,
            tool_use_id,
            agent_type,
        } => {
            execute_push_agent(state, agent, task, tool_use_id, agent_type).await;
        }
        DeferredAction::PopAgent {
            success,
            result,
            tool_use_id,
        } => {
            execute_pop_agent(state, success, result, tool_use_id).await;
        }
    }
}

async fn execute_push_agent(
    state: &mut ActorState,
    agent: Arc<dyn Agent>,
    task: String,
    tool_use_id: String,
    agent_type: String,
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

    state.event_sender.send_message(ChatMessage::system(format!(
        "🔄 Spawning agent for task: {task}"
    )));

    let tool_name = match agent_type.as_str() {
        "coder" => "spawn_coder",
        "recon" => "spawn_recon",
        _ => "spawn_agent",
    };

    let event = ChatEvent::ToolExecutionCompleted {
        tool_call_id: tool_use_id,
        tool_name: tool_name.to_string(),
        tool_result: ToolExecutionResult::Other {
            result: json!({ "agent_type": agent_type, "task": task }),
        },
        success: true,
        error: None,
    };

    state.event_sender.send(event);
}

async fn execute_pop_agent(
    state: &mut ActorState,
    success: bool,
    result: String,
    tool_use_id: String,
) {
    info!("Popping agent: success={}, result={}", success, result);

    let event = ChatEvent::ToolRequest(ToolRequest {
        tool_call_id: tool_use_id.clone(),
        tool_name: "complete_task".to_string(),
        tool_type: ToolRequestType::Other { args: json!({}) },
    });
    state.event_sender.send(event);

    // Don't pop if we're at the root agent
    if state.spawn_module.stack_depth() <= 1 {
        let event = ChatEvent::ToolExecutionCompleted {
            tool_call_id: tool_use_id,
            tool_name: "complete_task".to_string(),
            tool_result: ToolExecutionResult::Other {
                result: serde_json::to_value(&result).unwrap(),
            },
            success: true,
            error: None,
        };
        state.event_sender.send(event);

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

        let event = ChatEvent::ToolExecutionCompleted {
            tool_call_id: tool_use_id,
            tool_name: "complete_task".to_string(),
            tool_result: ToolExecutionResult::Other {
                result: serde_json::to_value(&result).unwrap(),
            },
            success,
            error: None,
        };
        state.event_sender.send(event);

        let review_agent: Arc<dyn Agent> = Arc::new(CodeReviewAgent::new());
        let review_task = format!(
            "Review the code changes for the following completed task: {}",
            result
        );

        let mut review_active = ActiveAgent::new(review_agent);
        review_active.conversation.push(Message {
            role: MessageRole::User,
            content: Content::text_only(review_task.clone()),
        });

        state.spawn_module.push_agent(review_active);

        state.event_sender.add_message(ChatMessage::system(
            "🔍 Spawning review agent to validate code changes".to_string(),
        ));
        return;
    }

    if current_agent_name == CodeReviewAgent::NAME {
        info!("Review agent completing: success={}", success);

        state.spawn_module.pop_agent();

        if success {
            info!("Review approved, popping coder agent");

            let coder_result = current_agent(state, |a| a.completion_result.clone())
                .expect("completion_result must be set before review agent spawns");

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

        let event = ChatEvent::ToolExecutionCompleted {
            tool_call_id: tool_use_id,
            tool_name: "complete_task".to_string(),
            tool_result: ToolExecutionResult::Other {
                result: serde_json::to_value(&result).unwrap(),
            },
            success,
            error: None,
        };
        state.event_sender.send(event);

        return;
    }

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

    let event = ChatEvent::ToolExecutionCompleted {
        tool_call_id: tool_use_id,
        tool_name: "complete_task".to_string(),
        tool_result: ToolExecutionResult::Other {
            result: serde_json::to_value(&result).unwrap(),
        },
        success,
        error: None,
    };
    state.event_sender.send(event);

    state
        .event_sender
        .send_message(ChatMessage::system(result_message));
}
