use crate::ai::{model::ModelCost, types::ModelSettings};
use crate::security::SecurityConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn is_default_file_modification_api(api: &FileModificationApi) -> bool {
    api == &FileModificationApi::Default
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum FileModificationApi {
    #[default]
    Default,
    Patch,
    FindReplace,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum ReviewLevel {
    #[default]
    None,
    Task,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// The name of the currently active provider
    #[serde(default)]
    pub active_provider: Option<String>,

    /// Map of provider name to configuration
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,

    /// Security configuration
    #[serde(default)]
    pub security: SecurityConfig,

    /// Agent-specific model overrides
    #[serde(default)]
    pub agent_models: HashMap<String, ModelSettings>,

    /// Default agent to use for new conversations
    #[serde(default = "default_agent_name")]
    pub default_agent: String,

    /// Global maximum quality tier applied across agents
    #[serde(default)]
    pub model_quality: Option<ModelCost>,

    /// Review level for messages
    #[serde(default)]
    pub review_level: ReviewLevel,

    /// MCP server configurations
    #[serde(default)]
    pub mcp_servers: HashMap<String, McpServerConfig>,

    /// File modification API configuration
    #[serde(default, skip_serializing_if = "is_default_file_modification_api")]
    pub file_modification_api: FileModificationApi,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Command to execute for the MCP server
    pub command: String,

    /// Arguments to pass to the command
    #[serde(default)]
    pub args: Vec<String>,

    /// Environment variables to set for the server process
    #[serde(default)]
    pub env: HashMap<String, String>,
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
        behavior: crate::ai::mock::MockBehavior,
    },
    #[serde(rename = "openrouter")]
    OpenRouter { api_key: String },
    #[serde(rename = "claude_code")]
    ClaudeCode {
        #[serde(default = "default_claude_command")]
        command: String,
        #[serde(default)]
        extra_args: Vec<String>,
        #[serde(default)]
        env: HashMap<String, String>,
    },
}

fn default_region() -> String {
    "us-west-2".to_string()
}

fn default_claude_command() -> String {
    "claude".to_string()
}

fn default_agent_name() -> String {
    "one_shot".to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            active_provider: None,
            providers: HashMap::new(),
            security: SecurityConfig::default(),
            agent_models: HashMap::new(),
            default_agent: default_agent_name(),
            model_quality: None,
            review_level: ReviewLevel::None,
            mcp_servers: HashMap::new(),
            file_modification_api: FileModificationApi::Default,
        }
    }
}

impl Settings {
    /// Get the active provider configuration
    pub fn active_provider(&self) -> Option<&ProviderConfig> {
        let provider = self.active_provider.as_ref()?;
        self.providers.get(provider)
    }

    /// Set the active provider (returns error if provider doesn't exist)
    pub fn set_active_provider(&mut self, name: &str) -> Result<(), String> {
        if self.providers.contains_key(name) {
            self.active_provider = Some(name.to_string());
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
        if Some(name) == self.active_provider.as_deref() {
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
            ProviderConfig::ClaudeCode { .. } => None,
        }
    }

    /// Get the API key for OpenRouter provider
    pub fn openrouter_api_key(&self) -> Option<&str> {
        match self {
            ProviderConfig::OpenRouter { api_key } => Some(api_key.as_str()),
            ProviderConfig::Bedrock { .. } => None,
            ProviderConfig::Mock { .. } => None,
            ProviderConfig::ClaudeCode { .. } => None,
        }
    }
}
