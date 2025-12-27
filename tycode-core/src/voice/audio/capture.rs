//! Audio capture from microphone using cpal
//! Captures at native device sample rate and resamples to target profile

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    Device, FromSample, Sample, SampleFormat, SizedSample, Stream, StreamConfig,
    SupportedStreamConfig,
};
use rubato::{FftFixedIn, Resampler};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

use super::AudioProfile;

/// Resampler with input buffer to accumulate samples
struct ResamplerWithBuffer {
    resampler: FftFixedIn<f32>,
    buffer: Vec<f32>,
}

/// Audio capture from microphone
pub struct AudioCapture {
    device: Device,
    supported_config: SupportedStreamConfig,
    target_profile: AudioProfile,
}

/// Audio stream that receives captured audio data
pub struct AudioStream {
    receiver: mpsc::Receiver<Vec<u8>>,
    running: Arc<AtomicBool>,
    _stream: Stream,
}

impl AudioStream {
    /// Receive the next audio chunk
    pub async fn recv(&mut self) -> Option<Vec<u8>> {
        self.receiver.recv().await
    }
}

impl Drop for AudioStream {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

impl AudioCapture {
    /// Create a new audio capture using the default input device
    /// Output will be resampled to match the provided AudioProfile
    pub fn new(profile: AudioProfile) -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_input_device()
            .context("no input device available")?;

        let supported_config = device
            .default_input_config()
            .context("failed to get default input config")?;

        tracing::debug!(
            device_name = ?device.name(),
            native_sample_rate = supported_config.sample_rate().0,
            native_channels = supported_config.channels(),
            native_format = ?supported_config.sample_format(),
            target_sample_rate = profile.sample_rate,
            target_channels = profile.channels,
            "audio capture initialized"
        );

        Ok(Self {
            device,
            supported_config,
            target_profile: profile,
        })
    }

    /// Start capturing audio, consumes self and returns audio stream
    /// Audio is resampled to match the configured AudioProfile
    /// Drop the stream to stop capture (RAII)
    pub fn start(self) -> Result<AudioStream> {
        let (tx, rx) = mpsc::channel::<Vec<u8>>(100);
        let running = Arc::new(AtomicBool::new(true));
        let callback_running = running.clone();
        let err_running = running.clone();

        let native_rate = self.supported_config.sample_rate().0;
        let native_channels = self.supported_config.channels() as usize;
        let sample_format = self.supported_config.sample_format();
        let target_rate = self.target_profile.sample_rate;

        let config: StreamConfig = self.supported_config.clone().into();

        // Create resampler with buffer (always resample for consistent code path)
        let chunk_size = 1024;
        let resampler = FftFixedIn::new(
            native_rate as usize,
            target_rate as usize,
            chunk_size,
            2,
            1, // mono output
        )
        .context("failed to create resampler")?;
        let resampler = Arc::new(Mutex::new(ResamplerWithBuffer {
            resampler,
            buffer: Vec::with_capacity(chunk_size * 2),
        }));

        let stream = match sample_format {
            SampleFormat::I16 => self.build_stream::<i16>(
                &config,
                tx,
                callback_running,
                err_running.clone(),
                native_channels,
                resampler,
            )?,
            SampleFormat::F32 => self.build_stream::<f32>(
                &config,
                tx,
                callback_running,
                err_running.clone(),
                native_channels,
                resampler,
            )?,
            format => anyhow::bail!("unsupported sample format: {:?}", format),
        };

        stream.play().context("failed to start audio stream")?;

        Ok(AudioStream {
            receiver: rx,
            running,
            _stream: stream,
        })
    }

    fn build_stream<T>(
        &self,
        config: &StreamConfig,
        tx: mpsc::Sender<Vec<u8>>,
        running: Arc<AtomicBool>,
        err_running: Arc<AtomicBool>,
        native_channels: usize,
        resampler: Arc<Mutex<ResamplerWithBuffer>>,
    ) -> Result<Stream>
    where
        T: SizedSample + Send + 'static,
        f32: FromSample<T>,
    {
        self.device
            .build_input_stream(
                config,
                move |data: &[T], _: &cpal::InputCallbackInfo| {
                    if !running.load(Ordering::SeqCst) {
                        return;
                    }
                    let mono = to_mono_f32(data, native_channels);
                    let processed = process_audio(&mono, &resampler);
                    let bytes = f32_to_i16_bytes(&processed);
                    if !bytes.is_empty() && tx.blocking_send(bytes).is_err() {
                        running.store(false, Ordering::SeqCst);
                    }
                },
                move |err| {
                    tracing::error!(error = ?err, "audio stream error");
                    err_running.store(false, Ordering::SeqCst);
                },
                None,
            )
            .context("failed to build input stream")
    }
}

/// Convert samples of any type to mono f32
fn to_mono_f32<T>(samples: &[T], channels: usize) -> Vec<f32>
where
    T: Copy,
    f32: FromSample<T>,
{
    if channels == 1 {
        return samples.iter().map(|&s| f32::from_sample(s)).collect();
    }
    samples
        .chunks(channels)
        .map(|frame| {
            let sum: f32 = frame.iter().map(|&s| f32::from_sample(s)).sum();
            sum / channels as f32
        })
        .collect()
}

/// Process audio through resampler, buffering until we have enough samples
fn process_audio(mono: &[f32], resampler: &Arc<Mutex<ResamplerWithBuffer>>) -> Vec<f32> {
    let Ok(mut state) = resampler.lock() else {
        return Vec::new();
    };

    // Add new samples to buffer
    state.buffer.extend_from_slice(mono);

    let mut output = Vec::new();

    // Process complete chunks
    loop {
        let frames_needed = state.resampler.input_frames_next();
        if state.buffer.len() < frames_needed {
            break;
        }

        let input = vec![state.buffer[..frames_needed].to_vec()];
        match state.resampler.process(&input, None) {
            Ok(resampled) => {
                if let Some(chunk) = resampled.into_iter().next() {
                    output.extend(chunk);
                }
            }
            Err(e) => {
                tracing::warn!(error = ?e, "resampling failed");
                break;
            }
        }

        // Remove processed samples from buffer
        state.buffer.drain(..frames_needed);
    }

    output
}

/// Convert f32 samples to i16 little-endian bytes
fn f32_to_i16_bytes(samples: &[f32]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(samples.len() * 2);
    for &sample in samples {
        let clamped = sample.clamp(-1.0, 1.0);
        let i16_sample = (clamped * 32767.0) as i16;
        bytes.extend_from_slice(&i16_sample.to_le_bytes());
    }
    bytes
}
