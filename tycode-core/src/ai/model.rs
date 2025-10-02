use crate::ai::provider::AiProvider;
use crate::ai::types::ReasoningBudget;
use crate::ai::ModelSettings;
use serde::{Deserialize, Serialize};
use strum::VariantArray;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord, Default)]
#[serde(rename_all = "snake_case")]
pub enum ModelCost {
    Free,
    Low,
    #[default]
    Medium,
    High,
    Unlimited,
}

impl ModelCost {
    pub const fn all_levels() -> [Self; 5] {
        [
            Self::Free,
            Self::Low,
            Self::Medium,
            Self::High,
            Self::Unlimited,
        ]
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::Free => "Restrict to free models only. Your data will likely used for training.",
            Self::Low => "Under $1/million tokens",
            Self::Medium => "Under $5/million tokens",
            Self::High => "Under $15/million tokens",
            Self::Unlimited => "No restrictions",
        }
    }
}

impl TryFrom<&str> for ModelCost {
    type Error = String;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let lower = value.to_lowercase();
        match lower.as_str() {
            "free" => Ok(Self::Free),
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            "unlimited" => Ok(Self::Unlimited),
            _ => Err(format!(
                "Invalid model cost level: {}. Valid options: free, low, medium, high, unlimited",
                value
            )),
        }
    }
}

/// The supported models, subjectively ranked by quality
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, strum::VariantArray)]
pub enum Model {
    ClaudeSonnet45,
    ClaudeOpus41,
    ClaudeOpus4,

    ClaudeSonnet4,
    Grok4Fast,

    ClaudeSonnet37,
    GrokCodeFast1,
    Gemini25Flash,

    Qwen3Coder,
    GptOss120b,

    /// This allows code to match all models, but still match _ => to
    /// avoid being *required* to match all models.
    None,
}

impl Model {
    pub const fn name(self) -> &'static str {
        match self {
            Self::ClaudeSonnet45 => "claude-sonnet-45",
            Self::ClaudeOpus41 => "claude-opus-4-1",
            Self::ClaudeOpus4 => "claude-opus-4",
            Self::ClaudeSonnet4 => "claude-sonnet-4",
            Self::ClaudeSonnet37 => "claude-sonnet-3-7",
            Self::GptOss120b => "gpt-oss-120b",
            Self::GrokCodeFast1 => "grok-code-fast-1",
            Self::Qwen3Coder => "qwen3-coder",
            Self::Gemini25Flash => "gemini-2-5-flash",
            Self::Grok4Fast => "grok-4-fast",
            Self::None => "None",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "claude-sonnet-45" => Some(Self::ClaudeSonnet45),
            "claude-opus-4-1" => Some(Self::ClaudeOpus41),
            "claude-opus-4" => Some(Self::ClaudeOpus4),
            "claude-sonnet-4" => Some(Self::ClaudeSonnet4),
            "claude-sonnet-3-7" => Some(Self::ClaudeSonnet37),
            "gpt-oss-120b" => Some(Self::GptOss120b),
            "grok-code-fast-1" => Some(Self::GrokCodeFast1),
            "qwen3-coder" => Some(Self::Qwen3Coder),
            "gemini-2-5-flash" => Some(Self::Gemini25Flash),
            "grok-4-fast" => Some(Self::Grok4Fast),
            _ => None,
        }
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

    /// Select the highest quality model supported by the provider that fits the cost threshold.
    /// For Unlimited, returns the highest supported model. For others, filters by max(input/output cost per 1k) <= threshold.
    /// Free requires exact 0.0 match. Ranked highest-to-lowest to prefer premium within budget.
    /// Returns None if no fit (surfaces error to caller—no fallback).
    pub fn select_for_cost(provider: &dyn AiProvider, quality: ModelCost) -> Option<ModelSettings> {
        let supported = provider.supported_models();
        let models: Vec<&'static Model> = Model::VARIANTS
            .into_iter()
            .filter(|m| supported.contains(m))
            .collect();

        let threshold = match quality {
            ModelCost::Free => 0.0,
            ModelCost::Low => 0.001,
            ModelCost::Medium => 0.003,
            ModelCost::High => 0.010,
            ModelCost::Unlimited => f64::MAX,
        };

        for model in models {
            let cost = provider.get_cost(model);
            // assume 5 is to 1 input to output
            let cost = (cost.input_cost_per_1k_tokens * 5.0 + cost.output_cost_per_1k_tokens) / 6.0;
            if cost <= threshold {
                return Some(model.default_settings());
            }
        }

        None
    }
}
