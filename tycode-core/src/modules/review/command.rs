use std::sync::Arc;

use futures_util::stream::{FuturesUnordered, StreamExt};

use chrono::Utc;

use crate::agents::agent::ActiveAgent;
use crate::agents::code_review::CodeReviewAgent;
use crate::agents::runner::AgentRunner;
use crate::ai::Message;
use crate::chat::actor::ActorState;
use crate::chat::events::{ChatMessage, MessageSender};
use crate::chat::tools::current_agent_mut;
use crate::module::SlashCommand;

const DIFF_REVIEW_PROMPT: &str = "\
You are reviewing unstaged git changes. The git diff is provided below. \
Use set_tracked_files to examine the full contents of changed files, \
run_build_test to verify compilation, search_types and get_type_docs to \
understand type definitions. After thorough investigation, call complete_task \
with your findings: approve or reject with specific recommendations. \
Be systematic — check correctness, style, completeness, and potential bugs.";

const HUNK_REVIEW_PROMPT: &str = "\
You are reviewing a single diff hunk. Analyze the change for correctness, \
potential bugs, style issues, and completeness. Call complete_task with your \
findings: note any issues found or confirm the change looks correct.";

const CONSOLIDATION_PROMPT: &str = "\
You received individual reviews for each diff hunk. Synthesize these into a \
unified code review report. Provide an overall approve/reject decision with a \
summary of all findings. Call complete_task with the consolidated report.";

pub struct ReviewSlashCommand;

#[async_trait::async_trait(?Send)]
impl SlashCommand for ReviewSlashCommand {
    fn name(&self) -> &'static str {
        "review"
    }

    fn description(&self) -> &'static str {
        "Review unstaged git changes"
    }

    fn usage(&self) -> &'static str {
        "/review [deep]"
    }

    async fn execute(&self, state: &mut ActorState, args: &[&str]) -> Vec<ChatMessage> {
        let results = if !args.is_empty() && args[0] == "deep" {
            deep_review(state).await
        } else {
            standard_review(state).await
        };

        let review_text: String = results
            .iter()
            .filter(|m| matches!(m.sender, MessageSender::System))
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        if !review_text.is_empty() {
            current_agent_mut(state, |a| {
                a.conversation.push(Message::user(review_text));
            });
        }

        results
    }
}

async fn standard_review(state: &mut ActorState) -> Vec<ChatMessage> {
    let Some(workspace_root) = state.workspace_roots.first() else {
        return vec![create_message(
            "No workspace root configured.".to_string(),
            MessageSender::Error,
        )];
    };

    let diff = match super::diff::git_diff(workspace_root).await {
        Ok(d) => d,
        Err(e) => {
            return vec![create_message(
                format!("Failed to get git diff: {e:?}"),
                MessageSender::Error,
            )];
        }
    };

    if diff.is_empty() {
        return vec![create_message(
            "No unstaged changes found.".to_string(),
            MessageSender::System,
        )];
    }

    state.event_sender.send_message(ChatMessage::system(
        "Reviewing unstaged changes...".to_string(),
    ));

    let runner = AgentRunner::new(
        state.provider.read().unwrap().clone(),
        state.settings.clone(),
        state.modules.clone(),
        state.steering.clone(),
        state.prompt_builder.clone(),
        state.context_builder.clone(),
    );

    let mut active_agent = ActiveAgent::new(Arc::new(CodeReviewAgent::new()));
    active_agent
        .conversation
        .push(Message::user(DIFF_REVIEW_PROMPT.to_string()));
    active_agent.conversation.push(Message::user(diff));

    match runner.run(active_agent, 15).await {
        Ok(result) => vec![create_message(
            format!("=== Code Review ===\n\n{result}"),
            MessageSender::System,
        )],
        Err(e) => vec![create_message(
            format!("Review failed: {e:?}"),
            MessageSender::Error,
        )],
    }
}

async fn deep_review(state: &mut ActorState) -> Vec<ChatMessage> {
    let Some(workspace_root) = state.workspace_roots.first() else {
        return vec![create_message(
            "No workspace root configured.".to_string(),
            MessageSender::Error,
        )];
    };

    let diff = match super::diff::git_diff_expanded(workspace_root, 15).await {
        Ok(d) => d,
        Err(e) => {
            return vec![create_message(
                format!("Failed to get git diff: {e:?}"),
                MessageSender::Error,
            )];
        }
    };

    if diff.is_empty() {
        return vec![create_message(
            "No unstaged changes found.".to_string(),
            MessageSender::System,
        )];
    }

    let hunks = super::diff::parse_hunks(&diff);

    if hunks.is_empty() {
        return vec![create_message(
            "No hunks found in diff.".to_string(),
            MessageSender::System,
        )];
    }

    let total = hunks.len();

    let futures: FuturesUnordered<_> = hunks
        .iter()
        .enumerate()
        .map(|(i, hunk)| {
            let runner = AgentRunner::new(
                state.provider.read().unwrap().clone(),
                state.settings.clone(),
                state.modules.clone(),
                state.steering.clone(),
                state.prompt_builder.clone(),
                state.context_builder.clone(),
            );

            let mut active_agent = ActiveAgent::new(Arc::new(CodeReviewAgent::new()));
            active_agent
                .conversation
                .push(Message::user(HUNK_REVIEW_PROMPT.to_string()));
            active_agent
                .conversation
                .push(Message::user(hunk.content.clone()));

            let label = format!("[{}/{}] {}: {}", i + 1, total, hunk.file_path, hunk.header);

            async move {
                let result = match runner.run(active_agent, 5).await {
                    Ok(r) => format!("{label}\n{r}"),
                    Err(e) => format!("{label}\nReview failed: {e:?}"),
                };
                (i, result)
            }
        })
        .collect();

    state.event_sender.send_message(ChatMessage::system(format!(
        "Launched {total} review sub-agents"
    )));

    let mut completed = 0usize;
    let milestone_step = (total / 5).max(1);
    let mut next_milestone = milestone_step;
    let mut indexed_results: Vec<(usize, String)> = Vec::with_capacity(total);

    let mut futures = futures;
    while let Some((idx, result)) = futures.next().await {
        indexed_results.push((idx, result));
        completed += 1;

        if completed >= next_milestone && completed < total {
            let pct = (completed * 100) / total;
            state.event_sender.send_message(ChatMessage::system(format!(
                "Review progress: {completed}/{total} ({pct}%) complete"
            )));
            next_milestone += milestone_step;
        }
    }

    indexed_results.sort_by_key(|(i, _)| *i);
    let hunk_results: Vec<String> = indexed_results.into_iter().map(|(_, s)| s).collect();

    state
        .event_sender
        .send_message(ChatMessage::system("Aggregating reviews...".to_string()));

    let runner = AgentRunner::new(
        state.provider.read().unwrap().clone(),
        state.settings.clone(),
        state.modules.clone(),
        state.steering.clone(),
        state.prompt_builder.clone(),
        state.context_builder.clone(),
    );

    let mut active_agent = ActiveAgent::new(Arc::new(CodeReviewAgent::new()));
    active_agent
        .conversation
        .push(Message::user(CONSOLIDATION_PROMPT.to_string()));
    active_agent
        .conversation
        .push(Message::user(hunk_results.join("\n\n---\n\n")));

    match runner.run(active_agent, 10).await {
        Ok(result) => vec![create_message(
            format!("=== Code Review ===\n\n{result}"),
            MessageSender::System,
        )],
        Err(e) => vec![create_message(
            format!("Review failed: {e:?}"),
            MessageSender::Error,
        )],
    }
}

fn create_message(content: String, sender: MessageSender) -> ChatMessage {
    ChatMessage {
        content,
        sender,
        timestamp: Utc::now().timestamp_millis() as u64,
        reasoning: None,
        tool_calls: Vec::new(),
        model_info: None,
        token_usage: None,
        context_breakdown: None,
        images: vec![],
    }
}
