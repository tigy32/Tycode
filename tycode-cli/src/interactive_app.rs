use anyhow::Result;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use std::path::PathBuf;
use tokio::sync::mpsc;
use tycode_core::chat::actor::ChatActor;
use tycode_core::chat::events::{ChatEvent, MessageSender};
use tycode_core::formatter::Formatter;
use tycode_core::settings::manager::SettingsManager;

use crate::commands::{handle_local_command, LocalCommandResult};
use crate::state::State;

pub struct InteractiveApp {
    chat_actor: ChatActor,
    event_rx: mpsc::UnboundedReceiver<ChatEvent>,
    formatter: Formatter,
    state: State,
}

impl InteractiveApp {
    pub async fn new(
        workspace_roots: Option<Vec<PathBuf>>,
        settings_path: Option<PathBuf>,
    ) -> Result<Self> {
        let settings_manager = match settings_path {
            Some(path) => SettingsManager::from_path(path)?,
            None => SettingsManager::new()?,
        };

        let workspace_roots = workspace_roots.unwrap_or_else(|| vec![PathBuf::from(".")]);
        let (chat_actor, event_rx) = ChatActor::launch(workspace_roots, settings_manager);
        let formatter = Formatter::new();

        let welcome_message =
            "💡 Type /help for commands, /settings to view configuration, /quit to exit";

        formatter.print_system(welcome_message);

        Ok(Self {
            chat_actor,
            event_rx,
            formatter,
            state: State::default(),
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut rl = DefaultEditor::new()?;

        // We do this handshake at the start of each run to ensure any system
        // messages from the chat actor get printed
        self.chat_actor.get_settings()?;
        self.wait_for_settings().await?;

        loop {
            let line = match rl.readline("\x1b[35m>\x1b[0m ") {
                Ok(line) => line,
                Err(err) => match err {
                    ReadlineError::Interrupted => {
                        continue;
                    }
                    _ => break,
                },
            };

            let input = line.trim();
            if input.is_empty() {
                continue;
            }

            match handle_local_command(&mut self.state, input) {
                LocalCommandResult::Handled { msg } => {
                    self.formatter.print_system(&msg);
                    continue;
                }
                LocalCommandResult::Exit => break,
                LocalCommandResult::Unhandled => (),
            }

            rl.add_history_entry(&line)?;
            self.chat_actor.send_message(input.to_string())?;
            self.wait_for_response().await?;
        }

        println!("\nGoodbye!");
        Ok(())
    }

    async fn wait_for_response(&mut self) -> Result<()> {
        use tokio::signal;
        loop {
            tokio::select! {
                recv = self.event_rx.recv() => {
                    match recv {
                        Some(event) => {
                            let is_complete = match &event {
                                ChatEvent::TypingStatusChanged(typing) => !*typing,
                                _ => false,
                            };
                            self.format_event(event)?;
                            if is_complete {
                                break;
                            }
                        }
                        None => {
                            break;
                        }
                    }
                }
                _ = signal::ctrl_c() => {
                    self.chat_actor.cancel()?;
                    continue;
                }
            }
        }

        Ok(())
    }

    async fn wait_for_settings(&mut self) -> Result<()> {
        loop {
            match self.event_rx.recv().await {
                Some(event) => {
                    self.format_event(event.clone())?;

                    if let ChatEvent::TypingStatusChanged(typing) = event {
                        if !typing {
                            break;
                        }
                    }
                }
                None => {
                    break;
                }
            }
        }

        Ok(())
    }

    fn format_event(&mut self, event: ChatEvent) -> Result<()> {
        match event {
            ChatEvent::MessageAdded(message) => match message.sender {
                MessageSender::Assistant { agent } => {
                    if self.state.show_reasoning {
                        if let Some(ref reasoning) = message.reasoning {
                            self.formatter
                                .print_system(&format!("💭 Reasoning: {}", reasoning.text));
                        }
                    }

                    self.formatter.print_ai(
                        &message.content,
                        &agent,
                        &message.model_info,
                        &message.token_usage,
                    );

                    if !message.tool_calls.is_empty() {
                        let count = message.tool_calls.len();
                        let call_text = if count == 1 { "call" } else { "calls" };
                        let names = message
                            .tool_calls
                            .iter()
                            .map(|tc| tc.name.as_str())
                            .collect::<Vec<&str>>()
                            .join(", ");
                        self.formatter
                            .print_system(&format!("🔧 {count} tool {call_text}: {names}"));
                    }
                }
                MessageSender::System => {
                    self.formatter.print_system(&message.content);
                }
                MessageSender::Error => {
                    self.formatter.print_error(&message.content);
                }
                MessageSender::User => {}
            },
            ChatEvent::TypingStatusChanged(_) => {}
            ChatEvent::Error(e) => self.formatter.print_error(&e),
            ChatEvent::ToolExecutionCompleted {
                tool_name,
                tool_result,
                success,
                ..
            } => {
                self.formatter
                    .print_tool_result(&tool_name, success, tool_result);
            }
            ChatEvent::OperationCancelled { .. } => {
                self.formatter.print_system("Operation Cancelled");
            }
            ChatEvent::Settings(_) => {
                // Settings events are handled elsewhere in the application
            }
            ChatEvent::ConversationCleared => {
                self.formatter.print_system("Conversation cleared");
            }
            ChatEvent::ToolRequest(tool_request) => {
                self.formatter.print_tool_request(&tool_request);
            }
            ChatEvent::RetryAttempt {
                attempt,
                max_retries,
                error,
                backoff_ms,
            } => {
                self.formatter.print_system(&format!(
                    "🔄 Retry attempt {attempt}/{max_retries}: {error} (waiting {backoff_ms}ms)"
                ));
            }
        }
        Ok(())
    }
}
