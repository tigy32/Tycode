use anyhow::Result;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use std::path::PathBuf;
use std::thread;
use terminal_size::{terminal_size, Width};
use tokio::sync::mpsc;
use tycode_core::chat::actor::{ChatActor, ChatActorBuilder};
use tycode_core::chat::events::{ChatEvent, MessageSender};
use tycode_core::formatter::{CompactFormatter, EventFormatter, VerboseFormatter};
use tycode_core::modules::memory::MemoryConfig;
use tycode_core::settings::SettingsManager;

use crate::banner::{print_startup_banner, BannerInfo};
use crate::commands::{handle_local_command, LocalCommandResult};
use crate::state::State;

enum ReadlineResponse {
    Line(String),
    Eof,
    Interrupted,
    Error(String),
}

fn handle_readline(rl: &mut DefaultEditor, prompt: &str) -> ReadlineResponse {
    match rl.readline(prompt) {
        Ok(line) => {
            if let Err(e) = rl.add_history_entry(&line) {
                eprintln!("Warning: failed to add history entry: {e:?}");
            }
            ReadlineResponse::Line(line)
        }
        Err(ReadlineError::Eof) => ReadlineResponse::Eof,
        Err(ReadlineError::Interrupted) => ReadlineResponse::Interrupted,
        Err(e) => ReadlineResponse::Error(format!("{e:?}")),
    }
}

fn spawn_readline_thread() -> (
    mpsc::UnboundedSender<String>,
    mpsc::UnboundedReceiver<ReadlineResponse>,
) {
    let (request_tx, mut request_rx) = mpsc::unbounded_channel::<String>();
    let (response_tx, response_rx) = mpsc::unbounded_channel::<ReadlineResponse>();

    thread::spawn(move || {
        let Ok(mut rl) = DefaultEditor::new() else {
            let _ = response_tx.send(ReadlineResponse::Error(
                "Failed to create editor".to_string(),
            ));
            return;
        };

        while let Some(prompt) = request_rx.blocking_recv() {
            let response = handle_readline(&mut rl, &prompt);
            if response_tx.send(response).is_err() {
                break;
            }
        }
    });

    (request_tx, response_rx)
}

pub struct InteractiveApp {
    chat_actor: ChatActor,
    event_rx: mpsc::UnboundedReceiver<ChatEvent>,
    formatter: Box<dyn EventFormatter>,
    state: State,
    is_thinking: bool,
    readline_tx: mpsc::UnboundedSender<String>,
    readline_rx: mpsc::UnboundedReceiver<ReadlineResponse>,
}

impl InteractiveApp {
    pub async fn new(
        workspace_roots: Option<Vec<PathBuf>>,
        profile: Option<String>,
        compact: bool,
    ) -> Result<Self> {
        let workspace_roots = workspace_roots.unwrap_or_else(|| vec![PathBuf::from(".")]);

        // Load settings for banner display
        let root_dir = dirs::home_dir()
            .expect("Failed to get home directory")
            .join(".tycode");
        let settings_manager = SettingsManager::from_settings_dir(root_dir, profile.as_deref())?;
        let settings = settings_manager.settings();

        // Get model from the default agent's config, or fall back to quality tier
        let model_display = settings
            .get_agent_model(&settings.default_agent)
            .map(|m| {
                use tycode_core::ai::model::Model;
                match m.model {
                    Model::ClaudeOpus45 => "claude-opus-4-5",
                    Model::ClaudeSonnet45 => "claude-sonnet-4-5",
                    Model::ClaudeHaiku45 => "claude-haiku-4-5",
                    _ => return format!("{:?}", m.model).to_lowercase(),
                }
                .to_string()
            })
            .or_else(|| {
                settings.model_quality.map(|q| {
                    match q {
                        tycode_core::ai::model::ModelCost::Unlimited => "claude-opus-4-5",
                        tycode_core::ai::model::ModelCost::High => "claude-opus-4-5",
                        tycode_core::ai::model::ModelCost::Medium => "claude-sonnet-4-5",
                        tycode_core::ai::model::ModelCost::Low => "claude-haiku-4-5",
                        tycode_core::ai::model::ModelCost::Free => "claude-haiku-4-5",
                    }
                    .to_string()
                })
            });

        // Print startup banner
        let banner_info = BannerInfo {
            version: env!("CARGO_PKG_VERSION").to_string(),
            provider: settings.active_provider.clone(),
            model: model_display,
            agent: settings.default_agent.clone(),
            workspace: workspace_roots
                .first()
                .and_then(|p| p.canonicalize().ok())
                .or_else(|| std::env::current_dir().ok())
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| ".".to_string()),
            memory_enabled: {
                let memory_config: MemoryConfig = settings.get_module_config("memory");
                memory_config.enabled
            },
            memory_count: {
                let memory_config: MemoryConfig = settings.get_module_config("memory");
                memory_config.recent_memories_count
            },
        };
        print_startup_banner(&banner_info);

        let (chat_actor, event_rx) =
            ChatActorBuilder::tycode(workspace_roots, None, profile)?.build()?;

        let formatter: Box<dyn EventFormatter> = if compact {
            let terminal_width = terminal_size()
                .map(|(Width(w), _)| w as usize)
                .unwrap_or(80);
            Box::new(CompactFormatter::new(terminal_width))
        } else {
            Box::new(VerboseFormatter::new())
        };

        let (readline_tx, readline_rx) = spawn_readline_thread();

        Ok(Self {
            chat_actor,
            event_rx,
            formatter,
            state: State::default(),
            is_thinking: false,
            readline_tx,
            readline_rx,
        })
    }

    async fn readline(&mut self, prompt: &str) -> Result<ReadlineResponse> {
        self.readline_tx
            .send(prompt.to_string())
            .map_err(|e| anyhow::anyhow!("Readline thread died: {e:?}"))?;

        match self.readline_rx.recv().await {
            Some(ReadlineResponse::Error(e)) => Err(anyhow::anyhow!("Readline error: {e}")),
            Some(r) => Ok(r),
            None => Ok(ReadlineResponse::Eof),
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        // We do this handshake at the start of each run to ensure any system
        // messages from the chat actor get printed
        self.chat_actor.get_settings()?;
        self.wait_for_settings().await?;

        loop {
            let line = match self.readline("\x1b[35m>\x1b[0m ").await? {
                ReadlineResponse::Line(l) => l,
                ReadlineResponse::Eof => break,
                ReadlineResponse::Interrupted => continue,
                ReadlineResponse::Error(_) => unreachable!("errors handled in readline()"),
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

            self.chat_actor.send_message(input.to_string())?;
            self.wait_for_response().await?
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
                _ = tokio::time::sleep(std::time::Duration::from_millis(100)) => {
                    if self.is_thinking {
                        self.formatter.print_thinking();
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
                    // Skip TaskUpdate events during startup to avoid showing default task list
                    if !matches!(event, ChatEvent::TaskUpdate(_)) {
                        self.format_event(event.clone())?;
                    }

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
                MessageSender::Warning => {
                    self.formatter.print_warning(&message.content);
                }
                MessageSender::Error => {
                    self.formatter.print_error(&message.content);
                }
                MessageSender::User => {}
            },
            ChatEvent::TypingStatusChanged(typing) => {
                self.is_thinking = typing;
                self.formatter.on_typing_status_changed(typing);
                if typing {
                    self.formatter.print_thinking();
                }
            }
            ChatEvent::Error(e) => self.formatter.print_error(&e),
            ChatEvent::ToolExecutionCompleted {
                tool_name,
                tool_result,
                success,
                ..
            } => {
                self.formatter.print_tool_result(
                    &tool_name,
                    success,
                    tool_result,
                    self.state.show_reasoning,
                );
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
                ..
            } => {
                self.formatter
                    .print_retry_attempt(attempt, max_retries, &error);
            }
            ChatEvent::TaskUpdate(task_list) => {
                self.formatter.print_task_update(&task_list);
            }
            ChatEvent::SessionsList { .. } => {
                // CLI handles sessions via slash commands, ignore this event
            }
            ChatEvent::ProfilesList { .. } => {
                // CLI handles profiles via slash commands, ignore this event
            }
            ChatEvent::ModuleSchemas { .. } => {
                // Module schemas are only used by VSCode extension UI
            }
            ChatEvent::TimingUpdate {
                waiting_for_human,
                ai_processing,
                tool_execution,
            } => {
                let total = waiting_for_human + ai_processing + tool_execution;
                if self.state.show_timing {
                    self.formatter.print_system(&format!(
                        "Timing => Human: {:.1}s, AI: {:.1}s, Tools: {:.1}s, Total: {:.1}s",
                        waiting_for_human.as_secs_f64(),
                        ai_processing.as_secs_f64(),
                        tool_execution.as_secs_f64(),
                        total.as_secs_f64(),
                    ));
                }
            }
            ChatEvent::StreamStart {
                message_id,
                agent,
                model,
            } => {
                self.is_thinking = false;
                self.formatter.on_typing_status_changed(false);
                self.formatter
                    .print_stream_start(&message_id, &agent, &model);
            }
            ChatEvent::StreamDelta { message_id, text } => {
                self.formatter.print_stream_delta(&message_id, &text);
            }
            ChatEvent::StreamReasoningDelta { text, .. } => {
                if self.state.show_reasoning {
                    self.formatter.print_stream_delta(&String::new(), &text);
                }
            }
            ChatEvent::StreamEnd { message } => {
                self.formatter.print_stream_end(&message);
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
        }
        Ok(())
    }
}
