//! Integration tests for voice/STT functionality
//!
//! # Running voice tests
//!
//! These tests require AWS credentials. They are marked #[ignore]
//! by default and won't run in normal CI.
//!
//! To run:
//! ```sh
//! cargo test -p tycode-core --features voice test_aws_transcribe -- --ignored
//! ```

#![cfg(feature = "voice")]

use std::env;
use std::path::Path;

use anyhow::{Context, Result};
use tycode_core::voice::audio::playback::AudioPlayer;
use tycode_core::voice::stt::aws_transcribe::{AwsTranscribe, AwsTranscribeConfig};
use tycode_core::voice::stt::provider::SpeechToText;
use tycode_core::voice::tts::aws_polly::{AwsPolly, AwsPollyConfig};
use tycode_core::voice::tts::provider::TextToSpeech;

/// Split audio data into fixed-size chunks for streaming
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
            "WAV file sample rate is {} Hz, AWS Transcribe expects 16000 Hz. Results may vary.",
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

#[tokio::test]
#[ignore] // Requires AWS credentials and audio file
async fn test_aws_transcribe_from_file() {
    tracing_subscriber::fmt::init();

    let audio_path = "tests/test.wav";
    let path = Path::new(audio_path);
    assert!(path.exists(), "Test audio file not found: {}", audio_path);

    println!("Loading audio from: {}", audio_path);

    let (pcm_data, sample_rate) = load_wav_as_pcm(path).expect("Failed to load WAV file");
    println!(
        "Loaded {} bytes of PCM data at {} Hz",
        pcm_data.len(),
        sample_rate
    );

    let config = AwsTranscribeConfig {
        profile: env::var("AWS_PROFILE").ok(),
        region: env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
        language_code: "en-US".to_string(),
        sample_rate_hz: sample_rate as i32,
    };

    let transcribe = AwsTranscribe::new(config)
        .await
        .expect("Failed to create AWS Transcribe client");

    let (audio_sink, mut transcriptions) =
        transcribe.start().await.expect("Failed to start stream");

    let chunks = chunk_audio(&pcm_data, 100, sample_rate, 2);
    for chunk in chunks {
        audio_sink
            .send(chunk)
            .await
            .expect("Failed to send audio chunk");
    }

    // Dropping audio_sink signals end of audio stream
    drop(audio_sink);

    let mut results = Vec::new();
    let timeout = tokio::time::Duration::from_secs(30);
    let deadline = tokio::time::Instant::now() + timeout;

    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(tokio::time::Duration::from_secs(5), transcriptions.recv()).await
        {
            Ok(Some(chunk)) => {
                println!(
                    "Received: {} (partial: {}, speaker: {:?})",
                    chunk.text, chunk.is_partial, chunk.speaker
                );
                if !chunk.is_partial {
                    results.push(chunk.text);
                }
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

#[tokio::test]
#[ignore] // Requires AWS credentials and working microphone
async fn test_live_microphone() {
    tracing_subscriber::fmt::init();

    use tycode_core::voice::audio::capture::AudioCapture;

    let config = AwsTranscribeConfig {
        profile: env::var("AWS_PROFILE").ok(),
        region: env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
        language_code: "en-US".to_string(),
        sample_rate_hz: 16000,
    };

    let transcribe = AwsTranscribe::new(config)
        .await
        .expect("Failed to create AWS Transcribe client");

    // Get the audio profile the provider needs and create capture with it
    let profile = transcribe.required_audio_profile();
    let capture = AudioCapture::new(profile).expect("Failed to create audio capture");
    let mut audio_stream = capture.start().expect("Failed to start audio capture");

    let (audio_sink, mut transcriptions) = transcribe
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

    // Use select! to handle both streams on same task (AudioStream is not Send)
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
                match audio {
                    Some(data) => {
                        audio_chunks_sent += 1;
                        if audio_chunks_sent % 50 == 0 {
                            println!("[debug] Sent {} audio chunks ({} bytes each)", audio_chunks_sent, data.len());
                        }
                        if audio_sink.send(data).await.is_err() {
                            println!("[debug] Audio sink closed");
                            break;
                        }
                    }
                    None => {
                        println!("[debug] Audio stream ended");
                        break;
                    }
                }
            }
            transcription = transcriptions.recv() => {
                match transcription {
                    Some(chunk) => {
                        transcriptions_received += 1;
                        if chunk.is_partial {
                            print!("\r[partial] {}", chunk.text);
                            use std::io::Write;
                            std::io::stdout().flush().ok();
                        } else {
                            println!("\n[final] {}", chunk.text);
                        }
                    }
                    None => {
                        println!("[debug] Transcription stream ended");
                        break;
                    }
                }
            }
        }
    }

    println!("\nDone.");
}

#[tokio::test]
#[ignore] // Requires AWS credentials and audio output device
async fn test_aws_polly_tts() {
    tracing_subscriber::fmt::init();

    let config = AwsPollyConfig {
        profile: env::var("AWS_PROFILE").ok(),
        region: env::var("AWS_REGION").unwrap_or_else(|_| "us-east-1".to_string()),
    };

    let polly = AwsPolly::new(config)
        .await
        .expect("Failed to create AWS Polly client");

    let text = "Hello, this is a test of text to speech.";

    println!("Synthesizing: {}", text);
    let audio = polly
        .synthesize(text, None) // Uses default voice (Joanna)
        .await
        .expect("Failed to synthesize speech");

    println!("Got {} bytes of audio, playing...", audio.pcm_data.len());
    let player = AudioPlayer::new().expect("Failed to create audio player");

    let playback = player.play(audio).expect("Failed to start playback");

    playback.wait().await;
    println!("Playback complete.");
}
