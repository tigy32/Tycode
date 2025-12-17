use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::mpsc;
use tracing_subscriber;
use tycode_core::{
    ai::mock::{MockBehavior, MockProvider},
    chat::{actor::ChatActor, events::ChatEvent},
    settings::{manager::SettingsManager, Settings},
};

pub struct Fixture {
    pub actor: ChatActor,
    pub event_rx: mpsc::UnboundedReceiver<ChatEvent>,
    pub workspace_dir: TempDir,
    pub sessions_dir: PathBuf,
    mock_provider: MockProvider,
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
    pub fn with_memory_enabled() -> Self {
        Self::with_agent_behavior_and_memory("one_shot", MockBehavior::Success, true)
    }

    #[allow(dead_code)]
    pub fn with_agent_and_behavior(agent_name: &str, behavior: MockBehavior) -> Self {
        Self::with_agent_behavior_and_memory(agent_name, behavior, false)
    }

    #[allow(dead_code)]
    pub fn with_agent_behavior_and_memory(
        agent_name: &str,
        behavior: MockBehavior,
        memory_enabled: bool,
    ) -> Self {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();

        let workspace_dir = TempDir::new().unwrap();
        let workspace_path = workspace_dir.path().to_path_buf();

        let tycode_dir = workspace_path.join(".tycode");
        std::fs::create_dir_all(&tycode_dir).unwrap();
        let sessions_dir = tycode_dir.join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();

        std::fs::write(workspace_path.join("example.txt"), "test content").unwrap();

        let settings_path = tycode_dir.join("settings.toml");
        let settings_manager = SettingsManager::from_path(settings_path.clone()).unwrap();

        let mut default_settings = Settings::default();
        default_settings.memory.enabled = memory_enabled;
        // Configure a mock provider in the settings so profile save/switch operations work
        default_settings.add_provider(
            "mock".to_string(),
            tycode_core::settings::ProviderConfig::Mock {
                behavior: behavior.clone(),
            },
        );
        default_settings.active_provider = Some("mock".to_string());
        default_settings.default_agent = agent_name.to_string();
        settings_manager.save_settings(default_settings).unwrap();

        // Share provider state across fixtures to enable runtime behavior modification
        let mock_provider = MockProvider::new(behavior);

        let (actor, event_rx) = ChatActor::builder()
            .workspace_roots(vec![workspace_path])
            .root_dir(tycode_dir)
            .provider(Arc::new(mock_provider.clone()))
            .build()
            .unwrap();

        Fixture {
            actor,
            event_rx,
            workspace_dir,
            sessions_dir,
            mock_provider,
        }
    }

    #[allow(dead_code)]
    pub fn set_mock_behavior(&self, behavior: MockBehavior) {
        self.mock_provider.set_behavior(behavior);
    }

    #[allow(dead_code)]
    pub fn get_last_ai_request(&self) -> Option<tycode_core::ai::types::ConversationRequest> {
        self.mock_provider.get_last_captured_request()
    }

    #[allow(dead_code)]
    pub fn get_all_ai_requests(&self) -> Vec<tycode_core::ai::types::ConversationRequest> {
        self.mock_provider.get_captured_requests()
    }

    #[allow(dead_code)]
    pub fn clear_captured_requests(&self) {
        self.mock_provider.clear_captured_requests();
    }

    #[allow(dead_code)]
    pub fn workspace_path(&self) -> PathBuf {
        self.workspace_dir.path().to_path_buf()
    }

    #[allow(dead_code)]
    pub fn sessions_dir(&self) -> PathBuf {
        self.sessions_dir.clone()
    }

    #[allow(dead_code)]
    pub fn memory_dir(&self) -> PathBuf {
        self.workspace_dir.path().join(".tycode/memory")
    }

    pub fn send_message(&mut self, message: impl Into<String>) {
        self.actor.send_message(message.into()).unwrap();
    }

    #[allow(dead_code)]
    pub async fn update_settings<F>(&mut self, update_fn: F)
    where
        F: FnOnce(&mut Settings),
    {
        self.actor.get_settings().unwrap();

        let mut settings_json = None;
        while let Some(event) = self.event_rx.recv().await {
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
        self.actor.save_settings(updated_json).unwrap();

        while let Some(event) = self.event_rx.recv().await {
            if matches!(event, ChatEvent::TypingStatusChanged(false)) {
                break;
            }
        }
    }

    /// Essential for end-to-end testing where we validate the full actor response cycle.
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

        if all_events.is_empty() {
            panic!("No events received");
        }

        assert!(
            all_events
                .iter()
                .any(|e| matches!(e, ChatEvent::TypingStatusChanged(true))),
            "Expected to receive typing started event"
        );

        all_events
            .into_iter()
            .filter(|e| !matches!(e, ChatEvent::TypingStatusChanged(_)))
            .collect()
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

#[allow(dead_code)]
pub fn run_with_memory<F, Fut>(test_fn: F)
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
        let fixture = Fixture::with_memory_enabled();
        let test_future = test_fn(fixture);
        timeout(Duration::from_secs(30), test_future)
            .await
            .expect("Test timed out after 30 seconds");
    }));
}
