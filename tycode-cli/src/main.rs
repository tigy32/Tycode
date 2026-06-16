use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use tracing::info;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;

mod auto;
mod auto_driver;
mod auto_pr;
mod banner;
mod commands;
mod github;
mod interactive_app;
mod state;
mod tui;

use crate::interactive_app::InteractiveApp;
use crate::tui::TuiApp;

#[derive(Parser, Debug)]
#[command(name = "tycode-cli")]
#[command(version = env!("CARGO_PKG_VERSION"))]
#[command(about = "TyCode CLI - Native terminal chat interface")]
struct Args {
    /// Workspace roots (for multi-root workspaces)
    #[arg(long, value_delimiter = ',')]
    workspace_roots: Option<Vec<String>>,

    /// Load settings from a specific profile
    #[arg(long, value_name = "NAME")]
    profile: Option<String>,

    /// Auto-PR mode: fetch GitHub issue, resolve it, and create a PR
    #[arg(long, value_name = "ISSUE_NUMBER")]
    auto_pr: Option<u32>,

    /// Create PR in draft mode (useful for testing)
    #[arg(long)]
    draft: bool,

    /// Use compact UI mode (single-line updates with loading indicators)
    #[arg(long)]
    compact: bool,

    /// Auto mode: run until task completion then exit
    #[arg(long)]
    auto: bool,

    /// Task description for auto mode
    #[arg(long)]
    task: Option<String>,

    /// Disable TUI and use legacy line-based interactive mode
    #[arg(long)]
    no_tui: bool,
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

    info!(
        "CLI startup: compact={}, profile={:?}, auto={}, auto_pr={:?}, task={}",
        args.compact,
        args.profile,
        args.auto,
        args.auto_pr,
        args.task.as_deref().unwrap_or("none")
    );

    if args.auto && args.task.is_none() {
        return Err(anyhow::anyhow!("--auto requires --task to be specified"));
    }

    let workspace_roots = args
        .workspace_roots
        .map(|roots| roots.into_iter().map(canonicalize_workspace_root).collect())
        .transpose()?;

    if let Some(issue_number) = args.auto_pr {
        let roots = workspace_roots.unwrap_or_else(|| {
            vec![std::env::current_dir().expect("Failed to get current directory")]
        });
        return auto_pr::run_auto_pr(issue_number, roots, args.profile, args.draft, args.compact)
            .await;
    }

    if args.auto {
        let roots = workspace_roots.unwrap_or_else(|| {
            vec![std::env::current_dir().expect("Failed to get current directory")]
        });
        return auto::run_auto(args.task.unwrap(), roots, args.profile, args.compact).await;
    }

    if args.no_tui {
        let mut app = InteractiveApp::new(workspace_roots, args.profile, args.compact).await?;
        app.run().await?;
    } else {
        let mut tui_app = TuiApp::new(workspace_roots, args.profile).await?;
        tui_app.run().await?;
    }

    Ok(())
}

fn canonicalize_workspace_root(root: String) -> Result<PathBuf> {
    let path = PathBuf::from(&root);
    path.canonicalize()
        .map_err(|e| anyhow::anyhow!("Failed to canonicalize workspace root {root}: {e:?}"))
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
