use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

fn default_enabled() -> bool {
    false
}

fn default_image_model() -> String {
    "google/gemini-2.5-flash-image".to_string()
}

fn default_aspect_ratio() -> String {
    "1:1".to_string()
}

fn default_image_size() -> String {
    "1K".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Image {
    #[serde(default = "default_enabled")]
    pub enabled: bool,

    #[serde(default = "default_image_model")]
    pub model: String,

    #[serde(default = "default_aspect_ratio")]
    pub default_aspect_ratio: String,

    #[serde(default = "default_image_size")]
    pub default_image_size: String,
}

impl Default for Image {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            model: default_image_model(),
            default_aspect_ratio: default_aspect_ratio(),
            default_image_size: default_image_size(),
        }
    }
}
