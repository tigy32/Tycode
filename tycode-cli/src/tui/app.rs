use anyhow::Result;
use crossterm::{
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event as CrosstermEvent, EventStream, KeyboardEnhancementFlags,
        MouseButton, MouseEvent, MouseEventKind, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use ratatui::{backend::CrosstermBackend, layout::Position, Terminal};
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
        execute!(stdout, EnterAlternateScreen, EnableBracketedPaste, EnableMouseCapture)?;
        // Try to enable keyboard enhancement (for Shift+Enter support).
        // This is only supported by some terminals (Kitty, WezTerm, foot, etc.)
        // so we ignore failures.
        let _ = execute!(
            stdout,
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        );
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
            let _ = execute!(io::stdout(), PopKeyboardEnhancementFlags, DisableMouseCapture, DisableBracketedPaste, LeaveAlternateScreen);
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
                    match crossterm_event {
                        CrosstermEvent::Key(key) => {
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
                        }
                        CrosstermEvent::Mouse(mouse) => {
                            self.handle_mouse_event(mouse);
                        }
                        CrosstermEvent::Paste(text) => {
                            textarea.insert_str(&text);
                        }
                        CrosstermEvent::Resize(_, _) => {
                            // Terminal will re-render on next loop iteration
                        }
                        _ => {}
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

    fn handle_mouse_event(&mut self, mouse: MouseEvent) {
        let col = mouse.column;
        let row = mouse.row;
        let in_chat = self
            .state
            .chat_area
            .contains(Position { x: col, y: row });

        match mouse.kind {
            MouseEventKind::ScrollUp if in_chat => {
                self.state.scroll_up(3);
            }
            MouseEventKind::ScrollDown if in_chat => {
                self.state.scroll_down(3);
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if in_chat {
                    self.state.selection.start = (col, row);
                    self.state.selection.end = (col, row);
                    self.state.selection.is_dragging = true;
                    self.state.selection.has_selection = false;
                } else {
                    self.state.selection.clear();
                }
            }
            MouseEventKind::Drag(MouseButton::Left) if self.state.selection.is_dragging => {
                // Clamp to chat area bounds
                let clamped_col = col.clamp(self.state.chat_area.x, self.state.chat_area.right().saturating_sub(1));
                let clamped_row = row.clamp(self.state.chat_area.y, self.state.chat_area.bottom().saturating_sub(1));
                self.state.selection.end = (clamped_col, clamped_row);
                self.state.selection.has_selection = true;
            }
            MouseEventKind::Up(MouseButton::Left) => {
                if self.state.selection.has_selection {
                    self.state.selection.is_dragging = false;
                    let text = self.extract_selected_text();
                    if !text.is_empty() {
                        Self::copy_to_clipboard(&text);
                    }
                } else {
                    self.state.selection.clear();
                }
            }
            _ => {}
        }
    }

    fn extract_selected_text(&self) -> String {
        let ((sx, sy), (ex, ey)) = self.state.selection.normalized();
        let area = self.state.chat_area;

        // Convert terminal-absolute coords to chat-area-relative
        let rel_sy = sy.saturating_sub(area.y) as usize;
        let rel_ey = ey.saturating_sub(area.y) as usize;
        let rel_sx = sx.saturating_sub(area.x) as usize;
        let rel_ex = ex.saturating_sub(area.x) as usize;

        let mut lines = Vec::new();
        for row_idx in rel_sy..=rel_ey {
            if row_idx >= self.state.screen_text.len() {
                break;
            }
            let line = &self.state.screen_text[row_idx];
            let start_col = if row_idx == rel_sy { rel_sx } else { 0 };
            let end_col = if row_idx == rel_ey {
                (rel_ex + 1).min(line.len())
            } else {
                line.len()
            };

            if start_col < line.len() {
                let selected: String = line
                    .chars()
                    .skip(start_col)
                    .take(end_col.saturating_sub(start_col))
                    .collect();
                lines.push(selected.trim_end().to_string());
            } else {
                lines.push(String::new());
            }
        }

        // Remove trailing empty lines
        while lines.last().map_or(false, |l| l.is_empty()) {
            lines.pop();
        }

        lines.join("\n")
    }

    fn copy_to_clipboard(text: &str) {
        #[cfg(target_os = "macos")]
        {
            use std::process::{Command, Stdio};
            if let Ok(mut child) = Command::new("pbcopy")
                .stdin(Stdio::piped())
                .spawn()
            {
                if let Some(stdin) = child.stdin.as_mut() {
                    use std::io::Write;
                    let _ = stdin.write_all(text.as_bytes());
                }
                let _ = child.wait();
            }
        }
        #[cfg(target_os = "linux")]
        {
            use std::process::{Command, Stdio};
            use std::io::Write;
            // Try xclip first, fall back to xsel
            let result = Command::new("xclip")
                .args(["-selection", "clipboard"])
                .stdin(Stdio::piped())
                .spawn();
            if let Ok(mut child) = result {
                if let Some(stdin) = child.stdin.as_mut() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                let _ = child.wait();
            } else if let Ok(mut child) = Command::new("xsel")
                .args(["--clipboard", "--input"])
                .stdin(Stdio::piped())
                .spawn()
            {
                if let Some(stdin) = child.stdin.as_mut() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                let _ = child.wait();
            }
        }
    }

    fn restore_terminal(&mut self) -> Result<()> {
        disable_raw_mode()?;
        execute!(
            self.terminal.backend_mut(),
            PopKeyboardEnhancementFlags,
            DisableMouseCapture,
            DisableBracketedPaste,
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
            PopKeyboardEnhancementFlags,
            DisableMouseCapture,
            DisableBracketedPaste,
            LeaveAlternateScreen
        );
        let _ = self.terminal.show_cursor();
    }
}
