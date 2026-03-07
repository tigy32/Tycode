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
                "No MCP servers configured.\n\n\
                 Usage:\n  \
                 /mcp add <name> <command> [--args \"args...\"] [--env \"KEY=VALUE\"]\n  \
                 /mcp add <name> --url <url> [--header \"Name: Value\"]"
                    .to_string(),
                MessageSender::System,
            )];
        }

        let mut message = String::from("Configured MCP servers:\n\n");
        for (name, config) in &settings.mcp_servers {
            match config {
                McpServerConfig::Stdio { command, args, env } => {
                    message.push_str(&format!(
                        "  {}:\n    Type: stdio\n    Command: {}\n    Args: {}\n    Env: {}\n\n",
                        name,
                        command,
                        if args.is_empty() {
                            "<none>".to_string()
                        } else {
                            args.join(" ")
                        },
                        if env.is_empty() {
                            "<none>".to_string()
                        } else {
                            env.iter()
                                .map(|(k, v)| format!("{}={}", k, v))
                                .collect::<Vec<_>>()
                                .join(", ")
                        }
                    ));
                }
                McpServerConfig::Http { url, headers } => {
                    message.push_str(&format!(
                        "  {}:\n    Type: http\n    URL: {}\n    Headers: {}\n\n",
                        name,
                        url,
                        if headers.is_empty() {
                            "<none>".to_string()
                        } else {
                            format!("{} configured", headers.len())
                        }
                    ));
                }
            }
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

const ADD_USAGE: &str = "Usage:\n  \
    /mcp add <name> <command> [--args \"args...\"] [--env \"KEY=VALUE\"]\n  \
    /mcp add <name> --url <url> [--header \"Name: Value\"]";

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

fn parse_header_value(parts: &[String], i: usize) -> Result<(String, String), String> {
    let header_str = parts
        .get(i + 1)
        .ok_or("--header requires a value in format \"Name: Value\"")?;
    let colon_pos = header_str
        .find(':')
        .ok_or("Header must be in format \"Name: Value\"")?;
    let key = header_str[..colon_pos].trim().to_string();
    if key.is_empty() {
        return Err("Header name cannot be empty".to_string());
    }
    let value = header_str[colon_pos + 1..].trim().to_string();
    Ok((key, value))
}

fn parse_stdio_config(parts: &[String]) -> Result<McpServerConfig, String> {
    let command = parts[3].trim().to_string();
    if command.is_empty() {
        return Err("Command path cannot be empty".to_string());
    }

    let mut args = Vec::new();
    let mut env = HashMap::new();

    let mut i = 4;
    while i < parts.len() {
        match parts[i].as_str() {
            "--args" => {
                args = parse_mcp_args_value(parts, i)?;
                i += 2;
            }
            "--env" => {
                let (key, value) = parse_mcp_env_var(parts, i)?;
                env.insert(key, value);
                i += 2;
            }
            arg => return Err(format!("Unknown argument: {}", arg)),
        }
    }

    Ok(McpServerConfig::Stdio { command, args, env })
}

fn parse_http_config(parts: &[String]) -> Result<McpServerConfig, String> {
    let url = parts
        .get(4)
        .ok_or("--url requires a URL value")?
        .trim()
        .to_string();
    if url.is_empty() {
        return Err("URL cannot be empty".to_string());
    }

    let mut headers = HashMap::new();
    let mut i = 5;
    while i < parts.len() {
        match parts[i].as_str() {
            "--header" => {
                let (key, value) = parse_header_value(parts, i)?;
                headers.insert(key, value);
                i += 2;
            }
            arg => return Err(format!("Unknown argument: {}", arg)),
        }
    }

    Ok(McpServerConfig::Http { url, headers })
}

async fn handle_mcp_add_command(state: &mut ActorState, parts: &[String]) -> Vec<ChatMessage> {
    if parts.len() < 4 {
        return vec![create_message(ADD_USAGE.to_string(), MessageSender::Error)];
    }

    let name = parts[2].trim().to_string();
    if name.is_empty() {
        return vec![create_message(
            "Server name cannot be empty".to_string(),
            MessageSender::Error,
        )];
    }

    // Detect HTTP vs stdio: if the third positional arg is --url, parse as HTTP
    let config = if parts[3] == "--url" {
        match parse_http_config(parts) {
            Ok(c) => c,
            Err(e) => return vec![create_message(e, MessageSender::Error)],
        }
    } else {
        match parse_stdio_config(parts) {
            Ok(c) => c,
            Err(e) => return vec![create_message(e, MessageSender::Error)],
        }
    };

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
