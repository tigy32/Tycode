//! Background memory management task.

use std::sync::Arc;

use tracing::{info, warn};

use crate::agents::agent::ActiveAgent;
use crate::agents::memory_manager::MemoryManagerAgent;
use crate::agents::runner::AgentRunner;
use crate::ai::provider::AiProvider;
use crate::ai::types::{ContentBlock, Message, MessageRole};
use crate::module::ContextBuilder;
use crate::module::Module;
use crate::module::PromptBuilder;
use crate::settings::manager::SettingsManager;
use crate::steering::SteeringDocuments;

use super::compaction;
use super::config::MemoryConfig;
use super::log::MemoryLog;

/// Spawn the memory manager agent as a background task.
/// This is fire-and-forget - errors are logged but not propagated.
///
/// # Arguments
/// * `ai_provider` - The AI provider to use
/// * `memory_log` - The memory log to store memories in
/// * `settings` - Settings manager
/// * `conversation` - The conversation messages to analyze (last N messages, pre-sliced by caller)
/// * `steering` - Steering documents
/// * `mcp_manager` - MCP manager for tool access
pub fn spawn_memory_manager(
    ai_provider: Arc<dyn AiProvider>,
    memory_log: Arc<MemoryLog>,
    settings: SettingsManager,
    conversation: Vec<Message>,
    steering: SteeringDocuments,
    prompt_builder: PromptBuilder,
    context_builder: ContextBuilder,
    modules: Vec<Arc<dyn Module>>,
) {
    let compaction_log = memory_log.clone();
    let compaction_provider = ai_provider.clone();
    let compaction_settings = settings.clone();
    let compaction_modules = modules.clone();
    let compaction_steering = steering.clone();
    let compaction_prompt = prompt_builder.clone();
    let compaction_context = context_builder.clone();

    tokio::task::spawn_local(async move {
        let msg_count = conversation.len();
        info!(messages = msg_count, "Memory manager starting");

        let mut active_agent = ActiveAgent::new(Arc::new(MemoryManagerAgent));
        active_agent.conversation = conversation;
        active_agent.conversation.push(Message::user(
            "=== MEMORY MANAGER AGENT ===\n\n\
            You are now the Memory Manager agent. Your conversation history contains the interaction \
            between the user and a coding agent that just concluded. Your task is to analyze that conversation \
            history (all messages before this one) and extract any learnings worth remembering.\n\n\
            Look for:\n\
            - User preferences or corrections\n\
            - Project-specific decisions\n\
            - Coding style preferences\n\
            - Technical constraints mentioned\n\n\
            Use append_memory for each distinct learning, then call complete_task. \
            If the conversation contains no extractable learnings, call complete_task immediately."
        ));

        let runner = AgentRunner::new(
            ai_provider,
            settings,
            modules,
            steering,
            prompt_builder,
            context_builder,
        );

        match runner.run(active_agent, 2).await {
            Ok(_) => info!("Memory manager completed"),
            Err(e) => warn!(error = ?e, "Memory manager failed"),
        }

        maybe_auto_compact(
            &compaction_log,
            &compaction_settings,
            compaction_provider,
            compaction_modules,
            compaction_steering,
            compaction_prompt,
            compaction_context,
        )
        .await;
    });
}

/// Spawn a background compaction task. Fire-and-forget.
pub fn spawn_background_compaction(
    memory_log: Arc<MemoryLog>,
    ai_provider: Arc<dyn AiProvider>,
    settings: SettingsManager,
    modules: Vec<Arc<dyn Module>>,
    steering: SteeringDocuments,
    prompt_builder: PromptBuilder,
    context_builder: ContextBuilder,
) {
    tokio::task::spawn_local(async move {
        info!("Background compaction starting");
        match compaction::run_compaction(
            &memory_log,
            ai_provider,
            settings,
            modules,
            steering,
            prompt_builder,
            context_builder,
        )
        .await
        {
            Ok(Some(c)) => info!(
                through_seq = c.through_seq,
                memories = c.memories_count,
                "Background compaction completed"
            ),
            Ok(None) => info!("Background compaction: no new memories"),
            Err(e) => warn!(error = ?e, "Background compaction failed"),
        }
    });
}

async fn maybe_auto_compact(
    memory_log: &MemoryLog,
    settings: &SettingsManager,
    provider: Arc<dyn AiProvider>,
    modules: Vec<Arc<dyn Module>>,
    steering: SteeringDocuments,
    prompt_builder: PromptBuilder,
    context_builder: ContextBuilder,
) {
    let config: MemoryConfig = settings.get_module_config::<MemoryConfig>("memory");
    let threshold = match config.auto_compaction_threshold {
        Some(t) if t > 0 => t,
        _ => return,
    };

    let pending = match compaction::memories_since_last_compaction(memory_log) {
        Ok(c) => c,
        Err(e) => {
            warn!(error = ?e, "Failed to check memories for auto-compaction");
            return;
        }
    };

    if pending < threshold {
        return;
    }

    info!(pending, threshold, "Auto-compaction threshold reached");
    match compaction::run_compaction(
        memory_log,
        provider,
        settings.clone(),
        modules,
        steering,
        prompt_builder,
        context_builder,
    )
    .await
    {
        Ok(Some(c)) => info!(
            through_seq = c.through_seq,
            memories = c.memories_count,
            "Auto-compaction completed"
        ),
        Ok(None) => info!("Auto-compaction: no new memories"),
        Err(e) => warn!(error = ?e, "Auto-compaction failed"),
    }
}

/// Safely slice a conversation to get the last N messages without tearing tool call pairs.
/// Returns messages starting from a clean boundary (User message without orphaned ToolResults).
pub fn safe_conversation_slice(conversation: &[Message], max_messages: usize) -> Vec<Message> {
    if conversation.len() <= max_messages {
        return conversation.to_vec();
    }

    let start_idx = conversation.len().saturating_sub(max_messages);
    let mut slice = &conversation[start_idx..];

    // Tool results require matching tool uses from prior assistant messages.
    // Starting mid-pair would create invalid conversation structure for the AI model.
    while !slice.is_empty() {
        let first = &slice[0];
        if first.role == MessageRole::User {
            let has_tool_results = first
                .content
                .blocks()
                .iter()
                .any(|b| matches!(b, ContentBlock::ToolResult(_)));
            if !has_tool_results {
                break;
            }
        }
        slice = &slice[1..];
    }

    slice.to_vec()
}
