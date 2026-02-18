use std::collections::HashMap;

use chrono::Utc;

use crate::chat::actor::ActorState;
use crate::chat::events::{ChatMessage, MessageSender};
use crate::module::SlashCommand;
use crate::settings::config::McpServerConfig;

pub struct McpSlashCommand;

#[async_trait::async_trait(?Send)]
impl SlashCommand for McpSlashCommand {
    fn name(&self) -> &'static str {
        "mcp"
    }

    fn description(&self) -> &'static str {
        "Manage MCP server configurations"
    }

    fn usage(&self) -> &'static str {
        "/mcp [add|remove] [args...]"
    }

    async fn execute(&self, state: &mut ActorState, args: &[&str]) -> Vec<ChatMessage> {
        let parts: Vec<String> = std::iter::once("mcp".to_string())
            .chain(args.iter().map(|s| s.to_string()))
            .collect();
        handle_mcp_command(state, &parts).await
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

async fn handle_mcp_command(state: &mut ActorState, parts: &[String]) -> Vec<ChatMessage> {
    if parts.len() < 2 {
        let settings = state.settings.settings();
        if settings.mcp_servers.is_empty() {
            return vec![create_message(
                "No MCP servers configured. Use `/mcp add <name> <command> [--args \"args...\"] [--env \"KEY=VALUE\"]` to add one.".to_string(),
                MessageSender::System,
            )];
        }

        let mut message = String::from("Configured MCP servers:\n\n");
        for (name, config) in &settings.mcp_servers {
            message.push_str(&format!(
                "  {}:\n    Command: {}\n    Args: {}\n    Env: {}\n\n",
                name,
                config.command,
                if config.args.is_empty() {
                    "<none>".to_string()
                } else {
                    config.args.join(" ")
                },
                if config.env.is_empty() {
                    "<none>".to_string()
                } else {
                    config
                        .env
                        .iter()
                        .map(|(k, v)| format!("{}={}", k, v))
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            ));
        }
        return vec![create_message(message, MessageSender::System)];
    }

    match parts[1].as_str() {
        "add" => handle_mcp_add_command(state, parts).await,
        "remove" => handle_mcp_remove_command(state, parts).await,
        _ => vec![create_message(
            "Usage: /mcp [add|remove] [args...]. Use `/mcp` to list all servers.".to_string(),
            MessageSender::Error,
        )],
    }
}

fn parse_mcp_args_value(parts: &[String], i: usize) -> Result<Vec<String>, String> {
    let args_str = parts.get(i + 1).ok_or("--args requires a value")?;
    Ok(args_str.split_whitespace().map(|s| s.to_string()).collect())
}

fn parse_mcp_env_var(parts: &[String], i: usize) -> Result<(String, String), String> {
    let env_str = parts
        .get(i + 1)
        .ok_or("--env requires a value in format KEY=VALUE")?;
    let eq_pos = env_str
        .find('=')
        .ok_or("Environment variable must be in format KEY=VALUE")?;
    let key = env_str[..eq_pos].to_string();
    if key.is_empty() {
        return Err("Environment variable key cannot be empty".to_string());
    }
    let value = env_str[eq_pos + 1..].to_string();
    Ok((key, value))
}

fn process_mcp_optional_args(parts: &[String], config: &mut McpServerConfig) -> Result<(), String> {
    let mut i = 4;
    while i < parts.len() {
        match parts[i].as_str() {
            "--args" => {
                config.args = parse_mcp_args_value(parts, i)?;
                i += 2;
            }
            "--env" => {
                let (key, value) = parse_mcp_env_var(parts, i)?;
                config.env.insert(key, value);
                i += 2;
            }
            arg => return Err(format!("Unknown argument: {}", arg)),
        }
    }
    Ok(())
}

async fn handle_mcp_add_command(state: &mut ActorState, parts: &[String]) -> Vec<ChatMessage> {
    if parts.len() < 4 {
        return vec![create_message(
            "Usage: /mcp add <name> <command> [--args \"args...\"] [--env \"KEY=VALUE\"]"
                .to_string(),
            MessageSender::Error,
        )];
    }

    let name = parts[2].trim().to_string();
    let command = parts[3].trim().to_string();

    if name.is_empty() {
        return vec![create_message(
            "Server name cannot be empty".to_string(),
            MessageSender::Error,
        )];
    }

    if command.is_empty() {
        return vec![create_message(
            "Command path cannot be empty".to_string(),
            MessageSender::Error,
        )];
    }

    let mut config = McpServerConfig {
        command,
        args: Vec::new(),
        env: HashMap::new(),
    };

    if let Err(e) = process_mcp_optional_args(parts, &mut config) {
        return vec![create_message(e, MessageSender::Error)];
    }

    let current_settings = state.settings.settings();
    let replacing = current_settings.mcp_servers.contains_key(&name);

    state.settings.update_setting(|settings| {
        settings.mcp_servers.insert(name.clone(), config.clone());
    });

    if let Err(e) = state.settings.save() {
        return vec![create_message(
            format!("MCP server updated for this session but failed to save settings: {e:?}"),
            MessageSender::Error,
        )];
    }

    let connection_status = match state.mcp_manager.add_server(name.clone(), config).await {
        Ok(()) => "\nServer connected successfully.".to_string(),
        Err(e) => format!(
            "\nWarning: Failed to connect to server: {e:?}. Server will be retried on next session."
        ),
    };

    let response = if replacing {
        format!("Updated MCP server '{name}'")
    } else {
        format!("Added MCP server '{name}'")
    };

    vec![create_message(
        format!(
            "{}{}\n\nSettings saved to disk. The MCP server configuration is now persistent across sessions.",
            response, connection_status
        ),
        MessageSender::System,
    )]
}

async fn handle_mcp_remove_command(state: &mut ActorState, parts: &[String]) -> Vec<ChatMessage> {
    if parts.len() < 3 {
        return vec![create_message(
            "Usage: /mcp remove <name>".to_string(),
            MessageSender::Error,
        )];
    }

    let name = parts[2].trim();

    if name.is_empty() {
        return vec![create_message(
            "Server name cannot be empty".to_string(),
            MessageSender::Error,
        )];
    }

    let current_settings = state.settings.settings();
    if !current_settings.mcp_servers.contains_key(name) {
        return vec![create_message(
            format!("MCP server '{name}' not found"),
            MessageSender::Error,
        )];
    }

    state.settings.update_setting(|settings| {
        settings.mcp_servers.remove(name);
    });

    if let Err(e) = state.settings.save() {
        return vec![create_message(
            format!("MCP server removed for this session but failed to save settings: {e:?}"),
            MessageSender::Error,
        )];
    }

    vec![create_message(
        format!(
            "Removed MCP server '{name}'\n\nSettings saved to disk. The MCP server configuration is now persistent across sessions."
        ),
        MessageSender::System,
    )]
}
