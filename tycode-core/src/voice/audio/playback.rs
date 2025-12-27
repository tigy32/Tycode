//! Audio playback using cpal
//! Resamples from source rate to native device rate if needed

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{
    Device, FromSample, SampleFormat, SizedSample, Stream, StreamConfig, SupportedStreamConfig,
};
use rubato::{FftFixedIn, Resampler};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;

use super::super::tts::types::AudioData;

/// Audio player for TTS output
pub struct AudioPlayer {
    device: Device,
    supported_config: SupportedStreamConfig,
}

/// Audio playback handle - dropping stops playback (RAII)
pub struct AudioPlayback {
    _stream: Stream,
    finished: Arc<AtomicBool>,
}

impl AudioPlayback {
    /// Check if playback has finished
    pub fn is_finished(&self) -> bool {
        self.finished.load(Ordering::SeqCst)
    }

    /// Wait for playback to complete
    pub async fn wait(&self) {
        while !self.is_finished() {
            tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        }
    }
}

impl AudioPlayer {
    /// Create a new audio player using the default output device
    pub fn new() -> Result<Self> {
        let host = cpal::default_host();
        let device = host
            .default_output_device()
            .context("no output device available")?;

        let supported_config = device
            .default_output_config()
            .context("failed to get default output config")?;

        Ok(Self {
            device,
            supported_config,
        })
    }

    /// Play audio data, returns handle that stops on drop
    pub fn play(&self, audio: AudioData) -> Result<AudioPlayback> {
        let native_rate = self.supported_config.sample_rate().0;
        let native_channels = self.supported_config.channels() as usize;
        let sample_format = self.supported_config.sample_format();
        let config: StreamConfig = self.supported_config.clone().into();

        let input_samples = i16_bytes_to_f32(&audio.pcm_data);
        let resampled = resample(&input_samples, audio.sample_rate, native_rate)?;

        let samples = if audio.channels == 1 && native_channels > 1 {
            expand_to_channels(&resampled, native_channels)
        } else {
            resampled
        };

        let samples = Arc::new(samples);
        let position = Arc::new(AtomicUsize::new(0));
        let finished = Arc::new(AtomicBool::new(false));

        let stream = match sample_format {
            SampleFormat::F32 => {
                self.build_stream::<f32>(&config, samples, position, finished.clone())?
            }
            SampleFormat::I16 => {
                self.build_stream::<i16>(&config, samples, position, finished.clone())?
            }
            format => anyhow::bail!("unsupported sample format: {:?}", format),
        };

        stream.play().context("failed to start playback stream")?;

        Ok(AudioPlayback {
            _stream: stream,
            finished,
        })
    }

    fn build_stream<T>(
        &self,
        config: &StreamConfig,
        samples: Arc<Vec<f32>>,
        position: Arc<AtomicUsize>,
        finished: Arc<AtomicBool>,
    ) -> Result<Stream>
    where
        T: SizedSample + FromSample<f32> + Default + Send + 'static,
    {
        self.device
            .build_output_stream(
                config,
                move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
                    let pos = position.load(Ordering::SeqCst);
                    let remaining = samples.len().saturating_sub(pos);

                    if remaining == 0 {
                        data.fill(T::default());
                        finished.store(true, Ordering::SeqCst);
                        return;
                    }

                    let to_copy = remaining.min(data.len());
                    for (i, &sample) in samples[pos..pos + to_copy].iter().enumerate() {
                        data[i] = T::from_sample(sample);
                    }

                    if to_copy < data.len() {
                        data[to_copy..].fill(T::default());
                    }

                    position.store(pos + to_copy, Ordering::SeqCst);
                },
                move |err| {
                    tracing::error!(error = ?err, "playback stream error");
                },
                None,
            )
            .context("failed to build output stream")
    }
}

fn i16_bytes_to_f32(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(2)
        .map(|chunk| {
            let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
            sample as f32 / 32768.0
        })
        .collect()
}

fn resample(samples: &[f32], source_rate: u32, target_rate: u32) -> Result<Vec<f32>> {
    let chunk_size = 1024;
    let mut resampler =
        FftFixedIn::<f32>::new(source_rate as usize, target_rate as usize, chunk_size, 2, 1)
            .context("failed to create resampler")?;

    let mut output = Vec::new();
    let mut pos = 0;

    while pos < samples.len() {
        let frames_needed = resampler.input_frames_next();
        let end = (pos + frames_needed).min(samples.len());

        let mut input_chunk = samples[pos..end].to_vec();
        if input_chunk.len() < frames_needed {
            input_chunk.resize(frames_needed, 0.0);
        }

        let input = vec![input_chunk];
        match resampler.process(&input, None) {
            Ok(resampled) => {
                if let Some(chunk) = resampled.into_iter().next() {
                    output.extend(chunk);
                }
            }
            Err(e) => {
                anyhow::bail!("resampling failed: {:?}", e);
            }
        }

        pos = end;
        if end == samples.len() {
            break;
        }
    }

    Ok(output)
}

fn expand_to_channels(samples: &[f32], channels: usize) -> Vec<f32> {
    let mut output = Vec::with_capacity(samples.len() * channels);
    for &sample in samples {
        for _ in 0..channels {
            output.push(sample);
        }
    }
    output
}
