use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
    Frame,
};
use tycode_core::modules::task_list::TaskStatus;

use crate::tui::state::{ChatEntry, TuiState};

pub fn render(frame: &mut Frame, area: Rect, state: &mut TuiState) {
    // Reserve 1 column on the right for the scrollbar so it doesn't overwrite text
    let text_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width.saturating_sub(1),
        height: area.height,
    };

    // Build all lines from chat history (all owned data to avoid lifetime issues)
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Always render banner at the top of the scrollable history
    if let Some(ref banner) = state.banner_data {
        render_banner(&mut lines, banner, text_area.width);
    }

    for entry in &state.chat_history {
        render_entry(&mut lines, entry);
    }

    // Compute the true wrapped line count using the text area width (excludes scrollbar)
    let total_wrapped = compute_wrapped_line_count(&lines, text_area.width);
    let visible_height = area.height;
    let max_scroll = total_wrapped.saturating_sub(visible_height);

    // Store max_scroll so input_handler can clamp
    state.max_scroll = max_scroll;

    // Clamp scroll_offset
    if state.scroll_offset > max_scroll {
        state.scroll_offset = max_scroll;
    }

    let scroll_from_top = max_scroll.saturating_sub(state.scroll_offset);

    let paragraph = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .scroll((scroll_from_top, 0));

    frame.render_widget(paragraph, text_area);

    // Render scrollbar in the full area (occupies the rightmost column)
    if total_wrapped > visible_height {
        let mut scrollbar_state =
            ScrollbarState::new(max_scroll as usize).position(scroll_from_top as usize);
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight);
        frame.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
    }
}

/// Compute the number of visual lines after word wrapping.
fn compute_wrapped_line_count(lines: &[Line], width: u16) -> u16 {
    if width == 0 {
        return lines.len() as u16;
    }
    let w = width as usize;
    let mut count: u16 = 0;
    for line in lines {
        let line_width = line.width();
        if line_width == 0 {
            count = count.saturating_add(1);
        } else {
            // ceil(line_width / w)
            let wrapped = line_width.div_ceil(w) as u16;
            count = count.saturating_add(wrapped);
        }
    }
    count
}

fn render_banner(
    lines: &mut Vec<Line<'static>>,
    banner: &crate::tui::state::BannerData,
    width: u16,
) {
    let tiger = [
        r"  /\_/\  ",
        r" / o o \ ",
        r"=\  ^  /=",
        r"  )---(  ",
        r" /|   |\ ",
        r"(_|   |_)",
    ];
    let tiger_width = 9;

    // Pick a cute welcome message (short enough to fit the left pane)
    let greetings = [
        "Welcome back!",
        "Let's go!",
        "Ready to code!",
        "Let's build!",
    ];
    // Use a simple deterministic pick based on version length
    let greeting = greetings[banner.version.len() % greetings.len()];

    // Full terminal width
    let box_width = (width as usize).max(40);
    let border_style = Style::default().fg(Color::DarkGray);

    // Left panel: tiger centered + greeting
    // Right panel: title + info + help
    // Layout: │ <left_pad> tiger <left_pad> │ <right content> │
    // Left panel: snug fit for tiger + greeting with comfortable padding
    let left_width = (tiger_width + 14).min(box_width * 2 / 5); // ~23 chars, capped at 2/5
    let right_width = box_width.saturating_sub(left_width + 3); // 3 = "│" + "│" + "│"

    // Build right-side info rows
    let max_ws_len = right_width.saturating_sub(13); // "Workspace: " = 11 + 2 padding
    let mut right_rows: Vec<Vec<Span<'static>>> = Vec::new();

    // Title row
    right_rows.push(vec![Span::styled(
        format!("Tycode v{}", banner.version),
        Style::default()
            .fg(Color::Magenta)
            .add_modifier(Modifier::BOLD),
    )]);

    // Blank separator
    right_rows.push(vec![]);

    // Info fields
    if let Some(ref provider) = banner.provider {
        right_rows.push(vec![
            Span::styled("Provider:  ".to_string(), Style::default().fg(Color::DarkGray)),
            Span::styled(provider.clone(), Style::default().fg(Color::Green)),
        ]);
    }
    if let Some(ref model) = banner.model {
        right_rows.push(vec![
            Span::styled("Model:     ".to_string(), Style::default().fg(Color::DarkGray)),
            Span::styled(model.clone(), Style::default().fg(Color::Cyan)),
        ]);
    }
    right_rows.push(vec![
        Span::styled("Agent:     ".to_string(), Style::default().fg(Color::DarkGray)),
        Span::styled(banner.agent.clone(), Style::default().fg(Color::Yellow)),
    ]);
    right_rows.push(vec![
        Span::styled("Workspace: ".to_string(), Style::default().fg(Color::DarkGray)),
        Span::styled(
            shorten_path(&banner.workspace, max_ws_len),
            Style::default().fg(Color::White),
        ),
    ]);

    let memory_text = if banner.memory_enabled {
        format!("enabled ({} recent)", banner.memory_count)
    } else {
        "disabled".to_string()
    };
    let memory_color = if banner.memory_enabled {
        Color::Green
    } else {
        Color::DarkGray
    };
    right_rows.push(vec![
        Span::styled("Memory:    ".to_string(), Style::default().fg(Color::DarkGray)),
        Span::styled(memory_text, Style::default().fg(memory_color)),
    ]);

    // Blank row before help
    right_rows.push(vec![]);

    // Help commands
    right_rows.push(vec![
        Span::styled("/help ".to_string(), Style::default().fg(Color::White)),
        Span::styled("commands  ".to_string(), Style::default().fg(Color::DarkGray)),
        Span::styled("/settings ".to_string(), Style::default().fg(Color::White)),
        Span::styled("config  ".to_string(), Style::default().fg(Color::DarkGray)),
        Span::styled("/quit ".to_string(), Style::default().fg(Color::White)),
        Span::styled("exit".to_string(), Style::default().fg(Color::DarkGray)),
    ]);

    // Calculate total content rows
    // Left side: 1 blank + 6 tiger + 1 blank + 1 greeting + 1 blank = 10
    // Right side: right_rows.len()
    let left_content_rows = tiger.len() + 3; // top pad + tiger + blank + greeting
    let total_rows = left_content_rows.max(right_rows.len());

    // Where the tiger starts vertically (centered in left panel)
    let tiger_start = (total_rows.saturating_sub(tiger.len() + 2)) / 2; // +2 for blank+greeting
    let greeting_row = tiger_start + tiger.len() + 1; // blank line then greeting

    // Top border: ╭───┬───╮
    let left_border: String = "─".repeat(left_width);
    let right_border: String = "─".repeat(right_width);
    lines.push(Line::from(vec![
        Span::styled("╭".to_string(), border_style),
        Span::styled(left_border.clone(), border_style),
        Span::styled("┬".to_string(), border_style),
        Span::styled(right_border.clone(), border_style),
        Span::styled("╮".to_string(), border_style),
    ]));

    // Content rows
    for row in 0..total_rows {
        let mut spans: Vec<Span<'static>> = Vec::new();

        // Left border
        spans.push(Span::styled("│".to_string(), border_style));

        // Left panel content
        if row >= tiger_start && row < tiger_start + tiger.len() {
            // Tiger row - center it in the left panel
            let tiger_idx = row - tiger_start;
            let tiger_text = tiger[tiger_idx];
            let pad_total = left_width.saturating_sub(tiger_width);
            let pad_left = pad_total / 2;
            let pad_right = pad_total - pad_left;
            spans.push(Span::raw(" ".repeat(pad_left)));
            spans.push(Span::styled(tiger_text.to_string(), Style::default().fg(Color::Yellow)));
            spans.push(Span::raw(" ".repeat(pad_right)));
        } else if row == greeting_row {
            // Greeting row - center it
            let pad_total = left_width.saturating_sub(greeting.len());
            let pad_left = pad_total / 2;
            let pad_right = pad_total - pad_left;
            spans.push(Span::raw(" ".repeat(pad_left)));
            spans.push(Span::styled(
                greeting.to_string(),
                Style::default().fg(Color::DarkGray),
            ));
            spans.push(Span::raw(" ".repeat(pad_right)));
        } else {
            // Empty row
            spans.push(Span::raw(" ".repeat(left_width)));
        }

        // Vertical divider
        spans.push(Span::styled("│".to_string(), border_style));

        // Right panel content
        if row < right_rows.len() && !right_rows[row].is_empty() {
            spans.push(Span::raw(" ".to_string())); // left padding
            let content_used: usize = right_rows[row].iter().map(|s| s.width()).sum();
            for span in &right_rows[row] {
                spans.push(span.clone());
            }
            let pad = right_width.saturating_sub(content_used + 1); // -1 for left space
            spans.push(Span::raw(" ".repeat(pad)));
        } else {
            spans.push(Span::raw(" ".repeat(right_width)));
        }

        // Right border
        spans.push(Span::styled("│".to_string(), border_style));

        lines.push(Line::from(spans));
    }

    // Bottom border: ╰───┴───╯
    lines.push(Line::from(vec![
        Span::styled("╰".to_string(), border_style),
        Span::styled(left_border, border_style),
        Span::styled("┴".to_string(), border_style),
        Span::styled(right_border, border_style),
        Span::styled("╯".to_string(), border_style),
    ]));

    // Blank line after the box before chat entries
    lines.push(Line::from(""));
}

fn render_entry(lines: &mut Vec<Line<'static>>, entry: &ChatEntry) {
    match entry {
        ChatEntry::UserMessage { content } => {
            // Render each line of multi-line user messages
            for (i, line) in content.lines().enumerate() {
                let prefix = if i == 0 { "> " } else { "  " };
                lines.push(Line::from(vec![
                    Span::styled(prefix.to_string(), Style::default().fg(Color::Green)),
                    Span::styled(line.to_string(), Style::default().fg(Color::White)),
                ]));
            }
            lines.push(Line::from(""));
        }

        ChatEntry::AssistantMessage {
            agent,
            model,
            content,
            token_usage,
        } => {
            let mut header: Vec<Span<'static>> = vec![
                Span::styled(format!("[{agent}]"), Style::default().fg(Color::Green)),
                Span::styled(
                    format!(" ({model}) "),
                    Style::default().fg(Color::DarkGray),
                ),
            ];
            if let Some(usage) = token_usage {
                let input = usage.input_tokens + usage.cache_creation_input_tokens.unwrap_or(0);
                let output = usage.output_tokens + usage.reasoning_tokens.unwrap_or(0);
                header.push(Span::styled(
                    format!("({input}/{output})"),
                    Style::default().fg(Color::DarkGray),
                ));
            }
            lines.push(Line::from(header));

            for line in content.lines() {
                lines.push(Line::from(Span::raw(line.to_string())));
            }
            lines.push(Line::from(""));
        }

        ChatEntry::StreamingMessage {
            agent,
            model,
            content,
        } => {
            lines.push(Line::from(vec![
                Span::styled(format!("[{agent}]"), Style::default().fg(Color::Green)),
                Span::styled(
                    format!(" ({model}) "),
                    Style::default().fg(Color::DarkGray),
                ),
            ]));
            for line in content.lines() {
                lines.push(Line::from(Span::raw(line.to_string())));
            }
            // Blinking cursor at the end of streaming
            let cursor = Span::styled(
                "\u{258c}".to_string(),
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::SLOW_BLINK),
            );
            if content.is_empty() || content.ends_with('\n') {
                lines.push(Line::from(cursor));
            } else if let Some(last) = lines.last_mut() {
                let mut spans: Vec<Span<'static>> = last.spans.to_vec();
                spans.push(cursor);
                *last = Line::from(spans);
            }
        }

        ChatEntry::SystemMessage { content } => {
            lines.push(Line::from(vec![
                Span::styled("[system] ".to_string(), Style::default().fg(Color::Yellow)),
                Span::styled(content.clone(), Style::default().fg(Color::White)),
            ]));
        }

        ChatEntry::WarningMessage { content } => {
            lines.push(Line::from(vec![
                Span::styled(
                    "[warning] ".to_string(),
                    Style::default().fg(Color::Yellow),
                ),
                Span::styled(content.clone(), Style::default().fg(Color::Yellow)),
            ]));
        }

        ChatEntry::ErrorMessage { content } => {
            lines.push(Line::from(vec![
                Span::styled("[error] ".to_string(), Style::default().fg(Color::Red)),
                Span::styled(content.clone(), Style::default().fg(Color::Red)),
            ]));
        }

        ChatEntry::ToolRequest { summary, .. } => {
            lines.push(Line::from(vec![
                Span::styled("  -> ".to_string(), Style::default().fg(Color::Cyan)),
                Span::styled(summary.clone(), Style::default().fg(Color::DarkGray)),
            ]));
        }

        ChatEntry::ToolResult {
            success, summary, ..
        } => {
            let (icon, color) = if *success {
                ("  \u{2713} ".to_string(), Color::Green)
            } else {
                ("  \u{2717} ".to_string(), Color::Red)
            };
            lines.push(Line::from(vec![
                Span::styled(icon, Style::default().fg(color)),
                Span::styled(summary.clone(), Style::default().fg(Color::DarkGray)),
            ]));
        }

        ChatEntry::TaskUpdate { task_list } => {
            if !task_list.tasks.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("  Tasks: {}", task_list.title),
                    Style::default()
                        .fg(Color::Magenta)
                        .add_modifier(Modifier::BOLD),
                )));
                for task in &task_list.tasks {
                    let (icon, color) = match task.status {
                        TaskStatus::Completed => ("\u{2713}", Color::Green),
                        TaskStatus::InProgress => ("\u{25ce}", Color::Yellow),
                        TaskStatus::Pending => ("\u{25cb}", Color::DarkGray),
                        TaskStatus::Failed => ("\u{2717}", Color::Red),
                    };
                    lines.push(Line::from(vec![
                        Span::styled(format!("    {icon} "), Style::default().fg(color)),
                        Span::styled(
                            task.description.clone(),
                            Style::default().fg(Color::White),
                        ),
                    ]));
                }
            }
        }
    }
}

fn shorten_path(path: &str, max_len: usize) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let path = if !home.is_empty() && path.starts_with(&home) {
        format!("~{}", &path[home.len()..])
    } else {
        path.to_string()
    };

    if path.len() <= max_len {
        path
    } else {
        format!("...{}", &path[path.len().saturating_sub(max_len - 3)..])
    }
}
