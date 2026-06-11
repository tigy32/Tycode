use crate::ai::provider::AiProvider;
use crate::ai::tweaks::{ModelTweaks, RegistryFileModificationApi};
use crate::ai::types::ReasoningBudget;
use crate::ai::ModelSettings;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
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

/// Stable, user-selectable model families, subjectively ranked by quality.
///
/// Provider implementations resolve these families to the current tip model ID
/// for that provider. Historical/versioned names are accepted by `from_name` and
/// normalized back to these stable aliases.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::VariantArray)]
pub enum Model {
    // The default models for unlimited/high budget
    ClaudeOpus,
    ClaudeOpusFast,
    ClaudeSonnet,

    // Medium/high cost tier
    Gpt,
    GptPro,
    GptCodex,
    GptCodexMax,
    QwenMax,
    GeminiPro,
    GeminiFlash,
    ClaudeHaiku,
    GptMini,

    // Low cost models
    KimiK2,
    QwenPlus,
    GeminiFlashLite,
    DeepSeekPro,
    Grok,
    GrokBuild,
    GLM,
    MinimaxM2,

    // Even lower cost models
    DeepSeekFlash,
    Ring,
    StepFlash,
    QwenFlash,
    QwenCoder,
    GptOss120b,
    KimiK2Free,
    DeepSeekFlashFree,
    GptOss120bFree,
    OpenRouterAuto,

    /// This allows code to match all models, but still match _ => to
    /// avoid being *required* to match all models.
    None,
}

impl Serialize for Model {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for Model {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Model::from_name(&value)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown model: {value}")))
    }
}

impl Model {
    pub fn tweaks(self) -> ModelTweaks {
        match self {
            Self::Gpt | Self::GptPro | Self::GptMini | Self::GptCodex | Self::GptCodexMax => {
                ModelTweaks {
                    file_modification_api: Some(RegistryFileModificationApi::Patch),
                    ..Default::default()
                }
            }
            _ => ModelTweaks {
                file_modification_api: Some(RegistryFileModificationApi::FindReplace),
                ..Default::default()
            },
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Self::ClaudeOpus => "claude-opus",
            Self::ClaudeOpusFast => "claude-opus-fast",
            Self::ClaudeSonnet => "claude-sonnet",
            Self::ClaudeHaiku => "claude-haiku",

            Self::GeminiPro => "gemini-pro",
            Self::GeminiFlash => "gemini-flash",
            Self::GeminiFlashLite => "gemini-flash-lite",

            Self::Gpt => "gpt",
            Self::GptPro => "gpt-pro",
            Self::GptCodex => "gpt-codex",
            Self::GptCodexMax => "gpt-codex-max",
            Self::GptMini => "gpt-mini",
            Self::GptOss120b => "gpt-oss-120b",
            Self::GptOss120bFree => "gpt-oss-120b-free",

            Self::DeepSeekPro => "deepseek-pro",
            Self::DeepSeekFlash => "deepseek-flash",
            Self::DeepSeekFlashFree => "deepseek-flash-free",
            Self::GLM => "glm",
            Self::MinimaxM2 => "minimax-m2",

            Self::Grok => "grok",
            Self::GrokBuild => "grok-build",
            Self::KimiK2 => "kimi-k2",
            Self::KimiK2Free => "kimi-k2-free",

            Self::QwenMax => "qwen-max",
            Self::QwenPlus => "qwen-plus",
            Self::QwenFlash => "qwen-flash",
            Self::QwenCoder => "qwen-coder",
            Self::Ring => "ring",
            Self::StepFlash => "step-flash",

            Self::OpenRouterAuto => "openrouter/auto",
            Self::None => "None",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        let key = Self::normalized_key(s);
        match key.as_str() {
            "claudeopus" | "opus" | "claudeopus48" | "claudeopus47" | "claudeopus46"
            | "claudeopus45" => Some(Self::ClaudeOpus),
            "claudeopusfast" | "opusfast" | "claudeopus48fast" | "claudeopus47fast" => {
                Some(Self::ClaudeOpusFast)
            }
            "claudesonnet" | "sonnet" | "claudesonnet46" | "claudesonnet45" => {
                Some(Self::ClaudeSonnet)
            }
            "claudehaiku" | "haiku" | "claudehaiku45" => Some(Self::ClaudeHaiku),

            "gpt" | "gpt55" | "gpt54" | "gpt52" => Some(Self::Gpt),
            "gptpro" | "gpt55pro" => Some(Self::GptPro),
            "gptmini" | "gpt54mini" => Some(Self::GptMini),
            "gptcodex" | "gpt53codex" => Some(Self::GptCodex),
            "gptcodexmax" | "gpt51codexmax" => Some(Self::GptCodexMax),
            "gptoss120b" => Some(Self::GptOss120b),
            "gptoss120bfree" => Some(Self::GptOss120bFree),

            "geminipro" | "gemini31pro" => Some(Self::GeminiPro),
            "geminiflash" | "gemini35flash" | "gemini3flashpreview" => Some(Self::GeminiFlash),
            "geminiflashlite" | "gemini31flashlite" => Some(Self::GeminiFlashLite),

            "kimik2" | "kimik26" | "kimik25" => Some(Self::KimiK2),
            "kimik2free" | "kimik26free" => Some(Self::KimiK2Free),

            "qwenmax" | "qwen37max" => Some(Self::QwenMax),
            "qwenplus" | "qwen36plus" => Some(Self::QwenPlus),
            "qwenflash" | "qwen36flash" => Some(Self::QwenFlash),
            "qwencoder" | "qwen3coder" => Some(Self::QwenCoder),

            "deepseekpro" | "deepseekv4pro" => Some(Self::DeepSeekPro),
            "deepseekflash" | "deepseekv4flash" => Some(Self::DeepSeekFlash),
            "deepseekflashfree" | "deepseekv4flashfree" => Some(Self::DeepSeekFlashFree),

            "glm" | "glm51" => Some(Self::GLM),
            "minimaxm2" | "minimaxm27" => Some(Self::MinimaxM2),
            "grok" | "grok420" | "grok43" => Some(Self::Grok),
            "grokbuild" | "grokbuild01" | "grok41fast" | "grokcodefast1" => Some(Self::GrokBuild),
            "ring" | "ring261t" => Some(Self::Ring),
            "stepflash" | "step37flash" => Some(Self::StepFlash),
            "openrouterauto" => Some(Self::OpenRouterAuto),
            "none" => Some(Self::None),
            _ => None,
        }
    }

    fn normalized_key(s: &str) -> String {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect()
    }

    pub const fn supports_prompt_caching(self) -> bool {
        match self {
            Self::ClaudeOpus
            | Self::ClaudeOpusFast
            | Self::ClaudeSonnet
            | Self::ClaudeHaiku
            | Self::GeminiPro
            | Self::GeminiFlash
            | Self::GeminiFlashLite => true,
            Self::OpenRouterAuto => false,
            _ => false,
        }
    }

    /// Context window size in tokens for this model.
    pub const fn context_window(self) -> u32 {
        match self {
            Self::ClaudeOpus | Self::ClaudeOpusFast | Self::ClaudeSonnet => 1_000_000,
            Self::ClaudeHaiku => 200_000,

            Self::GeminiPro | Self::GeminiFlash | Self::GeminiFlashLite => 1_048_576,

            Self::Gpt | Self::GptPro => 1_050_000,
            Self::GptMini | Self::GptCodex | Self::GptCodexMax => 400_000,
            Self::GptOss120b | Self::GptOss120bFree => 131_072,

            Self::DeepSeekPro | Self::DeepSeekFlash | Self::DeepSeekFlashFree => 1_048_576,
            Self::GLM => 202_752,
            Self::MinimaxM2 => 204_800,

            Self::Grok => 1_000_000,
            Self::GrokBuild => 256_000,
            Self::KimiK2 | Self::KimiK2Free => 262_144,

            Self::QwenMax | Self::QwenPlus | Self::QwenFlash => 1_000_000,
            Self::QwenCoder => 1_048_576,
            Self::Ring => 262_144,
            Self::StepFlash => 256_000,

            Self::OpenRouterAuto => 2_000_000,
            Self::None => 200_000,
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

#[cfg(test)]
mod tests {
    use super::Model;

    #[test]
    fn versioned_model_names_deserialize_to_stable_family_aliases() {
        for (name, expected) in [
            ("claude-opus-4-8", Model::ClaudeOpus),
            ("claude-sonnet-4-6", Model::ClaudeSonnet),
            ("kimi-k2.6", Model::KimiK2),
            ("kimi-k2-5", Model::KimiK2),
            ("gemini-3.5-flash", Model::GeminiFlash),
            ("gemini-3-flash-preview", Model::GeminiFlash),
            ("gpt-5.5", Model::Gpt),
            ("gpt-5.2", Model::Gpt),
        ] {
            assert_eq!(Model::from_name(name), Some(expected));
        }

        assert_eq!(
            Model::from_name(&format!("ClaudeOpus{}", 46)),
            Some(Model::ClaudeOpus)
        );
        assert_eq!(
            Model::from_name(&format!("ClaudeSonnet{}", 45)),
            Some(Model::ClaudeSonnet)
        );
    }

    #[test]
    fn model_serializes_to_stable_alias_name() {
        assert_eq!(
            serde_json::to_string(&Model::ClaudeOpus).unwrap(),
            "\"claude-opus\""
        );
        assert_eq!(
            serde_json::to_string(&Model::KimiK2).unwrap(),
            "\"kimi-k2\""
        );
    }
}
