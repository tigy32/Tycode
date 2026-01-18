//! Background memory management task.

use std::collections::BTreeMap;
use std::sync::Arc;

use tracing::{info, warn};

use crate::agents::agent::ActiveAgent;
use crate::agents::memory_manager::MemoryManagerAgent;
use crate::agents::runner::AgentRunner;
use crate::ai::provider::AiProvider;
use crate::ai::types::{ContentBlock, Message, MessageRole};
use crate::context::ContextBuilder;
use crate::module::Module;
use crate::prompt::PromptBuilder;
use crate::settings::manager::SettingsManager;
use crate::steering::SteeringDocuments;
use crate::tools::complete_task::CompleteTask;
use crate::tools::r#trait::ToolExecutor;

use super::log::MemoryLog;
use super::tool::AppendMemoryTool;

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
    let mut tools: BTreeMap<String, Arc<dyn ToolExecutor + Send + Sync>> = BTreeMap::new();
    tools.insert(
        "append_memory".into(),
        Arc::new(AppendMemoryTool::new(memory_log.clone())),
    );
    tools.insert("complete_task".into(), Arc::new(CompleteTask));

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
            tools,
            modules,
            steering,
            prompt_builder,
            context_builder,
        );

        match runner.run(active_agent, 2).await {
            Ok(_) => info!("Memory manager completed"),
            Err(e) => warn!(error = ?e, "Memory manager failed"),
        }
    });
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
