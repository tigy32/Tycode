//! Log-based memory system with JSON-backed storage.
//!
//! Memories are stored as a JSON log at ~/.tycode/memory/memories_log.json.
//! Each memory has a monotonic sequence number, content, timestamp, and optional source.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub seq: u64,
    pub content: String,
    pub created_at: DateTime<Utc>,
    pub source: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct MemoryLogInner {
    memories: Vec<Memory>,
    next_seq: u64,
}

/// Memory log that loads from disk on every operation.
#[derive(Debug)]
pub struct MemoryLog {
    path: PathBuf,
}

impl MemoryLog {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Load current state from disk. Returns empty if file doesn't exist.
    fn load_inner(&self) -> Result<MemoryLogInner> {
        if !self.path.exists() {
            return Ok(MemoryLogInner {
                memories: Vec::new(),
                next_seq: 1,
            });
        }

        let content = fs::read_to_string(&self.path)
            .with_context(|| format!("Failed to read memory log: {}", self.path.display()))?;

        serde_json::from_str(&content)
            .with_context(|| format!("Failed to parse memory log: {}", self.path.display()))
    }

    /// Save state to disk, creating directories as needed.
    fn save_inner(&self, inner: &MemoryLogInner) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create memory directory: {}", parent.display())
            })?;
        }

        let content =
            serde_json::to_string_pretty(inner).context("Failed to serialize memory log")?;

        fs::write(&self.path, content)
            .with_context(|| format!("Failed to write memory log: {}", self.path.display()))
    }

    /// Append a new memory. Loads from disk, adds memory, saves back.
    /// Race condition: if two processes append simultaneously, one may lose.
    /// This is acceptable - we lose a few memories, not the entire log.
    pub fn append(&self, content: String, source: Option<String>) -> Result<u64> {
        let mut inner = self.load_inner()?;

        let seq = inner.next_seq;
        inner.next_seq += 1;

        inner.memories.push(Memory {
            seq,
            content,
            created_at: Utc::now(),
            source,
        });

        self.save_inner(&inner)?;
        Ok(seq)
    }

    /// Read all memories from disk.
    pub fn read_all(&self) -> Result<Vec<Memory>> {
        self.load_inner().map(|inner| inner.memories)
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}
