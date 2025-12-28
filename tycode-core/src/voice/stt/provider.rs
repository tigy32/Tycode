use anyhow::{Context, Result};
use async_trait::async_trait;
use tokio::sync::mpsc;

use super::types::{TranscriptionChunk, TranscriptionError};
use crate::voice::audio::AudioProfile;

/// Trait for speech-to-text providers
#[async_trait]
pub trait SpeechToText: Send + Sync {
    /// Returns the audio format this provider expects
    fn required_audio_profile(&self) -> AudioProfile;

    /// Start streaming transcription
    ///
    /// Returns two independent handles:
    /// - AudioSink: for sending audio data
    /// - TranscriptionStream: for receiving transcription results
    async fn start(&self) -> Result<(AudioSink, TranscriptionStream)>;
}

/// Handle for sending audio data to the transcription service
pub struct AudioSink {
    sender: mpsc::Sender<Vec<u8>>,
}

impl AudioSink {
    pub fn new(sender: mpsc::Sender<Vec<u8>>) -> Self {
        Self { sender }
    }

    /// Send audio data (PCM format)
    pub async fn send(&self, audio: Vec<u8>) -> Result<()> {
        self.sender
            .send(audio)
            .await
            .context("Audio channel closed")?;
        Ok(())
    }
}

/// Handle for receiving transcription results
pub struct TranscriptionStream {
    receiver: mpsc::Receiver<Result<TranscriptionChunk, TranscriptionError>>,
}

impl TranscriptionStream {
    pub fn new(receiver: mpsc::Receiver<Result<TranscriptionChunk, TranscriptionError>>) -> Self {
        Self { receiver }
    }

    /// Receive the next transcription result
    /// Returns None when the stream ends
    pub async fn recv(&mut self) -> Option<Result<TranscriptionChunk, TranscriptionError>> {
        self.receiver.recv().await
    }
}
