mod completer;
mod helper;
mod renderer;

pub use completer::CommandCompleter;
pub use helper::{
    AutocompleteBackspaceHandler, AutocompleteDownHandler, AutocompleteEscapeHandler,
    AutocompleteSelectHandler, AutocompleteUpHandler, TycodeHelper,
};
pub use renderer::SuggestionRenderer;

/// A command suggestion with display info
#[derive(Clone, Debug)]
pub struct CommandSuggestion {
    pub name: String,
    pub description: String,
}

/// State for tracking autocomplete suggestions
pub struct AutocompleteState {
    /// Whether autocomplete is currently active
    pub active: bool,
    /// Current filtered suggestions
    pub suggestions: Vec<CommandSuggestion>,
    /// Currently selected index (for arrow navigation)
    pub selected_index: usize,
    /// Current filter text (characters after "/")
    pub filter_text: String,
    /// Command that was just selected (to prevent immediate reactivation)
    pub just_selected_command: Option<String>,
}

impl Default for AutocompleteState {
    fn default() -> Self {
        Self::new()
    }
}

impl AutocompleteState {
    pub fn new() -> Self {
        Self {
            active: false,
            suggestions: Vec::new(),
            selected_index: 0,
            filter_text: String::new(),
            just_selected_command: None,
        }
    }

    pub fn move_selection_up(&mut self) {
        if self.selected_index > 0 {
            self.selected_index -= 1;
        } else if !self.suggestions.is_empty() {
            self.selected_index = self.suggestions.len() - 1; // Wrap around
        }
    }

    pub fn move_selection_down(&mut self) {
        if !self.suggestions.is_empty() {
            self.selected_index = (self.selected_index + 1) % self.suggestions.len();
        }
    }

    pub fn get_selected(&self) -> Option<&CommandSuggestion> {
        self.suggestions.get(self.selected_index)
    }

    pub fn deactivate(&mut self) {
        self.active = false;
        self.suggestions.clear();
        self.selected_index = 0;
        self.filter_text.clear();
        // Note: don't clear just_selected_command here - it's cleared separately
    }

    pub fn deactivate_full(&mut self) {
        self.deactivate();
        self.just_selected_command = None;
    }
}
