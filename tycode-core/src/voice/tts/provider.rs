use anyhow::Result;
use async_trait::async_trait;

use super::types::{AudioData, Voice};

/// Trait for text-to-speech providers
#[async_trait]
pub trait TextToSpeech: Send + Sync {
    /// Get the default voice for this provider
    fn default_voice(&self) -> Voice;

    /// Synthesize text to speech audio
    async fn synthesize(&self, text: &str, voice: Option<&Voice>) -> Result<AudioData>;

    /// List available voices
    async fn list_voices(&self) -> Result<Vec<Voice>>;
}
