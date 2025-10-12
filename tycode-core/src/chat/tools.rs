use crate::agents::agent::ActiveAgent;
use crate::agents::catalog::AgentCatalog;
use crate::agents::tool_type::ToolType;
use crate::ai::{Content, ContentBlock, Message, MessageRole, ToolResultData, ToolUseData};
use crate::chat::actor::ActorState;
use crate::chat::events::{
    ChatEvent, ChatMessage, ToolExecutionResult, ToolRequest, ToolRequestType,
};
use crate::cmd::run_cmd;
use crate::file::access::FileAccessManager;
use crate::file::manager::FileModificationManager;
use crate::security::evaluate;
use crate::tools::r#trait::ValidatedToolCall;
use crate::tools::registry::ToolRegistry;
use anyhow::{bail, Result};
use serde_json::json;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;
use tracing::{error, info};

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

pub async fn execute_tool_calls(
    state: &mut ActorState,
    tool_calls: Vec<ToolUseData>,
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

    let file_modification_api = state.config.file_modification_api.clone();
    let tool_registry = ToolRegistry::new(
        state.workspace_roots.clone(),
        file_modification_api,
        state.mcp_manager.as_ref(),
    )
    .await?;

    let mut validated: Vec<(ToolUseData, ValidatedToolCall)> = vec![];
    let mut errors = vec![];
    for tool_use in tool_calls {
        let result = tool_registry
            .validate_tools(&tool_use, &allowed_tool_types)
            .await;

        if let ValidatedToolCall::Error(e) = result {
            errors.push(e);
        } else {
            validated.push((tool_use, result));
        }
    }

    if !errors.is_empty() {
        bail!("AI made an invalid or malformed tool request: {errors:?}");
    }

    let validate_tool_calls = validated.iter().map(|(_, call)| call);
    if let Err(e) = evaluate(&state.settings, validate_tool_calls) {
        bail!("AI attempted to use tools not allowed by security settings: {e}")
    }

    let mut results = Vec::new();
    let mut continue_conversation = true;
    for (raw, parsed) in validated {
        let (content_block, should_continue) = handle_tool_call(state, parsed, &raw).await;
        results.push(content_block);
        continue_conversation = continue_conversation && should_continue;
    }

    Ok(ToolResults {
        results,
        continue_conversation,
    })
}

async fn handle_tool_call(
    state: &mut ActorState,
    tool_result: crate::tools::r#trait::ValidatedToolCall,
    tool_use: &ToolUseData,
) -> (ContentBlock, bool) {
    let result = match tool_result {
        ValidatedToolCall::NoOp {
            context_data,
            ui_data,
        } => handle_noop(state, tool_use, context_data, ui_data),
        ValidatedToolCall::Error(error) => Ok((handle_tool_error(state, tool_use, error), true)),
        ValidatedToolCall::FileModification(modification) => {
            handle_file_modification(state, modification, tool_use).await
        }
        ValidatedToolCall::RunCommand {
            command,
            working_directory,
            timeout_seconds,
        } => handle_run_command(state, command, working_directory, timeout_seconds, tool_use).await,
        ValidatedToolCall::PushAgent { agent_type, task } => {
            let content_block =
                handle_tool_push_agent(state, agent_type, task, tool_use.id.clone()).await;
            Ok((content_block, true))
        }
        ValidatedToolCall::PopAgent { success, result } => {
            Ok(handle_tool_pop_agent(state, success, result, tool_use.id.clone()).await)
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
    };

    match result {
        Ok((content_block, should_continue)) => (content_block, should_continue),
        Err(e) => {
            let error_content = handle_tool_error(state, tool_use, format!("{:?}", e));
            (error_content, true)
        }
    }
}

fn handle_tool_error(
    state: &mut ActorState,
    tool_use: &ToolUseData,
    error: String,
) -> ContentBlock {
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
            message: error.clone(),
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
) -> Result<(ContentBlock, bool)> {
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

    Ok((ContentBlock::ToolResult(result), true))
}

async fn handle_tool_push_agent(
    state: &mut ActorState,
    agent_type: String,
    task: String,
    tool_use_id: String,
) -> ContentBlock {
    info!(
        "Tool requesting agent push: type={}, task={}",
        agent_type, task
    );

    // Store the tool_use_id in the current agent before pushing
    current_agent_mut(state).spawn_tool_use_id = Some(tool_use_id.clone());
    info!("Pushing new agent: type={}, task={}", agent_type, task);

    let Some(agent) = AgentCatalog::create_agent(&agent_type) else {
        // On error, return error tool result
        let error_msg = format!("Unknown agent type: {agent_type}");
        state
            .event_sender
            .add_message(ChatMessage::error(error_msg.clone()));

        return ContentBlock::ToolResult(ToolResultData {
            tool_use_id,
            content: format!("Unknown agent: {agent_type:?}"),
            is_error: true,
        });
    };

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

    // Return success result - no content needed as this doesn't complete the tool call yet
    ContentBlock::ToolResult(ToolResultData {
        tool_use_id,
        content: json!({"status": "agent_spawned", "agent_type": agent_type, "task": task})
            .to_string(),
        is_error: false,
    })
}

async fn handle_tool_pop_agent(
    state: &mut ActorState,
    success: bool,
    result: String,
    tool_use_id: String,
) -> (ContentBlock, bool) {
    info!("Popping agent: success={}, result={}", success, result);

    // Don't pop if we're at the root agent
    if state.agent_stack.len() <= 1 {
        let tool_result = ToolResultData {
            tool_use_id,
            content: json!({}).to_string(),
            is_error: false,
        };

        state.event_sender.add_message(ChatMessage::system(format!(
            "Task completed [success={success}]: {result}"
        )));
        return (ContentBlock::ToolResult(tool_result), false);
    }

    state.agent_stack.pop();

    // Create result content
    let result_content = serde_json::json!({
        "success": success,
        "result": result
    });

    // If we have a tool_use_id, add the tool result to complete the spawn_agent call
    let Some(tool_id) = current_agent_mut(state).spawn_tool_use_id.take() else {
        panic!("BUG: no tool_use_id set on parent agent")
    };

    let tool_result = ToolResultData {
        tool_use_id: tool_id,
        content: result_content.to_string(),
        is_error: false,
    };

    // Add a user-friendly result message
    let result_message = if success {
        format!("✅ Sub-agent completed successfully:\n{result}")
    } else {
        format!("❌ Sub-agent failed:\n{result}")
    };

    // Notify user
    state
        .event_sender
        .add_message(ChatMessage::system(result_message));

    (ContentBlock::ToolResult(tool_result), true)
}

async fn handle_file_modification(
    state: &mut ActorState,
    modification: crate::tools::r#trait::FileModification,
    tool_use: &ToolUseData,
) -> Result<(ContentBlock, bool)> {
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

    Ok((ContentBlock::ToolResult(result), true))
}

async fn handle_run_command(
    state: &mut ActorState,
    command: String,
    working_directory: std::path::PathBuf,
    timeout_seconds: u64,
    tool_use: &ToolUseData,
) -> Result<(ContentBlock, bool)> {
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

    Ok((ContentBlock::ToolResult(result_data), true))
}

fn handle_prompt_user(
    state: &mut ActorState,
    question: String,
    tool_use: &ToolUseData,
) -> Result<(ContentBlock, bool)> {
    let result = ToolResultData {
        tool_use_id: tool_use.id.clone(),
        content: json!({}).to_string(),
        is_error: false,
    };

    state.event_sender.add_message(ChatMessage::system(format!(
        "The agent has a question: {question}"
    )));

    Ok((ContentBlock::ToolResult(result), false))
}

fn handle_set_tracked_files(
    state: &mut ActorState,
    file_paths: Vec<String>,
    tool_use: &ToolUseData,
) -> Result<(ContentBlock, bool)> {
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

    Ok((ContentBlock::ToolResult(result), true))
}

async fn handle_mcp_call(
    state: &mut ActorState,
    server_name: String,
    tool_name: String,
    arguments: Option<serde_json::Value>,
    tool_use: &ToolUseData,
) -> Result<(ContentBlock, bool)> {
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

    Ok((ContentBlock::ToolResult(result), true))
}
