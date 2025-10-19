use crate::{
    agents::{agent::ActiveAgent, one_shot::OneShotAgent},
    ai::{
        mock::{MockBehavior, MockProvider},
        provider::AiProvider,
        types::{Content, Message, MessageRole, TokenUsage},
    },
    chat::{
        ai,
        events::{ChatEvent, ChatMessage, EventSender},
        tools,
    },
    settings::{ProviderConfig, Settings, SettingsManager},
    tools::mcp::manager::McpManager,
    tools::tasks::TaskList,
};

use anyhow::{bail, Result};
use aws_config::timeout::TimeoutConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{error, info};

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
    /// Launch the chat actor and return a handle to it
    pub fn launch(
        workspace_roots: Vec<PathBuf>,
        settings_manager: SettingsManager,
    ) -> (Self, mpsc::UnboundedReceiver<ChatEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let (cancel_tx, cancel_rx) = mpsc::unbounded_channel();
        let (event_sender, event_rx) = EventSender::new();

        tokio::task::spawn_local(async move {
            let settings = settings_manager.settings();
            if settings.active_provider().is_none() {
                event_sender.add_message(ChatMessage::error(
                    "No AI provider is configured. Configure one in settings or with the command /provider add ..."
                        .to_string(),
                ));
            }

            // Check if cost preferences are set and send warning if not
            if settings.model_quality.is_none() && settings.agent_models.is_empty() {
                event_sender.add_message(ChatMessage::system(
                    "Warning: Cost preferences have not been set. Tycode will default to the highest quality model. Run /cost set <free|low|medium|high|unlimited> to explicitly set a preference.".to_string()
                ));
            }

            let provider = match create_default_provider(&settings_manager).await {
                Ok(p) => p,
                Err(e) => {
                    error!("Failed to initialize provider: {}", e);
                    Box::new(MockProvider::new(MockBehavior::AlwaysNonRetryableError))
                }
            };

            let mcp_manager = match McpManager::from_settings(&settings).await {
                Ok(manager) => Some(manager),
                Err(e) => {
                    error!("Failed to initialize MCP manager: {}", e);
                    None
                }
            };

            let actor_state = ActorState {
                event_sender,
                provider,
                agent_stack: vec![ActiveAgent::new(Box::new(OneShotAgent))],
                workspace_roots,
                settings: settings_manager,
                tracked_files: HashSet::new(),
                session_token_usage: TokenUsage::empty(),
                session_cost: 0.0,
                mcp_manager,
                task_list: None,
            };

            run_actor(actor_state, rx, cancel_rx).await;
        });

        (ChatActor { tx, cancel_tx }, event_rx)
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
    pub provider: Box<dyn AiProvider>,
    pub agent_stack: Vec<ActiveAgent>,
    pub workspace_roots: Vec<PathBuf>,
    pub settings: SettingsManager,
    pub tracked_files: HashSet<PathBuf>,
    pub session_token_usage: TokenUsage,
    pub session_cost: f64,
    pub mcp_manager: Option<McpManager>,
    pub task_list: Option<TaskList>,
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
                    state.event_sender.add_message(ChatMessage::error(format!("Error: {e:?}")));
                }
            }

            // Handle cancellation even when no message is being processed
            Some(_) = cancel_rx.recv() => {
                info!("Cancellation received while idle");
                handle_cancelled(&mut state);
            }
        }

        state.event_sender.set_typing(false);
    }
}

async fn process_message(
    rx: &mut mpsc::UnboundedReceiver<ChatActorMessage>,
    state: &mut ActorState,
) -> Result<()> {
    let Some(message) = rx.recv().await else {
        bail!("request queue dropped")
    };

    // At the start of each event processing, we set "typing" to true to
    // indicate to UI applications that we are thinking.
    state.event_sender.set_typing(true);

    match message {
        ChatActorMessage::UserInput(input) => handle_user_input(state, input).await,
        ChatActorMessage::ChangeProvider(provider) => handle_provider_change(state, provider).await,
        ChatActorMessage::GetSettings => {
            let settings = state.settings.settings();
            let settings_json = serde_json::to_value(settings)
                .map_err(|e| anyhow::anyhow!("Failed to serialize settings: {}", e));
            state
                .event_sender
                .event_tx
                .send(ChatEvent::Settings(settings_json?))?;
            Ok(())
        }
        ChatActorMessage::SaveSettings { settings } => {
            let new_settings: Settings = serde_json::from_value(settings)
                .map_err(|e| anyhow::anyhow!("Failed to deserialize settings: {}", e))?;
            state.settings.update_setting(|s| *s = new_settings);
            state.settings.save()?;
            Ok(())
        }
    }
}

fn handle_cancelled(state: &mut ActorState) {
    // Send cancellation event
    let _ = state
        .event_sender
        .event_tx
        .send(ChatEvent::OperationCancelled {
            message: "Operation cancelled by user".to_string(),
        });
}

async fn handle_user_input(state: &mut ActorState, input: String) -> Result<()> {
    if input.trim().is_empty() {
        return Ok(());
    }

    state
        .event_sender
        .add_message(ChatMessage::user(input.clone()));

    if let Some(command) = input.strip_prefix('/') {
        let messages = crate::chat::commands::process_command(state, command).await;

        for message in messages {
            state.event_sender.add_message(message);
        }
        return Ok(());
    }

    tools::current_agent_mut(state).conversation.push(Message {
        role: MessageRole::User,
        content: Content::text_only(input),
    });

    ai::send_ai_request(state).await
}

async fn handle_provider_change(state: &mut ActorState, provider_name: String) -> Result<()> {
    info!("Changing provider to: {}", provider_name);
    state.provider = create_provider(&state.settings, &provider_name).await?;

    state.event_sender.add_message(ChatMessage::system(format!(
        "Switched to provider: {provider_name}"
    )));

    Ok(())
}

/// Initializes the provider with the given name if it exists in settings, else
/// raises an error.
pub async fn create_provider(
    settings: &SettingsManager,
    provider: &str,
) -> Result<Box<dyn AiProvider>> {
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
            Ok(Box::new(BedrockProvider::new(client)))
        }
        ProviderConfig::OpenRouter { api_key } => {
            use crate::ai::openrouter::OpenRouterProvider;
            Ok(Box::new(OpenRouterProvider::new(api_key.clone())))
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

            Ok(Box::new(ClaudeCodeProvider::new(
                command_path,
                extra_args.clone(),
                env.clone(),
            )))
        }
        ProviderConfig::Mock { behavior } => {
            use crate::ai::mock::{MockBehavior, MockProvider};

            let mock_behavior = match behavior {
                crate::settings::config::MockBehaviorConfig::Success => MockBehavior::Success,
                crate::settings::config::MockBehaviorConfig::RetryThenSuccess {
                    errors_before_success,
                } => MockBehavior::RetryableErrorThenSuccess {
                    remaining_errors: *errors_before_success,
                },
                crate::settings::config::MockBehaviorConfig::AlwaysRetryError => {
                    MockBehavior::AlwaysRetryableError
                }
                crate::settings::config::MockBehaviorConfig::AlwaysError => {
                    MockBehavior::AlwaysNonRetryableError
                }
                crate::settings::config::MockBehaviorConfig::ToolUse {
                    tool_name,
                    tool_arguments,
                } => MockBehavior::ToolUse {
                    tool_name: tool_name.clone(),
                    tool_arguments: tool_arguments.clone(),
                },
            };

            Ok(Box::new(MockProvider::new(mock_behavior)))
        }
    }
}

/// Creates the provider marked as default from the current settings. Note: the
/// "active" provider in the settings is just the default that is used if the
/// user hasn't selected an overriding provider (using the ChangeProvider event)
async fn create_default_provider(settings: &SettingsManager) -> Result<Box<dyn AiProvider>> {
    let default = &settings.settings().active_provider.unwrap_or_default();
    create_provider(settings, default).await
}
