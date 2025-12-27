//! AWS Polly text-to-speech implementation

use anyhow::{Context, Result};
use async_trait::async_trait;
use aws_sdk_polly::config::Region;
use aws_sdk_polly::types::{Engine, OutputFormat, VoiceId};
use aws_sdk_polly::Client;

use super::provider::TextToSpeech;
use super::types::{AudioData, Voice};

/// Configuration for AWS Polly
#[derive(Debug, Clone)]
pub struct AwsPollyConfig {
    pub profile: Option<String>,
    pub region: String,
}

impl Default for AwsPollyConfig {
    fn default() -> Self {
        Self {
            profile: None,
            region: "us-east-1".to_string(),
        }
    }
}

/// AWS Polly text-to-speech provider
pub struct AwsPolly {
    client: Client,
}

impl AwsPolly {
    /// Create a new AWS Polly client
    pub async fn new(config: AwsPollyConfig) -> Result<Self> {
        let mut aws_config_loader = aws_config::defaults(aws_config::BehaviorVersion::latest());

        if let Some(profile) = &config.profile {
            aws_config_loader = aws_config_loader.profile_name(profile);
        }

        aws_config_loader = aws_config_loader.region(Region::new(config.region));

        let aws_config = aws_config_loader.load().await;
        let client = Client::new(&aws_config);

        Ok(Self { client })
    }

    fn parse_voice_id(voice_id: &str) -> Result<VoiceId> {
        match voice_id {
            "Joanna" => Ok(VoiceId::Joanna),
            "Matthew" => Ok(VoiceId::Matthew),
            "Amy" => Ok(VoiceId::Amy),
            "Brian" => Ok(VoiceId::Brian),
            "Emma" => Ok(VoiceId::Emma),
            "Ivy" => Ok(VoiceId::Ivy),
            "Kendra" => Ok(VoiceId::Kendra),
            "Kimberly" => Ok(VoiceId::Kimberly),
            "Salli" => Ok(VoiceId::Salli),
            "Joey" => Ok(VoiceId::Joey),
            "Justin" => Ok(VoiceId::Justin),
            "Kevin" => Ok(VoiceId::Kevin),
            "Ruth" => Ok(VoiceId::Ruth),
            "Stephen" => Ok(VoiceId::Stephen),
            _ => anyhow::bail!("unknown voice id: {voice_id}"),
        }
    }
}

#[async_trait]
impl TextToSpeech for AwsPolly {
    fn default_voice(&self) -> Voice {
        Voice {
            id: "Amy".to_string(),
            name: "Amy".to_string(),
            language_code: "en-US".to_string(),
        }
    }

    async fn synthesize(&self, text: &str, voice: Option<&Voice>) -> Result<AudioData> {
        let default_voice = self.default_voice();
        let voice = voice.unwrap_or(&default_voice);
        let voice_id = Self::parse_voice_id(&voice.id)?;

        let response = self
            .client
            .synthesize_speech()
            .text(text)
            .voice_id(voice_id)
            .output_format(OutputFormat::Pcm)
            .engine(Engine::Neural)
            .sample_rate("16000")
            .send()
            .await
            .context("Failed to synthesize speech")?;

        let audio_stream = response.audio_stream;
        let bytes = audio_stream
            .collect()
            .await
            .context("Failed to collect audio stream")?
            .into_bytes()
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
            .describe_voices()
            .engine(Engine::Neural)
            .language_code(aws_sdk_polly::types::LanguageCode::EnUs)
            .send()
            .await
            .context("Failed to list voices")?;

        let voices = response
            .voices
            .unwrap_or_default()
            .into_iter()
            .filter_map(|v| {
                let id = v.id?.to_string();
                let name = v.name?;
                let language_code = v.language_code?.to_string();
                Some(Voice {
                    id,
                    name,
                    language_code,
                })
            })
            .collect();

        Ok(voices)
    }
}
