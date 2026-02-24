use anyhow::Result;
use crossterm::{
    event::{EnableMouseCapture, DisableMouseCapture, Event as CrosstermEvent, EventStream, MouseEvent, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;
use tui_textarea::TextArea;
use tycode_core::ai::model::Model;
use tycode_core::chat::actor::{ChatActor, ChatActorBuilder};
use tycode_core::chat::events::ChatEvent;
use tycode_core::modules::memory::MemoryConfig;
use tycode_core::settings::SettingsManager;

use super::event_handler::handle_chat_event;
use super::input_handler::{configure_textarea, handle_key_event, TuiAction};
use super::state::{BannerData, TuiState};
use super::ui::draw_ui;

use crate::commands::{handle_local_command, LocalCommandResult};
use crate::state::State;

pub struct TuiApp {
    terminal: Terminal<CrosstermBackend<io::Stdout>>,
    chat_actor: ChatActor,
    event_rx: mpsc::UnboundedReceiver<ChatEvent>,
    state: TuiState,
}

impl TuiApp {
    pub async fn new(
        workspace_roots: Option<Vec<PathBuf>>,
        profile: Option<String>,
    ) -> Result<Self> {
        let workspace_roots = workspace_roots.unwrap_or_else(|| vec![PathBuf::from(".")]);

        // Load settings for banner display
        let root_dir = dirs::home_dir()
            .expect("Failed to get home directory")
            .join(".tycode");
        let settings_manager = SettingsManager::from_settings_dir(root_dir, profile.as_deref())?;
        let settings = settings_manager.settings();

        let model_display = settings
            .get_agent_model(&settings.default_agent)
            .map(|m| match m.model {
                Model::ClaudeOpus45 => "claude-opus-4-5".to_string(),
                Model::ClaudeSonnet45 => "claude-sonnet-4-5".to_string(),
                Model::ClaudeHaiku45 => "claude-haiku-4-5".to_string(),
                _ => format!("{:?}", m.model).to_lowercase(),
            })
            .or_else(|| {
                settings.model_quality.map(|q| {
                    match q {
                        tycode_core::ai::model::ModelCost::Unlimited
                        | tycode_core::ai::model::ModelCost::High => "claude-opus-4-5",
                        tycode_core::ai::model::ModelCost::Medium => "claude-sonnet-4-5",
                        tycode_core::ai::model::ModelCost::Low
                        | tycode_core::ai::model::ModelCost::Free => "claude-haiku-4-5",
                    }
                    .to_string()
                })
            });

        let memory_config: MemoryConfig = settings.get_module_config("memory");

        let banner_data = BannerData {
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
            memory_enabled: memory_config.enabled,
            memory_count: memory_config.recent_memories_count,
        };

        let (chat_actor, event_rx) =
            ChatActorBuilder::tycode(workspace_roots, None, profile)?.build()?;

        let state = TuiState::new(Some(banner_data));

        // Setup terminal
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;

        Ok(Self {
            terminal,
            chat_actor,
            event_rx,
            state,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        // Install panic hook to restore terminal on panic
        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
            original_hook(panic_info);
        }));

        // Initial settings handshake
        self.chat_actor.get_settings()?;
        self.wait_for_settings().await?;

        // Create textarea for input
        let mut textarea = TextArea::default();
        configure_textarea(&mut textarea);

        let tick_rate = Duration::from_millis(50);
        let mut crossterm_reader = EventStream::new();

        loop {
            // Render
            let state = &mut self.state;
            let ta = &textarea;
            self.terminal.draw(|frame| {
                draw_ui(frame, state, ta);
            })?;

            if self.state.should_quit {
                break;
            }

            tokio::select! {
                // Poll ChatEvents from the actor
                Some(chat_event) = self.event_rx.recv() => {
                    handle_chat_event(&mut self.state, chat_event);
                }

                // Poll crossterm events (async)
                Some(Ok(crossterm_event)) = crossterm_reader.next() => {
                    if let CrosstermEvent::Key(key) = crossterm_event {
                        match handle_key_event(key, &mut textarea, &mut self.state) {
                            TuiAction::SendMessage(text) => {
                                self.send_user_message(&text)?;
                            }
                            TuiAction::Cancel => {
                                self.chat_actor.cancel()?;
                            }
                            TuiAction::Quit => {
                                self.state.should_quit = true;
                            }
                            TuiAction::None => {}
                        }
                    } else if let CrosstermEvent::Mouse(mouse) = crossterm_event {
                        match mouse {
                            MouseEvent { kind: MouseEventKind::ScrollUp, .. } => {
                                self.state.scroll_up(3);
                            }
                            MouseEvent { kind: MouseEventKind::ScrollDown, .. } => {
                                self.state.scroll_down(3);
                            }
                            _ => {}
                        }
                    } else if let CrosstermEvent::Resize(_, _) = crossterm_event {
                        // Terminal will re-render on next loop iteration
                    }
                }

                // Tick for spinner animation
                _ = tokio::time::sleep(tick_rate) => {
                    if self.state.is_thinking {
                        self.state.spinner_frame += 1;
                    }
                }
            }
        }

        // Restore terminal
        self.restore_terminal()?;

        Ok(())
    }

    fn send_user_message(&mut self, text: &str) -> Result<()> {
        let input = text.trim().to_string();
        if input.is_empty() {
            return Ok(());
        }

        // Check for local commands
        let mut temp_state = State {
            show_reasoning: self.state.inner_state.show_reasoning,
            show_timing: self.state.inner_state.show_timing,
        };
        match handle_local_command(&mut temp_state, &input) {
            LocalCommandResult::Handled { msg } => {
                // Sync toggles back
                self.state.inner_state.show_reasoning = temp_state.show_reasoning;
                self.state.inner_state.show_timing = temp_state.show_timing;
                self.state
                    .push_entry(super::state::ChatEntry::SystemMessage { content: msg });
                return Ok(());
            }
            LocalCommandResult::Exit => {
                self.state.should_quit = true;
                return Ok(());
            }
            LocalCommandResult::Unhandled => {}
        }

        // Add user message to history
        self.state
            .push_entry(super::state::ChatEntry::UserMessage {
                content: input.clone(),
            });
        self.state.awaiting_response = true;
        self.state.auto_scroll = true;

        // Send to actor
        self.chat_actor.send_message(input)?;
        Ok(())
    }

    async fn wait_for_settings(&mut self) -> Result<()> {
        while let Some(event) = self.event_rx.recv().await {
            // Skip TaskUpdate events during startup
            if !matches!(event, ChatEvent::TaskUpdate(_)) {
                handle_chat_event(&mut self.state, event.clone());
            }

            if let ChatEvent::TypingStatusChanged(typing) = event {
                if !typing {
                    break;
                }
            }
        }
        Ok(())
    }

    fn restore_terminal(&mut self) -> Result<()> {
        disable_raw_mode()?;
        execute!(
            self.terminal.backend_mut(),
            DisableMouseCapture,
            LeaveAlternateScreen
        )?;
        self.terminal.show_cursor()?;
        Ok(())
    }
}

impl Drop for TuiApp {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            self.terminal.backend_mut(),
            DisableMouseCapture,
            LeaveAlternateScreen
        );
        let _ = self.terminal.show_cursor();
    }
}
