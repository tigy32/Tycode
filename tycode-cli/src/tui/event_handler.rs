use tycode_core::chat::events::{ChatEvent, MessageSender, ToolExecutionResult, ToolRequestType};

use super::state::{ChatEntry, TuiState};

pub fn handle_chat_event(state: &mut TuiState, event: ChatEvent) {
    match event {
        ChatEvent::MessageAdded(message) => match message.sender {
            MessageSender::Assistant { agent } => {
                // Update status bar model/agent
                state.current_agent = agent.clone();
                if let Some(ref info) = message.model_info {
                    state.current_model = info.model.name().to_string();
                }

                if state.inner_state.show_reasoning {
                    if let Some(ref reasoning) = message.reasoning {
                        state.push_entry(ChatEntry::SystemMessage {
                            content: format!("Reasoning: {}", reasoning.text),
                        });
                    }
                }
                state.push_entry(ChatEntry::AssistantMessage {
                    agent,
                    model: message
                        .model_info
                        .as_ref()
                        .map(|m| m.model.name().to_string())
                        .unwrap_or_default(),
                    content: message.content,
                    token_usage: message.token_usage.clone(),
                });
                if let Some(ref usage) = message.token_usage {
                    state.accumulate_tokens(usage);
                }
            }
            MessageSender::System => {
                state.push_entry(ChatEntry::SystemMessage {
                    content: message.content,
                });
            }
            MessageSender::Warning => {
                state.push_entry(ChatEntry::WarningMessage {
                    content: message.content,
                });
            }
            MessageSender::Error => {
                state.push_entry(ChatEntry::ErrorMessage {
                    content: message.content,
                });
            }
            MessageSender::User => {
                // Usually already added by send_user_message; skip to avoid duplicates.
            }
        },

        ChatEvent::StreamStart { agent, model, .. } => {
            state.is_thinking = false;
            state.thinking_text.clear();
            state.current_agent = agent.clone();
            state.current_model = model.name().to_string();
            state.push_entry(ChatEntry::StreamingMessage {
                agent,
                model: model.name().to_string(),
                content: String::new(),
            });
        }

        ChatEvent::StreamDelta { text, .. } => {
            if let Some(ChatEntry::StreamingMessage { content, .. }) =
                state.chat_history.last_mut()
            {
                content.push_str(&text);
            }
        }

        ChatEvent::StreamReasoningDelta { text, .. } => {
            if state.inner_state.show_reasoning {
                if let Some(ChatEntry::StreamingMessage { content, .. }) =
                    state.chat_history.last_mut()
                {
                    content.push_str(&text);
                }
            }
        }

        ChatEvent::StreamEnd { message } => {
            // Convert the last StreamingMessage to a finalized AssistantMessage.
            if let Some(last) = state.chat_history.last_mut() {
                if let ChatEntry::StreamingMessage {
                    agent,
                    model,
                    content,
                } = last
                {
                    *last = ChatEntry::AssistantMessage {
                        agent: agent.clone(),
                        model: model.clone(),
                        content: content.clone(),
                        token_usage: message.token_usage.clone(),
                    };
                }
            }
            if let Some(ref usage) = message.token_usage {
                state.accumulate_tokens(usage);
            }
            if !message.tool_calls.is_empty() {
                let count = message.tool_calls.len();
                let names: Vec<&str> =
                    message.tool_calls.iter().map(|tc| tc.name.as_str()).collect();
                let call_text = if count == 1 { "call" } else { "calls" };
                state.push_entry(ChatEntry::SystemMessage {
                    content: format!("{count} tool {call_text}: {}", names.join(", ")),
                });
            }
        }

        ChatEvent::TypingStatusChanged(typing) => {
            state.is_thinking = typing;
            if typing {
                state.thinking_text = "Thinking...".to_string();
            } else {
                state.thinking_text.clear();
                state.awaiting_response = false;
            }
        }

        ChatEvent::ToolRequest(tool_request) => {
            let summary = match &tool_request.tool_type {
                ToolRequestType::ModifyFile { file_path, .. } => {
                    format!("Modifying {file_path}")
                }
                ToolRequestType::RunCommand { command, .. } => {
                    format!("Running `{command}`")
                }
                ToolRequestType::ReadFiles { file_paths } => {
                    format!("Reading {} file(s)", file_paths.len())
                }
                ToolRequestType::SearchTypes { type_name, .. } => {
                    format!("Searching types: {type_name}")
                }
                ToolRequestType::GetTypeDocs { type_path, .. } => {
                    format!("Getting docs: {type_path}")
                }
                ToolRequestType::Other { .. } => {
                    format!("Executing {}", tool_request.tool_name)
                }
            };
            state.thinking_text = summary.clone();
            state.push_entry(ChatEntry::ToolRequest {
                tool_name: tool_request.tool_name,
                summary,
            });
        }

        ChatEvent::ToolExecutionCompleted {
            tool_name,
            tool_result,
            success,
            ..
        } => {
            let summary = format_tool_result_summary(&tool_name, &tool_result);
            state.push_entry(ChatEntry::ToolResult {
                tool_name,
                success,
                summary,
            });
            if state.is_thinking {
                state.thinking_text = "Thinking...".to_string();
            }
        }

        ChatEvent::OperationCancelled { .. } => {
            state.push_entry(ChatEntry::SystemMessage {
                content: "Operation cancelled".to_string(),
            });
            state.is_thinking = false;
            state.awaiting_response = false;
        }

        ChatEvent::RetryAttempt {
            attempt,
            max_retries,
            error,
            ..
        } => {
            state.push_entry(ChatEntry::WarningMessage {
                content: format!("Retry {attempt}/{max_retries}: {error}"),
            });
        }

        ChatEvent::TaskUpdate(task_list) => {
            state.current_tasks = Some(task_list.clone());
            state.push_entry(ChatEntry::TaskUpdate { task_list });
        }

        ChatEvent::TimingUpdate {
            waiting_for_human,
            ai_processing,
            tool_execution,
        } => {
            if state.inner_state.show_timing {
                let total = waiting_for_human + ai_processing + tool_execution;
                state.push_entry(ChatEntry::SystemMessage {
                    content: format!(
                        "Timing: Human {:.1}s, AI {:.1}s, Tools {:.1}s, Total {:.1}s",
                        waiting_for_human.as_secs_f64(),
                        ai_processing.as_secs_f64(),
                        tool_execution.as_secs_f64(),
                        total.as_secs_f64(),
                    ),
                });
            }
        }

        ChatEvent::Error(e) => {
            state.push_entry(ChatEntry::ErrorMessage { content: e });
        }

        ChatEvent::ConversationCleared => {
            state.chat_history.clear();
            state.push_entry(ChatEntry::SystemMessage {
                content: "Conversation cleared".to_string(),
            });
        }

        // Events not relevant to the TUI display
        ChatEvent::Settings(_)
        | ChatEvent::SessionsList { .. }
        | ChatEvent::ProfilesList { .. }
        | ChatEvent::ModuleSchemas { .. } => {}
    }
}

fn format_tool_result_summary(tool_name: &str, result: &ToolExecutionResult) -> String {
    match result {
        ToolExecutionResult::ModifyFile {
            lines_added,
            lines_removed,
        } => {
            format!("{tool_name}: +{lines_added}/-{lines_removed} lines")
        }
        ToolExecutionResult::RunCommand {
            exit_code, stderr, ..
        } => {
            if *exit_code == 0 {
                format!("{tool_name}: completed (exit 0)")
            } else {
                let err_preview = stderr.lines().next().unwrap_or("").chars().take(80).collect::<String>();
                format!("{tool_name}: exit {exit_code} - {err_preview}")
            }
        }
        ToolExecutionResult::ReadFiles { files } => {
            format!("{tool_name}: read {} file(s)", files.len())
        }
        ToolExecutionResult::SearchTypes { types } => {
            format!("{tool_name}: found {} type(s)", types.len())
        }
        ToolExecutionResult::GetTypeDocs { .. } => {
            format!("{tool_name}: docs retrieved")
        }
        ToolExecutionResult::Error { short_message, .. } => {
            format!("{tool_name}: error - {short_message}")
        }
        ToolExecutionResult::Other { .. } => {
            format!("{tool_name}: completed")
        }
    }
}
