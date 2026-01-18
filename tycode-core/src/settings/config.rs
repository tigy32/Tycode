use crate::ai::{model::ModelCost, types::ModelSettings};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

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

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum RunBuildTestOutputMode {
    #[default]
    ToolResponse,
    Context,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum CommandExecutionMode {
    #[default]
    Direct,
    Bash,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum SpawnContextMode {
    #[default]
    Fork,
    Fresh,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStyle {
    Xml,
    #[default]
    Json,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CommunicationTone {
    #[default]
    ConciseAndLogical,
    WarmAndFlowy,
    Cat,
    Meme,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutonomyLevel {
    /// Agent can proceed with implementation directly without presenting a plan
    FullyAutonomous,
    /// Agent must present and get approval before implementing changes
    #[default]
    PlanApprovalRequired,
}

fn default_memory_cost() -> ModelCost {
    ModelCost::High
}

fn default_context_message_count() -> usize {
    8
}

fn default_recent_memories_count() -> usize {
    8
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    pub enabled: bool,
    #[serde(default = "default_memory_cost")]
    pub summarizer_cost: ModelCost,
    #[serde(default = "default_memory_cost")]
    pub recorder_cost: ModelCost,
    #[serde(default = "default_context_message_count")]
    pub context_message_count: usize,
    #[serde(default = "default_recent_memories_count")]
    pub recent_memories_count: usize,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            summarizer_cost: default_memory_cost(),
            recorder_cost: default_memory_cost(),
            context_message_count: default_context_message_count(),
            recent_memories_count: default_recent_memories_count(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum VoiceProviderConfig {
    #[serde(rename = "aws_transcribe")]
    AwsTranscribe {
        profile: String,
        #[serde(default = "default_region")]
        region: String,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VoiceSettings {
    #[serde(default)]
    pub enabled: bool,
    pub provider: Option<VoiceProviderConfig>,
}

/// Configuration for the skills system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsConfig {
    /// Master switch to enable/disable skills
    #[serde(default = "default_skills_enabled")]
    pub enabled: bool,

    /// Skills to disable by name
    #[serde(default)]
    pub disabled_skills: HashSet<String>,

    /// Additional directories to search for skills
    #[serde(default)]
    pub additional_dirs: Vec<PathBuf>,

    /// Load skills from ~/.claude/skills/ for Claude Code compatibility
    #[serde(default = "default_claude_code_compat")]
    pub enable_claude_code_compat: bool,
}

fn default_skills_enabled() -> bool {
    true
}

fn default_claude_code_compat() -> bool {
    true
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            enabled: default_skills_enabled(),
            disabled_skills: HashSet::new(),
            additional_dirs: Vec::new(),
            enable_claude_code_compat: default_claude_code_compat(),
        }
    }
}

/// Configuration for the plugin system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginsConfig {
    /// Master switch to enable/disable plugin system
    #[serde(default = "default_plugins_enabled")]
    pub enabled: bool,

    /// Plugins to disable by name
    #[serde(default)]
    pub disabled_plugins: HashSet<String>,

    /// Additional directories to search for plugins
    #[serde(default)]
    pub additional_dirs: Vec<PathBuf>,

    /// Load plugins from ~/.claude/plugins/ for Claude Code compatibility
    #[serde(default = "default_claude_code_compat")]
    pub enable_claude_code_compat: bool,

    /// Allow loading native (.dylib/.so/.dll) plugins
    #[serde(default = "default_allow_native")]
    pub allow_native: bool,

    /// Per-plugin configuration values
    #[serde(default)]
    pub config: HashMap<String, serde_json::Value>,
}

fn default_plugins_enabled() -> bool {
    true
}

fn default_allow_native() -> bool {
    false // Disabled by default for security
}

impl Default for PluginsConfig {
    fn default() -> Self {
        Self {
            enabled: default_plugins_enabled(),
            disabled_plugins: HashSet::new(),
            additional_dirs: Vec::new(),
            enable_claude_code_compat: default_claude_code_compat(),
            allow_native: default_allow_native(),
            config: HashMap::new(),
        }
    }
}

/// Core application settings.
///
/// # Maintainer Note
///
/// When adding new settings fields, you must also update the VSCode extension
/// settings UI in:
/// - `tycode-vscode/src/settingsProvider.ts` - HTML form elements
/// - `tycode-vscode/src/webview/settings.js` - JavaScript state and handlers
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// The name of the currently active provider
    #[serde(default)]
    pub active_provider: Option<String>,

    /// Map of provider name to configuration
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,

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

    /// Maximum bytes for auto-including directory list in context
    #[serde(default = "default_auto_context_bytes")]
    pub auto_context_bytes: usize,

    /// Output mode for run_build_test tool
    #[serde(default)]
    pub run_build_test_output_mode: RunBuildTestOutputMode,

    /// Enable type analyzer tools (search_types, get_type_docs)
    #[serde(default)]
    pub enable_type_analyzer: bool,

    /// Controls how sub-agent context is initialized when spawning
    #[serde(default)]
    pub spawn_context_mode: SpawnContextMode,

    /// Enable XML-based tool calling instead of native tool use
    #[serde(default)]
    pub xml_tool_mode: bool,

    /// Disable custom steering documents (from .tycode and external agent configs)
    #[serde(default)]
    pub disable_custom_steering: bool,

    /// Communication tone for agent responses
    #[serde(default)]
    pub communication_tone: CommunicationTone,

    /// Memory system configuration
    #[serde(default)]
    pub memory: MemoryConfig,

    /// Controls whether agent must get plan approval before implementing
    #[serde(default)]
    pub autonomy_level: AutonomyLevel,

    /// Voice/speech-to-text configuration
    #[serde(default)]
    pub voice: VoiceSettings,

    /// Command execution mode (direct exec vs bash wrapper)
    #[serde(default)]
    pub command_execution_mode: CommandExecutionMode,

    /// Skills system configuration
    #[serde(default)]
    pub skills: SkillsConfig,

    /// Plugin system configuration
    #[serde(default)]
    pub plugins: PluginsConfig,
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

fn default_auto_context_bytes() -> usize {
    80_000
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            active_provider: None,
            providers: HashMap::new(),
            agent_models: HashMap::new(),
            default_agent: default_agent_name(),
            model_quality: None,
            review_level: ReviewLevel::None,
            mcp_servers: HashMap::new(),
            file_modification_api: FileModificationApi::Default,
            auto_context_bytes: default_auto_context_bytes(),
            run_build_test_output_mode: RunBuildTestOutputMode::default(),
            enable_type_analyzer: false,
            spawn_context_mode: SpawnContextMode::default(),
            xml_tool_mode: false,
            disable_custom_steering: false,
            communication_tone: CommunicationTone::default(),
            memory: MemoryConfig::default(),
            autonomy_level: AutonomyLevel::default(),
            voice: VoiceSettings::default(),
            command_execution_mode: CommandExecutionMode::default(),
            skills: SkillsConfig::default(),
            plugins: PluginsConfig::default(),
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
