use serde::{Deserialize, Serialize};

/// Audio data returned from TTS synthesis
#[derive(Debug, Clone)]
pub struct AudioData {
    pub pcm_data: Vec<u8>,
    pub sample_rate: u32,
    pub channels: u16,
}

/// Voice configuration for TTS
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Voice {
    pub id: String,
    pub name: String,
    pub language_code: String,
}
