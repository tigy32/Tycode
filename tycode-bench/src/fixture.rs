use anyhow::Result;
use fs_extra::dir::{copy, CopyOptions};
use std::env;
use std::path::PathBuf;
use tempfile::TempDir;
use tokio::sync::mpsc::UnboundedReceiver;
use tycode_core::chat::{ChatActor, ChatEvent};
use tycode_core::settings::manager::SettingsManager;
use tycode_core::settings::Settings;

pub struct MessageCapturingReceiver {
    inner: UnboundedReceiver<ChatEvent>,
    messages: Vec<ChatEvent>,
}

impl MessageCapturingReceiver {
    pub fn new(rx: UnboundedReceiver<ChatEvent>) -> Self {
        MessageCapturingReceiver {
            inner: rx,
            messages: vec![],
        }
    }

    // Receives messages, capturing them for debugging.
    pub async fn recv(&mut self) -> Option<ChatEvent> {
        let msg = self.inner.recv().await;
        if let Some(ref m) = msg {
            self.messages.push(m.clone());
        }
        msg
    }

    pub fn captured(&self) -> &[ChatEvent] {
        &self.messages
    }
}

pub struct TestResult {
    pub success: bool,
    pub reason: String,
    pub actor: ChatActor,
    pub event_rx: MessageCapturingReceiver,
}

#[async_trait::async_trait]
pub trait TestCase {
    fn directory(&self) -> String;

    async fn execute(
        self,
        working_dir: PathBuf,
        actor: ChatActor,
        event_rx: MessageCapturingReceiver,
    ) -> TestResult;
}

pub async fn run_bench(settings: Settings, test_case: impl TestCase + Send) -> Result<TestResult> {
    // To isolate each test run and prevent interference between tests,
    // use a tempdir for each execution, copying the test's directory from scenarios/ to it,
    // and creating isolated settings within the tempdir to avoid modifying global user settings.
    let temp_dir = TempDir::new()?;
    let temp_path = temp_dir.path();

    // Compute the scenario directory from the project root
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tycode-bench");
    let tycode_root = manifest_dir.parent().unwrap_or(&manifest_dir);
    let scenarios_dir = tycode_root.join("scenarios");
    let src = scenarios_dir.join(test_case.directory());

    copy(&src, temp_path, &CopyOptions::new())?;

    let temp_test_dir = temp_path.join(src.file_name().unwrap());
    env::set_current_dir(&temp_test_dir)?;

    let settings_path = temp_test_dir.join(".tycode/settings.toml");
    let settings_manager = SettingsManager::from_path(settings_path.clone())?;
    settings_manager.save_settings(settings)?;

    let workspace_roots = vec![temp_test_dir.clone()];
    let (actor, event_rx_inner) = ChatActor::builder()
        .workspace_roots(workspace_roots)
        .settings_path(settings_path)
        .build()?;
    let event_rx = MessageCapturingReceiver::new(event_rx_inner);

    let result = test_case
        .execute(temp_test_dir.clone(), actor, event_rx)
        .await;

    println!(
        "Test completed. Success: {}, Reason: {}",
        result.success, result.reason
    );

    drop(temp_dir);
    Ok(result)
}
