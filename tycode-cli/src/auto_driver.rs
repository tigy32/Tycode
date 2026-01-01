use anyhow::Result;
use tokio::sync::mpsc::UnboundedReceiver;
use tycode_core::{
    chat::{
        events::{ToolRequest, ToolRequestType},
        ChatActor, ChatEvent, ChatMessage, MessageSender,
    },
    formatter::EventFormatter,
    tools::{ask_user_question::AskUserQuestion, complete_task::CompleteTask},
};

const REMINDER_MESSAGE: &str =
    "Continue working on the task. When the build and tests pass, use the complete_task tool.";
const AUTO_MODE_MESSAGE: &str =
    "You are running in automated mode and cannot ask questions. Please make reasonable decisions and continue.";

pub struct AutoDriverConfig {
    pub initial_agent: String,
    pub max_messages: usize,
}

pub async fn drive_auto_conversation(
    actor: &mut ChatActor,
    event_rx: &mut UnboundedReceiver<ChatEvent>,
    formatter: &mut dyn EventFormatter,
    config: AutoDriverConfig,
) -> Result<String> {
    let mut pending_requests = 1;
    let mut message_count = 0;
    let mut summary = String::new();
    let mut current_agent = config.initial_agent.clone();

    while let Some(event) = event_rx.recv().await {
        match event {
            ChatEvent::TypingStatusChanged(false) => {
                pending_requests -= 1;
                if pending_requests > 0 {
                    continue;
                }
                pending_requests += 1;
                formatter.print_system("Sending reminder message");
                actor.send_message(REMINDER_MESSAGE.to_string())?;
            }
            ChatEvent::TypingStatusChanged(true) => {}
            ChatEvent::Error(msg) => {
                formatter.print_error(&msg);
                return Err(anyhow::anyhow!("Chat error: {}", msg));
            }
            ChatEvent::MessageAdded(chat_message) => {
                message_count =
                    track_assistant_message(&chat_message, &mut current_agent, message_count);
                if message_count > config.max_messages {
                    return Err(anyhow::anyhow!(
                        "Exceeded maximum of {} messages",
                        config.max_messages
                    ));
                }
                handle_message_added(chat_message, formatter);
            }
            ChatEvent::ToolRequest(tool_request) => {
                formatter.print_tool_request(&tool_request);
                if let Some(s) = extract_complete_task_summary(&tool_request) {
                    summary = s;
                }
                if tool_request.tool_name == AskUserQuestion::tool_name().as_str() {
                    pending_requests += 1;
                    actor.send_message(AUTO_MODE_MESSAGE.to_string())?;
                }
            }
            ChatEvent::ToolExecutionCompleted {
                tool_name,
                success,
                tool_result,
                ..
            } => {
                formatter.print_tool_result(&tool_name, success, tool_result, false);
                if tool_name != CompleteTask::tool_name().as_str() || !success {
                    continue;
                }
                if current_agent != config.initial_agent {
                    continue;
                }
                return Ok(summary);
            }
            ChatEvent::RetryAttempt {
                attempt,
                max_retries,
                error,
                backoff_ms,
            } => {
                formatter.print_system(&format!(
                    "Retry {attempt}/{max_retries}: {error}, backoff {backoff_ms}ms"
                ));
            }
            _ => (),
        }
    }

    Err(anyhow::anyhow!("Event stream ended unexpectedly"))
}

fn track_assistant_message(
    chat_message: &ChatMessage,
    current_agent: &mut String,
    message_count: usize,
) -> usize {
    let MessageSender::Assistant { agent } = &chat_message.sender else {
        return message_count;
    };
    *current_agent = agent.clone();
    message_count + 1
}

fn extract_complete_task_summary(tool_request: &ToolRequest) -> Option<String> {
    if tool_request.tool_name != CompleteTask::tool_name().as_str() {
        return None;
    }
    let ToolRequestType::Other { args } = &tool_request.tool_type else {
        return None;
    };
    let result_str = args.get("result")?;
    result_str.as_str().map(String::from)
}

pub fn handle_message_added(chat_message: ChatMessage, formatter: &mut dyn EventFormatter) {
    match chat_message.sender {
        MessageSender::Assistant { ref agent } => {
            print_assistant_message(&chat_message, &agent, formatter);
        }
        MessageSender::System | MessageSender::Warning | MessageSender::User => {
            formatter.print_system(&chat_message.content);
        }
        MessageSender::Error => formatter.print_error(&chat_message.content),
    }
}

fn print_assistant_message(
    chat_message: &ChatMessage,
    agent: &str,
    formatter: &mut dyn EventFormatter,
) {
    if let Some(reasoning) = &chat_message.reasoning {
        formatter.print_system(&format!("Reasoning: {reasoning}"));
    }
    formatter.print_ai(
        &chat_message.content,
        agent,
        &chat_message.model_info,
        &chat_message.token_usage,
    );
    if chat_message.tool_calls.is_empty() {
        return;
    }
    let count = chat_message.tool_calls.len();
    let call_text = if count == 1 { "call" } else { "calls" };
    let names: Vec<&str> = chat_message
        .tool_calls
        .iter()
        .map(|tc| tc.name.as_str())
        .collect();
    formatter.print_system(&format!("Tool {call_text}: {}", names.join(", ")));
}
