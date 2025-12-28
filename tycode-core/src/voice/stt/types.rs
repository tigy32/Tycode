use serde::{Deserialize, Serialize};
use std::fmt;

/// Errors that can occur during transcription
#[derive(Debug, Clone)]
pub enum TranscriptionError {
    /// AWS Transcribe failed to start streaming
    StartupFailed { message: String },
    /// Stream error during transcription
    StreamError { message: String },
}

impl fmt::Display for TranscriptionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::StartupFailed { message } => write!(f, "Transcription startup failed: {message}"),
            Self::StreamError { message } => write!(f, "Transcription stream error: {message}"),
        }
    }
}

impl std::error::Error for TranscriptionError {}

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
