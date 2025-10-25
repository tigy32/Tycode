use crate::agents::agent::{ActiveAgent, Agent};
use crate::agents::catalog::AgentCatalog;
use crate::agents::code_review::CodeReviewAgent;
use crate::agents::tool_type::ToolType;
use crate::ai::model::Model;
use crate::ai::{Content, ContentBlock, Message, MessageRole, ToolResultData, ToolUseData};
use crate::chat::actor::ActorState;
use crate::chat::events::{
    ChatEvent, ChatMessage, ToolExecutionResult, ToolRequest, ToolRequestType,
};
use crate::cmd::run_cmd;
use crate::file::access::FileAccessManager;
use crate::file::manager::FileModificationManager;
use crate::security::evaluate;
use crate::settings::config::ReviewLevel;
use crate::tools::r#trait::{ToolCategory, ValidatedToolCall};
use crate::tools::registry::{resolve_file_modification_api, ToolRegistry};
use crate::tools::tasks::{TaskList, TaskListOp};
use anyhow::{bail, Result};
use serde_json::json;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;
use tracing::{error, info, warn};

#[derive(Debug, Clone, PartialEq, Eq)]
enum ContinuationPreference {
    Stop,
    Continue,
    NoPreference,
}

#[derive(Debug)]
pub struct ToolResults {
    pub continue_conversation: bool,
}

struct ToolCallResult {
    content_block: ContentBlock,
    deferred_action: Option<DeferredAction>,
    continuation_preference: ContinuationPreference,
}

impl ToolCallResult {
    fn immediate(content_block: ContentBlock, preference: ContinuationPreference) -> Self {
        Self {
            content_block,
            deferred_action: None,
            continuation_preference: preference,
        }
    }

    fn deferred(
        content_block: ContentBlock,
        deferred_action: DeferredAction,
        preference: ContinuationPreference,
    ) -> Self {
        Self {
            content_block,
            deferred_action: Some(deferred_action),
            continuation_preference: preference,
        }
    }
}

enum DeferredAction {
    PushAgent {
        agent: Box<dyn Agent>,
        task: String,
    },
    PopAgent {
        success: bool,
        result: String,
        tool_use_id: String,
    },
}

// Helper functions for ActorState
pub fn current_agent(state: &ActorState) -> &ActiveAgent {
    state.agent_stack.last().expect("No active agent")
}

pub fn current_agent_mut(state: &mut ActorState) -> &mut ActiveAgent {
    state.agent_stack.last_mut().expect("No active agent")
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

        if category == Some(ToolCategory::AlwaysAllowed) {
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
    info!(
        tool_count = tool_calls.len(),
        tools = ?tool_calls.iter().map(|t| &t.name).collect::<Vec<_>>(),
        "Executing tool calls"
    );

    // Get allowed tools for security checks
    let current = current_agent(state);
    let allowed_tools: HashSet<ToolType> = current.agent.available_tools().into_iter().collect();
    let allowed_tool_types: Vec<ToolType> = allowed_tools.into_iter().collect();

    let file_modification_api = state.settings.settings().file_modification_api;
    let resolved_api = resolve_file_modification_api(file_modification_api, model);
    let tool_registry = ToolRegistry::new(
        state.workspace_roots.clone(),
        resolved_api,
        state.mcp_manager.as_ref(),
    )
    .await?;

    // Filter tool calls by minimum category
    let (tool_calls, error_responses) =
        filter_tool_calls_by_minimum_category(state, tool_calls, &tool_registry);
    let mut all_results = error_responses;

    // Initialize preferences vector early to track all error and success preferences
    let mut preferences = vec![];

    let mut validated: Vec<(ToolUseData, ValidatedToolCall)> = vec![];
    let mut invalid_tool_results = vec![];
    for tool_use in tool_calls {
        let result = tool_registry
            .validate_tools(&tool_use, &allowed_tool_types)
            .await;

        if let ValidatedToolCall::Error(error) = result {
            warn!(
                tool_name = %tool_use.name,
                error = %error,
                "Tool call validation failed, will return error response"
            );
            if let Ok(error_result) = handle_tool_error(state, &tool_use, error) {
                invalid_tool_results.push(error_result.content_block);
                preferences.push(error_result.continuation_preference);
            }
        } else {
            validated.push((tool_use, result));
        }
    }

    // Only perform security evaluation on valid tool calls
    let validate_tool_calls = validated.iter().map(|(_, call)| call);
    if let Err(e) = evaluate(&state.settings, validate_tool_calls) {
        bail!("AI attempted to use tools not allowed by security settings: {e}")
    }

    let mut results = Vec::new();
    let mut deferred_actions = Vec::new();
    for (raw, parsed) in validated {
        match handle_tool_call(state, parsed, &raw).await {
            Ok(tool_result) => {
                results.push(tool_result.content_block);
                if let Some(action) = tool_result.deferred_action {
                    deferred_actions.push(action);
                }
                preferences.push(tool_result.continuation_preference);
            }
            Err(e) => {
                let error_result = handle_tool_error(state, &raw, format!("{:?}", e))?;
                results.push(error_result.content_block);
                preferences.push(error_result.continuation_preference);
            }
        }
    }

    // Implement truth table for continuation preferences:
    // - Any Stop → stop conversation
    // - Otherwise, any Continue → continue conversation
    // - All NoPreference → stop conversation
    let continue_conversation = if preferences
        .iter()
        .any(|p| *p == ContinuationPreference::Stop)
    {
        false
    } else if preferences
        .iter()
        .any(|p| *p == ContinuationPreference::Continue)
    {
        true
    } else {
        false
    };

    // Combine invalid tool error responses with valid tool execution results
    all_results.extend(invalid_tool_results);
    all_results.extend(results);

    // Add all tool results as a single message to satisfy Bedrock's expectations
    if !all_results.is_empty() {
        current_agent_mut(state).conversation.push(Message {
            role: MessageRole::User,
            content: Content::from(all_results),
        });
    }

    // Execute deferred actions after conversation update
    for action in deferred_actions {
        execute_deferred_action(state, action).await;
    }

    Ok(ToolResults {
        continue_conversation,
    })
}

async fn handle_tool_call(
    state: &mut ActorState,
    tool_result: crate::tools::r#trait::ValidatedToolCall,
    tool_use: &ToolUseData,
) -> Result<ToolCallResult> {
    match tool_result {
        ValidatedToolCall::NoOp {
            context_data,
            ui_data,
        } => handle_noop(state, tool_use, context_data, ui_data),
        ValidatedToolCall::Error(error) => handle_tool_error(state, tool_use, error),
        ValidatedToolCall::FileModification(modification) => {
            handle_file_modification(state, modification, tool_use).await
        }
        ValidatedToolCall::RunCommand {
            command,
            working_directory,
            timeout_seconds,
        } => handle_run_command(state, command, working_directory, timeout_seconds, tool_use).await,
        ValidatedToolCall::PushAgent { agent_type, task } => {
            handle_tool_push_agent_deferred(state, agent_type, task, tool_use.id.clone()).await
        }
        ValidatedToolCall::PopAgent { success, result } => {
            handle_tool_pop_agent_deferred(state, success, result, tool_use.id.clone()).await
        }
        ValidatedToolCall::PromptUser { question } => handle_prompt_user(state, question, tool_use),
        ValidatedToolCall::SetTrackedFiles { file_paths } => {
            handle_set_tracked_files(state, file_paths, tool_use)
        }
        ValidatedToolCall::McpCall {
            server_name,
            tool_name,
            arguments,
        } => handle_mcp_call(state, server_name, tool_name, arguments, tool_use).await,
        ValidatedToolCall::PerformTaskListOp(op) => handle_task_list_op(state, op, tool_use),
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

    if let Err(e) = state.event_sender.event_tx.send(event) {
        error!("Failed to send tool completion event: {:?}", e);
    }

    Ok(ToolCallResult::immediate(
        ContentBlock::ToolResult(result),
        ContinuationPreference::Continue,
    ))
}

fn handle_noop(
    state: &mut ActorState,
    tool_use: &ToolUseData,
    context_data: serde_json::Value,
    ui_data: Option<serde_json::Value>,
) -> Result<ToolCallResult> {
    let result = ToolResultData {
        tool_use_id: tool_use.id.clone(),
        content: context_data.to_string(),
        is_error: false,
    };

    info!(
        tool_name = %tool_use.name,
        ?result,
        ?ui_data,
        "Tool execution completed"
    );

    // Emit tool completion event
    let parsed_result = serde_json::from_str(&result.content).ok();
    let event = ChatEvent::ToolExecutionCompleted {
        tool_call_id: tool_use.id.clone(),
        tool_name: tool_use.name.clone(),
        tool_result: ToolExecutionResult::Other {
            result: parsed_result.clone().unwrap_or_default(),
        },
        success: true,
        error: None,
    };

    state
        .event_sender
        .event_tx
        .send(event)
        .map_err(|e| anyhow::anyhow!("Failed to send tool completion event: {:?}", e))?;

    Ok(ToolCallResult::immediate(
        ContentBlock::ToolResult(result),
        ContinuationPreference::Continue,
    ))
}

async fn handle_tool_push_agent_deferred(
    state: &mut ActorState,
    agent_type: String,
    task: String,
    tool_use_id: String,
) -> Result<ToolCallResult> {
    info!(
        "Tool requesting agent push: type={}, task={}",
        agent_type, task
    );

    // Check if agent exists
    let Some(agent) = AgentCatalog::create_agent(&agent_type) else {
        let error_msg = format!("Unknown agent type: {agent_type}");
        return handle_tool_error(
            state,
            &ToolUseData {
                id: tool_use_id,
                name: "spawn_agent".to_string(),
                arguments: serde_json::Value::Null,
            },
            error_msg,
        );
    };

    let acknowledgment = ContentBlock::ToolResult(ToolResultData {
        tool_use_id: tool_use_id.clone(),
        content: json!({
            "status": "spawned",
            "agent_type": agent_type,
            "task": task
        })
        .to_string(),
        is_error: false,
    });

    Ok(ToolCallResult::deferred(
        acknowledgment,
        DeferredAction::PushAgent { agent, task },
        ContinuationPreference::Continue,
    ))
}

async fn execute_deferred_action(state: &mut ActorState, action: DeferredAction) {
    match action {
        DeferredAction::PushAgent { agent, task } => {
            execute_push_agent(state, agent, task).await;
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

async fn execute_push_agent(state: &mut ActorState, agent: Box<dyn Agent>, task: String) {
    info!("Pushing new agent: task={}", task);

    if state.settings.settings().review_level == ReviewLevel::Task {
        let review_agent = Box::new(CodeReviewAgent);
        let review_task = format!("Review the code changes for the following task: {}", task);

        let mut review_active = ActiveAgent::new(review_agent);
        review_active.conversation.push(Message {
            role: MessageRole::User,
            content: Content::text_only(review_task.clone()),
        });

        state.agent_stack.push(review_active);

        state.event_sender.add_message(ChatMessage::system(format!(
            "🔄 Spawning review agent for task: {}",
            task
        )));
    }

    let initial_message = task.clone();

    let mut new_agent = ActiveAgent::new(agent);
    new_agent.conversation.push(Message {
        role: MessageRole::User,
        content: Content::text_only(initial_message.clone()),
    });

    state.agent_stack.push(new_agent);

    state.event_sender.add_message(ChatMessage::system(format!(
        "🔄 Spawning agent for task: {task}"
    )));
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
    let _ = state.event_sender.event_tx.send(event);

    // Don't pop if we're at the root agent
    if state.agent_stack.len() <= 1 {
        let event = ChatEvent::ToolExecutionCompleted {
            tool_call_id: tool_use_id,
            tool_name: "complete_task".to_string(),
            tool_result: ToolExecutionResult::Other {
                result: serde_json::to_value(&result).unwrap(),
            },
            success: true,
            error: None,
        };
        let _ = state.event_sender.event_tx.send(event);

        state.event_sender.add_message(ChatMessage::system(format!(
            "Task completed [success={success}]: {result}"
        )));
        return;
    }

    // Pop the completed agent
    state.agent_stack.pop();

    current_agent_mut(state).conversation.push(Message {
        role: MessageRole::User,
        content: Content::text_only(format!(
            "Sub-agent completed [success={}]: {}",
            success, result
        )),
    });

    // Add a user-friendly result message
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
    let _ = state.event_sender.event_tx.send(event);

    state
        .event_sender
        .add_message(ChatMessage::system(result_message));
}

async fn handle_tool_pop_agent_deferred(
    state: &mut ActorState,
    success: bool,
    result: String,
    tool_use_id: String,
) -> Result<ToolCallResult> {
    info!(
        "Tool requesting agent pop: success={}, result={}",
        success, result
    );

    // Why: Propagate stop preference if this is the root agent to halt conversation after completion.
    let is_root = state.agent_stack.len() <= 1;
    let preference = if is_root {
        ContinuationPreference::Stop
    } else {
        ContinuationPreference::Continue
    };

    // Return immediate acknowledgment ToolResult
    let acknowledgment = ContentBlock::ToolResult(ToolResultData {
        tool_use_id: tool_use_id.clone(),
        content: json!({
            "status": "completing",
            "success": success,
            "result": result
        })
        .to_string(),
        is_error: false,
    });

    // Return deferred action for actual popping
    Ok(ToolCallResult::deferred(
        acknowledgment,
        DeferredAction::PopAgent {
            success,
            result,
            tool_use_id,
        },
        preference,
    ))
}

async fn handle_file_modification(
    state: &mut ActorState,
    modification: crate::tools::r#trait::FileModification,
    tool_use: &ToolUseData,
) -> Result<ToolCallResult> {
    let file_manager = FileAccessManager::new(state.workspace_roots.clone());
    let file_modification_manager = FileModificationManager::new(file_manager);

    // Send tool request event
    state
        .event_sender
        .event_tx
        .send(ChatEvent::ToolRequest(ToolRequest {
            tool_call_id: tool_use.id.clone(),
            tool_name: tool_use.name.clone(),
            tool_type: ToolRequestType::ModifyFile {
                file_path: modification.path.to_string_lossy().to_string(),
                before: modification.original_content.clone().unwrap_or_default(),
                after: modification.new_content.clone().unwrap_or_default(),
            },
        }))
        .map_err(|e| anyhow::anyhow!("Failed to send tool request event: {:?}", e))?;

    // Apply the modification and get statistics
    let stats = file_modification_manager
        .apply_modification(modification.clone())
        .await
        .map_err(|e| anyhow::anyhow!("File modification failed: {:?}", e))?;

    // Create context data for the result
    let context_data = json!({
        "success": true,
        "path": modification.path,
        "operation": match modification.operation {
            crate::tools::r#trait::FileOperation::Create => "create",
            crate::tools::r#trait::FileOperation::Update => "update",
            crate::tools::r#trait::FileOperation::Delete => "delete",
        },
        "lines_added": stats.lines_added,
        "lines_removed": stats.lines_removed,
        "warning": modification.warning,
    });

    // Create the ToolResultData for the content block
    let result = ToolResultData {
        tool_use_id: tool_use.id.clone(),
        content: context_data.to_string(),
        is_error: false,
    };

    info!(
        tool_name = %tool_use.name,
        ?result,
        "Tool execution completed"
    );

    // Create the strongly typed result using the actual statistics from the modification
    let tool_result = ToolExecutionResult::ModifyFile {
        lines_added: stats.lines_added,
        lines_removed: stats.lines_removed,
    };

    // Send tool completion event with the specific result type
    let event = ChatEvent::ToolExecutionCompleted {
        tool_call_id: tool_use.id.clone(),
        tool_name: tool_use.name.clone(),
        tool_result,
        success: true,
        error: None,
    };

    state
        .event_sender
        .event_tx
        .send(event)
        .map_err(|e| anyhow::anyhow!("Failed to send tool completion event: {:?}", e))?;

    Ok(ToolCallResult::immediate(
        ContentBlock::ToolResult(result),
        ContinuationPreference::Continue,
    ))
}

async fn handle_run_command(
    state: &mut ActorState,
    command: String,
    working_directory: std::path::PathBuf,
    timeout_seconds: u64,
    tool_use: &ToolUseData,
) -> Result<ToolCallResult> {
    // Send tool request event
    state
        .event_sender
        .event_tx
        .send(ChatEvent::ToolRequest(ToolRequest {
            tool_call_id: tool_use.id.clone(),
            tool_name: tool_use.name.clone(),
            tool_type: ToolRequestType::RunCommand {
                command: command.clone(),
                working_directory: working_directory.to_string_lossy().to_string(),
            },
        }))
        .map_err(|e| anyhow::anyhow!("Failed to send tool request event: {:?}", e))?;

    let timeout = Duration::from_secs(timeout_seconds);
    let output = run_cmd(working_directory, command, timeout)
        .await
        .map_err(|e| anyhow::anyhow!("Command execution failed: {:?}", e))?;
    let context_data = serde_json::to_value(&output).unwrap_or_else(|_| {
        json!({
            "code": output.code,
            "out": output.out,
            "err": output.err
        })
    });

    // Create the ToolResultData for the content block
    let result_data = ToolResultData {
        tool_use_id: tool_use.id.clone(),
        content: context_data.to_string(),
        is_error: false,
    };

    info!(
        tool_name = %tool_use.name,
        ?result_data,
        "Tool execution completed"
    );

    let tool_result = ToolExecutionResult::RunCommand {
        exit_code: output.code,
        stdout: output.out,
        stderr: output.err,
    };

    let event = ChatEvent::ToolExecutionCompleted {
        tool_call_id: tool_use.id.clone(),
        tool_name: tool_use.name.clone(),
        tool_result,
        success: true,
        error: None,
    };

    state
        .event_sender
        .event_tx
        .send(event)
        .map_err(|e| anyhow::anyhow!("Failed to send tool completion event: {:?}", e))?;

    Ok(ToolCallResult::immediate(
        ContentBlock::ToolResult(result_data),
        ContinuationPreference::Continue,
    ))
}

fn handle_prompt_user(
    state: &mut ActorState,
    question: String,
    tool_use: &ToolUseData,
) -> Result<ToolCallResult> {
    let result = ToolResultData {
        tool_use_id: tool_use.id.clone(),
        content: json!({}).to_string(),
        is_error: false,
    };

    state.event_sender.add_message(ChatMessage::system(format!(
        "The agent has a question: {question}"
    )));

    Ok(ToolCallResult::immediate(
        ContentBlock::ToolResult(result),
        ContinuationPreference::Stop,
    ))
}

fn handle_set_tracked_files(
    state: &mut ActorState,
    file_paths: Vec<String>,
    tool_use: &ToolUseData,
) -> Result<ToolCallResult> {
    let file_manager = FileAccessManager::new(state.workspace_roots.clone());

    state
        .event_sender
        .event_tx
        .send(ChatEvent::ToolRequest(ToolRequest {
            tool_call_id: tool_use.id.clone(),
            tool_name: tool_use.name.clone(),
            tool_type: ToolRequestType::ReadFiles {
                file_paths: file_paths.clone(),
            },
        }))
        .map_err(|e| anyhow::anyhow!("Failed to send tool request event: {e:?}"))?;

    state.tracked_files.clear();
    for file_path in &file_paths {
        state.tracked_files.insert(PathBuf::from(file_path));
    }
    info!("Updated tracked files: {:?}", state.tracked_files);

    let context_data = json!({
        "action": "set_tracked_files",
        "tracked_files": state.tracked_files.iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect::<Vec<_>>()
    });

    let mut files = Vec::new();
    for file_path in file_paths {
        let path = file_manager.resolve(&file_path)?;
        let size = std::fs::metadata(&path)
            .ok()
            .map(|metadata| metadata.len() as u64)
            .unwrap_or(0);

        files.push(crate::chat::events::FileInfo {
            path: file_path,
            bytes: size as usize,
        });
    }

    let tool_result = ToolExecutionResult::ReadFiles { files };
    let result = ToolResultData {
        tool_use_id: tool_use.id.clone(),
        content: context_data.to_string(),
        is_error: false,
    };

    info!(
        tool_name = %tool_use.name,
        ?result,
        "Tool execution completed"
    );

    let event = ChatEvent::ToolExecutionCompleted {
        tool_call_id: tool_use.id.clone(),
        tool_name: tool_use.name.clone(),
        tool_result,
        success: true,
        error: None,
    };

    state
        .event_sender
        .event_tx
        .send(event)
        .map_err(|e| anyhow::anyhow!("Failed to send tool completion event: {e:?}"))?;

    Ok(ToolCallResult::immediate(
        ContentBlock::ToolResult(result),
        ContinuationPreference::Continue,
    ))
}

fn handle_task_list_op(
    state: &mut ActorState,
    op: TaskListOp,
    tool_use: &ToolUseData,
) -> Result<ToolCallResult> {
    match op {
        TaskListOp::Replace { title, tasks } => {
            let task_count = tasks.len();
            state.task_list = Some(TaskList::from_tasks_with_status(title, tasks));

            if let Some(task_list) = &state.task_list {
                let _ = state
                    .event_sender
                    .event_tx
                    .send(ChatEvent::TaskUpdate(task_list.clone()));
            }

            let content = json!({
                "action": "replace_task_list",
                "task_count": task_count
            })
            .to_string();

            info!(tool_name = %tool_use.name, task_count, "Task list replaced");

            Ok(ToolCallResult::immediate(
                ContentBlock::ToolResult(ToolResultData {
                    tool_use_id: tool_use.id.clone(),
                    content,
                    is_error: false,
                }),
                ContinuationPreference::NoPreference,
            ))
        }
    }
}

async fn handle_mcp_call(
    state: &mut ActorState,
    server_name: String,
    tool_name: String,
    arguments: Option<serde_json::Value>,
    tool_use: &ToolUseData,
) -> Result<ToolCallResult> {
    state
        .event_sender
        .event_tx
        .send(ChatEvent::ToolRequest(ToolRequest {
            tool_call_id: tool_use.id.clone(),
            tool_name: tool_use.name.clone(),
            tool_type: ToolRequestType::Other {
                args: arguments.clone().unwrap_or(serde_json::Value::Null),
            },
        }))
        .map_err(|e| anyhow::anyhow!("Failed to send tool request event: {e:?}"))?;

    let mcp_manager = state
        .mcp_manager
        .as_mut()
        .ok_or(anyhow::anyhow!("MCP manager not initialized"))?;

    let output = mcp_manager
        .execute_tool(&server_name, &tool_name, arguments)
        .await?;

    let result = ToolResultData {
        tool_use_id: tool_use.id.clone(),
        content: output.clone(),
        is_error: false,
    };

    info!(
        tool_name = %tool_use.name,
        server_name = %server_name,
        "MCP tool execution completed"
    );

    let event = ChatEvent::ToolExecutionCompleted {
        tool_call_id: tool_use.id.clone(),
        tool_name: tool_use.name.clone(),
        tool_result: ToolExecutionResult::Other {
            result: serde_json::json!({ "output": output }),
        },
        success: true,
        error: None,
    };

    state
        .event_sender
        .event_tx
        .send(event)
        .map_err(|e| anyhow::anyhow!("Failed to send tool completion event: {e:?}"))?;

    Ok(ToolCallResult::immediate(
        ContentBlock::ToolResult(result),
        ContinuationPreference::Continue,
    ))
}
