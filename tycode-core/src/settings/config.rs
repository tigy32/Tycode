use crate::ai::types::ModelSettings;
use crate::security::types::SecurityConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub enum ReviewLevel {
    #[default]
    None,
    Modification,
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// The name of the currently active provider
    #[serde(default = "default_active_provider")]
    pub active_provider: String,

    /// Map of provider name to configuration
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,

    /// Security configuration
    #[serde(default)]
    pub security: SecurityConfig,

    /// Agent-specific model overrides
    #[serde(default)]
    pub agent_models: HashMap<String, ModelSettings>,

    /// Review level for messages
    #[serde(default)]
    pub review_level: ReviewLevel,
}

fn default_active_provider() -> String {
    "default".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ProviderConfig {
    #[serde(rename = "bedrock")]
    Bedrock {
        profile: String,
        #[serde(default = "default_region")]
        region: String,
    },
    #[serde(rename = "mock")]
    Mock {
        #[serde(default)]
        behavior: MockBehaviorConfig,
    },
    #[serde(rename = "openrouter")]
    OpenRouter { api_key: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum MockBehaviorConfig {
    #[default]
    Success,
    RetryThenSuccess {
        errors_before_success: usize,
    },
    AlwaysRetryError,
    AlwaysError,
    ToolUse {
        tool_name: String,
        tool_arguments: String,
    },
}

fn default_region() -> String {
    "us-west-2".to_string()
}

impl Default for Settings {
    fn default() -> Self {
        let mut providers = HashMap::new();
        providers.insert(
            "default".to_string(),
            ProviderConfig::Bedrock {
                profile: "default".to_string(),
                region: default_region(),
            },
        );

        Self {
            active_provider: "default".to_string(),
            providers,
            security: SecurityConfig::default(),
            agent_models: HashMap::new(),
            review_level: ReviewLevel::None,
        }
    }
}

impl Settings {
    /// Get the active provider configuration
    pub fn active_provider(&self) -> Option<&ProviderConfig> {
        self.providers.get(&self.active_provider)
    }

    /// Set the active provider (returns error if provider doesn't exist)
    pub fn set_active_provider(&mut self, name: &str) -> Result<(), String> {
        if self.providers.contains_key(name) {
            self.active_provider = name.to_string();
            Ok(())
        } else {
            Err(format!("Provider '{name}' not found"))
        }
    }

    /// Add or update a provider configuration
    pub fn add_provider(&mut self, name: String, config: ProviderConfig) {
        self.providers.insert(name, config);
    }

    /// Remove a provider configuration
    pub fn remove_provider(&mut self, name: &str) -> Result<(), String> {
        if name == self.active_provider {
            return Err("Cannot remove the active provider".to_string());
        }

        if self.providers.remove(name).is_some() {
            Ok(())
        } else {
            Err(format!("Provider '{name}' not found"))
        }
    }

    /// List all provider names
    pub fn list_providers(&self) -> Vec<String> {
        self.providers.keys().cloned().collect()
    }

    /// Get the model settings for a specific agent
    pub fn get_agent_model(&self, agent_name: &str) -> Option<&ModelSettings> {
        self.agent_models.get(agent_name)
    }

    /// Set the model settings for a specific agent
    pub fn set_agent_model(&mut self, agent_name: String, model: ModelSettings) {
        self.agent_models.insert(agent_name, model);
    }
}

impl ProviderConfig {
    /// Get the AWS profile for Bedrock provider
    pub fn bedrock_profile(&self) -> Option<&str> {
        match self {
            ProviderConfig::Bedrock { profile, .. } => Some(profile.as_str()),
            ProviderConfig::Mock { .. } => None,
            ProviderConfig::OpenRouter { .. } => None,
        }
    }

    /// Get the API key for OpenRouter provider
    pub fn openrouter_api_key(&self) -> Option<&str> {
        match self {
            ProviderConfig::OpenRouter { api_key } => Some(api_key.as_str()),
            ProviderConfig::Bedrock { .. } => None,
            ProviderConfig::Mock { .. } => None,
        }
    }
}
