//! Compaction planner: decides when rewriting conversation history pays for
//! itself and applies the mechanical (non-AI) compaction pass.
//!
//! Rewriting history invalidates the provider prompt cache from the first
//! changed byte, so edits are batched behind explicit triggers instead of
//! happening per request:
//!
//! - `WindowPressure`: the prompt is near the model's context window and must
//!   shrink regardless of cost.
//! - `ColdCache`: the cache is already invalid (TTL expired, model changed,
//!   or no request has been made yet this session), so rewriting is free.
//! - `BreakEven`: expected cache-read savings over the remaining requests
//!   exceed the one-time cost of re-writing the rebuilt prefix.
//!
//! The mechanical pass removes the two classes of dead weight that dominate
//! long agent conversations: old reasoning blocks and large, old tool
//! results (build logs, file dumps) that can be regenerated on demand.

use std::time::Duration;

use crate::ai::types::{Content, ContentBlock, Cost, Message, MessageRole};

use super::config::ContextManagementConfig;
use super::{count_reasoning_blocks, prune_reasoning_blocks};

/// Rough bytes-per-token used for planning. Triggers only need the right
/// order of magnitude, not exact token counts.
pub const BYTES_PER_TOKEN: usize = 4;

pub const TOOL_RESULT_STUB: &str =
    "[pruned: old tool output removed to conserve context. Re-run the tool if needed.]";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactionTrigger {
    /// Prompt is near the context window; compaction is mandatory.
    WindowPressure,
    /// Prompt cache is already cold; compaction costs nothing extra.
    ColdCache,
    /// Future cache-read savings exceed the one-time rebuild cost.
    BreakEven,
}

impl CompactionTrigger {
    pub fn describe(&self) -> &'static str {
        match self {
            Self::WindowPressure => "context window pressure",
            Self::ColdCache => "prompt cache already cold",
            Self::BreakEven => "cache savings exceed rebuild cost",
        }
    }
}

pub struct PlannerInputs<'a> {
    pub conversation: &'a [Message],
    /// Total prompt tokens of the most recent request (input + cached +
    /// cache-write), if one has completed. None means no request yet — the
    /// cache is necessarily cold.
    pub last_prefix_tokens: Option<u64>,
    pub context_window: u32,
    pub cost: Cost,
    pub elapsed_since_last_request: Option<Duration>,
    pub model_changed: bool,
    pub config: &'a ContextManagementConfig,
}

#[derive(Debug, Default)]
pub struct MechanicalOutcome {
    pub reasoning_blocks_pruned: usize,
    pub tool_results_stubbed: usize,
    pub bytes_removed: usize,
}

impl MechanicalOutcome {
    pub fn is_noop(&self) -> bool {
        self.reasoning_blocks_pruned == 0 && self.tool_results_stubbed == 0
    }
}

/// Decide whether to compact now, and why.
pub fn decide(inputs: &PlannerInputs) -> Option<CompactionTrigger> {
    let config = inputs.config;
    let prefix_tokens = inputs.last_prefix_tokens.unwrap_or_else(|| {
        (estimate_conversation_bytes(inputs.conversation) / BYTES_PER_TOKEN) as u64
    });

    let window_limit = (inputs.context_window as f64 * config.window_pressure_fraction) as u64;
    if prefix_tokens > window_limit {
        return Some(CompactionTrigger::WindowPressure);
    }

    let prunable_bytes = estimate_prunable_bytes(inputs.conversation, config);
    if prunable_bytes < config.min_compaction_bytes {
        return None;
    }

    let cache_cold = inputs.model_changed
        || inputs
            .elapsed_since_last_request
            .is_none_or(|elapsed| elapsed >= Duration::from_secs(config.cache_ttl_seconds));
    if cache_cold {
        return Some(CompactionTrigger::ColdCache);
    }

    let input_price = inputs.cost.input_cost_per_million_tokens;
    let write_price = inputs.cost.cache_write_cost_per_million_tokens;
    let read_price = inputs.cost.cache_read_cost_per_million_tokens;

    // Without a functional prompt cache every request re-pays full input
    // price for the whole prefix, so removing tokens always pays off.
    if read_price <= 0.0 {
        return Some(CompactionTrigger::BreakEven);
    }

    // Compaction rewrites the retained prefix at the rebuild price (cache
    // write where the provider charges a premium, otherwise plain input)
    // instead of the read price it would have ridden at. It saves the read
    // price on every removed token for each remaining request.
    let rebuild_price = if write_price > 0.0 {
        write_price
    } else {
        input_price
    };
    let prunable_tokens = (prunable_bytes / BYTES_PER_TOKEN) as f64;
    let retained_tokens = (prefix_tokens as f64 - prunable_tokens).max(0.0);

    let one_time_cost = retained_tokens * (rebuild_price - read_price).max(0.0);
    let savings = prunable_tokens * read_price * config.expected_remaining_requests as f64;

    if savings >= one_time_cost {
        Some(CompactionTrigger::BreakEven)
    } else {
        None
    }
}

/// Apply the mechanical compaction pass: stub old tool results and prune
/// reasoning blocks down to the configured retain count.
pub fn apply_mechanical(
    messages: &mut Vec<Message>,
    config: &ContextManagementConfig,
) -> MechanicalOutcome {
    let mut outcome = MechanicalOutcome::default();

    for (msg_idx, block_idx, bytes_savable) in prunable_tool_results(messages, config) {
        let mut blocks = messages[msg_idx].content.clone().into_blocks();
        if let ContentBlock::ToolResult(result) = &mut blocks[block_idx] {
            result.content = TOOL_RESULT_STUB.to_string();
        }
        messages[msg_idx].content = Content::new(blocks);
        outcome.tool_results_stubbed += 1;
        outcome.bytes_removed += bytes_savable;
    }

    let reasoning_before = count_reasoning_blocks(messages);
    if reasoning_before > config.reasoning_prune_retain {
        outcome.bytes_removed += prunable_reasoning_bytes(messages, config.reasoning_prune_retain);
        prune_reasoning_blocks(messages, config.reasoning_prune_retain);
        outcome.reasoning_blocks_pruned = reasoning_before - count_reasoning_blocks(messages);
    }

    outcome
}

/// Bytes the mechanical pass would remove, used by the decision triggers.
pub fn estimate_prunable_bytes(messages: &[Message], config: &ContextManagementConfig) -> usize {
    let tool_result_bytes: usize = prunable_tool_results(messages, config)
        .iter()
        .map(|(_, _, bytes)| bytes)
        .sum();
    tool_result_bytes + prunable_reasoning_bytes(messages, config.reasoning_prune_retain)
}

/// Tool results eligible for stubbing: `(message index, block index, bytes
/// saved)`. A result is eligible once more than `tool_result_keep_recent_turns`
/// assistant messages appear after it — results the model has not yet
/// responded to are never touched — and its content clears the size floor.
fn prunable_tool_results(
    messages: &[Message],
    config: &ContextManagementConfig,
) -> Vec<(usize, usize, usize)> {
    let mut eligible = Vec::new();
    let mut assistants_after = 0usize;

    for (msg_idx, message) in messages.iter().enumerate().rev() {
        if message.role == MessageRole::Assistant {
            assistants_after += 1;
            continue;
        }
        if assistants_after <= config.tool_result_keep_recent_turns {
            continue;
        }
        for (block_idx, block) in message.content.blocks().iter().enumerate() {
            if let ContentBlock::ToolResult(result) = block {
                // Results smaller than the stub itself are never worth
                // replacing (also keeps the pass idempotent).
                if result.content.len() >= config.tool_result_min_prune_bytes
                    && result.content.len() > TOOL_RESULT_STUB.len()
                {
                    eligible.push((
                        msg_idx,
                        block_idx,
                        result.content.len() - TOOL_RESULT_STUB.len(),
                    ));
                }
            }
        }
    }

    eligible
}

/// Bytes held by reasoning blocks that pruning to `retain_count` would drop.
fn prunable_reasoning_bytes(messages: &[Message], retain_count: usize) -> usize {
    let mut sizes = Vec::new();
    for message in messages {
        for block in message.content.blocks() {
            if let ContentBlock::ReasoningContent(reasoning) = block {
                let raw_json_bytes = reasoning
                    .raw_json
                    .as_ref()
                    .map(|v| v.to_string().len())
                    .unwrap_or(0);
                sizes.push(
                    reasoning.text.len()
                        + reasoning.signature.as_ref().map_or(0, |s| s.len())
                        + reasoning.blob.as_ref().map_or(0, |b| b.len())
                        + raw_json_bytes,
                );
            }
        }
    }
    if sizes.len() <= retain_count {
        return 0;
    }
    sizes[..sizes.len() - retain_count].iter().sum()
}

/// Rough content size of a conversation, used when no request telemetry is
/// available (fresh or restored sessions).
pub fn estimate_conversation_bytes(messages: &[Message]) -> usize {
    messages
        .iter()
        .flat_map(|m| m.content.blocks())
        .map(|block| match block {
            ContentBlock::Text(text) => text.len(),
            ContentBlock::ToolResult(result) => result.content.len(),
            ContentBlock::ToolUse(tool_use) => tool_use.arguments.to_string().len(),
            ContentBlock::ReasoningContent(reasoning) => reasoning.text.len(),
            ContentBlock::Image(image) => image.data.len(),
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::types::{ReasoningData, ToolResultData};

    fn config() -> ContextManagementConfig {
        ContextManagementConfig {
            tool_result_keep_recent_turns: 1,
            tool_result_min_prune_bytes: 100,
            reasoning_prune_retain: 1,
            min_compaction_bytes: 0,
            ..ContextManagementConfig::default()
        }
    }

    fn user_tool_result(id: &str, content: String) -> Message {
        Message {
            role: MessageRole::User,
            content: Content::new(vec![ContentBlock::ToolResult(ToolResultData {
                tool_use_id: id.to_string(),
                content,
                is_error: false,
            })]),
        }
    }

    fn assistant_with_reasoning(text: &str) -> Message {
        Message {
            role: MessageRole::Assistant,
            content: Content::new(vec![
                ContentBlock::ReasoningContent(ReasoningData {
                    text: text.to_string(),
                    signature: None,
                    blob: None,
                    raw_json: None,
                }),
                ContentBlock::Text("response".to_string()),
            ]),
        }
    }

    fn sample_conversation() -> Vec<Message> {
        vec![
            Message {
                role: MessageRole::User,
                content: Content::text_only("do the thing".to_string()),
            },
            assistant_with_reasoning("old reasoning one"),
            user_tool_result("t1", "x".repeat(5000)),
            assistant_with_reasoning("old reasoning two"),
            user_tool_result("t2", "y".repeat(5000)),
            assistant_with_reasoning("newest reasoning"),
        ]
    }

    #[test]
    fn mechanical_pass_stubs_old_results_and_prunes_reasoning() {
        let mut messages = sample_conversation();
        let outcome = apply_mechanical(&mut messages, &config());

        // t1 has two assistant messages after it (> keep_recent 1): stubbed.
        // t2 has one assistant message after it (== keep_recent): kept.
        assert_eq!(outcome.tool_results_stubbed, 1);
        let ContentBlock::ToolResult(first) = &messages[2].content.blocks()[0] else {
            panic!("expected tool result");
        };
        assert_eq!(first.content, TOOL_RESULT_STUB);
        assert_eq!(first.tool_use_id, "t1");
        let ContentBlock::ToolResult(second) = &messages[4].content.blocks()[0] else {
            panic!("expected tool result");
        };
        assert_eq!(second.content.len(), 5000);

        // Three reasoning blocks pruned down to retain=1 (the newest).
        assert_eq!(outcome.reasoning_blocks_pruned, 2);
        assert_eq!(count_reasoning_blocks(&messages), 1);
        assert!(outcome.bytes_removed > 4000);
    }

    #[test]
    fn mechanical_pass_is_idempotent() {
        let mut messages = sample_conversation();
        apply_mechanical(&mut messages, &config());
        let outcome = apply_mechanical(&mut messages, &config());
        assert!(outcome.is_noop());
        assert_eq!(outcome.bytes_removed, 0);
    }

    #[test]
    fn recent_tool_results_are_never_stubbed() {
        // Tool result with no assistant message after it: the model has not
        // consumed it yet and it must survive any configuration.
        let mut messages = vec![
            assistant_with_reasoning("thinking"),
            user_tool_result("fresh", "z".repeat(10_000)),
        ];
        let mut cfg = config();
        cfg.tool_result_keep_recent_turns = 0;
        let outcome = apply_mechanical(&mut messages, &cfg);
        assert_eq!(outcome.tool_results_stubbed, 0);
        let ContentBlock::ToolResult(fresh) = &messages[1].content.blocks()[0] else {
            panic!("expected tool result");
        };
        assert_eq!(fresh.content.len(), 10_000);
    }

    #[test]
    fn estimate_matches_mechanical_pass() {
        let mut messages = sample_conversation();
        let cfg = config();
        let estimated = estimate_prunable_bytes(&messages, &cfg);
        let outcome = apply_mechanical(&mut messages, &cfg);
        assert_eq!(estimated, outcome.bytes_removed);
    }

    fn planner_inputs<'a>(
        conversation: &'a [Message],
        cfg: &'a ContextManagementConfig,
        cost: Cost,
    ) -> PlannerInputs<'a> {
        PlannerInputs {
            conversation,
            last_prefix_tokens: Some(150_000),
            context_window: 1_000_000,
            cost,
            elapsed_since_last_request: Some(Duration::from_secs(10)),
            model_changed: false,
            config: cfg,
        }
    }

    /// Anthropic-style pricing: read 10% of input, write 125% of input.
    fn anthropic_cost() -> Cost {
        Cost::new(3.0, 15.0, 3.75, 0.3)
    }

    #[test]
    fn window_pressure_forces_compaction() {
        let conversation = sample_conversation();
        let cfg = config();
        let mut inputs = planner_inputs(&conversation, &cfg, anthropic_cost());
        inputs.context_window = 100_000;
        inputs.last_prefix_tokens = Some(90_000);
        assert_eq!(decide(&inputs), Some(CompactionTrigger::WindowPressure));
    }

    #[test]
    fn cold_cache_triggers_when_ttl_expired() {
        let conversation = sample_conversation();
        let cfg = config();
        let mut inputs = planner_inputs(&conversation, &cfg, anthropic_cost());
        inputs.elapsed_since_last_request = Some(Duration::from_secs(600));
        assert_eq!(decide(&inputs), Some(CompactionTrigger::ColdCache));
    }

    #[test]
    fn model_change_triggers_cold_cache() {
        let conversation = sample_conversation();
        let cfg = config();
        let mut inputs = planner_inputs(&conversation, &cfg, anthropic_cost());
        inputs.model_changed = true;
        assert_eq!(decide(&inputs), Some(CompactionTrigger::ColdCache));
    }

    #[test]
    fn break_even_requires_enough_prunable_material() {
        // ~10KB prunable out of a 150k-token prefix on Anthropic pricing:
        // savings (2.5k tokens * 0.3 * 25) are dwarfed by rebuilding ~147.5k
        // tokens at the 3.45 write-read spread. Warm cache: keep history.
        let conversation = sample_conversation();
        let cfg = config();
        let inputs = planner_inputs(&conversation, &cfg, anthropic_cost());
        assert_eq!(decide(&inputs), None);

        // Same shape but the conversation is mostly dead weight: a prefix
        // barely larger than the prunable material amortizes immediately.
        let mut inputs = planner_inputs(&conversation, &cfg, anthropic_cost());
        inputs.last_prefix_tokens = Some(3_000);
        assert_eq!(decide(&inputs), Some(CompactionTrigger::BreakEven));
    }

    #[test]
    fn no_cache_pricing_always_compacts_above_floor() {
        let conversation = sample_conversation();
        let cfg = config();
        let inputs = planner_inputs(&conversation, &cfg, Cost::new(1.0, 2.0, 0.0, 0.0));
        assert_eq!(decide(&inputs), Some(CompactionTrigger::BreakEven));
    }

    #[test]
    fn floor_blocks_small_compactions() {
        let conversation = sample_conversation();
        let mut cfg = config();
        cfg.min_compaction_bytes = 1_000_000;
        let mut inputs = planner_inputs(&conversation, &cfg, anthropic_cost());
        inputs.model_changed = true;
        assert_eq!(decide(&inputs), None);
    }

    #[test]
    fn no_prior_request_counts_as_cold() {
        let conversation = sample_conversation();
        let cfg = config();
        let mut inputs = planner_inputs(&conversation, &cfg, anthropic_cost());
        inputs.last_prefix_tokens = None;
        inputs.elapsed_since_last_request = None;
        assert_eq!(decide(&inputs), Some(CompactionTrigger::ColdCache));
    }
}
