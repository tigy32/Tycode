use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use tracing::info;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

mod commands;
mod interactive_app;
mod state;

use crate::interactive_app::InteractiveApp;

#[derive(Parser, Debug)]
#[command(name = "tycode-cli")]
#[command(about = "TyCode CLI - Native terminal chat interface")]
struct Args {
    /// Workspace roots (for multi-root workspaces)
    #[arg(long, value_delimiter = ',')]
    workspace_roots: Option<Vec<String>>,

    /// Load settings from a specific profile
    #[arg(long, value_name = "NAME")]
    profile: Option<String>,
}

fn main() -> Result<()> {
    setup_tracing()?;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    runtime.block_on(async {
        let local = tokio::task::LocalSet::new();
        local.run_until(async_main()).await
    })
}

async fn async_main() -> Result<()> {
    let args = Args::parse();

    if std::env::var("RUST_LOG").is_ok() {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_writer(std::io::stderr)
            .init();
    }

    let workspace_roots = args
        .workspace_roots
        .map(|roots| -> Result<Vec<PathBuf>> {
            roots
                .into_iter()
                .map(|root| {
                    let path = PathBuf::from(root);
                    path.canonicalize().map_err(|e| {
                        anyhow::anyhow!("Failed to canonicalize workspace root {:?}: {}", path, e)
                    })
                })
                .collect()
        })
        .transpose()?;

    let mut app = InteractiveApp::new(workspace_roots, args.profile).await?;
    app.run().await?;

    Ok(())
}

fn setup_tracing() -> Result<()> {
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
