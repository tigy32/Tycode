use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{connect_async, tungstenite::client::IntoClientRequest};

use super::provider::{AudioSink, SpeechToText, TranscriptionStream};
use super::types::{TranscriptionChunk, TranscriptionError};
use crate::voice::audio::AudioProfile;

#[derive(Debug, Clone)]
pub struct ElevenLabsTranscribeConfig {
    pub api_key: String,
    pub model_id: Option<String>,
    pub audio_format: String,
    pub commit_strategy: String,
}

impl ElevenLabsTranscribeConfig {
    pub fn new(api_key: String) -> Self {
        Self {
            api_key,
            model_id: None,
            audio_format: "pcm_16000".to_string(),
            commit_strategy: "vad".to_string(),
        }
    }
}

pub struct ElevenLabsTranscribe {
    config: ElevenLabsTranscribeConfig,
}

impl ElevenLabsTranscribe {
    pub fn new(config: ElevenLabsTranscribeConfig) -> Self {
        Self { config }
    }

    fn build_url(&self) -> String {
        let mut url = format!(
            "wss://api.elevenlabs.io/v1/speech-to-text/realtime?audio_format={}&commit_strategy={}",
            self.config.audio_format, self.config.commit_strategy
        );
        if let Some(model_id) = &self.config.model_id {
            url.push_str(&format!("&model_id={}", model_id));
        }
        url
    }
}

#[derive(Serialize)]
struct InputAudioChunk {
    message_type: &'static str,
    audio_base_64: String,
    commit: bool,
    sample_rate: u32,
}

#[derive(Deserialize)]
struct ServerMessage {
    message_type: String,
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

fn parse_server_message(
    server_msg: ServerMessage,
) -> Option<Result<TranscriptionChunk, TranscriptionError>> {
    match server_msg.message_type.as_str() {
        "session_started" => None,
        "partial_transcript" => server_msg.text.map(|text| {
            Ok(TranscriptionChunk {
                text,
                speaker: None,
                is_partial: true,
                timestamp_ms: 0,
            })
        }),
        "committed_transcript" | "committed_transcript_with_timestamps" => {
            server_msg.text.map(|text| {
                Ok(TranscriptionChunk {
                    text,
                    speaker: None,
                    is_partial: false,
                    timestamp_ms: 0,
                })
            })
        }
        "error" | "auth_error" | "quota_exceeded" | "rate_limited" | "resource_exhausted" => {
            let error_msg = server_msg.error.unwrap_or_else(|| "Unknown error".into());
            Some(Err(TranscriptionError::StreamError { message: error_msg }))
        }
        _ => None,
    }
}

#[async_trait]
impl SpeechToText for ElevenLabsTranscribe {
    fn required_audio_profile(&self) -> AudioProfile {
        AudioProfile {
            sample_rate: 16000,
            channels: 1,
        }
    }

    async fn start(&self) -> Result<(AudioSink, TranscriptionStream)> {
        let url = self.build_url();
        let mut request = url
            .into_client_request()
            .context("Failed to build request")?;
        request.headers_mut().insert(
            "xi-api-key",
            self.config
                .api_key
                .parse()
                .context("Invalid API key for HTTP header")?,
        );

        let (ws_stream, _) = connect_async(request)
            .await
            .context("Failed to connect to ElevenLabs WebSocket")?;

        let (mut write, mut read) = ws_stream.split();

        let (audio_tx, mut audio_rx) = mpsc::channel::<Vec<u8>>(32);
        let (result_tx, result_rx) =
            mpsc::channel::<Result<TranscriptionChunk, TranscriptionError>>(32);

        tokio::spawn(async move {
            while let Some(audio_data) = audio_rx.recv().await {
                let chunk = InputAudioChunk {
                    message_type: "input_audio_chunk",
                    audio_base_64: base64::engine::general_purpose::STANDARD.encode(&audio_data),
                    commit: false,
                    sample_rate: 16000,
                };

                let json = serde_json::to_string(&chunk)
                    .expect("Failed to serialize InputAudioChunk - this is a bug");

                if let Err(e) = write.send(Message::Text(json)).await {
                    tracing::error!("Failed to send audio to WebSocket: {e:?}");
                    break;
                }
            }
        });

        tokio::spawn(async move {
            while let Some(msg_result) = read.next().await {
                if let Err(e) = &msg_result {
                    let _ = result_tx
                        .send(Err(TranscriptionError::StreamError {
                            message: format!("{e:?}"),
                        }))
                        .await;
                    break;
                }
                let msg = msg_result.unwrap();

                let text = match msg {
                    Message::Text(t) => t,
                    Message::Close(_) => break,
                    _ => continue,
                };

                let server_msg = serde_json::from_str::<ServerMessage>(&text);
                if let Err(e) = &server_msg {
                    tracing::warn!("Failed to parse server message: {e:?}");
                    let _ = result_tx
                        .send(Err(TranscriptionError::StreamError {
                            message: format!("Failed to parse server message: {e:?}"),
                        }))
                        .await;
                    continue;
                }
                let server_msg = server_msg.unwrap();

                let Some(result) = parse_server_message(server_msg) else {
                    continue;
                };
                let is_error = result.is_err();

                if result_tx.send(result).await.is_err() {
                    break;
                }
                if is_error {
                    break;
                }
            }
        });

        Ok((
            AudioSink::new(audio_tx),
            TranscriptionStream::new(result_rx),
        ))
    }
}
