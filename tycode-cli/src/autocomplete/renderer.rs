use crossterm::{
    cursor::{MoveToColumn, MoveUp, RestorePosition, SavePosition},
    queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{Clear, ClearType},
};
use std::io::{stdout, Write};

use super::{CommandSuggestion, MAX_VISIBLE_SUGGESTIONS};

/// Handles rendering suggestions below the input line
pub struct SuggestionRenderer {
    /// Number of suggestion lines currently displayed (for cleanup)
    rendered_line_count: usize,
}

impl Default for SuggestionRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl SuggestionRenderer {
    pub fn new() -> Self {
        Self {
            rendered_line_count: 0,
        }
    }

    /// Render suggestions below the current input line
    /// Only renders visible suggestions based on scroll_offset
    pub fn render(
        &mut self,
        suggestions: &[CommandSuggestion],
        selected_index: usize,
        scroll_offset: usize,
        has_more_above: bool,
        has_more_below: bool,
        terminal_width: usize,
    ) -> std::io::Result<()> {
        let mut stdout = stdout();

        // First, clear any previously rendered suggestions
        self.clear_internal(&mut stdout)?;

        if suggestions.is_empty() {
            stdout.flush()?;
            return Ok(());
        }

        // Save cursor position (at end of input line)
        queue!(stdout, SavePosition)?;

        // Move to next line for suggestions
        queue!(stdout, Print("\n"))?;

        let mut lines_rendered = 0;

        // Show "more above" indicator
        if has_more_above {
            queue!(
                stdout,
                MoveToColumn(0),
                SetForegroundColor(Color::DarkGrey),
                Print("  ↑ more"),
                ResetColor,
                Print("\n"),
            )?;
            lines_rendered += 1;
        }

        // Calculate visible window
        let visible_end = (scroll_offset + MAX_VISIBLE_SUGGESTIONS).min(suggestions.len());
        let visible_suggestions = &suggestions[scroll_offset..visible_end];

        for (visible_idx, suggestion) in visible_suggestions.iter().enumerate() {
            let actual_idx = scroll_offset + visible_idx;
            let is_selected = actual_idx == selected_index;

            // Format: "  /command - description" (truncated to fit terminal)
            let prefix = if is_selected { "> " } else { "  " };
            let command_part = format!("/{}", suggestion.name);
            let separator = " - ";

            // Calculate available space for description
            let used_width = prefix.len() + command_part.len() + separator.len();
            let desc_width = terminal_width.saturating_sub(used_width + 3); // +3 for "..."

            let description = if suggestion.description.chars().count() > desc_width && desc_width > 0 {
                let truncated: String = suggestion.description.chars().take(desc_width).collect();
                format!("{}...", truncated)
            } else {
                suggestion.description.clone()
            };

            // Render with appropriate styling
            if is_selected {
                // Selected item: Cyan for command, DarkGrey for description
                queue!(
                    stdout,
                    MoveToColumn(0),
                    SetForegroundColor(Color::Cyan),
                    Print(prefix),
                    Print(&command_part),
                    SetForegroundColor(Color::DarkGrey),
                    Print(separator),
                    Print(&description),
                    ResetColor,
                )?;
            } else {
                // Non-selected: all DarkGrey (dimmed)
                queue!(
                    stdout,
                    MoveToColumn(0),
                    SetForegroundColor(Color::DarkGrey),
                    Print(prefix),
                    Print(&command_part),
                    Print(separator),
                    Print(&description),
                    ResetColor,
                )?;
            }

            lines_rendered += 1;

            // Move to next line if not the last visible suggestion
            if visible_idx < visible_suggestions.len() - 1 || has_more_below {
                queue!(stdout, Print("\n"))?;
            }
        }

        // Show "more below" indicator
        if has_more_below {
            queue!(
                stdout,
                MoveToColumn(0),
                SetForegroundColor(Color::DarkGrey),
                Print("  ↓ more"),
                ResetColor,
            )?;
            lines_rendered += 1;
        }

        self.rendered_line_count = lines_rendered;

        // Restore cursor position back to input line
        queue!(stdout, RestorePosition)?;

        stdout.flush()?;
        Ok(())
    }

    /// Clear previously rendered suggestions (internal helper)
    fn clear_internal(&mut self, stdout: &mut std::io::Stdout) -> std::io::Result<()> {
        if self.rendered_line_count == 0 {
            return Ok(());
        }

        queue!(stdout, SavePosition)?;

        // Move down and clear each line
        for _ in 0..self.rendered_line_count {
            queue!(stdout, Print("\n"), MoveToColumn(0), Clear(ClearType::CurrentLine))?;
        }

        // Move back up to original position
        for _ in 0..self.rendered_line_count {
            queue!(stdout, MoveUp(1))?;
        }

        queue!(stdout, RestorePosition)?;

        self.rendered_line_count = 0;
        Ok(())
    }

    /// Clear suggestions and flush
    pub fn clear(&mut self) -> std::io::Result<()> {
        let mut stdout = stdout();
        self.clear_internal(&mut stdout)?;
        stdout.flush()?;
        Ok(())
    }
}
