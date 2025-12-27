//! Audio capture and playback for voice functionality

pub mod capture;
pub mod playback;

/// Audio format profile specifying sample rate and channel count
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioProfile {
    pub sample_rate: u32,
    pub channels: u16,
}
