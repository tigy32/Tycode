use serde::{Deserialize, Serialize};

/// A chunk of transcribed text
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptionChunk {
    pub text: String,
    pub speaker: Option<Speaker>,
    pub is_partial: bool,
    pub timestamp_ms: u64,
}

/// Speaker identification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Speaker {
    /// Matched a known speaker profile
    Known(String),
    /// Unknown speaker (ID assigned by diarization)
    Unknown(u32),
}
