use crate::{
    agents::{agent::ActiveAgent, catalog::AgentCatalog, one_shot::OneShotAgent},
    ai::{
        mock::{MockBehavior, MockProvider},
        provider::AiProvider,
        types::{
            Content, ContentBlock, Message, MessageRole, TokenUsage, ToolResultData, ToolUseData,
        },
    },
    chat::{
        ai,
        events::{ChatEvent, ChatMessage, EventSender},
        tools,
    },
    cmd::CommandResult,
    memory::{safe_conversation_slice, spawn_memory_manager, MemoryLog},
    settings::{ProviderConfig, Settings, SettingsManager},
    steering::SteeringDocuments,
    tools::{mcp::manager::McpManager, tasks::TaskList},
};

use anyhow::{bail, Result};
use aws_config::timeout::TimeoutConfig;
use chrono::Utc;
use dirs;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;
use tracing::{error, info};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimingState {
    Idle,
    WaitingForHuman,
    ProcessingAI,
    ExecutingTools,
}

#[derive(Clone, Debug, Default)]
pub struct TimingStat {
    pub waiting_for_human: Duration,
    pub ai_processing: Duration,
    pub tool_execution: Duration,
}

impl std::ops::AddAssign for TimingStat {
    fn add_assign(&mut self, rhs: Self) {
        self.waiting_for_human += rhs.waiting_for_human;
        self.ai_processing += rhs.ai_processing;
        self.tool_execution += rhs.tool_execution;
    }
}

#[derive(Debug, Clone)]
pub struct TimingStats {
    message: TimingStat,
    session: TimingStat,
    current_state: TimingState,
    state_start: Option<Instant>,
}

impl TimingStats {
    fn new() -> Self {
        Self {
            message: TimingStat::default(),
            session: TimingStat::default(),
            current_state: TimingState::Idle,
            state_start: Some(Instant::now()),
        }
    }

    pub fn session(&self) -> TimingStat {
        self.session.clone()
    }
}

pub struct ChatActorBuilder {
    workspace_roots: Vec<PathBuf>,
    root_dir: Option<PathBuf>,
    profile: Option<String>,
    provider_override: Option<Arc<dyn AiProvider>>,
    agent_name_override: Option<String>,
}

impl ChatActorBuilder {
    fn new() -> Self {
        Self {
            workspace_roots: Vec::new(),
            root_dir: None,
            profile: None,
            provider_override: None,
            agent_name_override: None,
        }
    }

    pub fn workspace_roots(mut self, roots: Vec<PathBuf>) -> Self {
        self.workspace_roots = roots;
        self
    }

    pub fn root_dir(mut self, dir: PathBuf) -> Self {
        self.root_dir = Some(dir);
        self
    }

    pub fn profile(mut self, name: Option<String>) -> Self {
        self.profile = name;
        self
    }

    pub fn provider(mut self, provider: Arc<dyn AiProvider>) -> Self {
        self.provider_override = Some(provider);
        self
    }

    pub fn agent_name(mut self, name: String) -> Self {
        self.agent_name_override = Some(name);
        self
    }

    pub fn build(self) -> Result<(ChatActor, mpsc::UnboundedReceiver<ChatEvent>)> {
        let (tx, rx) = mpsc::unbounded_channel();
        let (cancel_tx, cancel_rx) = mpsc::unbounded_channel();
        let (event_sender, event_rx) = EventSender::new();

        let workspace_roots = self.workspace_roots;
        let root_dir = self.root_dir.unwrap_or_else(|| {
            dirs::home_dir()
                .expect("Failed to get home directory")
                .join(".tycode")
        });
        let profile = self.profile;
        let provider_override = self.provider_override;
        let agent_name_override = self.agent_name_override;

        tokio::task::spawn_local(async move {
            let mut actor_state = ActorState::new(
                workspace_roots,
                event_sender,
                root_dir,
                profile,
                agent_name_override,
            )
            .await;

            if let Some(p) = provider_override {
                actor_state.provider = p;
            }

            actor_state
                .event_sender
                .send(ChatEvent::TaskUpdate(actor_state.task_list.clone()));

            run_actor(actor_state, rx, cancel_rx).await;
        });

        Ok((ChatActor { tx, cancel_tx }, event_rx))
    }
}

/// Defines the possible input messages to the `ChatActor`.
///
/// These messages derive serde for use across processes. Applications such as
/// VSCode spawn tycode-core in a sub-process and communicate to the actor over
/// stdin/stdout. In such applications, these messages are serialized to json
/// and sent over stdin.
#[derive(Serialize, Deserialize)]
pub enum ChatActorMessage {
    /// A user input to the conversation with the current AI agent
    UserInput(String),

    /// Changes the AI provider (i.e. Bedrock, OpenRouter, etc) that this actor
    /// is using. This is an in-memory only change that only lasts for the
    /// duration of this actor's lifetime.
    ChangeProvider(String),

    /// Sends the current settings (from SettingsManager) to the EventSender
    GetSettings,
    SaveSettings {
        settings: serde_json::Value,
    },

    /// Switches to a different settings profile
    SwitchProfile {
        profile_name: String,
    },

    /// Saves current settings as a new profile
    SaveProfile {
        profile_name: String,
    },

    /// Lists all available settings profiles
    ListProfiles,

    /// Requests all available sessions
    ListSessions,

    /// Requests to resume a specific session
    ResumeSession {
        session_id: String,
    },
}

/// The `ChatActor` implements the core (or backend) of tycode.
///
/// Tycode UI applications (such as the CLI and VSCode extension) do not
/// contain any  application logic; instead they are simple UI wrappers that
/// take input from the user, send it to the actor, and render events from the
/// actor back in to the UI.
///
/// The interface to the actor is essentially two channels: an input and output
/// channel. `ChatActorMessage` are sent to the input channel by UI
/// applications and `ChatEvents` are emitted by the actor to the output queue.
/// The ChatActor struct wraps the input channel and provides some convenience
/// methods and offers cancellation (technically there is a third cancellation
/// channel, however that is encapsulated by the ChatActor). Events from the
/// actor are received through a `mpsc::UnboundedReceiver<ChatEvent>` which is
/// returned when the actor is launched.
pub struct ChatActor {
    pub tx: mpsc::UnboundedSender<ChatActorMessage>,
    pub cancel_tx: mpsc::UnboundedSender<()>,
}

impl ChatActor {
    /// Create a builder for configuring and launching a ChatActor
    pub fn builder() -> ChatActorBuilder {
        ChatActorBuilder::new()
    }

    /// Launch the chat actor and return a handle to it
    pub fn launch(
        workspace_roots: Vec<PathBuf>,
        profile_name: Option<String>,
    ) -> (Self, mpsc::UnboundedReceiver<ChatEvent>) {
        Self::launch_with_provider(workspace_roots, profile_name, None)
    }

    /// Launch the chat actor with an optional pre-created provider (for testing)
    pub fn launch_with_provider(
        workspace_roots: Vec<PathBuf>,
        profile_name: Option<String>,
        provider_override: Option<Arc<dyn AiProvider>>,
    ) -> (Self, mpsc::UnboundedReceiver<ChatEvent>) {
        let mut builder = ChatActorBuilder::new()
            .workspace_roots(workspace_roots)
            .profile(profile_name);

        if let Some(provider) = provider_override {
            builder = builder.provider(provider);
        }

        builder.build().expect("Failed to build ChatActor")
    }

    pub fn send_message(&self, message: String) -> Result<()> {
        self.tx.send(ChatActorMessage::UserInput(message))?;
        Ok(())
    }

    pub fn change_provider(&self, provider: String) -> Result<()> {
        self.tx.send(ChatActorMessage::ChangeProvider(provider))?;
        Ok(())
    }

    pub fn get_settings(&self) -> Result<()> {
        self.tx.send(ChatActorMessage::GetSettings)?;
        Ok(())
    }

    pub fn save_settings(&self, settings: serde_json::Value) -> Result<()> {
        self.tx.send(ChatActorMessage::SaveSettings { settings })?;
        Ok(())
    }

    pub fn cancel(&self) -> Result<()> {
        self.cancel_tx.send(())?;
        Ok(())
    }
}

pub struct ActorState {
    pub event_sender: EventSender,
    pub provider: Arc<dyn AiProvider>,
    pub agent_stack: Vec<ActiveAgent>,
    pub workspace_roots: Vec<PathBuf>,
    pub settings: SettingsManager,
    pub steering: SteeringDocuments,
    pub tracked_files: BTreeSet<PathBuf>,
    pub last_command_outputs: Vec<CommandResult>,
    pub session_token_usage: TokenUsage,
    pub session_cost: f64,
    pub mcp_manager: Option<McpManager>,
    pub task_list: TaskList,
    pub profile_name: Option<String>,
    pub session_id: Option<String>,
    pub sessions_dir: PathBuf,
    pub timing_stats: TimingStats,
    pub memory_log: Option<Arc<Mutex<MemoryLog>>>,
}

impl ActorState {
    fn generate_session_id() -> String {
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
        let random: u32 = rand::thread_rng().gen_range(1000..9999);
        format!("{}_{}", timestamp, random)
    }

    pub fn save_session(&mut self) -> Result<()> {
        let Some(ref session_id) = self.session_id else {
            return Ok(());
        };

        let current_agent = self
            .agent_stack
            .first()
            .ok_or_else(|| anyhow::anyhow!("No active agent"))?;
        let messages = current_agent.conversation.clone();
        let tracked_files: Vec<PathBuf> = self.tracked_files.iter().cloned().collect();

        let mut session =
            crate::persistence::storage::load_session(session_id, Some(&self.sessions_dir))
                .unwrap_or_else(|_| {
                    crate::persistence::session::SessionData::new(
                        session_id.clone(),
                        Vec::new(),
                        TaskList::default(),
                        Vec::new(),
                    )
                });

        session.messages = messages;
        session.task_list = self.task_list.clone();
        session.tracked_files = tracked_files;
        session
            .events
            .extend_from_slice(self.event_sender.event_history());

        crate::persistence::storage::save_session(&session, Some(&self.sessions_dir))?;

        self.event_sender.clear_history();

        Ok(())
    }

    async fn new(
        workspace_roots: Vec<PathBuf>,
        event_sender: EventSender,
        root_dir: PathBuf,
        profile: Option<String>,
        agent_name_override: Option<String>,
    ) -> Self {
        let settings = SettingsManager::from_settings_dir(root_dir.clone(), profile.as_deref())
            .expect("Failed to create settings");
        let profile_name = profile;
        let sessions_dir = root_dir.join("sessions");

        let settings_snapshot = settings.settings();

        if settings_snapshot.active_provider().is_none() {
            event_sender.add_message(ChatMessage::error(
                "No AI provider is configured. Configure one in settings or with the command /provider add ..."
                    .to_string(),
            ));
        }

        // Check if cost preferences are set and send warning if not
        if settings_snapshot.model_quality.is_none() && settings_snapshot.agent_models.is_empty() {
            event_sender.add_message(ChatMessage::warning(
                "Warning: Cost preferences have not been set. Tycode will default to the highest quality model. Run /cost set <free|low|medium|high|unlimited> to explicitly set a preference.".to_string()
            ));
        }

        let provider = match create_default_provider(&settings).await {
            Ok(p) => p,
            Err(e) => {
                error!("Failed to initialize provider: {}", e);
                Arc::new(MockProvider::new(MockBehavior::AlwaysNonRetryableError))
            }
        };

        let mcp_manager = match McpManager::from_settings(&settings_snapshot).await {
            Ok(manager) => Some(manager),
            Err(e) => {
                error!("Failed to initialize MCP manager: {}", e);
                None
            }
        };

        // Memory is optional - initialization failures are acceptable and
        // the system will continue without memory functionality
        let memory_log = try_create_memory_log(&root_dir, settings_snapshot.memory.enabled);

        let default_task_list = TaskList::default();

        let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let steering = SteeringDocuments::new(
            workspace_roots.clone(),
            home_dir,
            settings_snapshot.communication_tone,
        );

        let agent_name = agent_name_override
            .as_deref()
            .unwrap_or_else(|| settings_snapshot.default_agent.as_str());
        let agent =
            AgentCatalog::create_agent(agent_name).unwrap_or_else(|| Box::new(OneShotAgent::new()));

        Self {
            event_sender,
            provider,
            agent_stack: vec![ActiveAgent::new(agent)],
            workspace_roots,
            settings,
            steering,
            tracked_files: BTreeSet::new(),
            last_command_outputs: Vec::new(),
            session_token_usage: TokenUsage::empty(),
            session_cost: 0.0,
            mcp_manager,
            task_list: default_task_list,
            profile_name,
            session_id: None,
            sessions_dir,
            timing_stats: TimingStats::new(),
            memory_log,
        }
    }

    pub fn clear_conversation(&mut self) {
        self.event_sender
            .send_replay(ChatEvent::ConversationCleared);
    }

    pub(crate) fn send_event_replay(&mut self, event: ChatEvent) {
        self.event_sender.send_replay(event);
    }

    pub fn transition_timing_state(&mut self, new_state: TimingState) {
        if let Some(start) = self.timing_stats.state_start {
            let elapsed = start.elapsed();
            match self.timing_stats.current_state {
                TimingState::WaitingForHuman => {
                    self.timing_stats.message.waiting_for_human += elapsed;
                }
                TimingState::ProcessingAI => {
                    self.timing_stats.message.ai_processing += elapsed;
                }
                TimingState::ExecutingTools => {
                    self.timing_stats.message.tool_execution += elapsed;
                }
                TimingState::Idle => {}
            }
        }

        if matches!(new_state, TimingState::WaitingForHuman) {
            let message = std::mem::replace(&mut self.timing_stats.message, TimingStat::default());
            self.timing_stats.session += message;
        }

        self.timing_stats.current_state = new_state;
        self.timing_stats.state_start = Some(Instant::now());
    }

    pub async fn reload_from_settings(&mut self) -> Result<(), anyhow::Error> {
        let settings_snapshot = self.settings.settings();

        let active_provider = settings_snapshot
            .active_provider
            .clone()
            .unwrap_or_else(|| self.provider.name().to_string());
        self.provider = create_provider(&self.settings, &active_provider).await?;

        let old_conversation = if let Some(old_agent) = self.agent_stack.first() {
            old_agent.conversation.clone()
        } else {
            Vec::new()
        };

        let default_agent = settings_snapshot.default_agent.clone();
        self.agent_stack.clear();

        let home_dir = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        self.steering = SteeringDocuments::new(
            self.workspace_roots.clone(),
            home_dir,
            settings_snapshot.communication_tone,
        );

        let new_agent_dyn = AgentCatalog::create_agent(&default_agent)
            .ok_or(anyhow::anyhow!("Failed to create default agent"))?;
        let mut new_root_agent = ActiveAgent::new(new_agent_dyn);
        new_root_agent.conversation = old_conversation;
        self.agent_stack.push(new_root_agent);

        self.profile_name = self.settings.current_profile().map(|s| s.to_string());

        Ok(())
    }
}

// Actor implementation as free functions
async fn run_actor(
    mut state: ActorState,
    mut rx: mpsc::UnboundedReceiver<ChatActorMessage>,
    mut cancel_rx: mpsc::UnboundedReceiver<()>,
) {
    info!("ChatActor started");

    loop {
        tokio::select! {
            result = process_message(&mut rx, &mut state) => {
                if let Err(e) = result {
                    error!(?e, "Error processing message");
                    state.event_sender.send_message(ChatMessage::error(format!("Error: {e:?}")));
                }
            }

            // Handle cancellation even when no message is being processed
            Some(_) = cancel_rx.recv() => {
                info!("Cancellation received while idle");
                handle_cancelled(&mut state);
            }
        }

        state.event_sender.send(ChatEvent::TimingUpdate {
            waiting_for_human: state.timing_stats.message.waiting_for_human,
            ai_processing: state.timing_stats.message.ai_processing,
            tool_execution: state.timing_stats.message.tool_execution,
        });
        state.event_sender.set_typing(false);
        state.transition_timing_state(TimingState::WaitingForHuman);
    }
}

async fn process_message(
    rx: &mut mpsc::UnboundedReceiver<ChatActorMessage>,
    state: &mut ActorState,
) -> Result<()> {
    let Some(message) = rx.recv().await else {
        bail!("request queue dropped")
    };

    state.transition_timing_state(TimingState::Idle);

    // At the start of each event processing, we set "typing" to true to
    // indicate to UI applications that we are thinking.
    state.event_sender.set_typing(true);

    match message {
        ChatActorMessage::UserInput(input) => handle_user_input(state, input).await,
        ChatActorMessage::ChangeProvider(provider) => handle_provider_change(state, provider).await,
        ChatActorMessage::GetSettings => {
            let settings = state.settings.settings();
            let settings_json = serde_json::to_value(settings)
                .map_err(|e| anyhow::anyhow!("Failed to serialize settings: {}", e))?;
            state.event_sender.send(ChatEvent::Settings(settings_json));
            Ok(())
        }
        ChatActorMessage::SaveSettings { settings } => {
            let new_settings: Settings = serde_json::from_value(settings)
                .map_err(|e| anyhow::anyhow!("Failed to deserialize settings: {}", e))?;
            state.settings.update_setting(|s| *s = new_settings);
            state.settings.save()?;
            Ok(())
        }
        ChatActorMessage::SwitchProfile { profile_name } => {
            state.settings.switch_profile(&profile_name)?;
            state.reload_from_settings().await?;
            let settings = state.settings.settings();
            let settings_json = serde_json::to_value(settings)
                .map_err(|e| anyhow::anyhow!("Failed to serialize settings: {}", e))?;
            state.event_sender.send(ChatEvent::Settings(settings_json));
            state.event_sender.send_message(ChatMessage::system(format!(
                "Switched to profile: {}",
                profile_name
            )));
            Ok(())
        }
        ChatActorMessage::SaveProfile { profile_name } => {
            state.settings.save_as_profile(&profile_name)?;
            state.event_sender.send_message(ChatMessage::system(format!(
                "Settings saved as profile: {}",
                profile_name
            )));
            Ok(())
        }
        ChatActorMessage::ListProfiles => {
            let profiles = state.settings.list_profiles()?;
            state
                .event_sender
                .send(ChatEvent::ProfilesList { profiles });
            Ok(())
        }
        ChatActorMessage::ListSessions => {
            let sessions = crate::persistence::storage::list_session_metadata(&state.sessions_dir)?;
            state
                .event_sender
                .send(ChatEvent::SessionsList { sessions });
            Ok(())
        }
        ChatActorMessage::ResumeSession { session_id } => resume_session(state, &session_id).await,
    }
}

/// Extract any pending tool uses from the last assistant message
fn get_pending_tool_uses(state: &ActorState) -> Vec<ToolUseData> {
    let current = tools::current_agent(state);
    if let Some(last_message) = current.conversation.last() {
        if last_message.role == MessageRole::Assistant {
            return last_message
                .content
                .tool_uses()
                .into_iter()
                .cloned()
                .collect();
        }
    }
    Vec::new()
}

/// Create error results for cancelled tool calls
fn create_cancellation_error_results(
    tool_uses: Vec<ToolUseData>,
    state: &mut ActorState,
) -> Vec<ContentBlock> {
    tool_uses
        .into_iter()
        .map(|tool_use| {
            let result = ToolResultData {
                tool_use_id: tool_use.id.clone(),
                content: "Tool execution was cancelled by user".to_string(),
                is_error: true,
            };

            // Emit event for UI
            state.event_sender.send(ChatEvent::ToolExecutionCompleted {
                tool_call_id: tool_use.id.clone(),
                tool_name: tool_use.name.clone(),
                tool_result: crate::chat::events::ToolExecutionResult::Error {
                    short_message: "Cancelled".to_string(),
                    detailed_message: "Tool execution was cancelled by user".to_string(),
                },
                success: false,
                error: Some("Cancelled by user".to_string()),
            });

            ContentBlock::ToolResult(result)
        })
        .collect()
}

fn handle_cancelled(state: &mut ActorState) {
    // Check if there are any pending tool uses that need error results
    let pending_tool_uses = get_pending_tool_uses(state);

    if !pending_tool_uses.is_empty() {
        info!(
            "Cancellation with {} pending tool calls - generating error results",
            pending_tool_uses.len()
        );

        // Create error results for all pending tool calls
        let error_results = create_cancellation_error_results(pending_tool_uses, state);

        // Add these error results to the conversation as a User message
        tools::current_agent_mut(state).conversation.push(Message {
            role: MessageRole::User,
            content: Content::from(error_results),
        });
    }

    state.event_sender.send(ChatEvent::OperationCancelled {
        message: "Operation cancelled by user".to_string(),
    });
}

async fn handle_user_input(state: &mut ActorState, input: String) -> Result<()> {
    if input.trim().is_empty() {
        return Ok(());
    }

    // Generate session ID on first user message
    if state.session_id.is_none() {
        state.session_id = Some(ActorState::generate_session_id());
    }

    state
        .event_sender
        .send_message(ChatMessage::user(input.clone()));

    if let Some(command) = input.strip_prefix('/') {
        if crate::chat::commands::is_known_command(command) {
            let messages = crate::chat::commands::process_command(state, command).await;

            for message in messages {
                state.event_sender.send_message(message);
            }
            return Ok(());
        }
    }

    tools::current_agent_mut(state).conversation.push(Message {
        role: MessageRole::User,
        content: Content::text_only(input.clone()),
    });

    if let Some(ref memory_log) = state.memory_log {
        let settings_snapshot = state.settings.settings();
        let context_message_count = settings_snapshot.memory.context_message_count;

        let current_agent = tools::current_agent(state);
        let conversation =
            safe_conversation_slice(&current_agent.conversation, context_message_count);

        spawn_memory_manager(
            state.provider.clone(),
            memory_log.clone(),
            state.settings.clone(),
            conversation,
        );
    }

    ai::send_ai_request(state).await?;

    if let Err(e) = state.save_session() {
        tracing::warn!("Failed to auto-save session: {}", e);
    }

    Ok(())
}

async fn handle_provider_change(state: &mut ActorState, provider_name: String) -> Result<()> {
    info!("Changing provider to: {}", provider_name);
    state.provider = create_provider(&state.settings, &provider_name).await?;

    state.event_sender.send_message(ChatMessage::system(format!(
        "Switched to provider: {provider_name}"
    )));

    Ok(())
}

/// Initializes the provider with the given name if it exists in settings, else
/// raises an error.
pub async fn create_provider(
    settings: &SettingsManager,
    provider: &str,
) -> Result<Arc<dyn AiProvider>> {
    let config = settings.settings();
    let Some(provider_config) = config.providers.get(provider) else {
        bail!("No active provider configured in settings")
    };

    match provider_config {
        ProviderConfig::Bedrock { profile, region } => {
            use crate::ai::bedrock::BedrockProvider;
            use aws_config::retry::RetryConfig;
            use aws_config::Region;

            if region.is_empty() {
                bail!("AWS region is empty")
            };

            let aws_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
                .profile_name(profile)
                .region(Region::new(region.to_string()))
                .retry_config(RetryConfig::disabled())
                .timeout_config(
                    // Tuned for Alaska airline's Wifi
                    TimeoutConfig::builder()
                        .connect_timeout(Duration::from_secs(60))
                        .operation_attempt_timeout(Duration::from_secs(300))
                        .read_timeout(Duration::from_secs(300))
                        .build(),
                )
                .load()
                .await;

            let client = aws_sdk_bedrockruntime::Client::new(&aws_config);
            Ok(Arc::new(BedrockProvider::new(client)))
        }
        ProviderConfig::OpenRouter { api_key } => {
            use crate::ai::openrouter::OpenRouterProvider;
            Ok(Arc::new(OpenRouterProvider::new(api_key.clone())))
        }
        ProviderConfig::ClaudeCode {
            command,
            extra_args,
            env,
        } => {
            use crate::ai::claude_code::ClaudeCodeProvider;

            let command_path = if command.trim().is_empty() {
                PathBuf::from("claude")
            } else {
                PathBuf::from(command.as_str())
            };

            Ok(Arc::new(ClaudeCodeProvider::new(
                command_path,
                extra_args.clone(),
                env.clone(),
            )))
        }
        ProviderConfig::Mock { behavior } => Ok(Arc::new(MockProvider::new(behavior.clone()))),
    }
}

fn try_create_memory_log(root_dir: &PathBuf, enabled: bool) -> Option<Arc<Mutex<MemoryLog>>> {
    if !enabled {
        return None;
    }

    let memory_path = root_dir.join("memory").join("memories_log.json");
    let log = match MemoryLog::load(&memory_path) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(?e, ?memory_path, "Failed to load memory log");
            return None;
        }
    };

    Some(Arc::new(Mutex::new(log)))
}

/// Creates the provider marked as default from the current settings. Note: the
/// "active" provider in the settings is just the default that is used if the
/// user hasn't selected an overriding provider (using the ChangeProvider event)
async fn create_default_provider(settings: &SettingsManager) -> Result<Arc<dyn AiProvider>> {
    let default = &settings.settings().active_provider.unwrap_or_default();
    create_provider(settings, default).await
}

pub async fn resume_session(state: &mut ActorState, session_id: &str) -> Result<()> {
    let session_data =
        crate::persistence::storage::load_session(session_id, Some(&state.sessions_dir))?;

    let current_agent_mut = tools::current_agent_mut(state);
    current_agent_mut.conversation = session_data.messages;

    state.task_list = session_data.task_list.clone();

    state.tracked_files.clear();
    for path in session_data.tracked_files {
        state.tracked_files.insert(path);
    }

    state.session_id = Some(session_data.id.clone());

    state.clear_conversation();

    for event in session_data.events {
        state.send_event_replay(event);
    }

    Ok(())
}
