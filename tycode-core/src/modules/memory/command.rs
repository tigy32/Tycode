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

use super::compaction::{self, CompactionStore};

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
        "/memory <summarize|compact|show>"
    }

    async fn execute(&self, state: &mut ActorState, args: &[&str]) -> Vec<ChatMessage> {
        if args.is_empty() {
            return vec![create_message(
                "Usage: /memory <summarize|compact|show>".to_string(),
                MessageSender::System,
            )];
        }

        match args[0] {
            "summarize" => handle_memory_summarize_command(state).await,
            "compact" => handle_memory_compact_command(state).await,
            "show" => handle_memory_show_command(state),
            _ => vec![create_message(
                format!(
                    "Unknown memory subcommand: {}. Use: summarize, compact, show",
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
        images: vec![],
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
        state.provider.read().unwrap().clone(),
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
    let count = match compaction::memories_since_last_compaction(&state.memory_log) {
        Ok(c) => c,
        Err(e) => {
            return vec![create_message(
                format!("Failed to check memories: {e:?}"),
                MessageSender::Error,
            )];
        }
    };

    if count == 0 {
        return vec![create_message(
            "No new memories since last compaction.".to_string(),
            MessageSender::System,
        )];
    }

    state.event_sender.send_message(ChatMessage::system(format!(
        "Compacting {count} new memories..."
    )));

    match compaction::run_compaction(
        &state.memory_log,
        state.provider.read().unwrap().clone(),
        state.settings.clone(),
        state.modules.clone(),
        state.steering.clone(),
        state.prompt_builder.clone(),
        state.context_builder.clone(),
    )
    .await
    {
        Ok(Some(c)) => vec![create_message(
            format!(
                "=== Compaction Complete ===\n\n\
                Compacted {} memories through seq #{}.\n\
                Saved to: compaction_{}.json\n\n\
                Summary:\n{}",
                c.memories_count, c.through_seq, c.through_seq, c.summary
            ),
            MessageSender::System,
        )],
        Ok(None) => vec![create_message(
            "No new memories to compact.".to_string(),
            MessageSender::System,
        )],
        Err(e) => vec![create_message(
            format!("Memory compaction failed: {e:?}"),
            MessageSender::Error,
        )],
    }
}

fn handle_memory_show_command(state: &mut ActorState) -> Vec<ChatMessage> {
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

    let latest = match compaction_store.find_latest() {
        Ok(Some(c)) => c,
        Ok(None) => {
            return vec![create_message(
                "No compaction exists yet. Run /memory compact to create one.".to_string(),
                MessageSender::System,
            )];
        }
        Err(e) => {
            return vec![create_message(
                format!("Failed to read compaction: {e:?}"),
                MessageSender::Error,
            )];
        }
    };

    let pending = match compaction::memories_since_last_compaction(&state.memory_log) {
        Ok(c) => c,
        Err(e) => {
            return vec![create_message(
                format!("Failed to count pending memories: {e:?}"),
                MessageSender::Error,
            )];
        }
    };

    vec![create_message(
        format!(
            "=== Current Memory Compaction ===\n\n\
            Through seq: #{}\n\
            Memories compacted: {}\n\
            Created: {}\n\
            Pending (uncompacted): {}\n\n\
            ---\n\n{}",
            latest.through_seq,
            latest.memories_count,
            latest.created_at.format("%Y-%m-%d %H:%M:%S UTC"),
            pending,
            latest.summary
        ),
        MessageSender::System,
    )]
}
