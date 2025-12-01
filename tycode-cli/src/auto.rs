use anyhow::{Context, Result};
use std::path::PathBuf;
use terminal_size::{terminal_size, Width};
use tycode_core::{
    chat::ChatActor,
    formatter::{CompactFormatter, EventFormatter},
};

use crate::auto_driver::{drive_auto_conversation, AutoDriverConfig};

pub async fn run_auto(
    task: String,
    workspace_roots: Vec<PathBuf>,
    profile: Option<String>,
) -> Result<()> {
    let terminal_width = terminal_size()
        .map(|(Width(w), _)| w as usize)
        .unwrap_or(80);
    let mut formatter = CompactFormatter::new(terminal_width);

    formatter.print_system("Starting auto mode...");

    let settings_path = dirs::home_dir()
        .context("Failed to get home directory")?
        .join(".tycode")
        .join("settings.toml");

    let initial_agent = "coordinator".to_string();

    let (mut actor, mut event_rx) = ChatActor::builder()
        .workspace_roots(workspace_roots)
        .settings_path(settings_path)
        .profile_name(profile)
        .agent_name(initial_agent.clone())
        .build()?;

    actor.send_message(task)?;

    let config = AutoDriverConfig {
        initial_agent,
        max_messages: 500,
    };

    let result = drive_auto_conversation(&mut actor, &mut event_rx, &mut formatter, config).await;

    match result {
        Ok(summary) => {
            formatter.print_system(&format!("Task completed: {}", summary));
            Ok(())
        }
        Err(e) => {
            formatter.print_error(&format!("Auto mode failed: {}", e));
            Err(e)
        }
    }
}
