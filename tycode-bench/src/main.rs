pub mod driver;
pub mod fixture;
pub mod leetcode_21;
pub mod modify_file_stress;
pub mod settings;

use anyhow::Context;
use dirs;
use std::path::PathBuf;
use tokio::task::LocalSet;
use tracing::info;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::EnvFilter;
use tycode_core::settings::{Settings, SettingsManager};

use crate::{fixture::run_bench, modify_file_stress::ModifyFileStressTestCase};

use tokio::time::Instant;
use tycode_core::chat::{ChatEvent, MessageSender};

#[derive(Debug)]
struct TestStats {
    name: String,
    wall_time: tokio::time::Duration,
    input_tokens: u64,
    output_tokens: u64,
    total_calls: u64,
    tool_calls: u64,
    successful_tool_calls: u64,
    success: bool,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    setup_tracing()?;
    let home = dirs::home_dir().context("Failed to get home directory")?;
    let tycode_dir = home.join(".tycode");
    let settings =
        SettingsManager::from_settings_dir(tycode_dir, None).expect("Failed to create settings");
    let base_settings = settings.settings();

    let local = LocalSet::new();
    local.run_until(run_benchmarks(base_settings)).await?;
    Ok(())
}

async fn run_benchmarks(base_settings: Settings) -> anyhow::Result<()> {
    let test_settings = settings::get_test_settings(base_settings);
    let mut stats_vec: Vec<TestStats> = Vec::new();

    // Run Modify File Stress test
    for (name, settings) in test_settings {
        let name = name.clone();
        let start = Instant::now();
        let result = run_bench(settings, ModifyFileStressTestCase).await?;
        let elapsed = start.elapsed();

        // parse stats from captured events to extract performance and usage metrics
        let events = result.event_rx.captured();
        let mut total_input_tokens = 0;
        let mut total_output_tokens = 0;
        let mut total_calls = 0;
        let mut tool_calls = 0;
        let mut successful_tool_calls = 0;

        for event in events {
            match event {
                ChatEvent::MessageAdded(ref msg) => {
                    if let MessageSender::Assistant { .. } = msg.sender {
                        total_calls += 1;
                        if !msg.tool_calls.is_empty() {
                            tool_calls += 1;
                        }
                        if let Some(ref token_usage) = msg.token_usage {
                            total_input_tokens += token_usage.input_tokens as u64;
                            total_output_tokens += token_usage.output_tokens as u64;
                            // total_cost = 0.0; // cost calculation not implemented as total_cost not available in TokenUsage
                            // if user provides pricing info, compute as input_rate * input_tokens + output_rate * output_tokens
                        }
                    }
                }
                ChatEvent::ToolExecutionCompleted { success, .. } => {
                    if *success {
                        successful_tool_calls += 1;
                    }
                }
                _ => {}
            }
        }

        let stats = TestStats {
            name: name.clone(),
            wall_time: elapsed,
            input_tokens: total_input_tokens,
            output_tokens: total_output_tokens,
            total_calls,
            tool_calls,
            successful_tool_calls,
            success: result.success,
        };
        stats_vec.push(stats);
    }

    // Print Markdown table
    println!("| Setting | Success | Wall Time | Input Tokens | Output Tokens | Total Calls | Tool Calls | Successful Tool Calls |");
    println!("|---------|---------|-----------|--------------|--------------|--------------|------------|------------------------|");
    for stats in stats_vec {
        let success_symbol = if stats.success { '✓' } else { '✗' };
        println!(
            "| {} | {} | {:?} | {} | {} | {} | {} | {} |",
            stats.name,
            success_symbol,
            stats.wall_time,
            stats.input_tokens,
            stats.output_tokens,
            stats.total_calls,
            stats.tool_calls,
            stats.successful_tool_calls
        );
    }

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
