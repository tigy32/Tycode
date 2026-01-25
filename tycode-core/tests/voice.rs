//! Integration tests for voice/TTS/STT functionality
//!
//! # Running voice tests
//!
//! These tests require provider credentials. They are marked #[ignore]
//! by default and won't run in normal CI.
//!
//! To run AWS tests:
//! ```sh
//! cargo test -p tycode-core --features voice test_aws -- --ignored
//! ```
//!
//! To run ElevenLabs tests:
//! ```sh
//! ELEVENLABS_API_KEY=your_key cargo test -p tycode-core --features voice test_elevenlabs -- --ignored
//! ```

#![cfg(feature = "voice")]

use std::env;
use std::path::Path;

use anyhow::{Context, Result};
use tycode_core::voice::audio::playback::AudioPlayer;
use tycode_core::voice::stt::aws_transcribe::{AwsTranscribe, AwsTranscribeConfig};
use tycode_core::voice::stt::elevenlabs_transcribe::{
    ElevenLabsTranscribe, ElevenLabsTranscribeConfig,
};
use tycode_core::voice::stt::provider::{AudioSink, SpeechToText};
use tycode_core::voice::stt::types::{TranscriptionChunk, TranscriptionError};
use tycode_core::voice::tts::aws_polly::{AwsPolly, AwsPollyConfig};
use tycode_core::voice::tts::elevenlabs::{ElevenLabs, ElevenLabsConfig};
use tycode_core::voice::tts::provider::TextToSpeech;

// ============================================================================
// Generic Test Helpers
// ============================================================================

async fn run_tts_test(tts: impl TextToSpeech) {
    let text = "Hello, this is a test of text to speech.";
    println!("Synthesizing: {}", text);

    let audio = tts
        .synthesize(text, None)
        .await
        .expect("Failed to synthesize speech");

    println!("Got {} bytes of audio", audio.pcm_data.len());
    assert!(!audio.pcm_data.is_empty(), "Expected non-empty audio data");

    if let Ok(player) = AudioPlayer::new() {
        println!("Playing audio...");
        let playback = player.play(audio).expect("Failed to start playback");
        playback.wait().await;
        println!("Playback complete.");
    } else {
        println!("No audio player available, skipping playback");
    }
}

async fn run_stt_file_test(stt: impl SpeechToText, sample_rate: u32) {
    let audio_path = "tests/test.wav";
    let path = Path::new(audio_path);
    assert!(path.exists(), "Test audio file not found: {}", audio_path);

    println!("Loading audio from: {}", audio_path);

    let (pcm_data, file_sample_rate) = load_wav_as_pcm(path).expect("Failed to load WAV file");
    println!(
        "Loaded {} bytes of PCM data at {} Hz",
        pcm_data.len(),
        file_sample_rate
    );

    let (audio_sink, mut transcriptions) = stt.start().await.expect("Failed to start stream");

    let chunks = chunk_audio(&pcm_data, 100, sample_rate, 2);
    println!("Sending {} audio chunks...", chunks.len());
    for chunk in chunks {
        audio_sink
            .send(chunk)
            .await
            .expect("Failed to send audio chunk");
    }

    // Send trailing silence to trigger VAD commit (needs ~1.5s of silence)
    let silence_duration_ms = 2000;
    let silence_samples = (sample_rate * silence_duration_ms) / 1000;
    let silence_bytes = (silence_samples * 2) as usize;
    let silence = vec![0u8; silence_bytes];
    println!(
        "Sending {} bytes of trailing silence for VAD...",
        silence.len()
    );
    audio_sink
        .send(silence)
        .await
        .expect("Failed to send silence");

    // Give VAD time to process the silence
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    drop(audio_sink);
    println!("Audio sink closed, waiting for transcriptions...");

    let mut results = Vec::new();
    let timeout = tokio::time::Duration::from_secs(30);
    let deadline = tokio::time::Instant::now() + timeout;

    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(tokio::time::Duration::from_secs(5), transcriptions.recv()).await
        {
            Ok(Some(Ok(chunk))) => {
                println!(
                    "Received: {} (partial: {}, speaker: {:?})",
                    chunk.text, chunk.is_partial, chunk.speaker
                );
                if !chunk.is_partial {
                    results.push(chunk.text);
                }
            }
            Ok(Some(Err(e))) => {
                println!("Transcription error: {}", e);
                break;
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }

    println!("\nFinal transcriptions:");
    for (i, text) in results.iter().enumerate() {
        println!("  {}: {}", i + 1, text);
    }

    assert!(
        !results.is_empty(),
        "Expected at least one transcription result"
    );

    let full_text: String = results.join(" ");
    assert!(
        !full_text.trim().is_empty(),
        "Expected non-empty transcription"
    );
    println!("\nFull transcription: {}", full_text);
}

async fn handle_audio_chunk(
    audio: Option<Vec<u8>>,
    audio_chunks_sent: &mut u64,
    audio_sink: &AudioSink,
) -> bool {
    let Some(data) = audio else {
        return false;
    };

    *audio_chunks_sent += 1;

    if audio_sink.send(data).await.is_err() {
        return false;
    }

    true
}

fn handle_transcription(
    transcription: Option<Result<TranscriptionChunk, TranscriptionError>>,
    transcriptions_received: &mut u64,
) -> bool {
    use std::io::Write;

    let Some(result) = transcription else {
        return false;
    };

    match result {
        Ok(chunk) => {
            *transcriptions_received += 1;
            if chunk.is_partial {
                print!("\r[partial] {}", chunk.text);
                std::io::stdout().flush().ok();
            } else {
                println!("\n[final] {}", chunk.text);
            }
            true
        }
        Err(e) => {
            println!("[error] Transcription error: {}", e);
            false
        }
    }
}

async fn run_stt_live_test(stt: impl SpeechToText) {
    use tycode_core::voice::audio::capture::AudioCapture;

    let profile = stt.required_audio_profile();
    let capture = AudioCapture::new(profile).expect("Failed to create audio capture");
    let mut audio_stream = capture.start().expect("Failed to start audio capture");

    let (audio_sink, mut transcriptions) = stt
        .start()
        .await
        .expect("Failed to start transcription stream");

    println!(
        "Recording with profile: {}Hz, {} channel(s) (30 second timeout)",
        profile.sample_rate, profile.channels
    );

    let timeout = tokio::time::Duration::from_secs(30);
    let deadline = tokio::time::Instant::now() + timeout;

    let mut audio_chunks_sent = 0u64;
    let mut transcriptions_received = 0u64;

    loop {
        if tokio::time::Instant::now() >= deadline {
            println!(
                "Timeout reached. Sent {} audio chunks, received {} transcriptions",
                audio_chunks_sent, transcriptions_received
            );
            break;
        }

        tokio::select! {
            audio = audio_stream.recv() => {
                if !handle_audio_chunk(audio, &mut audio_chunks_sent, &audio_sink).await {
                    break;
                }
            }
            transcription = transcriptions.recv() => {
                if !handle_transcription(transcription, &mut transcriptions_received) {
                    break;
                }
            }
        }
    }

    println!("\nDone.");
}

// ============================================================================
// Utility Functions
// ============================================================================

fn chunk_audio(
    pcm_data: &[u8],
    chunk_duration_ms: u32,
    sample_rate: u32,
    bytes_per_sample: u32,
) -> Vec<Vec<u8>> {
    let samples_per_chunk = (sample_rate * chunk_duration_ms) / 1000;
    let chunk_size = (samples_per_chunk * bytes_per_sample) as usize;
    pcm_data.chunks(chunk_size).map(|c| c.to_vec()).collect()
}

fn load_wav_as_pcm(path: &Path) -> Result<(Vec<u8>, u32)> {
    let reader = hound::WavReader::open(path).context("Failed to open WAV file")?;

    let spec = reader.spec();
    let sample_rate = spec.sample_rate;

    if spec.sample_rate != 16000 {
        tracing::warn!(
            "WAV file sample rate is {} Hz, expected 16000 Hz. Results may vary.",
            spec.sample_rate
        );
    }

    let samples: Vec<i16> = match spec.sample_format {
        hound::SampleFormat::Int => match spec.bits_per_sample {
            16 => reader.into_samples::<i16>().map(|s| s.unwrap()).collect(),
            8 => reader
                .into_samples::<i8>()
                .map(|s| (s.unwrap() as i16) << 8)
                .collect(),
            32 => reader
                .into_samples::<i32>()
                .map(|s| (s.unwrap() >> 16) as i16)
                .collect(),
            _ => anyhow::bail!("Unsupported bit depth: {}", spec.bits_per_sample),
        },
        hound::SampleFormat::Float => reader
            .into_samples::<f32>()
            .map(|s| (s.unwrap() * 32767.0) as i16)
            .collect(),
    };

    let mono_samples: Vec<i16> = if spec.channels == 2 {
        samples
            .chunks(2)
            .map(|chunk| ((chunk[0] as i32 + chunk[1] as i32) / 2) as i16)
            .collect()
    } else {
        samples
    };

    let pcm_data: Vec<u8> = mono_samples
        .iter()
        .flat_map(|&sample| sample.to_le_bytes())
        .collect();

    Ok((pcm_data, sample_rate))
}

// ============================================================================
// AWS Provider Tests
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_aws_polly_tts() {
    tracing_subscriber::fmt::init();

    let config = AwsPollyConfig {
        profile: env::var("AWS_PROFILE").ok(),
        region: env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
    };

    let polly = AwsPolly::new(config)
        .await
        .expect("Failed to create AWS Polly client");

    run_tts_test(polly).await;
}

#[tokio::test]
#[ignore]
async fn test_aws_transcribe_from_file() {
    tracing_subscriber::fmt::init();

    let config = AwsTranscribeConfig {
        profile: env::var("AWS_PROFILE").ok(),
        region: env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
        language_code: "en-US".to_string(),
        sample_rate_hz: 16000,
    };

    let transcribe = AwsTranscribe::new(config)
        .await
        .expect("Failed to create AWS Transcribe client");

    run_stt_file_test(transcribe, 16000).await;
}

#[tokio::test]
#[ignore]
async fn test_aws_transcribe_live() {
    tracing_subscriber::fmt::init();

    let config = AwsTranscribeConfig {
        profile: env::var("AWS_PROFILE").ok(),
        region: env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
        language_code: "en-US".to_string(),
        sample_rate_hz: 16000,
    };

    let transcribe = AwsTranscribe::new(config)
        .await
        .expect("Failed to create AWS Transcribe client");

    run_stt_live_test(transcribe).await;
}

// ============================================================================
// ElevenLabs Provider Tests
// ============================================================================

#[tokio::test]
#[ignore]
async fn test_elevenlabs_tts() {
    tracing_subscriber::fmt::init();

    let api_key =
        env::var("ELEVENLABS_API_KEY").expect("ELEVENLABS_API_KEY environment variable required");

    let config = ElevenLabsConfig::new(api_key);
    let tts = ElevenLabs::new(config);

    run_tts_test(tts).await;
}

#[tokio::test]
#[ignore]
async fn test_elevenlabs_stt_from_file() {
    tracing_subscriber::fmt::init();

    let api_key =
        env::var("ELEVENLABS_API_KEY").expect("ELEVENLABS_API_KEY environment variable required");

    let config = ElevenLabsTranscribeConfig::new(api_key);
    let stt = ElevenLabsTranscribe::new(config);

    run_stt_file_test(stt, 16000).await;
}

#[tokio::test]
#[ignore]
async fn test_elevenlabs_stt_live() {
    tracing_subscriber::fmt::init();

    let api_key =
        env::var("ELEVENLABS_API_KEY").expect("ELEVENLABS_API_KEY environment variable required");

    let config = ElevenLabsTranscribeConfig::new(api_key);
    let stt = ElevenLabsTranscribe::new(config);

    run_stt_live_test(stt).await;
}
