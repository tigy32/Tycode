use std::path::PathBuf;

use chrono::Utc;

use crate::chat::actor::ActorState;
use crate::chat::events::{ChatMessage, MessageSender};
use crate::module::SlashCommand;

use super::discovery::SkillsManager;

pub struct SkillsListCommand {
    manager: SkillsManager,
}

impl SkillsListCommand {
    pub fn new(manager: SkillsManager) -> Self {
        Self { manager }
    }
}

#[async_trait::async_trait(?Send)]
impl SlashCommand for SkillsListCommand {
    fn name(&self) -> &'static str {
        "skills"
    }

    fn description(&self) -> &'static str {
        "List and manage available skills"
    }

    fn usage(&self) -> &'static str {
        "/skills [info <name>|reload]"
    }

    async fn execute(&self, _state: &mut ActorState, args: &[&str]) -> Vec<ChatMessage> {
        if args.is_empty() {
            let skills = self.manager.get_all_metadata();

            if skills.is_empty() {
                return vec![create_message(
                    "No skills found. Skills are discovered from (in priority order):\n\
                     - ~/.claude/skills/ (user-level Claude Code compatibility)\n\
                     - ~/.tycode/skills/ (user-level)\n\
                     - .claude/skills/ (project-level Claude Code compatibility)\n\
                     - .tycode/skills/ (project-level, highest priority)\n\n\
                     Each skill should be a directory containing a SKILL.md file."
                        .to_string(),
                    MessageSender::System,
                )];
            }

            let mut message = format!("Available Skills ({} found):\n\n", skills.len());
            for skill in &skills {
                let status = if skill.enabled { "" } else { " [disabled]" };
                message.push_str(&format!(
                    "  {} ({}){}\n    {}\n\n",
                    skill.name, skill.source, status, skill.description
                ));
            }

            message.push_str("Use `/skill <name>` to invoke a skill manually.\n");
            message.push_str("Use `/skills info <name>` to see skill details.\n");
            message.push_str("Use `/skills reload` to re-scan skill directories.");

            return vec![create_message(message, MessageSender::System)];
        }

        match args[0] {
            "info" => self.handle_info(args).await,
            "reload" => {
                self.manager.reload();
                let count = self.manager.get_all_metadata().len();
                vec![create_message(
                    format!("Skills reloaded. Found {} skill(s).", count),
                    MessageSender::System,
                )]
            }
            _ => vec![create_message(
                "Usage: /skills [info <name>|reload]\n\
                 Use `/skills` to list all available skills."
                    .to_string(),
                MessageSender::Error,
            )],
        }
    }
}

impl SkillsListCommand {
    async fn handle_info(&self, args: &[&str]) -> Vec<ChatMessage> {
        if args.len() < 2 {
            return vec![create_message(
                "Usage: /skills info <name>".to_string(),
                MessageSender::Error,
            )];
        }

        let name = args[1];
        match self.manager.get_skill(name) {
            Some(skill) => {
                let mut message = format!("# Skill: {}\n\n", skill.metadata.name);
                message.push_str(&format!("**Source:** {}\n", skill.metadata.source));
                message.push_str(&format!("**Path:** {}\n", skill.metadata.path.display()));
                message.push_str(&format!(
                    "**Status:** {}\n\n",
                    if skill.metadata.enabled {
                        "Enabled"
                    } else {
                        "Disabled"
                    }
                ));
                message.push_str(&format!(
                    "**Description:**\n{}\n\n",
                    skill.metadata.description
                ));
                message.push_str("**Instructions:**\n\n");
                message.push_str(&skill.instructions);

                if !skill.reference_files.is_empty() {
                    message.push_str("\n\n**Reference Files:**\n");
                    message.push_str(&format_path_list(&skill.reference_files));
                }

                if !skill.scripts.is_empty() {
                    message.push_str("\n**Scripts:**\n");
                    message.push_str(&format_path_list(&skill.scripts));
                }

                vec![create_message(message, MessageSender::System)]
            }
            None => vec![create_message(
                format!(
                    "Skill '{}' not found. Use `/skills` to list available skills.",
                    name
                ),
                MessageSender::Error,
            )],
        }
    }
}

pub struct SkillInvokeCommand {
    manager: SkillsManager,
}

impl SkillInvokeCommand {
    pub fn new(manager: SkillsManager) -> Self {
        Self { manager }
    }
}

#[async_trait::async_trait(?Send)]
impl SlashCommand for SkillInvokeCommand {
    fn name(&self) -> &'static str {
        "skill"
    }

    fn description(&self) -> &'static str {
        "Manually invoke a skill"
    }

    fn usage(&self) -> &'static str {
        "/skill <name>"
    }

    async fn execute(&self, _state: &mut ActorState, args: &[&str]) -> Vec<ChatMessage> {
        if args.is_empty() {
            return vec![create_message(
                "Usage: /skill <name>\n\
                 Use `/skills` to list available skills."
                    .to_string(),
                MessageSender::Error,
            )];
        }

        let name = args[0];

        match self.manager.get_skill(name) {
            Some(skill) => {
                if !skill.metadata.enabled {
                    return vec![create_message(
                        format!("Skill '{}' is disabled.", name),
                        MessageSender::Error,
                    )];
                }

                let mut message = format!(
                    "## Skill Invoked: {}\n\n{}\n\n---\n\n**Instructions:**\n\n{}",
                    skill.metadata.name, skill.metadata.description, skill.instructions
                );

                if !skill.reference_files.is_empty() {
                    message.push_str("\n\n**Reference Files:**\n");
                    message.push_str(&format_path_list(&skill.reference_files));
                }

                if !skill.scripts.is_empty() {
                    message.push_str("\n**Scripts:**\n");
                    message.push_str(&format_path_list(&skill.scripts));
                }

                vec![create_message(message, MessageSender::System)]
            }
            None => vec![create_message(
                format!(
                    "Skill '{}' not found. Use `/skills` to list available skills.",
                    name
                ),
                MessageSender::Error,
            )],
        }
    }
}

fn create_message(content: String, sender: MessageSender) -> ChatMessage {
    ChatMessage {
        content,
        sender,
        timestamp: Utc::now().timestamp_millis() as u64,
        reasoning: None,
        tool_calls: Vec::new(),
        model_info: None,
        token_usage: None,
        context_breakdown: None,
        images: vec![],
    }
}

fn format_path_list(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| format!("- {}\n", p.display()))
        .collect()
}
