use crate::agents::agent::ActiveAgent;
use crate::agents::catalog::AgentCatalog;
use crate::agents::tool_type::ToolType;
use crate::ai::{Content, ContentBlock, Message, MessageRole, ToolResultData, ToolUseData};
use crate::chat::actor::ActorState;
use crate::chat::events::{ChatEvent, ChatMessage, ToolRequest, ToolRequestType};
use crate::cmd::run_cmd;
use crate::file::access::FileAccessManager;
use crate::file::manager::FileModificationManager;
use crate::security::evaluate;
use crate::tools::r#trait::ValidatedToolCall;
use crate::tools::registry::ToolRegistry;
use anyhow::{bail, Result};
use serde_json::json;
use std::collections::HashSet;
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
    let tool_registry = ToolRegistry::new(state.workspace_roots.clone(), file_modification_api);

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
    match tool_result {
        ValidatedToolCall::NoOp {
            context_data,
            ui_data,
        } => {
            let content_block = handle_tool_success(state, tool_use, context_data, ui_data);
            (content_block, true)
        }
        ValidatedToolCall::Error(error) => {
            let content_block = handle_tool_error(state, tool_use, error);
            (content_block, true)
        }
        ValidatedToolCall::FileModification(modification) => {
            let file_manager = FileAccessManager::new(state.workspace_roots.clone());
            let file_modification_manager = FileModificationManager::new(file_manager);

            if let Err(e) = file_modification_manager
                .apply_modification(modification.clone())
                .await
            {
                let error_msg = format!("File modification failed: {e:?}");
                let content_block = handle_tool_error(state, tool_use, error_msg);
                return (content_block, true);
            }

            // Create success response with context and UI data
            let context_data = json!({
                "success": true,
                "path": modification.path,
                "operation": match modification.operation {
                    crate::tools::r#trait::FileOperation::Create => "create",
                    crate::tools::r#trait::FileOperation::Update => "update",
                    crate::tools::r#trait::FileOperation::Delete => "delete",
                }
            });

            let ui_data = json!({
                "path": modification.path,
                "original_content": modification.original_content,
                "new_content": modification.new_content
            });

            let _ = state
                .event_sender
                .event_tx
                .send(ChatEvent::ToolRequest(ToolRequest {
                    tool_call_id: tool_use.id.clone(),
                    tool_name: tool_use.name.clone(),
                    arguments: tool_use.arguments.clone(),
                    tool_type: ToolRequestType::ModifyFile {
                        file_path: modification.path.to_string_lossy().to_string(),
                        before: modification.original_content.clone().unwrap_or_default(),
                        after: modification.new_content.clone().unwrap_or_default(),
                    },
                }));

            let content_block = handle_tool_success(state, tool_use, context_data, Some(ui_data));
            (content_block, true)
        }
        ValidatedToolCall::RunCommand {
            command,
            working_directory,
            timeout_seconds,
        } => {
            let timeout = Duration::from_secs(timeout_seconds);
            let result = run_cmd(working_directory, command, timeout).await;
            let content_block = match result {
                Ok(output) => {
                    let context = serde_json::to_value(&output).unwrap_or_else(|_| {
                        json!({
                            "code": output.code,
                            "out": output.out,
                            "err": output.err
                        })
                    });
                    handle_tool_success(state, tool_use, context, None)
                }
                Err(e) => handle_tool_error(state, tool_use, format!("{e:?}")),
            };
            (content_block, true)
        }
        ValidatedToolCall::PushAgent {
            agent_type,
            task,
            context,
        } => {
            let content_block =
                handle_tool_push_agent(state, agent_type, task, context, tool_use.id.clone()).await;
            (content_block, true)
        }
        ValidatedToolCall::PopAgent {
            success,
            summary,
            artifacts,
        } => {
            let (content_block, invoke_ai) =
                handle_tool_pop_agent(state, success, summary, artifacts, tool_use.id.clone())
                    .await;
            (content_block, invoke_ai)
        }
        ValidatedToolCall::PromptUser { question } => {
            let result = ToolResultData {
                tool_use_id: tool_use.id.clone(),
                content: json!({}).to_string(),
                is_error: false,
            };

            state.event_sender.add_message(ChatMessage::system(format!(
                "The agent has a question: {question}"
            )));

            (ContentBlock::ToolResult(result), false)
        }
    }
}

fn handle_tool_success(
    state: &mut ActorState,
    tool_use: &ToolUseData,
    context_data: serde_json::Value,
    ui_data: Option<serde_json::Value>,
) -> ContentBlock {
    // Check if this is a set_tracked_files tool and update state accordingly
    if tool_use.name == "set_tracked_files" {
        if let Some(action) = context_data.get("action") {
            if action.as_str() == Some("set_tracked_files") {
                if let Some(tracked_files) = context_data.get("tracked_files") {
                    if let Some(files_array) = tracked_files.as_array() {
                        // Clear and update tracked files in actor state
                        state.tracked_files.clear();
                        for file_value in files_array {
                            if let Some(file_str) = file_value.as_str() {
                                state
                                    .tracked_files
                                    .insert(std::path::PathBuf::from(file_str));
                            }
                        }
                        info!("Updated tracked files: {:?}", state.tracked_files);
                    }
                }
            }
        }
    }

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
        success: true,
        result: parsed_result,
        ui_data,
        error: None,
    };

    if let Err(e) = state.event_sender.event_tx.send(event) {
        error!("Failed to send tool completion event: {:?}", e);
    }

    ContentBlock::ToolResult(result)
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
        success: false,
        result: None,
        ui_data: None,
        error: Some(error),
    };

    if let Err(e) = state.event_sender.event_tx.send(event) {
        error!("Failed to send tool completion event: {:?}", e);
    }

    ContentBlock::ToolResult(result)
}

async fn handle_tool_push_agent(
    state: &mut ActorState,
    agent_type: String,
    task: String,
    context: Option<String>,
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
    let mut initial_message = task.clone();
    if let Some(ctx) = context {
        initial_message.push_str(&format!("\n\nContext from parent agent:\n{ctx}"));
    }

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
    summary: String,
    artifacts: Option<serde_json::Value>,
    tool_use_id: String,
) -> (ContentBlock, bool) {
    info!("Popping agent: success={}, summary={}", success, summary);

    // Don't pop if we're at the root agent
    if state.agent_stack.len() <= 1 {
        let result = ToolResultData {
            tool_use_id,
            content: json!({}).to_string(),
            is_error: false,
        };

        state.event_sender.add_message(ChatMessage::system(format!(
            "Task completed [success={success}]: {summary}"
        )));
        return (ContentBlock::ToolResult(result), false);
    }

    state.agent_stack.pop();

    // Create result content
    let result_content = if let Some(ref artifacts_data) = artifacts {
        serde_json::json!({
            "success": success,
            "summary": summary,
            "artifacts": artifacts_data
        })
    } else {
        serde_json::json!({
            "success": success,
            "summary": summary
        })
    };

    // If we have a tool_use_id, add the tool result to complete the spawn_agent call
    let Some(tool_id) = current_agent_mut(state).spawn_tool_use_id.take() else {
        panic!("BUG: no tool_use_id set on parent agent")
    };

    let tool_result = ToolResultData {
        tool_use_id: tool_id,
        content: result_content.to_string(),
        is_error: false,
    };

    // Add a user-friendly summary message
    let result_message = if success {
        format!("✅ Sub-agent completed successfully:\n{summary}")
    } else {
        format!("❌ Sub-agent failed:\n{summary}")
    };

    // Notify user
    state
        .event_sender
        .add_message(ChatMessage::system(result_message));

    (ContentBlock::ToolResult(tool_result), true)
}
