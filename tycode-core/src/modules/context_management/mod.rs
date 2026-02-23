//! Context management module for controlling conversation growth.
//!
//! Provides functionality for:
//! - Pruning reasoning blocks to manage context window size
//! - Full conversation compaction (summarization)

use schemars::schema::RootSchema;
use schemars::schema_for;

use crate::ai::types::ContentBlock;
use crate::module::Module;
pub mod command;
pub mod config;

use crate::settings::manager::SettingsManager;
use crate::tools::r#trait::SharedTool;
pub use config::ContextManagementConfig;

use crate::ai::{Content, ConversationRequest, Message, MessageRole, ModelSettings};
use anyhow::Result;
use std::sync::Arc;

use crate::ai::provider::AiProvider;

/// Context management module.
///
/// Currently provides settings for automatic reasoning block pruning.
/// Future extensions may include manual compaction via slash commands.
pub struct ContextManagementModule {
    _settings: SettingsManager,
}

impl ContextManagementModule {
    pub fn new(settings: SettingsManager) -> Self {
        Self {
            _settings: settings,
        }
    }
}

impl Module for ContextManagementModule {
    fn prompt_components(&self) -> Vec<std::sync::Arc<dyn crate::module::PromptComponent>> {
        vec![]
    }

    fn context_components(&self) -> Vec<std::sync::Arc<dyn crate::module::ContextComponent>> {
        vec![]
    }

    fn tools(&self) -> Vec<SharedTool> {
        vec![]
    }

    fn slash_commands(&self) -> Vec<std::sync::Arc<dyn crate::module::SlashCommand>> {
        vec![std::sync::Arc::new(command::CompactReasoningCommand)]
    }

    fn settings_namespace(&self) -> Option<&'static str> {
        Some(ContextManagementConfig::NAMESPACE)
    }

    fn settings_json_schema(&self) -> Option<RootSchema> {
        Some(schema_for!(ContextManagementConfig))
    }
}

/// Used by the pruning threshold check and debug logging to track how many
/// reasoning blocks exist across the full conversation history.
pub fn count_reasoning_blocks(messages: &[crate::ai::types::Message]) -> usize {
    messages
        .iter()
        .map(|msg| {
            msg.content
                .blocks()
                .iter()
                .filter(|block| matches!(block, ContentBlock::ReasoningContent(_)))
                .count()
        })
        .sum()
}

/// Compacts a conversation by summarizing it into a single message.
///
/// This function sends the conversation to the AI provider for summarization,
/// filtering out ToolUse/ToolResult blocks (which would cause validation errors
/// when tools aren't offered). Returns the summary text.
///
/// The caller is responsible for replacing the conversation with the summary.
pub async fn compact_conversation(
    messages: &[Message],
    provider: &Arc<dyn AiProvider>,
    model_settings: &ModelSettings,
) -> Result<String> {
    let summarization_prompt = "Please provide a concise summary of the conversation so far, preserving all critical context, decisions, and important details. The summary will be used to continue the conversation efficiently. Focus on:
1. Key decisions made
2. Important context about the task
3. Current state of work and remaining work
4. Any critical information needed to continue effectively";

    // Filter ToolUse/ToolResult blocks before summarization to avoid Bedrock's
    // toolConfig validation error. Bedrock requires toolConfig when messages contain
    // these blocks, but summarization requests don't offer tools (tools: vec![]).
    // Only conversational content (Text, ReasoningContent) is needed for summarization.
    let filtered_messages: Vec<Message> = messages
        .iter()
        .cloned()
        .map(|mut msg| {
            let filtered_blocks: Vec<ContentBlock> = msg
                .content
                .clone()
                .into_blocks()
                .into_iter()
                .filter(|block| {
                    !matches!(block, ContentBlock::ToolUse { .. })
                        && !matches!(block, ContentBlock::ToolResult { .. })
                })
                .collect();
            msg.content = Content::new(filtered_blocks);
            msg
        })
        .collect();

    let mut summary_request = ConversationRequest {
        messages: filtered_messages,
        model: model_settings.clone(),
        system_prompt: "You are a conversation summarizer. Create concise, comprehensive summaries that preserve critical context.".to_string(),
        stop_sequences: vec![],
        tools: vec![],
    };

    summary_request.messages.push(Message {
        role: MessageRole::User,
        content: Content::text_only(summarization_prompt.to_string()),
    });

    let summary_response = provider
        .converse(summary_request.clone())
        .await
        .map_err(|e| anyhow::anyhow!("Failed to summarize conversation: {e:?}"))?;

    Ok(summary_response.content.text())
}

/// Reasoning blocks grow unboundedly and dominate context window usage in
/// long conversations. Removing oldest-first preserves the model's most
/// recent chain-of-thought while reclaiming space for new content.
pub fn prune_reasoning_blocks(messages: &mut Vec<crate::ai::types::Message>, retain_count: usize) {
    let total_blocks = count_reasoning_blocks(messages);
    if total_blocks <= retain_count {
        return;
    }

    let to_remove = total_blocks - retain_count;
    let mut removed = 0;

    // Process messages from oldest to newest, removing reasoning blocks
    for msg in messages.iter_mut() {
        if removed >= to_remove {
            break;
        }

        // Extract current blocks and filter
        let current_blocks: Vec<ContentBlock> = msg.content.clone().into_blocks();
        let mut new_blocks = Vec::with_capacity(current_blocks.len());

        for block in current_blocks {
            if matches!(block, ContentBlock::ReasoningContent(_)) {
                removed += 1;
                if removed > to_remove {
                    // We've removed enough, keep this one
                    new_blocks.push(block);
                }
            } else {
                new_blocks.push(block);
            }
        }
        msg.content = crate::ai::types::Content::new(new_blocks);
    }
}

/// Applies hysteresis to avoid pruning on every request — only acts when
/// the count reaches `trigger_count`, then drops back to `retain_count`.
/// This batching preserves prompt caching between prune events.
pub fn prune_with_thresholds(
    messages: &mut Vec<crate::ai::types::Message>,
    trigger_count: usize,
    retain_count: usize,
) -> bool {
    let current_count = count_reasoning_blocks(messages);
    if current_count >= trigger_count {
        prune_reasoning_blocks(messages, retain_count);
        true
    } else {
        false
    }
}
