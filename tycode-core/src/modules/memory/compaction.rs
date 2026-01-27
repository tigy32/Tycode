//! Compaction storage for memory summarization.
//!
//! Compactions are AI-generated summaries of memories stored as separate JSON files.
//! Each compaction covers all memories through a specific sequence number.
//! The raw memory log is never truncated - compactions provide a compressed view.
//!
//! Files are named `compaction_<through_seq>.json` (e.g., `compaction_42.json`).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::agents::agent::ActiveAgent;
use crate::agents::memory_summarizer::MemorySummarizerAgent;
use crate::agents::runner::AgentRunner;
use crate::ai::provider::AiProvider;
use crate::ai::Message;
use crate::module::{ContextBuilder, Module, PromptBuilder};
use crate::settings::manager::SettingsManager;
use crate::spawn::complete_task::CompleteTask;
use crate::steering::SteeringDocuments;
use crate::tools::r#trait::ToolExecutor;

use super::log::MemoryLog;

/// A compaction record representing a summary of memories through a specific sequence number.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Compaction {
    /// Last memory sequence number included in this compaction
    pub through_seq: u64,
    /// AI-generated summary of all memories through this sequence
    pub summary: String,
    /// When this compaction was created
    pub created_at: DateTime<Utc>,
    /// Number of memories that were compacted
    pub memories_count: usize,
    /// Sequence number of the previous compaction (for auditability)
    pub previous_compaction_seq: Option<u64>,
}

/// Manages compaction files in a directory.
///
/// Files are named `compaction_<through_seq>.json` where through_seq is the
/// last memory sequence number included in that compaction.
#[derive(Debug)]
pub struct CompactionStore {
    directory: PathBuf,
}

impl CompactionStore {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    /// Find the latest compaction by scanning for the highest sequence number.
    pub fn find_latest(&self) -> Result<Option<Compaction>> {
        if !self.directory.exists() {
            return Ok(None);
        }

        let mut highest_seq: Option<u64> = None;
        let entries = fs::read_dir(&self.directory)
            .with_context(|| format!("Failed to read directory: {}", self.directory.display()))?;

        for entry in entries {
            let entry = entry.context("Failed to read directory entry")?;
            let file_name = entry.file_name();
            let file_name_str = file_name.to_string_lossy();

            if let Some(seq) = parse_compaction_seq(&file_name_str) {
                highest_seq = Some(highest_seq.map_or(seq, |h| h.max(seq)));
            }
        }

        match highest_seq {
            Some(seq) => self.read(seq).map(Some),
            None => Ok(None),
        }
    }

    /// Save a compaction to disk.
    pub fn save(&self, compaction: &Compaction) -> Result<()> {
        fs::create_dir_all(&self.directory).with_context(|| {
            format!(
                "Failed to create compaction directory: {}",
                self.directory.display()
            )
        })?;

        let path = self.compaction_path(compaction.through_seq);
        let content =
            serde_json::to_string_pretty(compaction).context("Failed to serialize compaction")?;

        fs::write(&path, content)
            .with_context(|| format!("Failed to write compaction file: {}", path.display()))?;

        Ok(())
    }

    /// Read a specific compaction by its through_seq.
    pub fn read(&self, through_seq: u64) -> Result<Compaction> {
        let path = self.compaction_path(through_seq);
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read compaction file: {}", path.display()))?;

        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse compaction file: {}", path.display()))
    }

    fn compaction_path(&self, through_seq: u64) -> PathBuf {
        self.directory
            .join(format!("compaction_{}.json", through_seq))
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

/// Run compaction: summarize new memories since last compaction using an AI agent.
/// Returns None if there are no new memories to compact.
pub async fn run_compaction(
    memory_log: &MemoryLog,
    provider: Arc<dyn AiProvider>,
    settings: SettingsManager,
    modules: Vec<Arc<dyn Module>>,
    steering: SteeringDocuments,
    prompt_builder: PromptBuilder,
    context_builder: ContextBuilder,
) -> Result<Option<Compaction>> {
    let memory_dir = memory_log
        .path()
        .parent()
        .context("Failed to get memory directory")?;
    let compaction_store = CompactionStore::new(memory_dir.to_path_buf());

    let latest_compaction = compaction_store.find_latest()?;
    let through_seq = latest_compaction
        .as_ref()
        .map(|c| c.through_seq)
        .unwrap_or(0);
    let previous_summary = latest_compaction.as_ref().map(|c| c.summary.clone());

    let all_memories = memory_log.read_all()?;
    let new_memories: Vec<_> = all_memories
        .into_iter()
        .filter(|m| m.seq > through_seq)
        .collect();

    if new_memories.is_empty() {
        return Ok(None);
    }

    let memory_count = new_memories.len();
    let max_seq = new_memories.iter().map(|m| m.seq).max().unwrap_or(0);

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

    formatted.push_str(
        "\n---\n\n\
        Please consolidate the previous summary (if any) with the new memories \
        into a single comprehensive summary.",
    );

    let mut tools: BTreeMap<String, Arc<dyn ToolExecutor + Send + Sync>> = BTreeMap::new();
    tools.insert(
        CompleteTask::tool_name().to_string(),
        Arc::new(CompleteTask::standalone()),
    );

    let runner = AgentRunner::new(
        provider,
        settings,
        tools,
        modules,
        steering,
        prompt_builder,
        context_builder,
    );
    let agent = MemorySummarizerAgent::new();
    let mut active_agent = ActiveAgent::new(Arc::new(agent));
    active_agent.conversation.push(Message::user(formatted));

    let summary = runner.run(active_agent, 10).await?;

    let compaction = Compaction {
        through_seq: max_seq,
        summary,
        created_at: Utc::now(),
        memories_count: memory_count,
        previous_compaction_seq: latest_compaction.map(|c| c.through_seq),
    };

    compaction_store.save(&compaction)?;
    Ok(Some(compaction))
}

/// Count memories since the last compaction.
pub fn memories_since_last_compaction(memory_log: &MemoryLog) -> Result<usize> {
    let memory_dir = memory_log
        .path()
        .parent()
        .context("Failed to get memory directory")?;
    let compaction_store = CompactionStore::new(memory_dir.to_path_buf());
    let through_seq = compaction_store
        .find_latest()?
        .map(|c| c.through_seq)
        .unwrap_or(0);
    let all = memory_log.read_all()?;
    Ok(all.into_iter().filter(|m| m.seq > through_seq).count())
}

/// Parse sequence number from a compaction filename.
/// Returns None if the filename doesn't match the expected pattern.
fn parse_compaction_seq(filename: &str) -> Option<u64> {
    let stripped = filename.strip_prefix("compaction_")?;
    let seq_str = stripped.strip_suffix(".json")?;
    seq_str.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_compaction_seq_valid() {
        assert_eq!(parse_compaction_seq("compaction_42.json"), Some(42));
        assert_eq!(parse_compaction_seq("compaction_0.json"), Some(0));
        assert_eq!(parse_compaction_seq("compaction_12345.json"), Some(12345));
    }

    #[test]
    fn test_parse_compaction_seq_invalid() {
        assert_eq!(parse_compaction_seq("compaction_.json"), None);
        assert_eq!(parse_compaction_seq("compaction_abc.json"), None);
        assert_eq!(parse_compaction_seq("other_42.json"), None);
        assert_eq!(parse_compaction_seq("compaction_42.txt"), None);
        assert_eq!(parse_compaction_seq("compaction_42"), None);
    }
}
