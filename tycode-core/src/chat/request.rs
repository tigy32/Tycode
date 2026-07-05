use crate::agents::agent::Agent;
use crate::agents::catalog::AgentCatalog;
use crate::ai::error::AiError;
use crate::ai::model::{Model, ModelCost};
use crate::ai::provider::AiProvider;
use crate::ai::types::ContextBreakdown;
use crate::ai::{Content, ContentBlock, ConversationRequest, Message, MessageRole, ModelSettings};
use crate::module::ContextBuilder;
use crate::module::Module;
use crate::module::PromptBuilder;
use crate::modules::memory::MemoryConfig;
use crate::settings::config::Settings;
use crate::settings::SettingsManager;
use crate::spawn::build_tools;
use crate::steering::SteeringDocuments;
use crate::tools::r#trait::SharedTool;
use crate::tools::registry::ToolRegistry;
use anyhow::{bail, Context, Result};
use std::sync::Arc;
use tracing::debug;

/// Select the appropriate model for an agent based on settings and cost constraints.
pub fn select_model_for_agent(
    settings: &Settings,
    provider: &dyn AiProvider,
    agent_name: &str,
) -> Result<ModelSettings, AiError> {
    if let Some(override_model) = settings.get_agent_model(agent_name) {
        return Ok(override_model.clone());
    }

    let quality = match agent_name {
        "memory_summarizer" | "memory_manager" => {
            let memory_config: MemoryConfig = settings.get_module_config("memory");
            if agent_name == "memory_summarizer" {
                memory_config.summarizer_cost
            } else {
                memory_config.recorder_cost
            }
        }
        _ => settings.model_quality.unwrap_or(ModelCost::Unlimited),
    };

    let Some(mut model) = Model::select_for_cost(provider, quality) else {
        return Err(AiError::Terminal(anyhow::anyhow!(
            "No model available for {quality:?} in provider {}",
            provider.name()
        )));
    };

    if let Some(effort) = &settings.reasoning_effort {
        model.reasoning_budget = effort.clone();
    }

    Ok(model)
}

/// Build ModelSettings for an explicitly pinned model, applying the global
/// reasoning-effort override like name-based selection does.
pub fn pinned_model_settings(model: Model, settings: &Settings) -> ModelSettings {
    let mut model_settings = model.default_settings();
    if let Some(effort) = &settings.reasoning_effort {
        model_settings.reasoning_budget = effort.clone();
    }
    model_settings
}

/// Prepare an AI conversation request. This handles the work of fully
/// assembling a request - including building the prompt (from the agent and
/// prompt_builder), the context message (from the context_builder), selecting
/// the correct model, etc.
pub async fn prepare_request(
    agent: &dyn Agent,
    conversation: &[Message],
    provider: &dyn AiProvider,
    settings_manager: SettingsManager,
    steering: &SteeringDocuments,
    prompt_builder: &PromptBuilder,
    context_builder: &ContextBuilder,
    modules: &[Arc<dyn Module>],
    catalog: &Arc<AgentCatalog>,
    model_override: Option<ModelSettings>,
) -> Result<(
    ConversationRequest,
    ModelSettings,
    ContextBreakdown,
    Vec<SharedTool>,
)> {
    let agent_name = agent.name();
    let settings = settings_manager.settings();
    let tools = build_tools(
        modules,
        catalog.clone(),
        agent_name,
        settings.orchestration_mode,
    )
    .await;

    // Steering handles custom user-provided markdown files
    // Prompt components (autonomy, style, etc.) are handled by PromptBuilder
    let mut base_prompt =
        steering.build_system_prompt(agent.core_prompt(), !settings.disable_custom_steering);

    // The orchestration mode is a policy on the conversational root: it
    // governs how tycode implements changes (see the matching mechanical
    // swarm gate in the spawn allow-list).
    if agent_name == crate::agents::tycode::TycodeAgent::NAME {
        base_prompt.push_str("\n\n");
        base_prompt.push_str(crate::agents::tycode::orchestration_policy(
            settings.orchestration_mode,
        ));
    }

    let prompt_selection = agent.requested_prompt_components();
    let filtered_content = prompt_builder.build(&settings, &prompt_selection, modules);
    let system_prompt = format!("{}{}", base_prompt, filtered_content);

    let model_settings = match model_override {
        Some(pinned) => pinned,
        None => select_model_for_agent(&settings, provider, agent_name)?,
    };

    let allowed_tool_names: Vec<crate::tools::ToolName> = agent.available_tools();

    let tool_registry = ToolRegistry::new(tools.clone());
    let available_tools = tool_registry.get_tool_definitions(&allowed_tool_names);

    let context_selection = agent.requested_context_components();
    let context_content = context_builder.build(&context_selection, modules).await;
    let mut conversation = conversation.to_vec();
    if conversation.is_empty() {
        bail!("No messages to send to AI. Conversation is empty!")
    }

    let context_injection_bytes = context_content.len();

    let mut reasoning_bytes: usize = 0;
    let mut tool_io_bytes: usize = 0;
    let mut conversation_history_bytes: usize = 0;
    for msg in &conversation {
        for block in msg.content.blocks() {
            let block_bytes = serde_json::to_string(block)
                .context("failed to serialize content block for context breakdown")?
                .len();
            match block {
                ContentBlock::ReasoningContent(_) => reasoning_bytes += block_bytes,
                ContentBlock::ToolUse(_) | ContentBlock::ToolResult(_) => {
                    tool_io_bytes += block_bytes
                }
                _ => conversation_history_bytes += block_bytes,
            }
        }
    }

    if !context_content.is_empty() {
        let context_message = Message {
            role: MessageRole::User,
            content: Content::text_only(context_content),
        };
        let insert_at = match conversation.last() {
            Some(last)
                if last.role == MessageRole::User
                    && last
                        .content
                        .blocks()
                        .iter()
                        .any(|block| matches!(block, ContentBlock::ToolResult(_))) =>
            {
                conversation.len()
            }
            _ => conversation
                .iter()
                .rposition(|message| message.role == MessageRole::User)
                .unwrap_or(conversation.len()),
        };
        conversation.insert(insert_at, context_message);
    }

    let tool_definitions_bytes = serde_json::to_string(&available_tools)
        .context("failed to serialize tool definitions for context breakdown")?
        .len();
    let system_prompt_bytes = system_prompt.len() + tool_definitions_bytes;

    let context_breakdown = ContextBreakdown {
        context_window: model_settings.model.context_window(),
        input_tokens: 0,
        system_prompt_bytes,
        tool_io_bytes,
        conversation_history_bytes,
        reasoning_bytes,
        context_injection_bytes,
    };

    let request = ConversationRequest {
        messages: conversation,
        model: model_settings.clone(),
        system_prompt,
        stop_sequences: vec![],
        tools: available_tools,
    };

    debug!(?request, "AI request");

    Ok((request, model_settings, context_breakdown, tools))
}
