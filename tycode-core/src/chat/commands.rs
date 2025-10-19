use crate::agents::catalog::AgentCatalog;
use crate::ai::model::{Model, ModelCost};
use crate::ai::{ModelSettings, ReasoningBudget, TokenUsage, ToolUseData};
use crate::chat::actor::create_provider;
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
use serde_json::json;
use std::collections::HashMap;
use toml;

use crate::file::context::build_message_context;

#[derive(Clone, Debug)]
pub struct CommandInfo {
    pub name: String,
    pub description: String,
    pub usage: String,
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
        "mcp" => handle_mcp_command(state, &parts).await,
        "help" => handle_help_command().await,
        "models" => handle_models_command(state).await,
        "provider" => handle_provider_command(state, &parts).await,
        "debug_ui" => handle_debug_ui_command(state).await,
        _ => vec![create_message(
            format!("Unknown command: /{}", parts[0]),
            MessageSender::Error,
        )],
    }
}

/// Get all available commands with their descriptions
pub fn get_available_commands() -> Vec<CommandInfo> {
    vec![
        CommandInfo {
            name: "clear".to_string(),
            description: r"Clear the conversation history".to_string(),
            usage: "/clear".to_string(),
        },
        CommandInfo {
            name: "context".to_string(),
            description: r"Show what files would be included in the AI context".to_string(),
            usage: "/context".to_string(),
        },
        CommandInfo {
            name: "fileapi".to_string(),
            description: r"Set the file modification API (patch or find-replace)".to_string(),
            usage: "/fileapi <patch|findreplace>".to_string(),
        },
        CommandInfo {
            name: r"model".to_string(),
            description: r"Set the AI model for all agents".to_string(),
            usage: r"/model <name> [temperature=0.7] [max_tokens=4096] [top_p=1.0] [reasoning_budget=...]".to_string(),
        },
        CommandInfo {
            name: "trace".to_string(),
            description: r"Enable/disable trace logging to .tycode/trace".to_string(),
            usage: "/trace <on|off>".to_string(),
        },
        CommandInfo {
            name: "settings".to_string(),
            description: "Display current settings and configuration".to_string(),
            usage: "/settings or /settings save".to_string(),
        },
        CommandInfo {
            name: "security".to_string(),
            description: "Manage security mode and permissions".to_string(),
            usage: "/security [mode|whitelist|clear] [args...]".to_string(),
        },
        CommandInfo {
            name: "cost".to_string(),
            description: "Show session token usage and estimated cost, or set model cost limit".to_string(),
            usage: "/cost [set <free|low|medium|high|unlimited>]".to_string(),
        },
        // Remove model-cost entry, already handled by updating cost above
        CommandInfo {
            name: "help".to_string(),
            description: "Show this help message".to_string(),
            usage: "/help".to_string(),
        },
        CommandInfo {
            name: "models".to_string(),
            description: "List available AI models".to_string(),
            usage: "/models".to_string(),
        },
        CommandInfo {
            name: "provider".to_string(),
            description: "List, switch, or add AI providers".to_string(),
            usage: "/provider [name] | /provider add <name> <type> [args]".to_string(),
        },
        CommandInfo {
            name: "agentmodel".to_string(),
            description: "Set the AI model for a specific agent with tunings".to_string(),
            usage: "/agentmodel <agent_name> <model_name> [temperature=0.7] [max_tokens=4096] [top_p=1.0] [reasoning_budget=...]".to_string(),
        },
        CommandInfo {
            name: "agent".to_string(),
            description: "Switch the current agent".to_string(),
            usage: "/agent <name>".to_string(),
        },
        CommandInfo {
            name: "review_level".to_string(),
            description: "Set the review level (None, Modification, All)".to_string(),
            usage: "/review_level <level>".to_string(),
        },
        CommandInfo {
            name: "mcp".to_string(),
            description: "Manage MCP server configurations".to_string(),
            usage: "/mcp [add|remove] [args...]".to_string(),
        },
        CommandInfo {
            name: "quit".to_string(),
            description: "Exit the application".to_string(),
            usage: "/quit or /exit".to_string(),
        },
    ]
}

async fn handle_clear_command(state: &mut ActorState) -> Vec<ChatMessage> {
    state.event_sender.clear_conversation();
    current_agent_mut(state).conversation.clear();
    vec![create_message(
        "Conversation cleared.".to_string(),
        MessageSender::System,
    )]
}

async fn handle_context_command(state: &ActorState) -> Vec<ChatMessage> {
    let tracked_files: Vec<_> = state.tracked_files.iter().cloned().collect();
    let context = build_message_context(
        &state.workspace_roots,
        &tracked_files,
        state.task_list.clone(),
    )
    .await;
    vec![create_message(
        context.to_formatted_string(),
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

    let mut message = String::new();
    message.push_str("=== Session Cost Summary ===\n\n");
    message.push_str(&format!("Current Model: {current_model:?}\n"));
    message.push_str(&format!("Provider: {}\n\n", state.provider.name()));

    message.push_str("Token Usage:\n");
    message.push_str(&format!("  Input tokens:  {:>8}\n", usage.input_tokens));
    message.push_str(&format!("  Output tokens: {:>8}\n", usage.output_tokens));
    message.push_str(&format!("  Total tokens:  {:>8}\n\n", usage.total_tokens));

    message.push_str("Accumulated Cost:\n");
    message.push_str(&format!("  Total cost: ${:.6}\n", state.session_cost));

    if usage.total_tokens > 0 {
        let avg_cost_per_1k = (state.session_cost / usage.total_tokens as f64) * 1000.0;
        message.push_str(&format!("  Average per 1K tokens: ${avg_cost_per_1k:.6}\n"));
    }

    vec![create_message(message, MessageSender::System)]
}

async fn handle_help_command() -> Vec<ChatMessage> {
    let commands = get_available_commands();
    let mut message = String::from("Available commands:\n\n");

    for cmd in commands {
        message.push_str(&format!("/{} - {}\n", cmd.name, cmd.description));
        message.push_str(&format!("  Usage: {}\n\n", cmd.usage));
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

    // Check for sub-agents: block switch if sub-agents are active
    if state.agent_stack.len() > 1 {
        return vec![create_message(
            "Cannot switch agent while sub-agents are active.".to_string(),
            MessageSender::System,
        )];
    }

    // Check if already on the agent to avoid unnecessary switching
    if current_agent(state).agent.name() == agent_name {
        return vec![create_message(
            format!("Already switched to agent: {agent_name}"),
            MessageSender::System,
        )];
    }

    // Preserve conversation from current agent before switching
    let old_conversation = current_agent(state).conversation.clone();

    // Create new root agent and replace the current one
    let new_agent_dyn = AgentCatalog::create_agent(agent_name).unwrap();
    let mut new_root_agent = crate::agents::agent::ActiveAgent::new(new_agent_dyn);
    new_root_agent.conversation = old_conversation;
    new_root_agent.spawn_tool_use_id = None;
    state.agent_stack[0] = new_root_agent;

    vec![create_message(
        format!("Switched to agent: {agent_name}"),
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
        "modification" => ReviewLevel::Modification,
        "all" => ReviewLevel::All,
        _ => {
            return vec![create_message(
                "Invalid review level. Valid options: none, modification, all".to_string(),
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

    // Update the active provider in memory
    state.provider = new_provider;

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

async fn handle_mcp_command(state: &mut ActorState, parts: &[&str]) -> Vec<ChatMessage> {
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

    match parts[1] {
        "add" => handle_mcp_add_command(state, parts).await,
        "remove" => handle_mcp_remove_command(state, parts).await,
        _ => vec![create_message(
            "Usage: /mcp [add|remove] [args...]. Use `/mcp` to list all servers.".to_string(),
            MessageSender::Error,
        )],
    }
}

async fn handle_mcp_add_command(state: &mut ActorState, parts: &[&str]) -> Vec<ChatMessage> {
    if parts.len() < 4 {
        return vec![create_message(
            "Usage: /mcp add <name> <command> [--args \"args...\"] [--env \"KEY=VALUE\"]"
                .to_string(),
            MessageSender::Error,
        )];
    }

    let name = parts[2].to_string();
    let command = parts[3].to_string();

    let mut config = McpServerConfig {
        command,
        args: Vec::new(),
        env: HashMap::new(),
    };

    // Parse optional arguments
    let mut i = 4;
    while i < parts.len() {
        match parts[i] {
            "--args" => {
                if i + 1 >= parts.len() {
                    return vec![create_message(
                        "--args requires a value".to_string(),
                        MessageSender::Error,
                    )];
                }
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
                let env_str = parts[i + 1];
                if let Some(eq_pos) = env_str.find('=') {
                    let key = env_str[..eq_pos].to_string();
                    let value = env_str[eq_pos + 1..].to_string();
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

async fn handle_mcp_remove_command(state: &mut ActorState, parts: &[&str]) -> Vec<ChatMessage> {
    if parts.len() < 3 {
        return vec![create_message(
            "Usage: /mcp remove <name>".to_string(),
            MessageSender::Error,
        )];
    }

    let name = parts[2];

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

pub async fn handle_debug_ui_command(state: &ActorState) -> Vec<ChatMessage> {
    let _ = state
        .event_sender
        .add_message(ChatMessage::system("System message".to_string()));

    let _ = state
        .event_sender
        .add_message(ChatMessage::error("Error message".to_string()));

    // Create tool calls for the assistant message
    let tool_calls = vec![
        ToolUseData {
            id: "test_modify_0".to_string(),
            name: "function".to_string(),
            arguments: json!({
                "name": "modify_file",
                "arguments": {
                    "file_path": "/example/test.rs",
                    "before": "fn old_function() {\n    println!(\"old\");\n}",
                    "after": "fn new_function() {\n    println!(\"new\");\n    println!(\"improved\");\n}"
                }
            }),
        },
        ToolUseData {
            id: "test_modify_1".to_string(),
            name: "function".to_string(),
            arguments: json!({
                "name": "modify_file",
                "arguments": {
                    "file_path": "/example/missing.rs",
                    "before": "",
                    "after": "some content"
                }
            }),
        },
        ToolUseData {
            id: "test_run_2".to_string(),
            name: "function".to_string(),
            arguments: json!({
                "name": "run_build_test",
                "arguments": {
                    "command": "echo Hello World",
                    "timeout_seconds": 30,
                    "working_directory": "/"
                }
            }),
        },
    ];

    // Send assistant message with tool calls to simulate AI response
    state.event_sender.add_message(ChatMessage::assistant(
        "coder".to_string(),
        "I'll modify the file with improved code and run a test command.".to_string(),
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
            tool_call_id: "test_modify_0".to_string(),
            tool_name: "modify_file".to_string(),
            tool_type: ToolRequestType::ModifyFile {
                file_path: "/example/test.rs".to_string(),
                before: "fn old_function() {\n    println!(\"old\");\n}".to_string(),
                after:
                    "fn new_function() {\n    println!(\"new\");\n    println!(\"improved\");\n}"
                        .to_string(),
            },
        },
        ToolRequest {
            tool_call_id: "test_modify_1".to_string(),
            tool_name: "modify_file".to_string(),
            tool_type: ToolRequestType::ModifyFile {
                file_path: "/example/missing.rs".to_string(),
                before: "".to_string(),
                after: "some content".to_string(),
            },
        },
        ToolRequest {
            tool_call_id: "test_run_2".to_string(),
            tool_name: "run_build_test".to_string(),
            tool_type: ToolRequestType::RunCommand {
                command: "echo Hello World".to_string(),
                working_directory: "/".to_string(),
            },
        },
    ];

    // Send ToolRequest events
    for tool_request in &tool_requests {
        let _ = state
            .event_sender
            .event_tx
            .send(ChatEvent::ToolRequest(tool_request.clone()));
    }

    // Send successful ToolExecutionCompleted for first tool call
    let _ = state
        .event_sender
        .event_tx
        .send(ChatEvent::ToolExecutionCompleted {
            tool_call_id: "test_modify_0".to_string(),
            tool_name: "modify_file".to_string(),
            tool_result: ToolExecutionResult::ModifyFile {
                lines_added: 3,
                lines_removed: 2,
            },
            success: true,
            error: None,
        });

    // Send failed ToolExecutionCompleted for second tool call
    let _ = state
        .event_sender
        .event_tx
        .send(ChatEvent::ToolExecutionCompleted {
            tool_call_id: "test_modify_1".to_string(),
            tool_name: "modify_file".to_string(),
            tool_result: ToolExecutionResult::Error {
                short_message: "File not found".to_string(),
                detailed_message: "The file '/example/missing.rs' does not exist in the workspace"
                    .to_string(),
            },
            success: false,
            error: Some("File not found".to_string()),
        });

    // Send successful ToolExecutionCompleted for third tool call
    let _ = state
        .event_sender
        .event_tx
        .send(ChatEvent::ToolExecutionCompleted {
            tool_call_id: "test_run_2".to_string(),
            tool_name: "run_build_test".to_string(),
            tool_result: ToolExecutionResult::RunCommand {
                exit_code: 0,
                stdout: "Hello World\n".to_string(),
                stderr: "".to_string(),
            },
            success: true,
            error: None,
        });

    vec![create_message(
        "Debug UI test events sent successfully.".to_string(),
        MessageSender::System,
    )]
}
