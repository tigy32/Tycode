use tycode_core::chat::commands::{get_available_commands, CommandInfo};

use super::CommandSuggestion;

pub struct CommandCompleter {
    commands: Vec<CommandInfo>,
}

impl Default for CommandCompleter {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandCompleter {
    pub fn new() -> Self {
        // Filter out hidden commands
        let commands: Vec<CommandInfo> = get_available_commands()
            .into_iter()
            .filter(|cmd| !cmd.hidden)
            .collect();

        Self { commands }
    }

    /// Filter commands based on partial input (characters after "/")
    /// Returns all non-hidden commands when filter is empty
    pub fn filter(&self, filter: &str) -> Vec<CommandSuggestion> {
        let filter_lower = filter.to_lowercase();

        self.commands
            .iter()
            .filter(|cmd| {
                // Filter on command name only (not description)
                filter.is_empty() || cmd.name.to_lowercase().starts_with(&filter_lower)
            })
            .map(|cmd| CommandSuggestion {
                name: cmd.name.clone(),
                description: cmd.description.clone(),
            })
            .collect()
    }
}
