use std::{env, path::PathBuf};
use tokio::task::LocalSet;
use tracing::info;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use tycode_subprocess::run_subprocess;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    setup_tracing()?;

    let args: Vec<String> = env::args().collect();
    let mut workspace_roots: Vec<String> = vec![];
    let mut settings_path: Option<String> = None;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--workspace-roots" => {
                i += 1;
                if i < args.len() {
                    workspace_roots = serde_json::from_str(&args[i])?;
                }
            }
            "--settings-path" => {
                i += 1;
                if i < args.len() {
                    settings_path = Some(args[i].clone());
                }
            }
            _ => {}
        }
        i += 1;
    }

    let local = LocalSet::new();
    local
        .run_until(run_subprocess(workspace_roots, settings_path))
        .await?;
    Ok(())
}

fn setup_tracing() -> anyhow::Result<()> {
    use std::fs;
    use tracing_subscriber::fmt;

    // Create trace directory in user's home
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
    let trace_dir = PathBuf::from(home).join(".tycode").join("trace");
    fs::create_dir_all(&trace_dir)?;

    let log_file = trace_dir.join("tycode.log");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file)?;

    // Setup tracing subscriber with file output
    tracing_subscriber::registry()
        .with(
            fmt::layer()
                .with_writer(file)
                .with_ansi(false)
                .with_target(true)
                .with_thread_ids(true)
                .with_thread_names(true)
                .with_file(true)
                .with_line_number(true),
        )
        .with(EnvFilter::new("info"))
        .init();

    info!("Tracing initialized to {:?}", log_file);
    Ok(())
}
