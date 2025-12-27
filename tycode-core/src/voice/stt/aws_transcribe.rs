//! AWS Transcribe Streaming implementation

use anyhow::Result;
use async_trait::async_trait;
use aws_sdk_transcribestreaming::types::TranscriptEvent;
use aws_sdk_transcribestreaming::{
    config::Region,
    primitives::Blob,
    types::{
        Alternative, AudioEvent, AudioStream, LanguageCode, MediaEncoding, TranscriptResultStream,
    },
    Client,
};
use tokio::sync::mpsc;

use super::provider::{AudioSink, SpeechToText, TranscriptionStream};
use super::types::{Speaker, TranscriptionChunk};
use crate::voice::audio::AudioProfile;

/// Configuration for AWS Transcribe streaming
#[derive(Debug, Clone)]
pub struct AwsTranscribeConfig {
    pub profile: Option<String>,
    pub region: String,
    pub language_code: String,
    pub sample_rate_hz: i32,
}

impl Default for AwsTranscribeConfig {
    fn default() -> Self {
        Self {
            profile: None,
            region: "us-east-1".to_string(),
            language_code: "en-US".to_string(),
            sample_rate_hz: 16000,
        }
    }
}

/// AWS Transcribe streaming speech-to-text provider
pub struct AwsTranscribe {
    client: Client,
    config: AwsTranscribeConfig,
}

impl AwsTranscribe {
    /// Create a new AWS Transcribe client
    pub async fn new(config: AwsTranscribeConfig) -> Result<Self> {
        let mut aws_config_loader = aws_config::defaults(aws_config::BehaviorVersion::latest());

        if let Some(profile) = &config.profile {
            aws_config_loader = aws_config_loader.profile_name(profile);
        }

        aws_config_loader = aws_config_loader.region(Region::new(config.region.clone()));

        let aws_config = aws_config_loader.load().await;
        let client = Client::new(&aws_config);

        Ok(Self { client, config })
    }

    fn parse_language_code(code: &str) -> LanguageCode {
        match code {
            "en-US" => LanguageCode::EnUs,
            "en-GB" => LanguageCode::EnGb,
            "en-AU" => LanguageCode::EnAu,
            "es-US" => LanguageCode::EsUs,
            "es-ES" => LanguageCode::EsEs,
            "fr-FR" => LanguageCode::FrFr,
            "de-DE" => LanguageCode::DeDe,
            "ja-JP" => LanguageCode::JaJp,
            "zh-CN" => LanguageCode::ZhCn,
            _ => LanguageCode::EnUs,
        }
    }
}

#[async_trait]
impl SpeechToText for AwsTranscribe {
    fn required_audio_profile(&self) -> AudioProfile {
        AudioProfile {
            sample_rate: self.config.sample_rate_hz as u32,
            channels: 1,
        }
    }

    async fn start(&self) -> Result<(AudioSink, TranscriptionStream)> {
        let (result_tx, result_rx) = mpsc::channel::<TranscriptionChunk>(100);
        let (audio_tx, mut audio_rx) = mpsc::channel::<Vec<u8>>(100);

        let language_code = Self::parse_language_code(&self.config.language_code);
        let sample_rate = self.config.sample_rate_hz;
        let client = self.client.clone();

        tokio::spawn(async move {
            let audio_stream = async_stream::stream! {
                while let Some(pcm_data) = audio_rx.recv().await {
                    let audio_event = AudioEvent::builder()
                        .audio_chunk(Blob::new(pcm_data))
                        .build();
                    yield Ok(AudioStream::AudioEvent(audio_event));
                }
            };

            let response = client
                .start_stream_transcription()
                .language_code(language_code)
                .media_encoding(MediaEncoding::Pcm)
                .media_sample_rate_hertz(sample_rate)
                .show_speaker_label(true)
                .audio_stream(audio_stream.into())
                .send()
                .await;

            let output = match response {
                Ok(output) => output,
                Err(e) => {
                    tracing::error!("Failed to start AWS Transcribe stream: {e:?}");
                    return;
                }
            };

            let mut transcript_stream = output.transcript_result_stream;

            while let Ok(Some(event)) = transcript_stream.recv().await {
                let TranscriptResultStream::TranscriptEvent(transcript_event) = event else {
                    continue;
                };

                for chunk in extract_chunks(transcript_event) {
                    if result_tx.send(chunk).await.is_err() {
                        return;
                    }
                }
            }
        });

        let audio_sink = AudioSink::new(audio_tx);
        let transcription_stream = TranscriptionStream::new(result_rx);

        Ok((audio_sink, transcription_stream))
    }
}

/// Extract transcription chunks from a TranscriptEvent
fn extract_chunks(event: TranscriptEvent) -> Vec<TranscriptionChunk> {
    let Some(transcript) = event.transcript else {
        return Vec::new();
    };

    let results = transcript.results.unwrap_or_default();

    let mut chunks = Vec::new();

    for result in results {
        let is_partial = result.is_partial;
        let timestamp_ms = (result.start_time * 1000.0) as u64;

        for alternative in result.alternatives.unwrap_or_default() {
            let speaker = extract_speaker(&alternative);
            let text = alternative.transcript.unwrap_or_default();
            if text.is_empty() {
                continue;
            }

            chunks.push(TranscriptionChunk {
                text,
                speaker,
                is_partial,
                timestamp_ms,
            });
        }
    }

    chunks
}

/// Extract speaker information from an alternative
fn extract_speaker(alternative: &Alternative) -> Option<Speaker> {
    alternative
        .items
        .as_ref()
        .and_then(|items| items.first())
        .and_then(|item| item.speaker.as_ref())
        .map(|s| {
            s.parse::<u32>()
                .map(Speaker::Unknown)
                .unwrap_or_else(|_| Speaker::Known(s.clone()))
        })
}
