use crate::ai::{
    model::{Model, ModelCost},
    types::{ModelSettings, ReasoningBudget},
};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default, JsonSchema)]
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

/// How the tycode agent implements code changes. This is a policy on the
/// conversational root agent: questions and follow-ups are always answered
/// directly; the mode governs how changes are delegated.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OrchestrationMode {
    /// The model decides per task: edit directly, spawn a coder, or use the
    /// builder pipeline. The swarm workflow is never available in this mode.
    #[default]
    Auto,
    /// Every code change must go through the builder pipeline
    /// (plan -> implement -> review).
    Builder,
    /// Every code change must go through the swarm workflow (consensus
    /// planning and concurrent per-file implementation).
    Swarm,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub enum SpawnContextMode {
    #[default]
    Fork,
    Fresh,
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
    #[default]
    FullyAutonomous,
    /// Agent must present and get approval before implementing changes
    PlanApprovalRequired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TtsProviderConfig {
    #[serde(rename = "aws_polly")]
    AwsPolly {
        #[serde(default)]
        profile: Option<String>,
        #[serde(default = "default_region")]
        region: String,
    },
    #[serde(rename = "elevenlabs")]
    ElevenLabs {
        api_key: String,
        #[serde(default)]
        voice_id: Option<String>,
        #[serde(default)]
        model_id: Option<String>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SttProviderConfig {
    #[serde(rename = "aws_transcribe")]
    AwsTranscribe {
        #[serde(default)]
        profile: Option<String>,
        #[serde(default = "default_region")]
        region: String,
    },
    #[serde(rename = "elevenlabs")]
    ElevenLabs {
        api_key: String,
        #[serde(default)]
        model_id: Option<String>,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceSettings {
    #[serde(default)]
    pub default_tts: Option<String>,

    #[serde(default)]
    pub default_stt: Option<String>,

    #[serde(default)]
    pub tts_providers: HashMap<String, TtsProviderConfig>,

    #[serde(default)]
    pub stt_providers: HashMap<String, SttProviderConfig>,
}

impl Default for VoiceSettings {
    fn default() -> Self {
        Self {
            default_tts: None,
            default_stt: None,
            tts_providers: HashMap::new(),
            stt_providers: HashMap::new(),
        }
    }
}

impl VoiceSettings {
    pub fn active_tts(&self) -> Option<&TtsProviderConfig> {
        let name = self.default_tts.as_ref()?;
        self.tts_providers.get(name)
    }

    pub fn active_stt(&self) -> Option<&SttProviderConfig> {
        let name = self.default_stt.as_ref()?;
        self.stt_providers.get(name)
    }
}

/// Configuration for the skills system.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
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

    /// Maximum review reject/fix rounds before a completion is accepted with
    /// the unresolved feedback attached, preventing unbounded review loops
    #[serde(default = "default_max_review_rounds")]
    pub max_review_rounds: u32,

    /// Maximum concurrent workers during fan-out orchestration
    #[serde(default = "default_fanout_concurrency")]
    pub fanout_concurrency: usize,

    /// How the tycode agent implements code changes: auto (model decides,
    /// swarm unavailable), builder (all changes through the builder
    /// pipeline), or swarm (all changes through the swarm workflow).
    #[serde(default)]
    pub orchestration_mode: OrchestrationMode,

    /// Emit human-readable system chat messages for orchestration progress
    /// (agent spawns, fan-out worker status). UIs that render the structured
    /// Orchestration events can disable this to avoid double-rendering; the
    /// structured events are always emitted.
    #[serde(default = "default_orchestration_progress_messages")]
    pub orchestration_progress_messages: bool,

    /// Models for multi-model consensus in the swarm workflow. With two or
    /// more entries, planning fans out one planner per model, a judge panel
    /// of all models votes on the best plan, the winning model implements,
    /// and integration review requires approval from every model. Empty or
    /// single-entry lists keep the single-model workflow.
    #[serde(default)]
    pub swarm_models: Vec<Model>,

    /// MCP server configurations
    #[serde(default)]
    pub mcp_servers: HashMap<String, McpServerConfig>,

    /// Controls how sub-agent context is initialized when spawning
    #[serde(default)]
    pub spawn_context_mode: SpawnContextMode,

    /// Disable custom steering documents (from .tycode and external agent configs)
    #[serde(default)]
    pub disable_custom_steering: bool,

    /// Communication tone for agent responses
    #[serde(default)]
    pub communication_tone: CommunicationTone,

    /// Controls whether agent must get plan approval before implementing
    #[serde(default)]
    pub autonomy_level: AutonomyLevel,

    /// Voice/speech-to-text configuration
    #[serde(default)]
    pub voice: VoiceSettings,

    /// Skills system configuration
    #[serde(default)]
    pub skills: SkillsConfig,

    /// Global default reasoning effort applied to all agents unless overridden
    #[serde(default)]
    pub reasoning_effort: Option<ReasoningBudget>,

    /// When true, AI responses arrive as a single complete message instead of streaming incrementally
    #[serde(default)]
    pub disable_streaming: bool,

    /// Enables modules to own their configuration without modifying tycode-core,
    /// supporting external/plugin modules that aren't known at compile time.
    #[serde(default)]
    pub modules: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpServerConfig {
    Http {
        /// The URL endpoint for the MCP server
        url: String,

        /// Optional HTTP headers (e.g. Authorization)
        #[serde(default)]
        headers: HashMap<String, String>,
    },
    Stdio {
        /// Command to execute for the MCP server
        command: String,

        /// Arguments to pass to the command
        #[serde(default)]
        args: Vec<String>,

        /// Environment variables to set for the server process
        #[serde(default)]
        env: HashMap<String, String>,
    },
}

impl McpServerConfig {
    /// Returns a display label for logging/UI (the command or URL)
    pub fn display_label(&self) -> &str {
        match self {
            McpServerConfig::Stdio { command, .. } => command,
            McpServerConfig::Http { url, .. } => url,
        }
    }
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
    #[serde(other)]
    Unknown,
}

fn default_region() -> String {
    "us-west-2".to_string()
}

fn default_max_review_rounds() -> u32 {
    3
}

fn default_fanout_concurrency() -> usize {
    4
}

fn default_orchestration_progress_messages() -> bool {
    true
}

fn default_agent_name() -> String {
    "tycode".to_string()
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
            max_review_rounds: default_max_review_rounds(),
            fanout_concurrency: default_fanout_concurrency(),
            orchestration_mode: OrchestrationMode::default(),
            orchestration_progress_messages: default_orchestration_progress_messages(),
            swarm_models: Vec::new(),
            mcp_servers: HashMap::new(),
            spawn_context_mode: SpawnContextMode::default(),
            disable_custom_steering: false,
            communication_tone: CommunicationTone::default(),
            autonomy_level: AutonomyLevel::default(),
            reasoning_effort: None,
            disable_streaming: false,
            voice: VoiceSettings::default(),
            skills: SkillsConfig::default(),
            modules: HashMap::new(),
        }
    }
}

impl Settings {
    /// Returns true when settings carry no meaningful user configuration and
    /// should not be persisted.
    pub fn is_empty_for_persistence(&self) -> bool {
        fn normalize_default_agent(value: &mut serde_json::Value) {
            let serde_json::Value::Object(map) = value else {
                return;
            };

            let Some(serde_json::Value::String(agent)) = map.get_mut("default_agent") else {
                return;
            };

            if agent.trim().is_empty() {
                *agent = default_agent_name();
            }
        }

        let mut current = match serde_json::to_value(self) {
            Ok(v) => v,
            Err(_) => return false,
        };
        let mut default = match serde_json::to_value(Settings::default()) {
            Ok(v) => v,
            Err(_) => return false,
        };

        normalize_default_agent(&mut current);
        normalize_default_agent(&mut default);

        current == default
    }

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

    /// Get module-specific configuration, deserializing from the modules map
    pub fn get_module_config<T: Default + DeserializeOwned>(&self, namespace: &str) -> T {
        self.modules
            .get(namespace)
            .and_then(|v| {
                serde_json::from_value(v.clone())
                    .map_err(|e| tracing::warn!("Failed to parse module config '{namespace}': {e}"))
                    .ok()
            })
            .unwrap_or_default()
    }

    /// Set module-specific configuration, serializing to the modules map
    pub fn set_module_config<T: serde::Serialize>(&mut self, namespace: &str, config: T) {
        if let Ok(value) = serde_json::to_value(&config) {
            self.modules.insert(namespace.to_string(), value);
        }
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
            _ => None,
        }
    }

    /// Get the API key for OpenRouter provider
    pub fn openrouter_api_key(&self) -> Option<&str> {
        match self {
            ProviderConfig::OpenRouter { api_key } => Some(api_key.as_str()),
            _ => None,
        }
    }
}
