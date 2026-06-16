use ratatui::{
    layout::Rect,
    style::{Color, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use crate::tui::state::TuiState;

const SPINNER_CHARS: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

pub fn render(frame: &mut Frame, area: Rect, state: &TuiState) {
    let status = if state.is_thinking {
        let spinner = SPINNER_CHARS[state.spinner_frame % SPINNER_CHARS.len()];
        let text = if state.thinking_text.is_empty() {
            "Thinking...".to_string()
        } else {
            state.thinking_text.clone()
        };
        format!("{spinner} {text}")
    } else if state.awaiting_response {
        "Processing...".to_string()
    } else {
        "Ready".to_string()
    };

    let sep = Span::styled(" | ", Style::default().fg(Color::DarkGray));

    let mut parts: Vec<Span<'static>> = vec![
        Span::styled(" ", Style::default()),
        Span::styled(state.current_agent.clone(), Style::default().fg(Color::Yellow)),
        sep.clone(),
        Span::styled(state.current_model.clone(), Style::default().fg(Color::Cyan)),
    ];

    // Only show token usage once there's actual usage
    if state.total_input_tokens > 0 || state.total_output_tokens > 0 {
        let input_str = format_token_count(state.total_input_tokens);
        let output_str = format_token_count(state.total_output_tokens);
        parts.push(sep.clone());
        parts.push(Span::styled(
            format!("{input_str} in / {output_str} out"),
            Style::default().fg(Color::White),
        ));
    }

    parts.push(sep);
    parts.push(Span::styled(status, Style::default().fg(Color::Green)));

    let bar = Paragraph::new(Line::from(parts))
        .style(Style::default().bg(Color::Rgb(30, 30, 30)));

    frame.render_widget(bar, area);
}

fn format_token_count(tokens: u32) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}k", tokens as f64 / 1_000.0)
    } else {
        format!("{tokens}")
    }
}
