//! ElevenLabs text-to-speech implementation

use anyhow::{Context, Result};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::provider::TextToSpeech;
use super::types::{AudioData, Voice};

#[derive(Debug, Clone)]
pub struct ElevenLabsConfig {
    pub api_key: String,
    pub voice_id: String,
    pub model_id: String,
}

impl ElevenLabsConfig {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            voice_id: "G3hRJZ8nXEfgXIpKdanG".to_string(),
            model_id: "eleven_multilingual_v2".to_string(),
        }
    }
}

pub struct ElevenLabs {
    config: ElevenLabsConfig,
    client: Client,
}

impl ElevenLabs {
    pub fn new(config: ElevenLabsConfig) -> Self {
        Self {
            config,
            client: Client::new(),
        }
    }
}

#[derive(Serialize)]
struct SynthesizeRequest {
    text: String,
    model_id: String,
}

#[derive(Deserialize)]
struct VoicesResponse {
    voices: Vec<VoiceData>,
}

#[derive(Deserialize)]
struct VoiceData {
    voice_id: String,
    name: String,
}

#[async_trait]
impl TextToSpeech for ElevenLabs {
    fn default_voice(&self) -> Voice {
        Voice {
            id: self.config.voice_id.clone(),
            name: "Default".to_string(),
            language_code: "en".to_string(),
        }
    }

    async fn synthesize(&self, text: &str, voice: Option<&Voice>) -> Result<AudioData> {
        let voice_id = voice
            .map(|v| v.id.as_str())
            .unwrap_or(&self.config.voice_id);

        let url = format!(
            "https://api.elevenlabs.io/v1/text-to-speech/{}/stream?output_format=pcm_16000",
            voice_id
        );

        let request_body = SynthesizeRequest {
            text: text.to_string(),
            model_id: self.config.model_id.clone(),
        };

        let response = self
            .client
            .post(&url)
            .header("xi-api-key", &self.config.api_key)
            .header("Content-Type", "application/json")
            .json(&request_body)
            .send()
            .await
            .context("Failed to send request to ElevenLabs")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("ElevenLabs API error {status}: {body}");
        }

        let bytes = response
            .bytes()
            .await
            .context("Failed to read audio bytes")?
            .to_vec();

        Ok(AudioData {
            pcm_data: bytes,
            sample_rate: 16000,
            channels: 1,
        })
    }

    async fn list_voices(&self) -> Result<Vec<Voice>> {
        let response = self
            .client
            .get("https://api.elevenlabs.io/v1/voices")
            .header("xi-api-key", &self.config.api_key)
            .send()
            .await
            .context("Failed to list voices from ElevenLabs")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            anyhow::bail!("ElevenLabs API error {status}: {body}");
        }

        let voices_response: VoicesResponse = response
            .json()
            .await
            .context("Failed to parse voices response")?;

        let voices = voices_response
            .voices
            .into_iter()
            .map(|v| Voice {
                id: v.voice_id,
                name: v.name,
                language_code: "en".to_string(),
            })
            .collect();

        Ok(voices)
    }
}
