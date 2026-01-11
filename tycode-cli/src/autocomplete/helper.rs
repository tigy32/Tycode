use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::{CmdKind, Highlighter};
use rustyline::hint::Hinter;
use rustyline::validate::{ValidationContext, ValidationResult, Validator};
use rustyline::{Cmd, ConditionalEventHandler, Context, Event, EventContext, RepeatCount};
use rustyline::Helper;
use std::borrow::Cow;
use std::sync::{Arc, Mutex};

use super::{AutocompleteState, CommandCompleter, SuggestionRenderer};

/// Shared state between the helper and event handlers
pub struct SharedAutocompleteState {
    pub state: Mutex<AutocompleteState>,
    pub completer: CommandCompleter,
    pub renderer: Mutex<SuggestionRenderer>,
    pub terminal_width: usize,
}

impl SharedAutocompleteState {
    pub fn new(terminal_width: usize) -> Self {
        Self {
            state: Mutex::new(AutocompleteState::new()),
            completer: CommandCompleter::new(),
            renderer: Mutex::new(SuggestionRenderer::new()),
            terminal_width,
        }
    }

    /// Update suggestions based on current input line
    pub fn update_suggestions(&self, line: &str) {
        let mut state = self.state.lock().unwrap();

        // If we just selected a command, suppress reactivation entirely
        // The flag is only cleared by backspace handler or when line is submitted
        if state.just_selected_command.is_some() {
            // Don't reactivate suggestions after a selection
            // User must press backspace or submit the line to reset
            return;
        }

        if let Some(filter) = line.strip_prefix('/') {
            state.filter_text = filter.to_string();
            state.suggestions = self.completer.filter(filter);
            state.active = !state.suggestions.is_empty();
            state.selected_index = 0; // Reset selection when filter changes

            // Render suggestions
            if state.active {
                if let Ok(mut renderer) = self.renderer.lock() {
                    let _ = renderer.render(
                        &state.suggestions,
                        state.selected_index,
                        self.terminal_width,
                    );
                }
            }
        } else {
            self.deactivate_internal(&mut state);
        }
    }

    fn deactivate_internal(&self, state: &mut AutocompleteState) {
        if state.active {
            state.deactivate();
            if let Ok(mut renderer) = self.renderer.lock() {
                let _ = renderer.clear();
            }
        }
    }

    /// Full deactivation including clearing the just_selected_command flag
    pub fn deactivate_full(&self) {
        let mut state = self.state.lock().unwrap();
        state.deactivate_full();
        if let Ok(mut renderer) = self.renderer.lock() {
            let _ = renderer.clear();
        }
    }
}

/// Custom helper that integrates autocomplete
pub struct TycodeHelper {
    pub shared: Arc<SharedAutocompleteState>,
}

impl TycodeHelper {
    pub fn new(terminal_width: usize) -> Self {
        Self {
            shared: Arc::new(SharedAutocompleteState::new(terminal_width)),
        }
    }
}

// Implement required traits for Helper
impl Completer for TycodeHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        _line: &str,
        _pos: usize,
        _ctx: &Context<'_>,
    ) -> Result<(usize, Vec<Self::Candidate>), ReadlineError> {
        // We handle completion manually via our overlay, so return empty
        Ok((0, Vec::new()))
    }
}

impl Hinter for TycodeHelper {
    type Hint = String;

    fn hint(&self, line: &str, _pos: usize, _ctx: &Context<'_>) -> Option<Self::Hint> {
        // Trigger suggestion updates on hint calls (called after each character)
        self.shared.update_suggestions(line);
        None
    }
}

impl Highlighter for TycodeHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        // Highlight commands in magenta
        if line.starts_with('/') {
            Cow::Owned(format!("\x1b[35m{}\x1b[0m", line))
        } else {
            Cow::Borrowed(line)
        }
    }

    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        _default: bool,
    ) -> Cow<'b, str> {
        Cow::Borrowed(prompt)
    }

    fn highlight_char(&self, _line: &str, _pos: usize, _kind: CmdKind) -> bool {
        true // Always refresh highlighting
    }
}

impl Validator for TycodeHelper {
    fn validate(&self, _ctx: &mut ValidationContext) -> Result<ValidationResult, ReadlineError> {
        Ok(ValidationResult::Valid(None))
    }
}

impl Helper for TycodeHelper {}

/// Event handler for Up arrow - navigate suggestions
#[derive(Clone)]
pub struct AutocompleteUpHandler {
    pub shared: Arc<SharedAutocompleteState>,
}

impl ConditionalEventHandler for AutocompleteUpHandler {
    fn handle(
        &self,
        _evt: &Event,
        _n: RepeatCount,
        _positive: bool,
        _ctx: &EventContext,
    ) -> Option<Cmd> {
        let mut state = self.shared.state.lock().unwrap();

        if state.active && !state.suggestions.is_empty() {
            state.move_selection_up();

            let suggestions = state.suggestions.clone();
            let selected = state.selected_index;
            drop(state);

            if let Ok(mut renderer) = self.shared.renderer.lock() {
                let _ = renderer.render(&suggestions, selected, self.shared.terminal_width);
            }

            Some(Cmd::Noop) // Consume the key, don't do history navigation
        } else {
            None // Default behavior (history navigation)
        }
    }
}

/// Event handler for Down arrow - navigate suggestions
#[derive(Clone)]
pub struct AutocompleteDownHandler {
    pub shared: Arc<SharedAutocompleteState>,
}

impl ConditionalEventHandler for AutocompleteDownHandler {
    fn handle(
        &self,
        _evt: &Event,
        _n: RepeatCount,
        _positive: bool,
        _ctx: &EventContext,
    ) -> Option<Cmd> {
        let mut state = self.shared.state.lock().unwrap();

        if state.active && !state.suggestions.is_empty() {
            state.move_selection_down();

            let suggestions = state.suggestions.clone();
            let selected = state.selected_index;
            drop(state);

            if let Ok(mut renderer) = self.shared.renderer.lock() {
                let _ = renderer.render(&suggestions, selected, self.shared.terminal_width);
            }

            Some(Cmd::Noop) // Consume the key
        } else {
            None // Default behavior
        }
    }
}

/// Event handler for Tab - select suggestion and populate input
#[derive(Clone)]
pub struct AutocompleteSelectHandler {
    pub shared: Arc<SharedAutocompleteState>,
}

impl ConditionalEventHandler for AutocompleteSelectHandler {
    fn handle(
        &self,
        evt: &Event,
        _n: RepeatCount,
        _positive: bool,
        _ctx: &EventContext,
    ) -> Option<Cmd> {
        let mut state = self.shared.state.lock().unwrap();

        if state.active && !state.suggestions.is_empty() {
            if let Some(suggestion) = state.get_selected().cloned() {
                // Replace current line with selected command
                let command = format!("/{}", suggestion.name);

                // Mark this command as just selected to prevent immediate reactivation
                state.just_selected_command = Some(command.clone());

                // Clear the suggestions display
                state.deactivate();
                drop(state);

                if let Ok(mut renderer) = self.shared.renderer.lock() {
                    let _ = renderer.clear();
                }

                // Use Cmd::Replace to replace the entire line with the selected command
                // This just populates the input; user needs to press Enter to execute
                return Some(Cmd::Replace(rustyline::Movement::WholeBuffer, Some(command)));
            }
        }

        // Check if this is Enter key - if so, explicitly accept line
        // (Tab can use default behavior which is None)
        if let Some(key_evt) = evt.get(0) {
            if matches!(key_evt, rustyline::KeyEvent(rustyline::KeyCode::Enter, _)) {
                return Some(Cmd::AcceptLine);
            }
        }

        // Default behavior for Tab
        None
    }
}

/// Event handler for Escape - deactivate suggestions
#[derive(Clone)]
pub struct AutocompleteEscapeHandler {
    pub shared: Arc<SharedAutocompleteState>,
}

impl ConditionalEventHandler for AutocompleteEscapeHandler {
    fn handle(
        &self,
        _evt: &Event,
        _n: RepeatCount,
        _positive: bool,
        _ctx: &EventContext,
    ) -> Option<Cmd> {
        let mut state = self.shared.state.lock().unwrap();

        if state.active || state.just_selected_command.is_some() {
            state.deactivate_full();
            drop(state);

            if let Ok(mut renderer) = self.shared.renderer.lock() {
                let _ = renderer.clear();
            }

            Some(Cmd::Noop) // Consume escape
        } else {
            None // Default behavior
        }
    }
}

/// Event handler for Backspace - update suggestions after deletion
#[derive(Clone)]
pub struct AutocompleteBackspaceHandler {
    pub shared: Arc<SharedAutocompleteState>,
}

impl ConditionalEventHandler for AutocompleteBackspaceHandler {
    fn handle(
        &self,
        _evt: &Event,
        _n: RepeatCount,
        _positive: bool,
        ctx: &EventContext,
    ) -> Option<Cmd> {
        let line = ctx.line();
        let pos = ctx.pos();

        // Simulate what line will look like after backspace
        if pos > 0 && pos <= line.len() {
            let new_line = format!("{}{}", &line[..pos - 1], &line[pos..]);

            let mut state = self.shared.state.lock().unwrap();

            // Clear the just_selected flag - user is editing
            state.just_selected_command = None;

            if let Some(filter) = new_line.strip_prefix('/') {
                state.filter_text = filter.to_string();
                state.suggestions = self.shared.completer.filter(filter);
                state.active = !state.suggestions.is_empty();
                state.selected_index = 0;

                if state.active {
                    let suggestions = state.suggestions.clone();
                    let selected = state.selected_index;
                    drop(state);

                    if let Ok(mut renderer) = self.shared.renderer.lock() {
                        let _ = renderer.render(&suggestions, selected, self.shared.terminal_width);
                    }
                } else {
                    drop(state);
                    if let Ok(mut renderer) = self.shared.renderer.lock() {
                        let _ = renderer.clear();
                    }
                }
            } else {
                state.deactivate();
                drop(state);
                if let Ok(mut renderer) = self.shared.renderer.lock() {
                    let _ = renderer.clear();
                }
            }
        }

        None // Use default backspace handling
    }
}
