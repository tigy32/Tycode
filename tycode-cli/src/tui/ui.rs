use ratatui::{
    layout::{Constraint, Direction, Layout},
    Frame,
};
use tui_textarea::TextArea;

use super::state::TuiState;
use super::widgets::{chat_panel, input_area, status_bar};

pub fn draw_ui(frame: &mut Frame, state: &mut TuiState, textarea: &TextArea) {
    // Input height: textarea lines + 2 for top/bottom borders, min 3, max 12
    let textarea_lines = textarea.lines().len().clamp(1, 10) as u16;
    let input_height = textarea_lines + 2; // +2 for top and bottom border lines

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),              // Chat panel (fills remaining space)
            Constraint::Length(input_height), // Input area (dynamic, with borders)
            Constraint::Length(1),            // Empty line gap
            Constraint::Length(1),            // Status bar
        ])
        .split(frame.area());

    chat_panel::render(frame, chunks[0], state);
    input_area::render(frame, chunks[1], textarea);
    // chunks[2] is the empty gap line - just leave it blank
    status_bar::render(frame, chunks[3], state);
}
