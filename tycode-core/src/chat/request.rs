use crate::agents::agent::Agent;
use crate::agents::defaults::prepare_system_prompt_and_tools;
use crate::ai::error::AiError;
use crate::ai::model::{Model, ModelCost};
use crate::ai::provider::AiProvider;
use crate::ai::tweaks::resolve_from_settings;
use crate::ai::{ContentBlock, ConversationRequest, Message, MessageRole, ModelSettings};
use crate::context::ContextBuilder;
use crate::module::Module;
use crate::prompt::PromptBuilder;
use crate::settings::config::Settings;
use crate::settings::SettingsManager;
use crate::steering::SteeringDocuments;
use crate::tools::r#trait::ToolExecutor;
use crate::tools::registry::ToolRegistry;
use crate::tools::ToolName;
use anyhow::{bail, Result};
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
    tools: Vec<Arc<dyn ToolExecutor>>,
    prompt_builder: &PromptBuilder,
    context_builder: &ContextBuilder,
    modules: &[Arc<dyn Module>],
) -> Result<(ConversationRequest, ModelSettings)> {
    let settings = settings_manager.settings();
    let agent_name = agent.name();

    // Steering handles custom user-provided markdown files
    // Prompt components (autonomy, style, etc.) are handled by PromptBuilder
    let base_prompt =
        steering.build_system_prompt(agent.core_prompt(), !settings.disable_custom_steering);

    // Append prompt sections from components, filtered by agent's selection
    let prompt_selection = agent.requested_prompt_components();
    let filtered_content = prompt_builder.build(&settings, &prompt_selection, modules);
    let system_prompt = format!("{}{}", base_prompt, filtered_content);

    let model_settings = select_model_for_agent(&settings, provider, agent_name)?;

    let allowed_tool_names: Vec<ToolName> = agent.available_tools();

    let resolved_tweaks = resolve_from_settings(&settings, provider, model_settings.model);

    let module_tools: Vec<Arc<dyn ToolExecutor>> = modules.iter().flat_map(|m| m.tools()).collect();
    let all_tools: Vec<Arc<dyn ToolExecutor>> = tools.into_iter().chain(module_tools).collect();

    let tool_registry = ToolRegistry::new(all_tools);
    let available_tools = tool_registry.get_tool_definitions(&allowed_tool_names);

    let context_selection = agent.requested_context_components();
    let context_content = context_builder.build(&context_selection, modules).await;
    let mut conversation = conversation.to_vec();
    if conversation.is_empty() {
        bail!("No messages to send to AI. Conversation is empty!")
    }

    if !context_content.is_empty() {
        if let Some(last_msg) = conversation.last_mut() {
            if last_msg.role == MessageRole::User {
                last_msg.content.push(ContentBlock::Text(context_content));
            }
        }
    }

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

    Ok((request, model_settings))
}
