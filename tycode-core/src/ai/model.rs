use serde::{Deserialize, Serialize};

use crate::ai::ModelSettings;

use crate::ai::types::ReasoningBudget;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[derive(Default)]
pub enum Model {
    ClaudeOpus41,
    ClaudeOpus4,
    #[default]
    ClaudeSonnet4,
    ClaudeSonnet37,
    GptOss120b,
    GrokCodeFast1,
    Qwen3Coder,
    Gemini25Flash,

    /// This allows code to match all models, but still match _ => to
    /// avoid being *required* to match all models.
    None,
}

impl Model {
    pub const fn name(self) -> &'static str {
        match self {
            Self::ClaudeOpus41 => "claude-opus-4-1",
            Self::ClaudeOpus4 => "claude-opus-4",
            Self::ClaudeSonnet4 => "claude-sonnet-4",
            Self::ClaudeSonnet37 => "claude-sonnet-3-7",
            Self::GptOss120b => "gpt-oss-120b",
            Self::GrokCodeFast1 => "grok-code-fast-1",
            Self::Qwen3Coder => "qwen3-coder",
            Self::Gemini25Flash => "gemini-2-5-flash",
            Self::None => "None",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "claude-opus-4-1" => Some(Self::ClaudeOpus41),
            "claude-opus-4" => Some(Self::ClaudeOpus4),
            "claude-sonnet-4" => Some(Self::ClaudeSonnet4),
            "claude-sonnet-3-7" => Some(Self::ClaudeSonnet37),
            "gpt-oss-120b" => Some(Self::GptOss120b),
            "grok-code-fast-1" => Some(Self::GrokCodeFast1),
            "qwen3-coder" => Some(Self::Qwen3Coder),
            "gemini-2-5-flash" => Some(Self::Gemini25Flash),
            _ => None,
        }
    }

    pub fn all_models() -> Vec<Self> {
        vec![
            Self::ClaudeOpus41,
            Self::ClaudeOpus4,
            Self::ClaudeSonnet4,
            Self::ClaudeSonnet37,
            Self::GptOss120b,
            Self::GrokCodeFast1,
            Self::Qwen3Coder,
            Self::Gemini25Flash,
        ]
    }

    // Return default model settings for the model
    pub fn default_settings(self) -> ModelSettings {
        ModelSettings {
            model: self,
            max_tokens: Some(32000),
            temperature: Some(1.0),
            top_p: None,
            reasoning_budget: ReasoningBudget::High,
        }
    }
}

