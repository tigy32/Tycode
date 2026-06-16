use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tui_textarea::TextArea;

use super::state::TuiState;

pub enum TuiAction {
    /// Send the current input text as a message.
    SendMessage(String),
    /// Cancel the current AI operation.
    Cancel,
    /// Quit the application.
    Quit,
    /// No action needed.
    None,
}

pub fn handle_key_event(
    key: KeyEvent,
    textarea: &mut TextArea,
    state: &mut TuiState,
) -> TuiAction {
    match (key.code, key.modifiers) {
        // Ctrl+C: cancel if awaiting response, quit if idle
        (KeyCode::Char('c'), m) if m.contains(KeyModifiers::CONTROL) => {
            if state.awaiting_response {
                TuiAction::Cancel
            } else {
                TuiAction::Quit
            }
        }

        // Ctrl+D: quit
        (KeyCode::Char('d'), m) if m.contains(KeyModifiers::CONTROL) => TuiAction::Quit,

        // Enter without modifier: send message
        (KeyCode::Enter, KeyModifiers::NONE) => {
            if state.awaiting_response {
                return TuiAction::None;
            }
            let lines: Vec<String> = textarea.lines().to_vec();
            let text = lines.join("\n");
            if text.trim().is_empty() {
                return TuiAction::None;
            }
            // Clear the textarea
            *textarea = TextArea::default();
            configure_textarea(textarea);
            TuiAction::SendMessage(text)
        }

        // Shift+Enter or Alt+Enter: insert newline
        (KeyCode::Enter, m)
            if m.contains(KeyModifiers::SHIFT) || m.contains(KeyModifiers::ALT) =>
        {
            textarea.insert_newline();
            TuiAction::None
        }

        // PageUp: scroll chat history up
        (KeyCode::PageUp, _) => {
            state.scroll_up(10);
            TuiAction::None
        }

        // PageDown: scroll chat history down
        (KeyCode::PageDown, _) => {
            state.scroll_down(10);
            TuiAction::None
        }

        // Ctrl+Up: scroll one line up
        (KeyCode::Up, m) if m.contains(KeyModifiers::CONTROL) => {
            state.scroll_up(1);
            TuiAction::None
        }

        // Ctrl+Down: scroll one line down
        (KeyCode::Down, m) if m.contains(KeyModifiers::CONTROL) => {
            state.scroll_down(1);
            TuiAction::None
        }

        // Escape: clear input
        (KeyCode::Esc, _) => {
            *textarea = TextArea::default();
            configure_textarea(textarea);
            TuiAction::None
        }

        // All other keys: forward to textarea
        _ => {
            textarea.input(key);
            TuiAction::None
        }
    }
}

pub fn configure_textarea(textarea: &mut TextArea) {
    textarea.set_placeholder_text("Type a message... (Enter to send, Shift+Enter for new line)");
    textarea.set_cursor_line_style(ratatui::style::Style::default());
    textarea.set_style(ratatui::style::Style::default().fg(ratatui::style::Color::White));
}
