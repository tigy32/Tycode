use crate::agents::agent::ActiveAgent;
use crate::agents::catalog::AgentCatalog;
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
use crate::tools::r#trait::{ToolCategory, ValidatedToolCall};
use crate::tools::registry::{resolve_file_modification_api, ToolRegistry};
use crate::tools::tasks::{TaskList, TaskListOp, TaskStatus};
use anyhow::{bail, Result};
use serde_json::json;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;
use tracing::{error, info, warn};

#[derive(Debug)]
pub struct ToolResults {
    pub results: Vec<ContentBlock>,
    pub continue_conversation: bool,
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

/// Filter tool calls to only those in the minimum category, returning both the filtered calls and error responses for dropped tools
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

                error_responses.push(handle_tool_error(state, tool_call, error_msg));
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
            invalid_tool_results.push(handle_tool_error(state, &tool_use, error));
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
    let mut continue_conversation = true;
    for (raw, parsed) in validated {
        let (content_block, should_continue) = handle_tool_call(state, parsed, &raw).await;
        if let Some(block) = content_block {
            results.push(block);
        }
        continue_conversation = continue_conversation && should_continue;
    }

    // Combine invalid tool error responses with valid tool execution results
    all_results.extend(invalid_tool_results);
    all_results.extend(results);

    Ok(ToolResults {
        results: all_results,
        continue_conversation,
    })
}

async fn handle_tool_call(
    state: &mut ActorState,
    tool_result: crate::tools::r#trait::ValidatedToolCall,
    tool_use: &ToolUseData,
) -> (Option<ContentBlock>, bool) {
    let result = match tool_result {
        ValidatedToolCall::NoOp {
            context_data,
            ui_data,
        } => handle_noop(state, tool_use, context_data, ui_data),
        ValidatedToolCall::Error(error) => {
            Ok((Some(handle_tool_error(state, tool_use, error)), true))
        }
        ValidatedToolCall::FileModification(modification) => {
            handle_file_modification(state, modification, tool_use).await
        }
        ValidatedToolCall::RunCommand {
            command,
            working_directory,
            timeout_seconds,
        } => handle_run_command(state, command, working_directory, timeout_seconds, tool_use).await,
        ValidatedToolCall::PushAgent { agent_type, task } => {
            handle_tool_push_agent(state, agent_type, task, tool_use.id.clone()).await;
            Ok((None, true))
        }
        ValidatedToolCall::PopAgent { success, result } => {
            handle_tool_pop_agent(state, success, result, tool_use).await
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
    };

    match result {
        Ok((content_block, should_continue)) => (content_block, should_continue),
        Err(e) => {
            let error_content = handle_tool_error(state, tool_use, format!("{:?}", e));
            (Some(error_content), true)
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
) -> ContentBlock {
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

    ContentBlock::ToolResult(result)
}

fn handle_noop(
    state: &mut ActorState,
    tool_use: &ToolUseData,
    context_data: serde_json::Value,
    ui_data: Option<serde_json::Value>,
) -> Result<(Option<ContentBlock>, bool)> {
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

    Ok((Some(ContentBlock::ToolResult(result)), true))
}

async fn handle_tool_push_agent(
    state: &mut ActorState,
    agent_type: String,
    task: String,
    tool_use_id: String,
) {
    info!(
        "Tool requesting agent push: type={}, task={}",
        agent_type, task
    );

    // Check if agent exists BEFORE adding any results
    let Some(agent) = AgentCatalog::create_agent(&agent_type) else {
        let error_msg = format!("Unknown agent type: {agent_type}");
        state
            .event_sender
            .add_message(ChatMessage::error(error_msg.clone()));

        let error_result = ContentBlock::ToolResult(ToolResultData {
            tool_use_id: tool_use_id.clone(),
            content: format!("Unknown agent: {agent_type:?}"),
            is_error: true,
        });
        current_agent_mut(state).conversation.push(Message {
            role: MessageRole::User,
            content: Content::from(vec![error_result]),
        });
        return;
    };

    // Store the tool_use_id in the current agent before pushing
    current_agent_mut(state).spawn_tool_use_id = Some(tool_use_id.clone());
    info!("Pushing new agent: type={}, task={}", agent_type, task);

    // Create initial message for the new agent
    let initial_message = task.clone();

    // Push the new agent onto the stack
    let mut new_agent = ActiveAgent::new(agent);
    new_agent.conversation.push(Message {
        role: MessageRole::User,
        content: Content::text_only(initial_message.clone()),
    });

    state.agent_stack.push(new_agent);

    // Notify user
    state.event_sender.add_message(ChatMessage::system(format!(
        "🔄 Spawning {agent_type} agent for task: {task}"
    )));
}

async fn handle_tool_pop_agent(
    state: &mut ActorState,
    success: bool,
    result: String,
    tool_use: &ToolUseData,
) -> Result<(Option<ContentBlock>, bool)> {
    info!("Popping agent: success={}, result={}", success, result);
    let event = ChatEvent::ToolRequest(ToolRequest {
        tool_call_id: tool_use.id.clone(),
        tool_name: tool_use.name.clone(),
        tool_type: ToolRequestType::Other { args: json!({}) },
    });
    let _ = state.event_sender.event_tx.send(event);

    // Don't pop if we're at the root agent
    if state.agent_stack.len() <= 1 {
        let result_content = serde_json::json!({
            "success": success,
            "result": result
        });

        let tool_result = ContentBlock::ToolResult(ToolResultData {
            tool_use_id: tool_use.id.clone(),
            content: result_content.to_string(),
            is_error: false,
        });

        let event = ChatEvent::ToolExecutionCompleted {
            tool_call_id: tool_use.id.clone(),
            tool_name: tool_use.name.clone(),
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
        return Ok((Some(tool_result), true));
    }

    // Create result content
    let result_content = serde_json::json!({
        "success": success,
        "result": result
    });

    // Get parent's tool_use_id BEFORE popping
    let parent_index = state.agent_stack.len() - 2;
    let Some(tool_id) = state.agent_stack[parent_index].spawn_tool_use_id.take() else {
        panic!("BUG: no tool_use_id set on parent agent")
    };

    // Add result to parent's conversation BEFORE popping
    state.agent_stack[parent_index].conversation.push(Message {
        role: MessageRole::User,
        content: Content::from(vec![ContentBlock::ToolResult(ToolResultData {
            tool_use_id: tool_id,
            content: result_content.to_string(),
            is_error: false,
        })]),
    });

    // Now safe to pop
    state.agent_stack.pop();

    // Add a user-friendly result message
    let result_message = if success {
        format!("✅ Sub-agent completed successfully:\n{result}")
    } else {
        format!("❌ Sub-agent failed:\n{result}")
    };

    let event = ChatEvent::ToolExecutionCompleted {
        tool_call_id: tool_use.id.clone(),
        tool_name: tool_use.name.clone(),
        tool_result: ToolExecutionResult::Other {
            result: serde_json::to_value(&result).unwrap(),
        },
        success,
        error: None,
    };
    let _ = state.event_sender.event_tx.send(event);

    // Notify user
    state
        .event_sender
        .add_message(ChatMessage::system(result_message));

    Ok((None, true))
}

async fn handle_file_modification(
    state: &mut ActorState,
    modification: crate::tools::r#trait::FileModification,
    tool_use: &ToolUseData,
) -> Result<(Option<ContentBlock>, bool)> {
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

    Ok((Some(ContentBlock::ToolResult(result)), true))
}

async fn handle_run_command(
    state: &mut ActorState,
    command: String,
    working_directory: std::path::PathBuf,
    timeout_seconds: u64,
    tool_use: &ToolUseData,
) -> Result<(Option<ContentBlock>, bool)> {
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

    Ok((Some(ContentBlock::ToolResult(result_data)), true))
}

fn handle_prompt_user(
    state: &mut ActorState,
    question: String,
    tool_use: &ToolUseData,
) -> Result<(Option<ContentBlock>, bool)> {
    let result = ToolResultData {
        tool_use_id: tool_use.id.clone(),
        content: json!({}).to_string(),
        is_error: false,
    };

    state.event_sender.add_message(ChatMessage::system(format!(
        "The agent has a question: {question}"
    )));

    Ok((Some(ContentBlock::ToolResult(result)), false))
}

fn handle_set_tracked_files(
    state: &mut ActorState,
    file_paths: Vec<String>,
    tool_use: &ToolUseData,
) -> Result<(Option<ContentBlock>, bool)> {
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

    Ok((Some(ContentBlock::ToolResult(result)), true))
}

fn handle_task_list_op(
    state: &mut ActorState,
    op: TaskListOp,
    tool_use: &ToolUseData,
) -> Result<(Option<ContentBlock>, bool)> {
    match op {
        TaskListOp::Create { title, tasks } => {
            let task_count = tasks.len();
            state.task_list = Some(TaskList::new(title, tasks));

            if let Some(task_list) = &state.task_list {
                let _ = state
                    .event_sender
                    .event_tx
                    .send(ChatEvent::TaskUpdate(task_list.clone()));
            }

            let content = json!({
                "action": "create_task_list",
                "task_count": task_count
            })
            .to_string();

            info!(tool_name = %tool_use.name, task_count, "Task list created");

            Ok((
                Some(ContentBlock::ToolResult(ToolResultData {
                    tool_use_id: tool_use.id.clone(),
                    content,
                    is_error: false,
                })),
                true,
            ))
        }
        TaskListOp::UpdateStatus { task_id, status } => {
            handle_update_task_status(state, task_id, status, tool_use)
        }
    }
}

fn handle_update_task_status(
    state: &mut ActorState,
    task_id: usize,
    status: TaskStatus,
    tool_use: &ToolUseData,
) -> Result<(Option<ContentBlock>, bool)> {
    let Some(task_list) = &mut state.task_list else {
        warn!(tool_name = %tool_use.name, "Attempted update without task list");
        return Ok((
            Some(ContentBlock::ToolResult(ToolResultData {
                tool_use_id: tool_use.id.clone(),
                content: "No task list created. Use propose_task_list first.".to_string(),
                is_error: true,
            })),
            true,
        ));
    };

    match task_list.update_task_status(task_id, status) {
        Ok(()) => {
            if let Some(task_list) = &state.task_list {
                let _ = state
                    .event_sender
                    .event_tx
                    .send(ChatEvent::TaskUpdate(task_list.clone()));
            }

            info!(tool_name = %tool_use.name, task_id, status = ?status, "Task status updated");

            let content = json!({
                "action": "update_task_status",
                "task_id": task_id,
                "status": status,
                "success": true
            })
            .to_string();
            Ok((
                Some(ContentBlock::ToolResult(ToolResultData {
                    tool_use_id: tool_use.id.clone(),
                    content,
                    is_error: false,
                })),
                true,
            ))
        }
        Err(e) => {
            warn!(tool_name = %tool_use.name, task_id, error = %e, "Failed to update task status");
            let error_msg = format!("Failed to update task status: {}", e);
            Ok((
                Some(ContentBlock::ToolResult(ToolResultData {
                    tool_use_id: tool_use.id.clone(),
                    content: error_msg,
                    is_error: true,
                })),
                true,
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
) -> Result<(Option<ContentBlock>, bool)> {
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

    Ok((Some(ContentBlock::ToolResult(result)), true))
}
