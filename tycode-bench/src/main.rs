pub mod driver;
pub mod fixture;
pub mod leetcode_21;
pub mod settings;

use tokio::task::LocalSet;
use tycode_core::settings::{Settings, SettingsManager};

use crate::{fixture::run_bench, leetcode_21::LeetCode21TestCase};
use tokio::time::Instant;
use tycode_core::chat::{ChatEvent, MessageSender};

#[derive(Debug)]
struct TestStats {
    name: String,
    wall_time: tokio::time::Duration,
    input_tokens: u64,
    output_tokens: u64,
    cost: f64,
    tool_calls: u64,
    successful_tool_calls: u64,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    // tracing_subscriber::fmt().with_env_filter("info").init();
    let settings = SettingsManager::new()?;
    let base_settings = settings.settings();

    let local = LocalSet::new();
    local.run_until(run_benchmarks(base_settings)).await?;
    Ok(())
}

async fn run_benchmarks(base_settings: Settings) -> anyhow::Result<()> {
    let test_settings = settings::get_test_settings(base_settings);
    let mut stats_vec: Vec<TestStats> = Vec::new();
    for (name, settings) in test_settings {
        let name = name.clone();
        let start = Instant::now();
        let result = run_bench(settings, LeetCode21TestCase).await?;
        let elapsed = start.elapsed();

        // parse stats from captured events to extract performance and usage metrics
        let events = result.event_rx.captured();
        let mut total_input_tokens = 0;
        let mut total_output_tokens = 0;
        let total_cost = 0.0;
        let mut tool_calls = 0;
        let mut successful_tool_calls = 0;

        for event in events {
            match event {
                ChatEvent::MessageAdded(ref msg) => {
                    if let MessageSender::Assistant { .. } = msg.sender {
                        if let Some(ref token_usage) = msg.token_usage {
                            total_input_tokens += token_usage.input_tokens as u64;
                            total_output_tokens += token_usage.output_tokens as u64;
                            // total_cost = 0.0; // cost calculation not implemented as total_cost not available in TokenUsage
                            // if user provides pricing info, compute as input_rate * input_tokens + output_rate * output_tokens
                        }
                    }
                }
                ChatEvent::ToolExecutionCompleted { success, .. } => {
                    tool_calls += 1;
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
            cost: total_cost,
            tool_calls,
            successful_tool_calls,
        };
        stats_vec.push(stats);
    }

    // Print Markdown table
    println!("| Setting | Wall Time | Input Tokens | Output Tokens | Cost | Tool Calls | Successful Tool Calls |");
    println!("|---------|-----------|---------------|----------------|------|------------|------------------------|");
    for stats in stats_vec {
        println!(
            "| {} | {:?} | {} | {} | {:.5} | {} | {} |",
            stats.name,
            stats.wall_time,
            stats.input_tokens,
            stats.output_tokens,
            stats.cost,
            stats.tool_calls,
            stats.successful_tool_calls
        );
    }

    Ok(())
}
