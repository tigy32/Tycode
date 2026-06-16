use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::Modifier,
    Frame,
};
use tui_textarea::TextArea;

use super::state::TuiState;
use super::widgets::{chat_panel, input_area, status_bar};

pub fn draw_ui(frame: &mut Frame, state: &mut TuiState, textarea: &TextArea) {
    let inner = frame.area();

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
        .split(inner);

    chat_panel::render(frame, chunks[0], state);
    input_area::render(frame, chunks[1], textarea);
    // chunks[2] is the empty gap line - just leave it blank
    status_bar::render(frame, chunks[3], state);

    // Store chat area rect for mouse hit-testing
    state.chat_area = chunks[0];

    // Snapshot the chat panel buffer text for text extraction
    let area = chunks[0];
    let buf = frame.buffer_mut();
    let mut screen_text = Vec::with_capacity(area.height as usize);
    for row in area.y..area.bottom() {
        let mut line = String::with_capacity(area.width as usize);
        for col in area.x..area.right() {
            let cell = &buf[(col, row)];
            line.push_str(cell.symbol());
        }
        screen_text.push(line);
    }
    state.screen_text = screen_text;

    // Apply selection highlight (reversed video)
    if state.selection.has_selection {
        let ((sx, sy), (ex, ey)) = state.selection.normalized();
        for row in sy..=ey {
            if row < area.y || row >= area.bottom() {
                continue;
            }
            let col_start = if row == sy { sx.max(area.x) } else { area.x };
            let col_end = if row == ey {
                (ex + 1).min(area.right())
            } else {
                area.right()
            };
            for col in col_start..col_end {
                let cell = &mut buf[(col, row)];
                let style = cell.style().add_modifier(Modifier::REVERSED);
                cell.set_style(style);
            }
        }
    }
}
