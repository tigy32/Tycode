use ratatui::{
    layout::Rect,
    style::{Color, Style},
    symbols::border,
    widgets::{Block, Borders},
    Frame,
};
use tui_textarea::TextArea;

pub fn render(frame: &mut Frame, area: Rect, textarea: &TextArea) {
    // Top and bottom borders only (no left/right)
    let border_set = border::Set {
        top_left: "─",
        top_right: "─",
        bottom_left: "─",
        bottom_right: "─",
        ..border::PLAIN
    };

    let block = Block::default()
        .borders(Borders::TOP | Borders::BOTTOM)
        .border_set(border_set)
        .border_style(Style::default().fg(Color::DarkGray));

    // Render the textarea with the horizontal-only borders
    let inner = block.inner(area);
    frame.render_widget(block, area);
    frame.render_widget(textarea, inner);
}
