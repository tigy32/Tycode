use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::Utc;

use crate::agents::agent::ActiveAgent;
use crate::agents::memory_summarizer::MemorySummarizerAgent;
use crate::agents::runner::AgentRunner;
use crate::ai::Message;
use crate::chat::actor::ActorState;
use crate::chat::events::{ChatMessage, MessageSender};
use crate::module::SlashCommand;
use crate::spawn::complete_task::CompleteTask;
use crate::tools::r#trait::ToolExecutor;

use super::compaction::{Compaction, CompactionStore};

pub struct MemorySlashCommand;

#[async_trait::async_trait(?Send)]
impl SlashCommand for MemorySlashCommand {
    fn name(&self) -> &'static str {
        "memory"
    }

    fn description(&self) -> &'static str {
        "Manage memories (summarize, compact)"
    }

    fn usage(&self) -> &'static str {
        "/memory <summarize|compact>"
    }

    async fn execute(&self, state: &mut ActorState, args: &[&str]) -> Vec<ChatMessage> {
        if args.is_empty() {
            return vec![create_message(
                "Usage: /memory <summarize|compact>".to_string(),
                MessageSender::System,
            )];
        }

        match args[0] {
            "summarize" => handle_memory_summarize_command(state).await,
            "compact" => handle_memory_compact_command(state).await,
            _ => vec![create_message(
                format!(
                    "Unknown memory subcommand: {}. Use: summarize, compact",
                    args[0]
                ),
                MessageSender::Error,
            )],
        }
    }
}

fn create_message(content: String, sender: MessageSender) -> ChatMessage {
    ChatMessage {
        content,
        sender,
        timestamp: Utc::now().timestamp_millis() as u64,
        reasoning: None,
        tool_calls: Vec::new(),
        model_info: None,
        token_usage: None,
    }
}

async fn handle_memory_summarize_command(state: &mut ActorState) -> Vec<ChatMessage> {
    let memories = match state.memory_log.read_all() {
        Ok(m) => m,
        Err(e) => {
            return vec![create_message(
                format!("Failed to read memories: {e:?}"),
                MessageSender::Error,
            )];
        }
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

    let mut tools: BTreeMap<String, Arc<dyn ToolExecutor + Send + Sync>> = BTreeMap::new();
    tools.insert(
        CompleteTask::tool_name().to_string(),
        Arc::new(CompleteTask::standalone()),
    );

    let runner = AgentRunner::new(
        state.provider.clone(),
        state.settings.clone(),
        tools,
        state.modules.clone(),
        state.steering.clone(),
        state.prompt_builder.clone(),
        state.context_builder.clone(),
    );
    let agent = MemorySummarizerAgent::new();
    let mut active_agent = ActiveAgent::new(Arc::new(agent));
    active_agent.conversation.push(Message::user(formatted));

    match runner.run(active_agent, 10).await {
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

async fn handle_memory_compact_command(state: &mut ActorState) -> Vec<ChatMessage> {
    let memory_dir = match state.memory_log.path().parent() {
        Some(dir) => dir.to_path_buf(),
        None => {
            return vec![create_message(
                "Failed to get memory directory".to_string(),
                MessageSender::Error,
            )];
        }
    };
    let compaction_store = CompactionStore::new(memory_dir);

    let latest_compaction = match compaction_store.find_latest() {
        Ok(c) => c,
        Err(e) => {
            return vec![create_message(
                format!("Failed to read compaction history: {e:?}"),
                MessageSender::Error,
            )];
        }
    };

    let through_seq = latest_compaction
        .as_ref()
        .map(|c| c.through_seq)
        .unwrap_or(0);
    let previous_summary = latest_compaction.as_ref().map(|c| c.summary.clone());

    let all_memories = match state.memory_log.read_all() {
        Ok(m) => m,
        Err(e) => {
            return vec![create_message(
                format!("Failed to read memories: {e:?}"),
                MessageSender::Error,
            )];
        }
    };

    let new_memories: Vec<_> = all_memories
        .into_iter()
        .filter(|m| m.seq > through_seq)
        .collect();

    if new_memories.is_empty() && previous_summary.is_none() {
        return vec![create_message(
            "No memories to compact.".to_string(),
            MessageSender::System,
        )];
    }

    if new_memories.is_empty() {
        return vec![create_message(
            "No new memories since last compaction.".to_string(),
            MessageSender::System,
        )];
    }

    let memory_count = new_memories.len();
    let max_seq = new_memories.iter().map(|m| m.seq).max().unwrap_or(0);

    state.event_sender.send_message(ChatMessage::system(format!(
        "Compacting {} new memories (through seq #{})...",
        memory_count, max_seq
    )));

    let mut formatted = String::new();

    if let Some(prev_summary) = &previous_summary {
        formatted.push_str("# Previous Compaction Summary\n\n");
        formatted.push_str(prev_summary);
        formatted.push_str("\n\n---\n\n");
    }

    formatted.push_str("# New Memories Since Last Compaction\n\n");
    for memory in &new_memories {
        formatted.push_str(&format!(
            "## Memory #{} ({})\n",
            memory.seq,
            memory.source.as_deref().unwrap_or("global")
        ));
        formatted.push_str(&memory.content);
        formatted.push_str("\n\n");
    }

    formatted.push_str("\n---\n\n");
    formatted.push_str(
        "Please consolidate the previous summary (if any) with the new memories \
        into a single comprehensive summary.",
    );

    let mut tools: BTreeMap<String, Arc<dyn ToolExecutor + Send + Sync>> = BTreeMap::new();
    tools.insert(
        CompleteTask::tool_name().to_string(),
        Arc::new(CompleteTask::standalone()),
    );

    let runner = AgentRunner::new(
        state.provider.clone(),
        state.settings.clone(),
        tools,
        state.modules.clone(),
        state.steering.clone(),
        state.prompt_builder.clone(),
        state.context_builder.clone(),
    );
    let agent = MemorySummarizerAgent::new();
    let mut active_agent = ActiveAgent::new(Arc::new(agent));
    active_agent.conversation.push(Message::user(formatted));

    match runner.run(active_agent, 10).await {
        Ok(summary) => {
            let compaction = Compaction {
                through_seq: max_seq,
                summary: summary.clone(),
                created_at: Utc::now(),
                memories_count: memory_count,
                previous_compaction_seq: latest_compaction.map(|c| c.through_seq),
            };

            if let Err(e) = compaction_store.save(&compaction) {
                return vec![create_message(
                    format!(
                        "Compaction generated but failed to save: {e:?}\n\nSummary:\n{}",
                        summary
                    ),
                    MessageSender::Error,
                )];
            }

            vec![create_message(
                format!(
                    "=== Compaction Complete ===\n\n\
                    Compacted {} memories through seq #{}.\n\
                    Saved to: compaction_{}.json\n\n\
                    Summary:\n{}",
                    memory_count, max_seq, max_seq, summary
                ),
                MessageSender::System,
            )]
        }
        Err(e) => vec![create_message(
            format!("Memory compaction failed: {e:?}"),
            MessageSender::Error,
        )],
    }
}
