use chrono::Utc;

use crate::ai::{Content, Message, MessageRole};
use crate::chat::actor::ActorState;
use crate::chat::events::{ChatMessage, MessageSender};
use crate::chat::request::select_model_for_agent;
use crate::chat::tools;
use crate::module::SlashCommand;

use super::{compact_conversation, count_reasoning_blocks, prune_reasoning_blocks};

pub struct CompactReasoningCommand;

#[async_trait::async_trait(?Send)]
impl SlashCommand for CompactReasoningCommand {
    fn name(&self) -> &'static str {
        "compact"
    }

    fn description(&self) -> &'static str {
        "Compact conversation: summarize entire history or prune reasoning blocks"
    }

    fn usage(&self) -> &'static str {
        "/compact              - Summarize entire conversation into one message
/compact reasoning <N> - Keep only N most recent reasoning blocks"
    }

    async fn execute(&self, state: &mut ActorState, args: &[&str]) -> Vec<ChatMessage> {
        if args.is_empty() {
            return compact_conversation_cmd(state).await;
        }

        if args[0] != "reasoning" {
            return vec![create_system_message(format!(
                "Unknown compact subcommand: {}. Use: /compact or /compact reasoning <count>",
                args[0]
            ))];
        }

        let count = args.get(1).and_then(|s| s.parse::<usize>().ok());
        let count = match count {
            Some(c) if c > 0 => c,
            _ => {
                return vec![create_system_message(
                    "Please specify a positive number of reasoning blocks to retain. Example: /compact reasoning 10".to_string(),
                )];
            }
        };

        prune_reasoning_blocks_cmd(state, count)
    }
}

async fn compact_conversation_cmd(state: &mut ActorState) -> Vec<ChatMessage> {
    let (conversation, agent_name) = tools::current_agent(state, |a| {
        (a.conversation.clone(), a.agent.name().to_string())
    });

    if conversation.is_empty() {
        return vec![create_system_message(
            "No conversation to compact.".to_string(),
        )];
    }

    let messages_before = conversation.len();

    let provider = state.provider.read().unwrap().clone();
    let settings_snapshot = state.settings.settings();
    let model_settings =
        match select_model_for_agent(&settings_snapshot, provider.as_ref(), &agent_name) {
            Ok(ms) => ms,
            Err(e) => {
                return vec![create_system_message(format!(
                    "Failed to get model settings for compaction: {e}"
                ))];
            }
        };

    match compact_conversation(&conversation, &provider, &model_settings).await {
        Ok(summary_text) => {
            tools::current_agent_mut(state, |agent| {
                agent.conversation.clear();
                agent.conversation.push(Message {
                    role: MessageRole::User,
                    content: Content::text_only(format!(
                        "Context summary from previous conversation:\n{}\n\nPlease continue assisting based on this context.",
                        summary_text
                    )),
                });
            });

            state.tracked_files.clear();

            vec![create_system_message(format!(
                "Compaction complete: {} messages → 1 (summary). Tracked files cleared.",
                messages_before
            ))]
        }
        Err(e) => {
            vec![create_system_message(format!("Compaction failed: {e}"))]
        }
    }
}

fn create_system_message(content: String) -> ChatMessage {
    ChatMessage {
        content,
        sender: MessageSender::System,
        timestamp: Utc::now().timestamp_millis() as u64,
        reasoning: None,
        tool_calls: Vec::new(),
        model_info: None,
        token_usage: None,
        context_breakdown: None,
        images: vec![],
    }
}

fn prune_reasoning_blocks_cmd(state: &mut ActorState, count: usize) -> Vec<ChatMessage> {
    // Get the current count of reasoning blocks
    let total_blocks =
        tools::current_agent(state, |agent| count_reasoning_blocks(&agent.conversation));

    if total_blocks == 0 {
        return vec![create_system_message(
            "No reasoning blocks found in conversation history.".to_string(),
        )];
    }

    if total_blocks <= count {
        return vec![create_system_message(format!(
            "Conversation has {} reasoning block(s). No pruning needed (threshold: {}).",
            total_blocks, count
        ))];
    }

    // Prune reasoning blocks using the existing module function
    let pruned_count = total_blocks - count;
    tools::current_agent_mut(state, |agent| {
        prune_reasoning_blocks(&mut agent.conversation, count);
    });

    vec![create_system_message(format!(
        "Compacted conversation by pruning {} reasoning block(s). Retained {} most recent reasoning block(s).",
        pruned_count, count
    ))]
}
