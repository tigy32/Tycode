use crate::agents::catalog::AgentCatalog;
use crate::ai::model::Model;
use crate::ai::{ModelSettings, ReasoningBudget};
use crate::chat::actor::create_provider;
use crate::chat::events::EventSender;
use crate::chat::tools::{current_agent, current_agent_mut};
use crate::chat::{
    actor::ActorState,
    events::{ChatMessage, MessageSender},
    state::FileModificationApi,
};
use crate::settings::config::ReviewLevel;
use chrono::Utc;

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
        "settings" => handle_settings_command(state).await,
        "security" => handle_security_command(&state.event_sender, &parts).await,
        "agentmodel" => handle_agentmodel_command(state, &parts).await,
        "agent" => handle_agent_command(state, &parts).await,
        "review_level" => handle_review_level_command(state, &parts).await,
        "cost" => handle_cost_command(state).await,
        "help" => handle_help_command().await,
        "models" => handle_models_command(state).await,
        "provider" => handle_provider_command(state, &parts).await,
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
            usage: "/settings".to_string(),
        },
        CommandInfo {
            name: "security".to_string(),
            description: "Manage security mode and permissions".to_string(),
            usage: "/security [mode|whitelist|clear] [args...]".to_string(),
        },
        CommandInfo {
            name: "cost".to_string(),
            description: "Show session token usage and estimated cost".to_string(),
            usage: "/cost".to_string(),
        },
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
            description: "List or change the active AI provider".to_string(),
            usage: "/provider [name]".to_string(),
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
    let context = build_message_context(&state.workspace_roots, &tracked_files).await;
    vec![create_message(
        context.to_formatted_string(),
        MessageSender::System,
    )]
}

async fn handle_fileapi_command(state: &mut ActorState, parts: &[&str]) -> Vec<ChatMessage> {
    if let Some(api_name) = parts.get(1) {
        match api_name.to_lowercase().as_str() {
            "patch" => {
                state.config.file_modification_api = FileModificationApi::Patch;
                vec![create_message(
                    "File modification API set to: patch".to_string(),
                    MessageSender::Error,
                )]
            }
            "findreplace" | "find-replace" => {
                state.config.file_modification_api = FileModificationApi::FindReplace;
                vec![create_message(
                    "File modification API set to: find-replace".to_string(),
                    MessageSender::Error,
                )]
            }
            _ => vec![create_message(
                "Unknown file API. Use: patch, findreplace".to_string(),
                MessageSender::Error,
            )],
        }
    } else {
        let current_api = match state.config.file_modification_api {
            FileModificationApi::Patch => "patch",
            FileModificationApi::FindReplace => "find-replace",
        };
        vec![create_message(
            format!(
                "Current file modification API: {current_api}. Usage: /fileapi <patch|findreplace>"
            ),
            MessageSender::System,
        )]
    }
}

async fn handle_settings_command(state: &ActorState) -> Vec<ChatMessage> {
    let mut message = String::new();
    message.push_str("=== Current Settings ===\n\n");

    // Config settings
    let current_api = match state.config.file_modification_api {
        FileModificationApi::Patch => "patch",
        FileModificationApi::FindReplace => "find-replace",
    };
    message.push_str(&format!("FILE API: {current_api}\n"));
    message.push_str(&format!(
        "TRACE LOGGING: {}\n",
        if state.config.trace {
            "enabled"
        } else {
            "disabled"
        }
    ));

    // Add provider and security info from ActorState
    let settings = state.settings.settings();
    message.push_str(&format!(
        "\nACTIVE PROVIDER: {}\n",
        settings.active_provider
    ));
    message.push_str(&format!("SECURITY MODE: {:?}\n", settings.security.mode));

    vec![create_message(message, MessageSender::System)]
}

async fn handle_security_command(_state: &EventSender, parts: &[&str]) -> Vec<ChatMessage> {
    if parts.len() < 2 {
        return vec![create_message(
            "Security commands:\n\
              /security mode [all|auto|readonly] - Set security mode\n\
              /security status - Show current security settings"
                .to_string(),
            MessageSender::System,
        )];
    }

    match parts[1] {
        "mode" => {
            if let Some(mode_str) = parts.get(2) {
                vec![create_message(
                    format!(
                        "Security mode changes must be made through the settings file.\n\
                         Requested mode: {mode_str}"
                    ),
                    MessageSender::Error,
                )]
            } else {
                vec![create_message(
                    "Security mode information is available via /settings command".to_string(),
                    MessageSender::System,
                )]
            }
        }

        "status" => {
            vec![create_message(
                "Security status information is available via /settings command".to_string(),
                MessageSender::System,
            )]
        }
        _ => vec![create_message(
            format!("Unknown security subcommand: {}", parts[1]),
            MessageSender::System,
        )],
    }
}

async fn handle_cost_command(state: &ActorState) -> Vec<ChatMessage> {
    let usage = &state.session_token_usage;
    let current_model = current_agent(state).agent.default_model().model;

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
        message.push_str(&format!(
            "  Average per 1K tokens: ${avg_cost_per_1k:.6}\n"
        ));
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
                format!(
                    "Unknown model: {model_name}. Use /models to list available models."
                ),
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
        let result = state
            .settings
            .update_setting(|s| s.set_agent_model(agent_name, settings.clone()));
        if let Err(e) = result {
            return vec![ChatMessage::error(format!(
                "Failed to save settings: {e:?}"
            ))];
        }
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
            "Model successfully set to {} for all agents{}.",
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
                format!(
                    "Unknown model: {model_name}. Use /models to list available models."
                ),
                MessageSender::Error,
            )]
        }
    };
    let settings = match parse_model_settings_overrides(&model, &parts[3..]) {
        Ok(s) => s,
        Err(e) => return vec![create_message(e, MessageSender::Error)],
    };
    let result = state
        .settings
        .update_setting(|s| s.set_agent_model(agent_name.to_string(), settings.clone()));
    if let Err(e) = result {
        return vec![create_message(
            format!("Failed to save settings: {e:?}"),
            MessageSender::System,
        )];
    }
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
            "Model successfully set to {} for agent {}{}.",
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
    let result = state
        .settings
        .update_setting(|s| s.review_level = new_level.clone());

    // Save the settings
    if let Err(e) = result {
        return vec![create_message(
            format!("Failed to save settings: {e}"),
            MessageSender::Error,
        )];
    }

    vec![create_message(
        format!("Review level set to: {new_level:?}"),
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
