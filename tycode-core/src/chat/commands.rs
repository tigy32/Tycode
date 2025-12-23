use crate::agents::agent::ActiveAgent;
use crate::agents::catalog::AgentCatalog;
use crate::ai::model::{Model, ModelCost};
use crate::ai::{
    Content, Message, MessageRole, ModelSettings, ReasoningBudget, TokenUsage, ToolUseData,
};
use crate::chat::actor::{create_provider, resume_session, TimingStat};
use crate::chat::ai::select_model_for_agent;
use crate::chat::tools::{current_agent, current_agent_mut};
use crate::chat::{
    actor::ActorState,
    events::{
        ChatEvent, ChatMessage, ContextInfo, MessageSender, ModelInfo, ToolExecutionResult,
        ToolRequest, ToolRequestType,
    },
};
use crate::security::SecurityMode;
use crate::settings::config::FileModificationApi;
use crate::settings::config::{McpServerConfig, ProviderConfig, ReviewLevel};
use chrono::Utc;
use dirs;
use serde_json::json;
use std::collections::HashMap;
use std::fs;
use toml;

use crate::chat::context::build_message_context;
use crate::persistence::storage;

/// Parse a command string respecting quoted strings.
/// This is a simple shell-like parser that handles double quotes.
///
/// Examples:
/// - `foo bar baz` -> ["foo", "bar", "baz"]
/// - `foo "bar baz"` -> ["foo", "bar baz"]
/// - `foo "bar \"quoted\" baz"` -> ["foo", "bar \"quoted\" baz"]
fn parse_command_with_quotes(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
            }
            ' ' | '\t' if !in_quotes => {
                if !current.is_empty() {
                    parts.push(current.clone());
                    current.clear();
                }
            }
            '\\' if in_quotes => {
                // Handle escape sequences in quotes
                if let Some(&next) = chars.peek() {
                    if next == '"' || next == '\\' {
                        chars.next();
                        current.push(next);
                    } else {
                        current.push(c);
                    }
                } else {
                    current.push(c);
                }
            }
            _ => {
                current.push(c);
            }
        }
    }

    if !current.is_empty() {
        parts.push(current);
    }

    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_command_with_quotes_basic() {
        let result = parse_command_with_quotes("foo bar baz");
        assert_eq!(result, vec!["foo", "bar", "baz"]);
    }

    #[test]
    fn test_parse_command_with_quotes_quoted_string() {
        let result = parse_command_with_quotes(r#"foo "bar baz" qux"#);
        assert_eq!(result, vec!["foo", "bar baz", "qux"]);
    }

    #[test]
    fn test_parse_command_with_quotes_multiple_quoted() {
        let result = parse_command_with_quotes(r#"cmd "arg one" "arg two" normal"#);
        assert_eq!(result, vec!["cmd", "arg one", "arg two", "normal"]);
    }

    #[test]
    fn test_parse_command_with_quotes_escaped_quotes() {
        let result = parse_command_with_quotes(r#"cmd "say \"hello\"""#);
        assert_eq!(result, vec!["cmd", r#"say "hello""#]);
    }

    #[test]
    fn test_parse_command_with_quotes_empty() {
        let result = parse_command_with_quotes("");
        assert_eq!(result, Vec::<String>::new());
    }

    #[test]
    fn test_parse_command_with_quotes_only_spaces() {
        let result = parse_command_with_quotes("   ");
        assert_eq!(result, Vec::<String>::new());
    }

    #[test]
    fn test_parse_command_with_quotes_mcp_example() {
        let result = parse_command_with_quotes(
            r#"mcp add server /path/to/cmd --args "arg1 arg2" --env KEY=value"#,
        );
        assert_eq!(
            result,
            vec![
                "mcp",
                "add",
                "server",
                "/path/to/cmd",
                "--args",
                "arg1 arg2",
                "--env",
                "KEY=value"
            ]
        );
    }

    #[test]
    fn test_parse_command_with_quotes_env_with_spaces() {
        let result = parse_command_with_quotes(r#"mcp add server cmd --env "MESSAGE=hello world""#);
        assert_eq!(
            result,
            vec![
                "mcp",
                "add",
                "server",
                "cmd",
                "--env",
                "MESSAGE=hello world"
            ]
        );
    }

    #[test]
    fn test_parse_command_with_quotes_multiple_env() {
        let result = parse_command_with_quotes(
            r#"mcp add srv cmd --env KEY1=val1 --env "KEY2=val with spaces""#,
        );
        assert_eq!(
            result,
            vec![
                "mcp",
                "add",
                "srv",
                "cmd",
                "--env",
                "KEY1=val1",
                "--env",
                "KEY2=val with spaces"
            ]
        );
    }
}

#[derive(Clone, Debug)]
pub struct CommandInfo {
    pub name: String,
    pub description: String,
    pub usage: String,
    pub hidden: bool,
}

/// Process a command and directly mutate the actor state
pub async fn process_command(state: &mut ActorState, command: &str) -> Vec<ChatMessage> {
    let parts: Vec<&str> = command.split_whitespace().collect();
    if parts.is_empty() {
        return vec![];
    }

    match parts[0] {
        "clear" => handle_clear_command(state).await,
        "context" => handle_context_command(state).await,
        "fileapi" => handle_fileapi_command(state, &parts).await,
        "model" => handle_model_command(state, &parts).await,
        "settings" => handle_settings_command(state, &parts).await,
        "security" => handle_security_command(state, &parts).await,
        "agentmodel" => handle_agentmodel_command(state, &parts).await,
        "agent" => handle_agent_command(state, &parts).await,
        "review_level" => handle_review_level_command(state, &parts).await,
        "cost" => handle_cost_command_with_subcommands(state, &parts).await,
        "mcp" => {
            // MCP commands need quoted string parsing for args and env vars
            let parts_owned = parse_command_with_quotes(command);
            handle_mcp_command(state, &parts_owned).await
        }
        "help" => handle_help_command().await,
        "models" => handle_models_command(state).await,
        "provider" => handle_provider_command(state, &parts).await,
        "profile" => handle_profile_command(state, &parts).await,
        "sessions" => handle_sessions_command(state, &parts).await,
        "debug_ui" => handle_debug_ui_command(state).await,
        "memory" => handle_memory_command(state, &parts).await,
        _ => vec![create_message(
            format!("Unknown command: /{}", parts[0]),
            MessageSender::Error,
        )],
    }
}

/// Check if the given input string starts with a known command
pub fn is_known_command(input: &str) -> bool {
    let first_word = input.split_whitespace().next().unwrap_or("");
    let commands = get_available_commands();
    commands.iter().any(|cmd| cmd.name == first_word)
}

/// Get all available commands with their descriptions
pub fn get_available_commands() -> Vec<CommandInfo> {
    vec![
        CommandInfo {
            name: "clear".to_string(),
            description: r"Clear the conversation history".to_string(),
            usage: "/clear".to_string(),
            hidden: false,
        },
        CommandInfo {
            name: "context".to_string(),
            description: r"Show what files would be included in the AI context".to_string(),
            usage: "/context".to_string(),
            hidden: false,
        },
        CommandInfo {
            name: "fileapi".to_string(),
            description: r"Set the file modification API (patch or find-replace)".to_string(),
            usage: "/fileapi <patch|findreplace>".to_string(),
            hidden: false,
        },
        CommandInfo {
            name: r"model".to_string(),
            description: r"Set the AI model for all agents".to_string(),
            usage: r"/model <name> [temperature=0.7] [max_tokens=4096] [top_p=1.0] [reasoning_budget=...]".to_string(),
            hidden: false,
        },
        CommandInfo {
            name: "trace".to_string(),
            description: r"Enable/disable trace logging to .tycode/trace".to_string(),
            usage: "/trace <on|off>".to_string(),
            hidden: false,
        },
        CommandInfo {
            name: "settings".to_string(),
            description: "Display current settings and configuration".to_string(),
            usage: "/settings or /settings save".to_string(),
            hidden: false,
        },
        CommandInfo {
            name: "security".to_string(),
            description: "Manage security mode and permissions".to_string(),
            usage: "/security [mode|whitelist|clear] [args...]".to_string(),
            hidden: false,
        },
        CommandInfo {
            name: "cost".to_string(),
            description: "Show session token usage and estimated cost, or set model cost limit".to_string(),
            usage: "/cost [set <free|low|medium|high|unlimited>]".to_string(),
            hidden: false,
        },
        CommandInfo {
            name: "help".to_string(),
            description: "Show this help message".to_string(),
            usage: "/help".to_string(),
            hidden: false,
        },
        CommandInfo {
            name: "models".to_string(),
            description: "List available AI models".to_string(),
            usage: "/models".to_string(),
            hidden: false,
        },
        CommandInfo {
            name: "provider".to_string(),
            description: "List, switch, or add AI providers".to_string(),
            usage: "/provider [name] | /provider add <name> <type> [args]".to_string(),
            hidden: false,
        },
        CommandInfo {
            name: "agentmodel".to_string(),
            description: "Set the AI model for a specific agent with tunings".to_string(),
            usage: "/agentmodel <agent_name> <model_name> [temperature=0.7] [max_tokens=4096] [top_p=1.0] [reasoning_budget=...]".to_string(),
            hidden: false,
        },
        CommandInfo {
            name: "agent".to_string(),
            description: "Switch the current agent".to_string(),
            usage: "/agent <name>".to_string(),
            hidden: false,
        },
        CommandInfo {
            name: "review_level".to_string(),
            description: "Set the review level (None, Task)".to_string(),
            usage: "/review_level <none|task>".to_string(),
            hidden: false,
        },
        CommandInfo {
            name: "mcp".to_string(),
            description: "Manage MCP server configurations".to_string(),
            usage: "/mcp [add|remove] [args...]".to_string(),
            hidden: false,
        },
        CommandInfo {
            name: "quit".to_string(),
            description: "Exit the application".to_string(),
            usage: "/quit or /exit".to_string(),
            hidden: false,
        },
        CommandInfo {
            name: "profile".to_string(),
            description: "Manage settings profiles (switch, save, list, show current)".to_string(),
            usage: "/profile [switch|save|list|show] [<name>]".to_string(),
            hidden: false,
        },
        CommandInfo {
            name: "sessions".to_string(),
            description: "Manage conversation sessions (list, resume, delete, gc)".to_string(),
            usage: "/sessions [list|resume <id>|delete <id>|gc [days]]".to_string(),
            hidden: false,
        },
        CommandInfo {
            name: "debug_ui".to_string(),
            description: "Internal: Test UI components without AI calls".to_string(),
            usage: "/debug_ui".to_string(),
            hidden: true,
        },
        CommandInfo {
            name: "memory".to_string(),
            description: "Manage memories (summarize)".to_string(),
            usage: "/memory summarize".to_string(),
            hidden: false,
        },
    ]
}

async fn handle_clear_command(state: &mut ActorState) -> Vec<ChatMessage> {
    state.clear_conversation();
    current_agent_mut(state).conversation.clear();
    vec![create_message(
        "Conversation cleared.".to_string(),
        MessageSender::System,
    )]
}

async fn handle_context_command(state: &ActorState) -> Vec<ChatMessage> {
    let tracked_files: Vec<_> = state.tracked_files.iter().cloned().collect();
    let context = match build_message_context(
        &state.workspace_roots,
        &tracked_files,
        state.task_list.clone(),
        state.last_command_outputs.clone(),
        state.settings.settings().auto_context_bytes,
    )
    .await
    {
        Ok(ctx) => ctx,
        Err(e) => {
            return vec![create_message(
                format!("Failed to build context: {}", e),
                MessageSender::Error,
            )];
        }
    };
    vec![create_message(
        context.to_formatted_string(true),
        MessageSender::System,
    )]
}

async fn handle_fileapi_command(state: &mut ActorState, parts: &[&str]) -> Vec<ChatMessage> {
    if let Some(api_name) = parts.get(1) {
        match api_name.to_lowercase().as_str() {
            "patch" => {
                state
                    .settings
                    .update_setting(|s| s.file_modification_api = FileModificationApi::Patch);
                vec![create_message(
                    "File modification API set to: patch".to_string(),
                    MessageSender::System,
                )]
            }
            "findreplace" | "find-replace" => {
                state
                    .settings
                    .update_setting(|s| s.file_modification_api = FileModificationApi::FindReplace);
                vec![create_message(
                    "File modification API set to: find-replace".to_string(),
                    MessageSender::System,
                )]
            }
            _ => vec![create_message(
                "Unknown file API. Use: patch, findreplace".to_string(),
                MessageSender::Error,
            )],
        }
    } else {
        let current_api = match state.settings.settings().file_modification_api {
            FileModificationApi::Patch => "patch",
            FileModificationApi::FindReplace => "find-replace",
            FileModificationApi::Default => "default",
        };
        vec![create_message(
            format!(
                "Current file modification API: {current_api}. Usage: /fileapi <patch|findreplace>"
            ),
            MessageSender::System,
        )]
    }
}

async fn handle_settings_command(state: &ActorState, parts: &[&str]) -> Vec<ChatMessage> {
    let settings = state.settings.settings();

    if parts.is_empty() || (parts.len() == 1) {
        let content = match toml::to_string_pretty(&settings) {
            Ok(c) => c,
            Err(e) => {
                return vec![create_message(
                    format!("Failed to serialize settings: {}", e),
                    MessageSender::Error,
                )];
            }
        };

        let message = format!("=== Current Settings ===\n\n{}", content);

        vec![create_message(message, MessageSender::System)]
    } else if parts.len() > 1 && parts[1] == "save" {
        match state.settings.save() {
            Ok(()) => vec![create_message(
                "Settings saved to disk successfully.".to_string(),
                MessageSender::System,
            )],
            Err(e) => vec![create_message(
                format!("Failed to save settings: {e}"),
                MessageSender::Error,
            )],
        }
    } else {
        vec![create_message(
            format!("Unknown arguments: {parts:?}"),
            MessageSender::Error,
        )]
    }
}

async fn handle_security_command(state: &mut ActorState, parts: &[&str]) -> Vec<ChatMessage> {
    if parts.len() == 1 {
        let current_mode = state.settings.get_mode();
        return vec![create_message(
            format!("Current security mode: {:?}", current_mode),
            MessageSender::System,
        )];
    }

    if parts.len() == 3 && parts[1] == "set" {
        let mode_str = parts[2].to_lowercase();
        let mode = match mode_str.as_str() {
            "readonly" => SecurityMode::ReadOnly,
            "auto" => SecurityMode::Auto,
            "all" => SecurityMode::All,
            _ => {
                return vec![create_message(
                    "Invalid mode. Valid options: readonly, auto, all".to_string(),
                    MessageSender::Error,
                )];
            }
        };

        state.settings.set_mode(mode);

        return vec![create_message(
            format!(
                "Security mode set to: {:?}.\n\nSettings updated for this session. Call `/settings save` to use these settings as default for all future sessions.",
                mode
            ),
            MessageSender::System,
        )];
    }

    vec![create_message(
        "Usage: /security [set <readonly|auto|all>]".to_string(),
        MessageSender::Error,
    )]
}

async fn handle_cost_command_with_subcommands(
    state: &mut ActorState,
    parts: &[&str],
) -> Vec<ChatMessage> {
    if parts.len() >= 3 && parts[1] == "set" {
        let level_str = parts[2];
        let new_level = match ModelCost::try_from(level_str) {
            Ok(level) => level,
            Err(e) => {
                return vec![create_message(
                    format!("Invalid cost level. {}", e),
                    MessageSender::Error,
                )];
            }
        };

        state
            .settings
            .update_setting(|s| s.model_quality = Some(new_level));

        return vec![create_message(
            format!("Model cost level set to: {:?}.\n\nSettings updated for this session. Call `/settings save` to use these settings as default for all future sessions.", new_level),
            MessageSender::System,
        )];
    } else if parts.len() >= 2 && parts[1] == "set" {
        // Insufficient args for set
        return vec![create_message(
            "Usage: /cost set <free|low|medium|high|unlimited>".to_string(),
            MessageSender::Error,
        )];
    }

    // Default: show cost summary
    handle_cost_command(&state).await
}

async fn handle_cost_command(state: &ActorState) -> Vec<ChatMessage> {
    let usage = &state.session_token_usage;
    let current = current_agent(state);
    let settings_snapshot = state.settings.settings();
    let model_settings = select_model_for_agent(
        &settings_snapshot,
        state.provider.as_ref(),
        current.agent.name(),
    )
    .unwrap_or_else(|_| Model::None.default_settings());
    let current_model = model_settings.model;
    let total_input_tokens = usage.input_tokens + usage.cache_creation_input_tokens.unwrap_or(0);

    let mut message = String::new();
    message.push_str("=== Session Cost Summary ===\n\n");
    message.push_str(&format!("Current Model: {current_model:?}\n"));
    message.push_str(&format!("Provider: {}\n\n", state.provider.name()));

    message.push_str("Token Usage:\n");
    message.push_str(&format!("  Input tokens:  {:>8}\n", total_input_tokens));
    message.push_str(&format!("  Output tokens: {:>8}\n", usage.output_tokens));
    message.push_str(&format!("  Total tokens:  {:>8}\n\n", usage.total_tokens));

    // Add breakdown if there are cache-related tokens
    if usage.cache_creation_input_tokens.unwrap_or(0) > 0
        || usage.cached_prompt_tokens.unwrap_or(0) > 0
    {
        message.push_str("Token Breakdown:\n");
        message.push_str(&format!(
            "  Base input tokens:            {:>8}\n",
            usage.input_tokens
        ));
        if let Some(cache_creation) = usage.cache_creation_input_tokens {
            if cache_creation > 0 {
                message.push_str(&format!(
                    "  Cache creation input tokens:  {:>8}\n",
                    cache_creation
                ));
            }
        }
        if let Some(cached) = usage.cached_prompt_tokens {
            if cached > 0 {
                message.push_str(&format!("  Cached prompt tokens:         {:>8}\n", cached));
            }
        }
        message.push_str("\n");
    }

    message.push_str("Accumulated Cost:\n");
    message.push_str(&format!("  Total cost: ${:.6}\n", state.session_cost));

    if usage.total_tokens > 0 {
        let avg_cost_per_1k = (state.session_cost / usage.total_tokens as f64) * 1000.0;
        message.push_str(&format!("  Average per 1K tokens: ${avg_cost_per_1k:.6}\n"));
    }

    let TimingStat {
        waiting_for_human,
        ai_processing,
        tool_execution,
    } = state.timing_stats.session();
    let total_time = waiting_for_human + ai_processing + tool_execution;
    message.push_str("\nTime Spent:\n");
    message.push_str(&format!(
        "  Waiting for human: {:>6.1}s\n",
        waiting_for_human.as_secs_f64()
    ));
    message.push_str(&format!(
        "  AI processing:     {:>6.1}s\n",
        ai_processing.as_secs_f64()
    ));
    message.push_str(&format!(
        "  Tool execution:    {:>6.1}s\n",
        tool_execution.as_secs_f64()
    ));
    message.push_str(&format!(
        "  Total session:     {:>6.1}s\n",
        total_time.as_secs_f64()
    ));

    vec![create_message(message, MessageSender::System)]
}

async fn handle_help_command() -> Vec<ChatMessage> {
    let commands = get_available_commands();
    let mut message = String::from("Available commands:\n\n");

    for cmd in commands {
        if !cmd.hidden {
            message.push_str(&format!("/{} - {}\n", cmd.name, cmd.description));
            message.push_str(&format!("  Usage: {}\n\n", cmd.usage));
        }
    }
    vec![create_message(message, MessageSender::System)]
}

async fn handle_models_command(state: &ActorState) -> Vec<ChatMessage> {
    let models = state.provider.supported_models();
    let model_names: Vec<String> = if models.is_empty() {
        vec![Model::GrokCodeFast1.name().to_string()]
    } else {
        models.iter().map(|m| m.name().to_string()).collect()
    };
    let response = model_names.join(", ");
    vec![create_message(response, MessageSender::System)]
}

async fn handle_model_command(state: &mut ActorState, parts: &[&str]) -> Vec<ChatMessage> {
    if parts.len() < 2 {
        return vec![create_message(
            "Usage: /model <name> [key=value...]\nValid keys: temperature, max_tokens, top_p, reasoning_budget\nUse /models to list available models.".to_string(),
            MessageSender::System,
        )];
    }

    let model_name = parts[1];
    let model = match Model::from_name(model_name) {
        Some(m) => m,
        None => {
            return vec![create_message(
                format!("Unknown model: {model_name}. Use /models to list available models."),
                MessageSender::Error,
            )];
        }
    };

    let settings = match parse_model_settings_overrides(&model, &parts[2..]) {
        Ok(s) => s,
        Err(e) => return vec![create_message(e, MessageSender::Error)],
    };

    // Set for all agents
    let agent_names: Vec<String> = AgentCatalog::get_agent_names();
    for agent_name in agent_names {
        state
            .settings
            .update_setting(|s| s.set_agent_model(agent_name, settings.clone()));
    }

    // Success message
    let mut overrides = Vec::new();
    if settings.temperature.is_some() {
        overrides.push(format!("temperature={}", settings.temperature.unwrap()));
    }
    if settings.max_tokens.is_some() {
        overrides.push(format!("max_tokens={}", settings.max_tokens.unwrap()));
    }
    if settings.top_p.is_some() {
        overrides.push(format!("top_p={}", settings.top_p.unwrap()));
    }
    overrides.push(format!("reasoning_budget={}", settings.reasoning_budget));

    let overrides_str = if overrides.is_empty() {
        "".to_string()
    } else {
        format!(" (with {})", overrides.join(", "))
    };

    vec![create_message(
        format!(
            "Model successfully set to {} for all agents{}.\n\nSettings updated for this session. Call `/settings save` to use these settings as default for all future sessions.",
            model.name(),
            overrides_str
        ),
        MessageSender::System,
    )]
}

async fn handle_agentmodel_command(state: &mut ActorState, parts: &[&str]) -> Vec<ChatMessage> {
    if parts.len() < 3 {
        return vec![create_message(format!("Usage: /agentmodel <agent_name> <model_name> [temperature=0.7] [max_tokens=4096] [top_p=1.0] [reasoning_budget=...]\nValid agents: {}", AgentCatalog::get_agent_names().join(", ")), MessageSender::System)];
    }
    let agent_name = parts[1];
    if !AgentCatalog::get_agent_names().contains(&agent_name.to_string()) {
        return vec![create_message(
            format!(
                "Unknown agent: {}. Valid agents: {}",
                agent_name,
                AgentCatalog::get_agent_names().join(", ")
            ),
            MessageSender::Error,
        )];
    }
    let model_name = parts[2];
    let model = match Model::from_name(model_name) {
        Some(m) => m,
        None => {
            return vec![create_message(
                format!("Unknown model: {model_name}. Use /models to list available models."),
                MessageSender::Error,
            )]
        }
    };
    let settings = match parse_model_settings_overrides(&model, &parts[3..]) {
        Ok(s) => s,
        Err(e) => return vec![create_message(e, MessageSender::Error)],
    };
    state
        .settings
        .update_setting(|s| s.set_agent_model(agent_name.to_string(), settings.clone()));
    // Collect overrides for message
    let mut overrides = Vec::new();
    if let Some(v) = settings.temperature {
        overrides.push(format!("temperature={v}"));
    }
    if let Some(v) = settings.max_tokens {
        overrides.push(format!("max_tokens={v}"));
    }
    if let Some(v) = settings.top_p {
        overrides.push(format!("top_p={v}"));
    }
    overrides.push(format!("reasoning_budget={}", settings.reasoning_budget));

    let overrides_str = if overrides.is_empty() {
        "".to_string()
    } else {
        format!(" (with {})", overrides.join(", "))
    };
    vec![create_message(
        format!(
            "Model successfully set to {} for agent {}{}.\n\nSettings updated for this session. Call `/settings save` to use these settings as default for all future sessions.",
            model.name(),
            agent_name,
            overrides_str
        ),
        MessageSender::System,
    )]
}

fn parse_model_settings_overrides(
    model: &Model,
    overrides: &[&str],
) -> Result<ModelSettings, String> {
    let mut settings = model.default_settings();
    for &arg in overrides {
        let eq_pos = arg
            .find('=')
            .ok_or(format!("Invalid argument: {arg}. Expected key=value"))?;
        let key = &arg[..eq_pos];
        let value_str = &arg[eq_pos + 1..];
        match key {
            "temperature" => {
                let v: f32 = value_str.parse().map_err(|_| format!("Invalid temperature value: {value_str}. Expected a float (e.g., 0.7)."))?;
                settings.temperature = Some(v);
            }
            "max_tokens" => {
                let v: u32 = value_str.parse().map_err(|_| format!("Invalid max_tokens value: {value_str}. Expected a positive integer (e.g., 4096)."))?;
                settings.max_tokens = Some(v);
            }
            "top_p" => {
                let v: f32 = value_str.parse().map_err(|_| format!("Invalid top_p value: {value_str}. Expected a float (e.g., 1.0)."))?;
                settings.top_p = Some(v);
            }
            "reasoning_budget" => {
                let reasoning_budget = match value_str {
                    "High" | "high" => ReasoningBudget::High,
                    "Low" | "low" => ReasoningBudget::Low,
                    "Off" | "off" => ReasoningBudget::Off,
                    _ => return Err("Unsupported reasoning budget - must be one of high low or off".to_string())
                };
                settings.reasoning_budget = reasoning_budget;
            }
            _ => return Err(format!("Unknown parameter: {key}. Valid parameters: temperature, max_tokens, top_p, reasoning_budget")),
        }
    }
    Ok(settings)
}

fn create_message(content: String, sender: MessageSender) -> ChatMessage {
    ChatMessage {
        content,
        sender,
        timestamp: Utc::now().timestamp_millis() as u64,
        reasoning: None,
        tool_calls: Vec::new(),
        model_info: None,
        context_info: None,
        token_usage: None,
    }
}

async fn handle_agent_command(state: &mut ActorState, parts: &[&str]) -> Vec<ChatMessage> {
    if parts.len() < 2 {
        return vec![create_message(
            format!(
                "Usage: /agent <name>. Valid agents: {}",
                AgentCatalog::get_agent_names().join(", ")
            ),
            MessageSender::System,
        )];
    }

    let agent_name = parts[1];

    if !AgentCatalog::get_agent_names().contains(&agent_name.to_string()) {
        return vec![create_message(
            format!(
                "Unknown agent: {}. Valid agents: {}",
                agent_name,
                AgentCatalog::get_agent_names().join(", ")
            ),
            MessageSender::System,
        )];
    }

    let had_sub_agents = state.agent_stack.len() > 1;

    if state.agent_stack.len() == 1 && current_agent(state).agent.name() == agent_name {
        return vec![create_message(
            format!("Already switched to agent: {agent_name}"),
            MessageSender::System,
        )];
    }

    let mut merged_conversation = Vec::new();
    let agent_count = state.agent_stack.len();
    for (index, active_agent) in state.agent_stack.iter().enumerate() {
        merged_conversation.extend(active_agent.conversation.clone());

        // Add delimiter between agent conversations to preserve context awareness
        if agent_count > 1 && index < agent_count - 1 {
            merged_conversation.push(Message {
                role: MessageRole::Assistant,
                content: Content::text_only(format!(
                    "[Context transition: The above is from the {} agent. Sub-agent context follows. All prior conversation history remains relevant.]",
                    active_agent.agent.name()
                )),
            });
        }
    }

    let new_agent_dyn = AgentCatalog::create_agent(agent_name).unwrap();
    let mut new_root_agent = crate::agents::agent::ActiveAgent::new(new_agent_dyn);
    new_root_agent.conversation = merged_conversation;

    state.agent_stack.clear();
    state.agent_stack.push(new_root_agent);

    let suffix = if had_sub_agents {
        " (sub-agent conversations merged)"
    } else {
        ""
    };

    vec![create_message(
        format!("Switched to agent: {agent_name}{suffix}"),
        MessageSender::System,
    )]
}

async fn handle_review_level_command(state: &mut ActorState, parts: &[&str]) -> Vec<ChatMessage> {
    if parts.len() < 2 {
        // Show current review level
        let current_level = &state.settings.settings().review_level;
        return vec![create_message(
            format!("Current review level: {current_level:?}"),
            MessageSender::System,
        )];
    }

    // Parse the review level from the command
    let level_str = parts[1].to_lowercase();
    let new_level = match level_str.as_str() {
        "none" => ReviewLevel::None,
        "task" => ReviewLevel::Task,
        _ => {
            return vec![create_message(
                "Invalid review level. Valid options: none, task".to_string(),
                MessageSender::Error,
            )]
        }
    };

    // Update the setting
    state
        .settings
        .update_setting(|s| s.review_level = new_level.clone());

    vec![create_message(
        format!("Review level set to: {:?}.\n\nSettings updated for this session. Call `/settings save` to use these settings as default for all future sessions.", new_level),
        MessageSender::System,
    )]
}

async fn handle_provider_command(state: &mut ActorState, parts: &[&str]) -> Vec<ChatMessage> {
    if parts.len() < 2 {
        let settings = state.settings.settings();
        let providers = settings.list_providers();
        let current_provider = state.provider.name();

        let mut message = String::new();
        message.push_str("Available providers:\n\n");

        for provider in providers {
            if provider == current_provider {
                message.push_str(&format!("  {provider} (active)\n"));
            } else {
                message.push_str(&format!("  {provider}\n"));
            }
        }

        return vec![create_message(message, MessageSender::System)];
    }

    if parts[1].eq_ignore_ascii_case("add") {
        return handle_provider_add_command(state, parts).await;
    }

    let provider_name = parts[1];

    // Create new provider instance
    let new_provider = match create_provider(&state.settings, provider_name).await {
        Ok(provider) => provider,
        Err(e) => {
            return vec![create_message(
                format!("Failed to create provider '{provider_name}': {e}"),
                MessageSender::Error,
            )];
        }
    };

    // Update the active provider in memory (but don't save to disk)
    state.provider = new_provider;
    state.settings.update_setting(|settings| {
        settings.active_provider = Some(provider_name.to_string());
    });

    vec![create_message(
        format!("Active provider changed to: {provider_name}"),
        MessageSender::System,
    )]
}

async fn handle_provider_add_command(state: &mut ActorState, parts: &[&str]) -> Vec<ChatMessage> {
    if parts.len() < 4 {
        return vec![create_message(
            "Usage: /provider add <name> <bedrock|openrouter|claude_code> <args...>".to_string(),
            MessageSender::System,
        )];
    }

    let alias = parts[2].to_string();
    let provider_type = parts[3].to_lowercase();

    let provider_config = match provider_type.as_str() {
        "bedrock" => {
            let profile = parts[4].to_string();
            if profile.is_empty() {
                return vec![create_message(
                    "Bedrock provider requires a profile name".to_string(),
                    MessageSender::Error,
                )];
            }

            let region = if parts.len() > 5 {
                parts[5..].join(" ")
            } else {
                "us-west-2".to_string()
            };

            ProviderConfig::Bedrock { profile, region }
        }
        "openrouter" => {
            let api_key = parts[4..].join(" ");
            if api_key.is_empty() {
                return vec![create_message(
                    "OpenRouter provider requires an API key".to_string(),
                    MessageSender::Error,
                )];
            }

            ProviderConfig::OpenRouter { api_key }
        }
        "claude_code" => {
            let command = if parts.len() > 4 {
                parts[4].to_string()
            } else {
                "claude".to_string()
            };
            let extra_args = if parts.len() > 5 {
                parts[5..].iter().map(|s| s.to_string()).collect()
            } else {
                Vec::new()
            };

            ProviderConfig::ClaudeCode {
                command,
                extra_args,
                env: HashMap::new(),
            }
        }
        other => {
            return vec![create_message(
                format!(
                    "Unsupported provider type '{other}'. Supported types: bedrock, openrouter, claude_code"
                ),
                MessageSender::Error,
            )]
        }
    };

    let current_settings = state.settings.settings();
    let replacing = current_settings.providers.contains_key(&alias);
    let should_set_active = current_settings.active_provider.is_none();

    state.settings.update_setting(|settings| {
        settings.add_provider(alias.clone(), provider_config.clone());
        if should_set_active {
            settings.active_provider = Some(alias.clone());
        }
    });

    if let Err(e) = state.settings.save() {
        return vec![create_message(
            format!("Provider updated for this session but failed to save settings: {e}"),
            MessageSender::Error,
        )];
    }

    let mut response = if replacing {
        format!("Updated provider '{alias}' ({provider_type})")
    } else {
        format!("Added provider '{alias}' ({provider_type})")
    };

    let mut messages = Vec::new();

    if should_set_active {
        response.push_str(" and set as the active provider");

        match create_provider(&state.settings, &alias).await {
            Ok(provider) => {
                state.provider = provider;
            }
            Err(e) => {
                messages.push(create_message(
                    format!("Failed to initialize provider '{alias}': {e}"),
                    MessageSender::Error,
                ));
            }
        }
    }

    response.push('.');

    messages.insert(0, create_message(response, MessageSender::System));
    messages
}

async fn handle_mcp_command(state: &mut ActorState, parts: &[String]) -> Vec<ChatMessage> {
    if parts.len() < 2 {
        // List all MCP servers
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

    // Validate server name (Bug fix #3)
    if name.is_empty() {
        return vec![create_message(
            "Server name cannot be empty".to_string(),
            MessageSender::Error,
        )];
    }

    // Validate command path (Bug fix #4)
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

    // Parse optional arguments (Bug fix #1 and #2: now properly handles quoted strings)
    let mut i = 4;
    while i < parts.len() {
        match parts[i].as_str() {
            "--args" => {
                if i + 1 >= parts.len() {
                    return vec![create_message(
                        "--args requires a value".to_string(),
                        MessageSender::Error,
                    )];
                }
                // With proper quote parsing, parts[i+1] now contains the complete quoted string
                // We split on whitespace to get individual arguments
                config.args = parts[i + 1]
                    .split_whitespace()
                    .map(|s| s.to_string())
                    .collect();
                i += 2;
            }
            "--env" => {
                if i + 1 >= parts.len() {
                    return vec![create_message(
                        "--env requires a value in format KEY=VALUE".to_string(),
                        MessageSender::Error,
                    )];
                }
                let env_str = &parts[i + 1];
                if let Some(eq_pos) = env_str.find('=') {
                    let key = env_str[..eq_pos].to_string();
                    let value = env_str[eq_pos + 1..].to_string();

                    // Validate key is not empty
                    if key.is_empty() {
                        return vec![create_message(
                            "Environment variable key cannot be empty".to_string(),
                            MessageSender::Error,
                        )];
                    }

                    config.env.insert(key, value);
                } else {
                    return vec![create_message(
                        "Environment variable must be in format KEY=VALUE".to_string(),
                        MessageSender::Error,
                    )];
                }
                i += 2;
            }
            _ => {
                return vec![create_message(
                    format!("Unknown argument: {}", parts[i]),
                    MessageSender::Error,
                )];
            }
        }
    }

    let current_settings = state.settings.settings();
    let replacing = current_settings.mcp_servers.contains_key(&name);

    state.settings.update_setting(|settings| {
        settings.mcp_servers.insert(name.clone(), config);
    });

    if let Err(e) = state.settings.save() {
        return vec![create_message(
            format!("MCP server updated for this session but failed to save settings: {e}"),
            MessageSender::Error,
        )];
    }

    let response = if replacing {
        format!("Updated MCP server '{name}'")
    } else {
        format!("Added MCP server '{name}'")
    };

    vec![create_message(
        format!(
            "{}\n\nSettings saved to disk. The MCP server configuration is now persistent across sessions.",
            response
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

    // Validate server name is not empty
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
            format!("MCP server removed for this session but failed to save settings: {e}"),
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

pub async fn handle_debug_ui_command(state: &mut ActorState) -> Vec<ChatMessage> {
    state
        .event_sender
        .send_message(ChatMessage::system("System message".to_string()));

    state
        .event_sender
        .send_message(ChatMessage::warning("Warning message".to_string()));

    state
        .event_sender
        .send_message(ChatMessage::error("Error message".to_string()));

    // Test Bug #1: Retry counter positioning
    // Send multiple retry attempts with messages in between to test retry positioning
    state.send_event_replay(ChatEvent::RetryAttempt {
        attempt: 1,
        max_retries: 3,
        backoff_ms: 2000,
        error: "Network timeout - testing retry counter positioning bug".to_string(),
    });

    // Add some messages between retries to simulate the bug condition
    state.event_sender.send_message(ChatMessage::system(
        "Test message added between retry attempts to verify retry counter stays at bottom"
            .to_string(),
    ));

    state.send_event_replay(ChatEvent::RetryAttempt {
        attempt: 2,
        max_retries: 3,
        backoff_ms: 4000,
        error: "Connection refused - retry counter should move to bottom".to_string(),
    });

    // Test Bug #3: Agent spawning messages should appear before agent messages
    state.event_sender.send_message(ChatMessage::system(
        "🔄 Spawning agent for task: Testing UI bug fixes".to_string(),
    ));

    // Test Bug #2: View diff button with long file path
    // Create tool calls including one with an extremely long file path
    let tool_calls = vec![
        ToolUseData {
            id: "test_long_path_0".to_string(),
            name: "function".to_string(),
            arguments: json!({
                "name": "modify_file",
                "arguments": {
                    "file_path": "/very/long/nested/directory/structure/that/goes/on/and/on/and/on/testing/view/diff/button/overflow/bug/with/extremely/long/file/path/names/that/should/not/push/button/off/screen/component/module/submodule/feature/implementation/details/config/settings/final_file.rs",
                    "before": "// old code",
                    "after": "// new code with fixes"
                }
            }),
        },
        ToolUseData {
            id: "test_modify_1".to_string(),
            name: "function".to_string(),
            arguments: json!({
                "name": "modify_file",
                "arguments": {
                    "file_path": "/example/normal_path.rs",
                    "before": "fn old_function() {\n    println!(\"old\");\n}",
                    "after": "fn new_function() {\n    println!(\"new\");\n    println!(\"improved\");\n}"
                }
            }),
        },
        ToolUseData {
            id: "test_run_2".to_string(),
            name: "function".to_string(),
            arguments: json!({
                "name": "run_build_test",
                "arguments": {
                    "command": "echo Testing UI fixes",
                    "timeout_seconds": 30,
                    "working_directory": "/"
                }
            }),
        },
    ];

    // Send assistant message with tool calls to simulate AI response
    state.event_sender.send_message(ChatMessage::assistant(
        "coder".to_string(),
        "Testing UI bug fixes:\n1. Retry counter positioning (should always be at bottom)\n2. View diff button with long file paths (should not overflow off-screen)".to_string(),
        tool_calls.clone(),
        ModelInfo {
            model: crate::ai::model::Model::GrokCodeFast1,
        },
        TokenUsage {
            input_tokens: 100,
            output_tokens: 200,
            total_tokens: 300,
            cached_prompt_tokens: None,
            reasoning_tokens: None,
            cache_creation_input_tokens: None,
        },
        ContextInfo {
            directory_list_bytes: 1024,
            files: vec![],
        },
        None,
    ));

    // Create mock tool requests
    let tool_requests = vec![
        ToolRequest {
            tool_call_id: "test_long_path_0".to_string(),
            tool_name: "modify_file".to_string(),
            tool_type: ToolRequestType::ModifyFile {
                file_path: "/very/long/nested/directory/structure/that/goes/on/and/on/and/on/testing/view/diff/button/overflow/bug/with/extremely/long/file/path/names/that/should/not/push/button/off/screen/component/module/submodule/feature/implementation/details/config/settings/final_file.rs".to_string(),
                before: "// old code".to_string(),
                after: "// new code with fixes".to_string(),
            },
        },
        ToolRequest {
            tool_call_id: "test_modify_1".to_string(),
            tool_name: "modify_file".to_string(),
            tool_type: ToolRequestType::ModifyFile {
                file_path: "/example/normal_path.rs".to_string(),
                before: "fn old_function() {\n    println!(\"old\");\n}".to_string(),
                after:
                    "fn new_function() {\n    println!(\"new\");\n    println!(\"improved\");\n}"
                        .to_string(),
            },
        },
        ToolRequest {
            tool_call_id: "test_run_2".to_string(),
            tool_name: "run_build_test".to_string(),
            tool_type: ToolRequestType::RunCommand {
                command: "echo Testing UI fixes".to_string(),
                working_directory: "/".to_string(),
            },
        },
    ];

    // Send ToolRequest events
    for tool_request in &tool_requests {
        state.send_event_replay(ChatEvent::ToolRequest(tool_request.clone()));
    }

    // Send successful ToolExecutionCompleted for long path (this will test the view diff button)
    state.send_event_replay(ChatEvent::ToolExecutionCompleted {
        tool_call_id: "test_long_path_0".to_string(),
        tool_name: "modify_file".to_string(),
        tool_result: ToolExecutionResult::ModifyFile {
            lines_added: 5,
            lines_removed: 1,
        },
        success: true,
        error: None,
    });

    // Send successful ToolExecutionCompleted for normal path
    state.send_event_replay(ChatEvent::ToolExecutionCompleted {
        tool_call_id: "test_modify_1".to_string(),
        tool_name: "modify_file".to_string(),
        tool_result: ToolExecutionResult::ModifyFile {
            lines_added: 3,
            lines_removed: 2,
        },
        success: true,
        error: None,
    });

    // Send successful ToolExecutionCompleted for command
    state.send_event_replay(ChatEvent::ToolExecutionCompleted {
        tool_call_id: "test_run_2".to_string(),
        tool_name: "run_build_test".to_string(),
        tool_result: ToolExecutionResult::RunCommand {
            exit_code: 0,
            stdout: "Testing UI fixes\n".to_string(),
            stderr: "".to_string(),
        },
        success: true,
        error: None,
    });

    // Test SearchTypes and GetTypeDocs tool requests
    // First, send an assistant message with tool_calls to create the tool items in the UI
    let analyzer_tool_calls = vec![
        ToolUseData {
            id: "test_search_types".to_string(),
            name: "search_types".to_string(),
            arguments: json!({
                "type_name": "Config",
                "language": "rust",
                "workspace_root": "/example/project"
            }),
        },
        ToolUseData {
            id: "test_get_type_docs".to_string(),
            name: "get_type_docs".to_string(),
            arguments: json!({
                "type_path": "src/config.rs::Config",
                "language": "rust",
                "workspace_root": "/example/project"
            }),
        },
    ];

    state.event_sender.send_message(ChatMessage::assistant(
        "coder".to_string(),
        "Testing analyzer tools: search_types and get_type_docs".to_string(),
        analyzer_tool_calls,
        ModelInfo {
            model: crate::ai::model::Model::GrokCodeFast1,
        },
        TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            total_tokens: 150,
            cached_prompt_tokens: None,
            reasoning_tokens: None,
            cache_creation_input_tokens: None,
        },
        ContextInfo {
            directory_list_bytes: 512,
            files: vec![],
        },
        None,
    ));

    // Now send ToolRequest events to update the tool items
    state.send_event_replay(ChatEvent::ToolRequest(ToolRequest {
        tool_call_id: "test_search_types".to_string(),
        tool_name: "search_types".to_string(),
        tool_type: ToolRequestType::SearchTypes {
            language: "rust".to_string(),
            workspace_root: "/example/project".to_string(),
            type_name: "Config".to_string(),
        },
    }));

    state.send_event_replay(ChatEvent::ToolExecutionCompleted {
        tool_call_id: "test_search_types".to_string(),
        tool_name: "search_types".to_string(),
        tool_result: ToolExecutionResult::SearchTypes {
            types: vec![
                "src/config.rs::Config".to_string(),
                "src/settings/mod.rs::Config".to_string(),
            ],
        },
        success: true,
        error: None,
    });

    // Test GetTypeDocs tool request and completion
    state.send_event_replay(ChatEvent::ToolRequest(ToolRequest {
        tool_call_id: "test_get_type_docs".to_string(),
        tool_name: "get_type_docs".to_string(),
        tool_type: ToolRequestType::GetTypeDocs {
            language: "rust".to_string(),
            workspace_root: "/example/project".to_string(),
            type_path: "src/config.rs::Config".to_string(),
        },
    }));

    state.send_event_replay(ChatEvent::ToolExecutionCompleted {
        tool_call_id: "test_get_type_docs".to_string(),
        tool_name: "get_type_docs".to_string(),
        tool_result: ToolExecutionResult::GetTypeDocs {
            documentation: "/// Configuration struct for the application\npub struct Config {\n    pub host: String,\n    pub port: u16,\n}".to_string(),
        },
        success: true,
        error: None,
    });

    // Add one more retry to ensure it appears at the bottom after all the tool messages
    state.send_event_replay(ChatEvent::RetryAttempt {
        attempt: 3,
        max_retries: 3,
        backoff_ms: 8000,
        error: "Final retry test - should appear at the very bottom of chat".to_string(),
    });

    // Simulate spawning a coordinator agent
    state.event_sender.send_message(ChatMessage::system(
        "🔄 Spawning agent for task: Coordinate multiple sub-tasks for testing".to_string(),
    ));

    // Coordinator agent sends a message and uses a tool
    state.event_sender.send_message(ChatMessage::assistant(
        "coordinator".to_string(),
        "I'll coordinate this workflow by spawning a review agent.".to_string(),
        vec![ToolUseData {
            id: "test_coord_tool".to_string(),
            name: "function".to_string(),
            arguments: json!({
                "name": "set_tracked_files",
                "arguments": {
                    "file_paths": ["/example/test.rs"]
                }
            }),
        }],
        ModelInfo {
            model: crate::ai::model::Model::GrokCodeFast1,
        },
        TokenUsage {
            input_tokens: 50,
            output_tokens: 25,
            total_tokens: 75,
            cached_prompt_tokens: None,
            reasoning_tokens: None,
            cache_creation_input_tokens: None,
        },
        ContextInfo {
            directory_list_bytes: 512,
            files: vec![],
        },
        None,
    ));

    // Simulate the coordinator spawning a review agent
    state.event_sender.send_message(ChatMessage::system(
        "🔄 Spawning agent for task: Review the code changes".to_string(),
    ));

    // Review agent sends a message and uses a tool
    state.event_sender.send_message(ChatMessage::assistant(
        "review".to_string(),
        "Reviewing the changes now. I'll check for potential issues.".to_string(),
        vec![ToolUseData {
            id: "test_review_tool".to_string(),
            name: "function".to_string(),
            arguments: json!({
                "name": "set_tracked_files",
                "arguments": {
                    "file_paths": ["/example/test.rs", "/example/lib.rs"]
                }
            }),
        }],
        ModelInfo {
            model: crate::ai::model::Model::GrokCodeFast1,
        },
        TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            total_tokens: 150,
            cached_prompt_tokens: None,
            reasoning_tokens: None,
            cache_creation_input_tokens: None,
        },
        ContextInfo {
            directory_list_bytes: 1024,
            files: vec![],
        },
        None,
    ));

    // Simulate complete_task being called
    state.send_event_replay(ChatEvent::ToolRequest(ToolRequest {
        tool_call_id: "test_complete_task".to_string(),
        tool_name: "complete_task".to_string(),
        tool_type: ToolRequestType::Other { args: json!({}) },
    }));

    state.send_event_replay(ChatEvent::ToolExecutionCompleted {
        tool_call_id: "test_complete_task".to_string(),
        tool_name: "complete_task".to_string(),
        tool_result: ToolExecutionResult::Other {
            result: json!({
                "status": "success",
                "message": "Review completed successfully"
            }),
        },
        success: true,
        error: None,
    });

    state.event_sender.send_message(ChatMessage::system(
        "✅ Sub-agent completed successfully:\nReview completed successfully".to_string(),
    ));

    // Add a comprehensive markdown test message for copy button testing
    let markdown_test = r#"# TyCode Debug UI - Markdown Test

This is a comprehensive test message with extensive markdown formatting to test the copy button functionality.

## Code Examples

Here's a simple Python function:

```python
def fibonacci(n):
    """Calculate the nth Fibonacci number."""
    if n <= 1:
        return n
    return fibonacci(n-1) + fibonacci(n-2)

# Test the function
for i in range(10):
    print(f"F({i}) = {fibonacci(i)}")
```

And here's a TypeScript example:

```typescript
interface User {
    id: string;
    name: string;
    email: string;
    createdAt: Date;
}

class UserService {
    private users: Map<string, User> = new Map();

    async createUser(name: string, email: string): Promise<User> {
        const user: User = {
            id: crypto.randomUUID(),
            name,
            email,
            createdAt: new Date()
        };
        this.users.set(user.id, user);
        return user;
    }

    async getUser(id: string): Promise<User | undefined> {
        return this.users.get(id);
    }
}
```

## Rust Code

Here's a Rust implementation:

```rust
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub debug: bool,
}

impl Config {
    pub fn new(host: String, port: u16) -> Self {
        Self {
            host,
            port,
            debug: false,
        }
    }

    pub fn with_debug(mut self, debug: bool) -> Self {
        self.debug = debug;
        self
    }
}

fn main() {
    let config = Config::new("localhost".to_string(), 8080)
        .with_debug(true);
    println!("Config: {:?}", config);
}
```

## Lists and Text Formatting

### Unordered List

- **Bold text** for emphasis
- *Italic text* for subtle emphasis
- `Inline code` for variable names
- [Links to documentation](https://example.com)

### Ordered List

1. First step: Initialize the project
2. Second step: Install dependencies
3. Third step: Configure settings
4. Fourth step: Run tests
5. Fifth step: Deploy to production

### Nested Lists

- Top level item
  - Nested item 1
  - Nested item 2
    - Double nested item
- Another top level item
  - More nesting

## Blockquotes

> This is a blockquote with important information.
> It can span multiple lines and provides context
> for the discussion at hand.

> **Note:** Always test your code before deploying to production!

## Tables

| Feature | Status | Priority |
|---------|--------|----------|
| Copy button | ✅ Done | High |
| Insert button | ❌ Removed | N/A |
| Message copy | ✅ Done | High |
| Line numbers | ✅ Fixed | Medium |

## More Code Examples

Bash script:

```bash
#!/bin/bash

for file in *.tar.xz; do
  if [ -f "$file" ]; then
    echo "Verifying attestation for $file"
    gh attestation verify "$file" --owner tigy32
  fi
done
```

SQL query:

```sql
SELECT u.id, u.name, COUNT(o.id) as order_count
FROM users u
LEFT JOIN orders o ON u.id = o.user_id
WHERE u.created_at > '2024-01-01'
GROUP BY u.id, u.name
HAVING COUNT(o.id) > 5
ORDER BY order_count DESC;
```

JSON configuration:

```json
{
  "name": "tycode-vscode",
  "version": "1.0.0",
  "dependencies": {
    "vscode": "^1.80.0"
  },
  "scripts": {
    "compile": "webpack",
    "test": "node ./out/test/runTest.js"
  }
}
```

## Conclusion

This debug message contains:
- Multiple code blocks with syntax highlighting
- Headings at various levels
- Lists (ordered, unordered, nested)
- Text formatting (bold, italic, inline code)
- Blockquotes
- Tables
- Links

**Test the copy button** by clicking the ⧉ button at the bottom of this message!"#;

    state.event_sender.send_message(ChatMessage::assistant(
        "debug".to_string(),
        markdown_test.to_string(),
        vec![],
        ModelInfo {
            model: crate::ai::model::Model::GrokCodeFast1,
        },
        TokenUsage {
            input_tokens: 500,
            output_tokens: 1000,
            total_tokens: 1500,
            cached_prompt_tokens: None,
            reasoning_tokens: None,
            cache_creation_input_tokens: None,
        },
        ContextInfo {
            directory_list_bytes: 2048,
            files: vec![],
        },
        None,
    ));

    vec![create_message(
        "Debug UI test completed. Check:\n1. Retry counter messages should always be at the bottom of chat\n2. View Diff button should be visible even with very long file paths (text should truncate with ...)\n3. Agent spawning messages and complete_task should appear correctly\n4. Long markdown message with copy button for testing copy functionality".to_string(),
        MessageSender::System,
    )]
}

async fn handle_profile_command(state: &mut ActorState, parts: &[&str]) -> Vec<ChatMessage> {
    let show_current = parts.len() < 2 || parts[1].to_lowercase() == "show";
    if show_current {
        let current = state.settings.current_profile().unwrap_or("default");
        return vec![create_message(
            format!("Current profile: {}", current),
            MessageSender::System,
        )];
    }

    let subcommand = parts[1].to_lowercase();
    match subcommand.as_str() {
        "list" => {
            let home = match dirs::home_dir() {
                Some(h) => h,
                None => {
                    return vec![create_message(
                        "Failed to get home directory.".to_string(),
                        MessageSender::Error,
                    )];
                }
            };

            let tycode_dir = home.join(".tycode");
            let mut profiles: Vec<String> = vec!["default".to_string()];

            if tycode_dir.exists() {
                match fs::read_dir(&tycode_dir) {
                    Ok(entries) => {
                        for entry in entries {
                            match entry {
                                Ok(e) => {
                                    let path = e.path();
                                    let file_name = path.file_name().and_then(|n| n.to_str());
                                    if let Some(name) = file_name {
                                        if let Some(profile_name) = name
                                            .strip_prefix("settings_")
                                            .and_then(|s| s.strip_suffix(".toml"))
                                        {
                                            if !profile_name.is_empty() {
                                                profiles.push(profile_name.to_string());
                                            }
                                        }
                                    }
                                }
                                Err(_) => {
                                    // ignore
                                }
                            }
                        }
                    }
                    Err(_) => {
                        return vec![create_message(
                            "Failed to read .tycode directory.".to_string(),
                            MessageSender::Error,
                        )];
                    }
                }
            }

            profiles.sort();
            let msg = format!("Available profiles: {}", profiles.join(", "));
            vec![create_message(msg, MessageSender::System)]
        }
        "switch" => {
            if parts.len() < 3 {
                return vec![create_message(
                    "Usage: /profile switch <name>".to_string(),
                    MessageSender::Error,
                )];
            }
            let name = parts[2];
            if let Err(e) = state.settings.switch_profile(name) {
                return vec![create_message(
                    format!("Failed to switch to {}: {}", name, e),
                    MessageSender::Error,
                )];
            }
            if let Err(e) = state.settings.save() {
                return vec![create_message(
                    format!("Switched to profile {}, but failed to persist: {}", name, e),
                    MessageSender::Error,
                )];
            }
            match state.reload_from_settings().await {
                Ok(()) => vec![create_message(
                    format!("Switched to profile: {}.", name),
                    MessageSender::System,
                )],
                Err(e) => vec![create_message(
                    format!(
                        "Switched to profile: {}, but failed to reload: {}",
                        name,
                        e.to_string()
                    ),
                    MessageSender::Error,
                )],
            }
        }
        "save" => {
            if parts.len() < 3 {
                return vec![create_message(
                    "Usage: /profile save <name>".to_string(),
                    MessageSender::Error,
                )];
            }
            let name = parts[2];
            if let Err(e) = state.settings.save_as_profile(name) {
                return vec![create_message(
                    format!("Failed to save as {}: {}", name, e),
                    MessageSender::Error,
                )];
            }
            vec![create_message(
                format!("Saved current settings as profile: {}.", name),
                MessageSender::System,
            )]
        }
        _ => vec![create_message(
            format!(
                "Unknown subcommand '{}'. Usage: /profile [switch|save|list|show] [<name>]",
                subcommand
            ),
            MessageSender::Error,
        )],
    }
}

async fn handle_sessions_command(state: &mut ActorState, parts: &[&str]) -> Vec<ChatMessage> {
    if parts.len() < 2 {
        return vec![create_message(
            "Usage: /sessions [list|resume <id>|delete <id>|gc [days]]".to_string(),
            MessageSender::System,
        )];
    }

    match parts[1] {
        "list" => handle_sessions_list_command(state).await,
        "resume" => handle_sessions_resume_command(state, parts).await,
        "delete" => handle_sessions_delete_command(state, parts).await,
        "gc" => handle_sessions_gc_command(state, parts).await,
        _ => vec![create_message(
            format!(
                "Unknown sessions subcommand: {}. Use: list, resume, delete, gc",
                parts[1]
            ),
            MessageSender::Error,
        )],
    }
}

async fn handle_sessions_list_command(state: &ActorState) -> Vec<ChatMessage> {
    let sessions = match storage::list_sessions(Some(&state.sessions_dir)) {
        Ok(s) => s,
        Err(e) => {
            return vec![create_message(
                format!("Failed to list sessions: {e:?}"),
                MessageSender::Error,
            )];
        }
    };

    if sessions.is_empty() {
        return vec![create_message(
            "No saved sessions found.".to_string(),
            MessageSender::System,
        )];
    }

    let mut message = String::from("=== Saved Sessions ===\n\n");
    for session_meta in sessions {
        let created = chrono::DateTime::from_timestamp_millis(session_meta.created_at as i64)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| session_meta.created_at.to_string());

        let modified = chrono::DateTime::from_timestamp_millis(session_meta.last_modified as i64)
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| session_meta.last_modified.to_string());

        message.push_str(&format!(
            "  ID: {}\n    Task List: {}\n    Preview: {}\n    Created: {}\n    Last modified: {}\n\n",
            session_meta.id, session_meta.task_list_title, session_meta.preview, created, modified
        ));
    }

    message.push_str("Use `/sessions resume <id>` to load a session.\n");

    vec![create_message(message, MessageSender::System)]
}

async fn handle_sessions_resume_command(
    state: &mut ActorState,
    parts: &[&str],
) -> Vec<ChatMessage> {
    if parts.len() < 3 {
        return vec![create_message(
            "Usage: /sessions resume <id>".to_string(),
            MessageSender::Error,
        )];
    }

    let session_id = parts[2];

    match resume_session(state, session_id).await {
        Ok(()) => vec![create_message(
            format!("Session '{}' resumed successfully.", session_id),
            MessageSender::System,
        )],
        Err(e) => vec![create_message(
            format!("Failed to resume session '{}': {e:?}", session_id),
            MessageSender::Error,
        )],
    }
}

async fn handle_sessions_delete_command(state: &ActorState, parts: &[&str]) -> Vec<ChatMessage> {
    if parts.len() < 3 {
        return vec![create_message(
            "Usage: /sessions delete <id>".to_string(),
            MessageSender::Error,
        )];
    }

    let session_id = parts[2];

    match storage::delete_session(session_id, Some(&state.sessions_dir)) {
        Ok(()) => vec![create_message(
            format!("Session '{}' deleted successfully.", session_id),
            MessageSender::System,
        )],
        Err(e) => vec![create_message(
            format!("Failed to delete session '{}': {e:?}", session_id),
            MessageSender::Error,
        )],
    }
}

async fn handle_memory_command(state: &mut ActorState, parts: &[&str]) -> Vec<ChatMessage> {
    if parts.len() < 2 {
        return vec![create_message(
            "Usage: /memory summarize".to_string(),
            MessageSender::System,
        )];
    }

    match parts[1] {
        "summarize" => handle_memory_summarize_command(state).await,
        _ => vec![create_message(
            format!("Unknown memory subcommand: {}. Use: summarize", parts[1]),
            MessageSender::Error,
        )],
    }
}

async fn handle_memory_summarize_command(state: &mut ActorState) -> Vec<ChatMessage> {
    use std::collections::BTreeMap;

    use crate::agents::memory_summarizer::MemorySummarizerAgent;
    use crate::agents::runner::AgentRunner;
    use crate::tools::complete_task::CompleteTask;
    use crate::tools::r#trait::ToolExecutor;

    let Some(ref memory_log) = state.memory_log else {
        return vec![create_message(
            "Memory system is not enabled. Enable it in settings with [memory] enabled = true"
                .to_string(),
            MessageSender::Error,
        )];
    };

    let memories = {
        let log = memory_log.lock().unwrap();
        log.read_all().to_vec()
    };

    if memories.is_empty() {
        return vec![create_message(
            "No memories to summarize.".to_string(),
            MessageSender::System,
        )];
    }

    let mut formatted = String::from("# Memories to Summarize\n\n");
    for memory in &memories {
        formatted.push_str(&format!(
            "## Memory #{} ({})\n",
            memory.seq,
            memory.source.as_deref().unwrap_or("global")
        ));
        formatted.push_str(&memory.content);
        formatted.push_str("\n\n");
    }

    let memory_count = memories.len();
    state.event_sender.send_message(ChatMessage::system(format!(
        "Summarizing {} memories...",
        memory_count
    )));

    let mut tools: BTreeMap<String, std::sync::Arc<dyn ToolExecutor + Send + Sync>> =
        BTreeMap::new();
    tools.insert("complete_task".into(), std::sync::Arc::new(CompleteTask));

    let runner = AgentRunner::new(
        state.provider.clone(),
        state.settings.clone(),
        tools,
        state.steering.clone(),
    );
    let agent = MemorySummarizerAgent::new();
    let mut active_agent = ActiveAgent::new(Box::new(agent));
    active_agent.conversation.push(Message::user(formatted));

    match runner.run(active_agent).await {
        Ok(result) => vec![create_message(
            format!("=== Memory Summary ===\n\n{}", result),
            MessageSender::System,
        )],
        Err(e) => vec![create_message(
            format!("Memory summarization failed: {e:?}"),
            MessageSender::Error,
        )],
    }
}

async fn handle_sessions_gc_command(state: &ActorState, parts: &[&str]) -> Vec<ChatMessage> {
    let days = if parts.len() >= 3 {
        match parts[2].parse::<u64>() {
            Ok(d) => d,
            Err(_) => {
                return vec![create_message(
                    "Usage: /sessions gc [days]. Days must be a positive number.".to_string(),
                    MessageSender::Error,
                )];
            }
        }
    } else {
        7
    };

    let sessions = match storage::list_sessions(Some(&state.sessions_dir)) {
        Ok(s) => s,
        Err(e) => {
            return vec![create_message(
                format!("Failed to list sessions: {e:?}"),
                MessageSender::Error,
            )];
        }
    };

    let cutoff_time = Utc::now().timestamp_millis() as u64 - (days * 24 * 60 * 60 * 1000);
    let mut deleted_count = 0;
    let mut failed_deletes = Vec::new();

    for session_meta in sessions {
        if session_meta.last_modified >= cutoff_time {
            continue;
        }

        match storage::delete_session(&session_meta.id, Some(&state.sessions_dir)) {
            Ok(()) => deleted_count += 1,
            Err(e) => {
                failed_deletes.push(format!("{}: {e:?}", session_meta.id));
            }
        }
    }

    let mut message = format!(
        "Garbage collection complete. Deleted {} session(s) older than {} days.",
        deleted_count, days
    );

    if !failed_deletes.is_empty() {
        message.push_str(&format!(
            "\n\nFailed to delete {} session(s):\n",
            failed_deletes.len()
        ));
        for failure in failed_deletes {
            message.push_str(&format!("  {failure}\n"));
        }
    }

    vec![create_message(message, MessageSender::System)]
}
