use crate::fixture::MessageCapturingReceiver;
use anyhow::Result;
use tycode_core::{
    agents::tool_type::ToolType,
    chat::{ChatActor, ChatEvent, MessageSender},
    formatter::Formatter,
};

/// Drives the conversation by receiving events until a terminal condition
/// occurs. Exits early if AI stops typing or an error is encountered, allowing
/// the caller to decide next steps.
pub async fn drive_conversation(
    actor: &mut ChatActor,
    event_rx: &mut MessageCapturingReceiver,
    max_messages: usize,
) -> Result<()> {
    let formatter = Formatter::new();

    let mut requests = 1;
    let mut message_count = 0;
    while let Some(event) = event_rx.recv().await {
        match event {
            ChatEvent::TypingStatusChanged(typing) => {
                if !typing {
                    requests -= 1;
                    if requests == 0 {
                        requests += 1;
                        formatter.print_system("Sending reminder message");
                        actor
                            .send_message("Remember: you are running in an automated benchmark and are unable to ask the user questions. Complete the assigned task and then use the complete_task tool".to_string())?;
                    }
                }
            }
            ChatEvent::Error(msg) => {
                // Surface errors immediately to fail the test or caller logic
                formatter.print_error(&msg);
                return Err(anyhow::anyhow!("Chat error: {}", msg));
            }
            ChatEvent::MessageAdded(chat_message) => match chat_message.sender {
                MessageSender::Assistant { agent } => {
                    message_count += 1;
                    if message_count > max_messages {
                        return Err(anyhow::anyhow!(
                            "Exceeded maximum of {} messages",
                            max_messages
                        ));
                    }

                    if let Some(reasoning) = &chat_message.reasoning {
                        formatter.print_system(&format!("💭 {reasoning}"));
                    }

                    formatter.print_ai(
                        &chat_message.content,
                        &agent,
                        &chat_message.model_info,
                        &chat_message.token_usage,
                    );

                    if !chat_message.tool_calls.is_empty() {
                        let count = chat_message.tool_calls.len();
                        let call_text = if count == 1 { "call" } else { "calls" };
                        let names = chat_message
                            .tool_calls
                            .iter()
                            .map(|tc| tc.name.as_str())
                            .collect::<Vec<&str>>()
                            .join(", ");
                        formatter.print_system(&format!("🔧 {count} tool {call_text}: {names}"));
                    }
                }
                MessageSender::System => formatter.print_system(&chat_message.content),
                MessageSender::Warning => formatter.print_system(&chat_message.content),
                MessageSender::Error => formatter.print_error(&chat_message.content),
                MessageSender::User => formatter.print_system(&chat_message.content),
            },
            ChatEvent::ToolRequest(tool_request) => {
                formatter.print_tool_request(&tool_request);
                if tool_request.tool_name == ToolType::CompleteTask.name() {
                    // This is the only intended exit path, but its not wired in
                    // the actor so we also look in assisstant messages for tool calls
                    return Ok(());
                }
                if tool_request.tool_name == ToolType::AskUserQuestion.name() {
                    requests += 1;
                    actor.send_message("Remember: you are running in an automated benchmark and are unable to ask the user questions. Complete the assigned task and then complete_task tool".to_string())?;
                }
            }
            ChatEvent::ToolExecutionCompleted {
                tool_name,
                success,
                tool_result,
                ..
            } => {
                formatter.print_tool_result(&tool_name, success, tool_result, false);
                if tool_name == ToolType::CompleteTask.name() && success {
                    return Ok(());
                }
            }
            ChatEvent::RetryAttempt {
                attempt,
                max_retries,
                error,
                backoff_ms,
            } => {
                formatter.print_system(&format!(
                    "🔄 Retry {attempt}/{max_retries}: {error}, backoff {backoff_ms}ms"
                ));
            }
            _ => (),
        }
    }
    Ok(())
}
