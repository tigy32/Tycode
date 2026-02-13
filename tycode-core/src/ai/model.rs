use crate::ai::provider::AiProvider;
use crate::ai::tweaks::{ModelTweaks, RegistryFileModificationApi};
use crate::ai::types::ReasoningBudget;
use crate::ai::ModelSettings;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::VariantArray;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord, Default, JsonSchema,
)]
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
    // The default models for unlimited/high budget
    ClaudeOpus46,
    ClaudeOpus45,
    ClaudeSonnet45,

    // Medium cost tier
    ClaudeHaiku45,
    Gemini3ProPreview,
    Gpt52,
    Gpt51CodexMax,

    // Low cost models
    KimiK25,
    Gemini3FlashPreview,
    GLM5,
    MinimaxM25,
    Grok41Fast,
    GrokCodeFast1,

    // Even lower cost models
    Qwen3Coder,
    GptOss120b,
    OpenRouterAuto,

    /// This allows code to match all models, but still match _ => to
    /// avoid being *required* to match all models.
    None,
}

impl Model {
    pub fn tweaks(self) -> ModelTweaks {
        match self {
            Self::Gpt52 | Self::Gpt51CodexMax => ModelTweaks {
                file_modification_api: Some(RegistryFileModificationApi::Patch),
                ..Default::default()
            },
            _ => ModelTweaks {
                file_modification_api: Some(RegistryFileModificationApi::FindReplace),
                ..Default::default()
            },
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::ClaudeSonnet45 => "claude-sonnet-45",
            Self::ClaudeOpus46 => "claude-opus-4-6",
            Self::ClaudeOpus45 => "claude-opus-4-5",
            Self::ClaudeHaiku45 => "claude-haiku-45",

            Self::Gemini3ProPreview => "gemini-3-pro-preview",
            Self::Gemini3FlashPreview => "gemini-3-flash-preview",

            Self::Gpt52 => "gpt-5-2",
            Self::Gpt51CodexMax => "gpt-5-1-codex-max",
            Self::GptOss120b => "gpt-oss-120b",

            Self::GLM5 => "glm-5",
            Self::MinimaxM25 => "minimax-m2-5",

            Self::Grok41Fast => "grok-4-1-fast",
            Self::GrokCodeFast1 => "grok-code-fast-1",
            Self::KimiK25 => "kimi-k2-5",

            Self::Qwen3Coder => "qwen3-coder",

            Self::OpenRouterAuto => "openrouter/auto",
            Self::None => "None",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "claude-sonnet-45" => Some(Self::ClaudeSonnet45),
            "claude-opus-4-6" => Some(Self::ClaudeOpus46),
            "claude-opus-4-5" => Some(Self::ClaudeOpus45),
            "claude-haiku-45" => Some(Self::ClaudeHaiku45),
            "gemini-3-pro-preview" => Some(Self::Gemini3ProPreview),
            "gemini-3-flash-preview" => Some(Self::Gemini3FlashPreview),
            "gpt-5-2" => Some(Self::Gpt52),
            "gpt-5-1-codex-max" => Some(Self::Gpt51CodexMax),
            "gpt-oss-120b" => Some(Self::GptOss120b),
            "glm-5" => Some(Self::GLM5),
            "minimax-m2-5" => Some(Self::MinimaxM25),
            "grok-4-1-fast" => Some(Self::Grok41Fast),
            "grok-code-fast-1" => Some(Self::GrokCodeFast1),
            "kimi-k2-5" => Some(Self::KimiK25),
            "qwen3-coder" => Some(Self::Qwen3Coder),
            "openrouter/auto" => Some(Self::OpenRouterAuto),
            _ => None,
        }
    }

    pub const fn supports_prompt_caching(self) -> bool {
        match self {
            Self::ClaudeSonnet45
            | Self::ClaudeOpus46
            | Self::ClaudeOpus45
            | Self::ClaudeHaiku45 => true,
            Self::OpenRouterAuto => false,
            _ => false,
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
    /// For Unlimited, returns the highest supported model. For others, filters by max(input/output cost per million tokens) <= threshold.
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
            ModelCost::Low => 1.0,
            ModelCost::Medium => 3.0,
            ModelCost::High => 10.0,
            ModelCost::Unlimited => f64::MAX,
        };

        for model in models {
            let cost = provider.get_cost(model);
            // assume 5 is to 1 input to output
            let cost = (cost.input_cost_per_million_tokens * 5.0
                + cost.output_cost_per_million_tokens)
                / 6.0;
            if cost <= threshold {
                return Some(model.default_settings());
            }
        }

        None
    }
}
