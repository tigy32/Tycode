use std::ops::{Deref, DerefMut};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tracing_subscriber;
use tycode_core::{
    ai::{mock::MockProvider, ConversationRequest},
    chat::{actor::ChatActorBuilder, events::ChatEvent},
    settings::{manager::SettingsManager, Settings},
    ChatActor,
};

pub use tycode_core::ai::mock::MockBehavior;

/// Workspace owns the TempDir and shared resources.
/// Persists across multiple session spawns.
pub struct Workspace {
    dir: TempDir,
    tycode_dir: PathBuf,
    sessions_dir: PathBuf,
}

impl Workspace {
    #[allow(dead_code)]
    pub fn new() -> Self {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();

        let dir = TempDir::new().unwrap();
        let workspace_path = dir.path().to_path_buf();

        let tycode_dir = workspace_path.join(".tycode");
        std::fs::create_dir_all(&tycode_dir).unwrap();
        let sessions_dir = tycode_dir.join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();

        std::fs::write(workspace_path.join("example.txt"), "test content").unwrap();

        Self {
            dir,
            tycode_dir,
            sessions_dir,
        }
    }

    /// Spawn a new session (ChatActor) using this workspace.
    #[allow(dead_code)]
    pub fn spawn_session(&self, agent_name: &str, behavior: MockBehavior) -> Session {
        let workspace_path = self.dir.path().to_path_buf();

        let settings_path = self.tycode_dir.join("settings.toml");
        let settings_manager = SettingsManager::from_path(settings_path.clone()).unwrap();

        let mut default_settings = settings_manager.settings();
        default_settings.add_provider(
            "mock".to_string(),
            tycode_core::settings::ProviderConfig::Mock {
                behavior: behavior.clone(),
            },
        );
        default_settings.active_provider = Some("mock".to_string());
        default_settings.default_agent = agent_name.to_string();
        settings_manager.save_settings(default_settings).unwrap();

        let mock_provider = MockProvider::new(behavior);

        let (actor, event_rx) =
            ChatActorBuilder::tycode(vec![workspace_path], Some(self.tycode_dir.clone()), None)
                .unwrap()
                .provider(Arc::new(mock_provider.clone()))
                .build()
                .unwrap();

        Session {
            actor,
            event_rx,
            mock_provider,
        }
    }

    #[allow(dead_code)]
    pub fn tycode_dir(&self) -> PathBuf {
        self.tycode_dir.clone()
    }

    #[allow(dead_code)]
    pub fn workspace_path(&self) -> PathBuf {
        self.dir.path().to_path_buf()
    }

    #[allow(dead_code)]
    pub fn sessions_dir(&self) -> PathBuf {
        self.sessions_dir.clone()
    }
}

/// Lightweight actor handle. Does not own the workspace.
pub struct Session {
    pub actor: ChatActor,
    pub event_rx: mpsc::UnboundedReceiver<ChatEvent>,
    mock_provider: MockProvider,
}

impl Session {
    #[allow(dead_code)]
    pub fn set_mock_behavior(&self, behavior: MockBehavior) {
        self.mock_provider.set_behavior(behavior);
    }

    #[allow(dead_code)]
    pub fn get_last_ai_request(&self) -> Option<ConversationRequest> {
        self.mock_provider.get_last_captured_request()
    }

    #[allow(dead_code)]
    pub fn get_all_ai_requests(&self) -> Vec<ConversationRequest> {
        self.mock_provider.get_captured_requests()
    }

    #[allow(dead_code)]
    pub fn clear_captured_requests(&self) {
        self.mock_provider.clear_captured_requests();
    }

    #[allow(dead_code)]
    pub fn send_message(&mut self, message: impl Into<String>) {
        self.actor.send_message(message.into()).unwrap();
    }

    #[allow(dead_code)]
    pub async fn step(&mut self, message: impl Into<String>) -> Vec<ChatEvent> {
        self.send_message(message);

        let mut all_events = Vec::new();
        let mut typing_stopped = false;

        while !typing_stopped {
            match self.event_rx.recv().await {
                Some(event) => {
                    if matches!(event, ChatEvent::TypingStatusChanged(false)) {
                        typing_stopped = true;
                    }
                    all_events.push(event);
                }
                None => break,
            }
        }

        all_events
            .into_iter()
            .filter(|e| !matches!(e, ChatEvent::TypingStatusChanged(_)))
            .collect()
    }
}

/// Convenience wrapper for single-actor tests.
/// Owns both Workspace and Session, derefs to Session.
pub struct Fixture {
    workspace: Workspace,
    session: Session,
}

impl Deref for Fixture {
    type Target = Session;
    fn deref(&self) -> &Self::Target {
        &self.session
    }
}

impl DerefMut for Fixture {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.session
    }
}

impl Fixture {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::with_agent_and_behavior("one_shot", MockBehavior::Success)
    }

    #[allow(dead_code)]
    pub fn with_agent(agent_name: &str) -> Self {
        Self::with_agent_and_behavior(agent_name, MockBehavior::Success)
    }

    #[allow(dead_code)]
    pub fn with_mock_behavior(behavior: MockBehavior) -> Self {
        Self::with_agent_and_behavior("one_shot", behavior)
    }

    #[allow(dead_code)]
    pub fn with_agent_and_behavior(agent_name: &str, behavior: MockBehavior) -> Self {
        let workspace = Workspace::new();
        let session = workspace.spawn_session(agent_name, behavior);
        Fixture { workspace, session }
    }

    #[allow(dead_code)]
    pub fn workspace_path(&self) -> PathBuf {
        self.workspace.workspace_path()
    }

    #[allow(dead_code)]
    pub fn sessions_dir(&self) -> PathBuf {
        self.workspace.sessions_dir()
    }

    #[allow(dead_code)]
    pub async fn update_settings<F>(&mut self, update_fn: F)
    where
        F: FnOnce(&mut Settings),
    {
        self.session.actor.get_settings().unwrap();

        let mut settings_json = None;
        while let Some(event) = self.session.event_rx.recv().await {
            match event {
                ChatEvent::Settings(s) => {
                    settings_json = Some(s);
                }
                ChatEvent::TypingStatusChanged(false) => {
                    break;
                }
                _ => {}
            }
        }

        let settings_json = settings_json.expect("Failed to get settings");
        let mut settings: Settings =
            serde_json::from_value(settings_json).expect("Failed to deserialize settings");

        update_fn(&mut settings);

        let updated_json = serde_json::to_value(&settings).expect("Failed to serialize settings");
        self.session
            .actor
            .save_settings(updated_json, true)
            .unwrap();

        while let Some(event) = self.session.event_rx.recv().await {
            if matches!(event, ChatEvent::TypingStatusChanged(false)) {
                break;
            }
        }
    }
}

#[allow(dead_code)]
pub fn run<F, Fut>(test_fn: F)
where
    F: FnOnce(Fixture) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    run_with_agent("one_shot", test_fn)
}

pub fn run_with_agent<F, Fut>(agent_name: &str, test_fn: F)
where
    F: FnOnce(Fixture) -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    use tokio::time::{timeout, Duration};

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");

    let local = tokio::task::LocalSet::new();

    runtime.block_on(local.run_until(async {
        let fixture = Fixture::with_agent(agent_name);
        let test_future = test_fn(fixture);
        timeout(Duration::from_secs(30), test_future)
            .await
            .expect("Test timed out after 30 seconds");
    }));
}
