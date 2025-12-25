use crate::agents::agent::Agent;
use crate::agents::defaults::prepare_system_prompt_and_tools;
use crate::agents::tool_type::ToolType;
use crate::ai::error::AiError;
use crate::ai::model::{Model, ModelCost};
use crate::ai::provider::AiProvider;
use crate::ai::tweaks::resolve_from_settings;
use crate::ai::{ContentBlock, ConversationRequest, Message, ModelSettings};
use crate::chat::context::{build_context, ContextInputs};
use crate::chat::events::ContextInfo;
use crate::settings::config::Settings;
use crate::steering::SteeringDocuments;
use crate::tools::mcp::manager::McpManager;
use crate::tools::registry::ToolRegistry;
use anyhow::{bail, Result};
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
        "memory_summarizer" => settings.memory.summarizer_cost,
        "memory_manager" => settings.memory.recorder_cost,
        _ => settings.model_quality.unwrap_or(ModelCost::Unlimited),
    };

    let Some(model) = Model::select_for_cost(provider, quality) else {
        return Err(AiError::Terminal(anyhow::anyhow!(
            "No model available for {quality:?} in provider {}",
            provider.name()
        )));
    };
    Ok(model)
}

/// Prepare an AI conversation request with full context injection.
/// This is the shared implementation used by both chat/ai.rs and agents/runner.rs.
pub async fn prepare_request(
    agent: &dyn Agent,
    conversation: &[Message],
    provider: &dyn AiProvider,
    settings: &Settings,
    steering: &SteeringDocuments,
    context_inputs: &ContextInputs,
    mcp_manager: Option<&McpManager>,
) -> Result<(ConversationRequest, ContextInfo, ModelSettings)> {
    let agent_name = agent.name();

    let system_prompt = steering.build_system_prompt(
        agent.core_prompt(),
        agent.requested_builtins(),
        !settings.disable_custom_steering,
        settings.autonomy_level,
    );

    let model_settings = select_model_for_agent(settings, provider, agent_name)?;

    let allowed_tool_types: Vec<ToolType> = agent.available_tools().into_iter().collect();

    let resolved_tweaks = resolve_from_settings(settings, provider, model_settings.model);

    let tool_registry = ToolRegistry::new(
        context_inputs.workspace_roots.clone(),
        resolved_tweaks.file_modification_api,
        mcp_manager,
        settings.enable_type_analyzer,
        context_inputs.memory_log.clone(),
        context_inputs.additional_tools.clone(),
    )
    .await?;
    let available_tools = tool_registry.get_tool_definitions_for_types(&allowed_tool_types);

    let (context_text, context_info) = build_context(context_inputs, settings).await?;

    let mut conversation = conversation.to_vec();
    if conversation.is_empty() {
        bail!("No messages to send to AI. Conversation is empty!")
    }

    conversation
        .last_mut()
        .unwrap()
        .content
        .push(ContentBlock::Text(context_text));

    let (final_system_prompt, final_tools) = prepare_system_prompt_and_tools(
        &system_prompt,
        available_tools,
        resolved_tweaks.tool_call_style.clone(),
    );

    let request = ConversationRequest {
        messages: conversation,
        model: model_settings.clone(),
        system_prompt: final_system_prompt,
        stop_sequences: vec![],
        tools: final_tools,
    };

    debug!(?request, "AI request");

    Ok((request, context_info, model_settings))
}
