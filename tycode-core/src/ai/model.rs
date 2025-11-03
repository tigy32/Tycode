use crate::ai::provider::AiProvider;
use crate::ai::types::ReasoningBudget;
use crate::ai::ModelSettings;
use crate::tools::registry::RegistryFileModificationApi;
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
    // The default models for unlimited/high budet. Sonnet seems strictly better
    // than opus right now, but if you prefer opus or hate money, go for it?
    ClaudeSonnet45,
    ClaudeOpus41,

    // The medium cost tier models. Haiku is working better than gpt5 for me,
    // and its cheaper. GPT5/GPT5Codex don't seem worth using.
    ClaudeHaiku45,
    Gpt5Codex,
    Gpt5,

    // The low cost models - Grok low cost models still seem better than GLM to
    // me after a bunch more manual testing. There may be quantization problems
    // making GLM quality variable? Perhaps we should pin to zai
    Grok4Fast,
    GrokCodeFast1,
    GLM46,

    // Even lower cost models - These aren't that useful at whole tasks, but if
    // are setting up a complex multi-agent flow they can edit files cheaply and
    // effectively.
    Qwen3Coder,
    GptOss120b,

    // Gemini models don't understand tycode tools well. Gemini pro seems to
    // not make a tool choice and I haven't spent much time trying to figure
    // out why. Gemini models are kinda old anyway...
    Gemini25Pro,
    Gemini25Flash,

    /// This allows code to match all models, but still match _ => to
    /// avoid being *required* to match all models.
    None,
}

impl Model {
    /// Returns the preferred file modification API for this model
    pub const fn preferred_file_modification_api(self) -> RegistryFileModificationApi {
        match self {
            // GPT models prefer patch format
            Self::Gpt5 | Self::Gpt5Codex => RegistryFileModificationApi::Patch,

            // All other models default to search/replace
            _ => RegistryFileModificationApi::FindReplace,
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::ClaudeSonnet45 => "claude-sonnet-45",
            Self::ClaudeOpus41 => "claude-opus-4-1",

            Self::GLM46 => "glm-4-6",

            Self::Grok4Fast => "grok-4-fast",
            Self::GrokCodeFast1 => "grok-code-fast-1",
            Self::ClaudeHaiku45 => "claude-haiku-45",

            Self::Gemini25Pro => "gemini-2-5-pro",
            Self::Gemini25Flash => "gemini-2-5-flash",

            Self::Gpt5 => "gpt-5",
            Self::Gpt5Codex => "gpt-5-codex",
            Self::GptOss120b => "gpt-oss-120b",

            Self::Qwen3Coder => "qwen3-coder",

            Self::None => "None",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "claude-sonnet-45" => Some(Self::ClaudeSonnet45),
            "claude-opus-4-1" => Some(Self::ClaudeOpus41),
            "gemini-2-5-pro" => Some(Self::Gemini25Pro),
            "gpt-oss-120b" => Some(Self::GptOss120b),
            "grok-code-fast-1" => Some(Self::GrokCodeFast1),
            "qwen3-coder" => Some(Self::Qwen3Coder),
            "gemini-2-5-flash" => Some(Self::Gemini25Flash),
            "grok-4-fast" => Some(Self::Grok4Fast),
            "gpt-5-codex" => Some(Self::Gpt5Codex),
            "gpt-5" => Some(Self::Gpt5),
            "glm-4-6" => Some(Self::GLM46),
            "claude-haiku-45" => Some(Self::ClaudeHaiku45),
            _ => None,
        }
    }

    pub const fn supports_prompt_caching(self) -> bool {
        match self {
            Self::ClaudeSonnet45 | Self::ClaudeOpus41 | Self::ClaudeHaiku45 => true,
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
