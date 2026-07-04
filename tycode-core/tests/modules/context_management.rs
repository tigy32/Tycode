//! End-to-end tests for context management module.
//!
//! Tests the compaction planner (automatic reasoning pruning and tool-result
//! stubbing at cache-friendly trigger points) and the `/compact reasoning`
//! slash command.

use tycode_core::ai::types::ContentBlock;
use tycode_core::chat::events::ChatEvent;
use tycode_core::modules::context_management::planner::TOOL_RESULT_STUB;

#[path = "../fixture.rs"]
mod fixture;

/// Planner config that fires on every request: the cache always counts as
/// cold (ttl 0) and there is no floor on removable bytes.
fn always_compact_config(retain: usize) -> serde_json::Value {
    serde_json::json!({
        "enabled": true,
        "auto_compact": true,
        "reasoning_prune_retain": retain,
        "cache_ttl_seconds": 0,
        "min_compaction_bytes": 0,
        "tool_result_keep_recent_turns": 0,
        "tool_result_min_prune_bytes": 0,
    })
}

fn count_reasoning(request: &tycode_core::ai::ConversationRequest) -> usize {
    request
        .messages
        .iter()
        .map(|m| {
            m.content
                .blocks()
                .iter()
                .filter(|b| matches!(b, ContentBlock::ReasoningContent(_)))
                .count()
        })
        .sum()
}

#[test]
fn test_reasoning_blocks_pruned_when_planner_fires() {
    fixture::run(|mut f: fixture::Fixture| async move {
        f.set_mock_behavior(fixture::MockBehavior::ReasoningContentThenSuccess {
            remaining_reasonings: 5,
            reasoning_text: "Reasoning response".to_string(),
        });

        f.update_settings(|s| {
            s.set_module_config("context_management", always_compact_config(1));
        })
        .await;

        // First exchange - will add reasoning blocks
        let _ = f.step("First message").await;
        f.clear_captured_requests();

        // Second exchange - planner runs before the request and prunes
        let _ = f.step("Second message").await;
        let request = f.get_last_ai_request().expect("Expected AI request");

        assert!(
            count_reasoning(&request) <= 1,
            "Expected at most 1 reasoning block after pruning, got {}",
            count_reasoning(&request)
        );
    });
}

#[test]
fn test_old_tool_results_stubbed_when_planner_fires() {
    fixture::run(|mut f: fixture::Fixture| async move {
        // One tool call producing output comfortably larger than the stub,
        // then plain success responses.
        let long_output = "x".repeat(500);
        f.set_mock_behavior(fixture::MockBehavior::ToolUseThenSuccess {
            tool_name: "bash".to_string(),
            tool_arguments: serde_json::json!({"command": format!("echo {long_output}")})
                .to_string(),
        });

        f.update_settings(|s| {
            s.set_module_config("context_management", always_compact_config(8));
        })
        .await;

        // Produces: assistant tool call -> tool result -> assistant success.
        let _ = f.step("Run a command").await;
        f.clear_captured_requests();

        // Next request: the tool result now has an assistant message after it
        // (keep_recent_turns = 0) and should arrive stubbed.
        let _ = f.step("Follow up").await;
        let request = f.get_last_ai_request().expect("Expected AI request");

        let tool_results: Vec<String> = request
            .messages
            .iter()
            .flat_map(|m| {
                m.content
                    .blocks()
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::ToolResult(r) => Some(r.content.clone()),
                        _ => None,
                    })
                    .collect::<Vec<String>>()
            })
            .collect();

        assert!(
            !tool_results.is_empty(),
            "Expected the conversation to contain a tool result"
        );
        assert!(
            tool_results.iter().any(|c| c == TOOL_RESULT_STUB),
            "Expected old tool result to be stubbed, got: {:?}",
            tool_results
        );
    });
}

#[test]
fn test_context_management_disabled_does_not_prune() {
    fixture::run(|mut f: fixture::Fixture| async move {
        f.set_mock_behavior(fixture::MockBehavior::ReasoningContentThenSuccess {
            remaining_reasonings: 5,
            reasoning_text: "Reasoning response".to_string(),
        });

        let mut config = always_compact_config(0);
        config["enabled"] = serde_json::json!(false);
        f.update_settings(|s| {
            s.set_module_config("context_management", config);
        })
        .await;

        let _ = f.step("First message").await;
        f.clear_captured_requests();

        let _ = f.step("Second message").await;
        let request = f.get_last_ai_request().expect("Expected AI request");

        assert!(
            count_reasoning(&request) > 0,
            "With context management disabled, reasoning blocks should be preserved"
        );
    });
}

#[test]
fn test_auto_compaction_toggle_disabled_does_not_prune() {
    fixture::run(|mut f: fixture::Fixture| async move {
        f.set_mock_behavior(fixture::MockBehavior::ReasoningContentThenSuccess {
            remaining_reasonings: 5,
            reasoning_text: "Reasoning response".to_string(),
        });

        let mut config = always_compact_config(0);
        config["auto_compact"] = serde_json::json!(false);
        f.update_settings(|s| {
            s.set_module_config("context_management", config);
        })
        .await;

        let _ = f.step("First message").await;
        f.clear_captured_requests();

        let _ = f.step("Second message").await;
        let request = f.get_last_ai_request().expect("Expected AI request");

        assert!(
            count_reasoning(&request) > 0,
            "With auto_compact disabled, reasoning blocks should be preserved"
        );
    });
}

// ============================================================================
// Slash Command Tests
// ============================================================================

#[test]
fn test_compact_reasoning_command_prunes_blocks() {
    fixture::run(|mut f: fixture::Fixture| async move {
        // Configure mock to return reasoning content multiple times
        f.set_mock_behavior(fixture::MockBehavior::ReasoningContentThenSuccess {
            remaining_reasonings: 5,
            reasoning_text: "Step by step reasoning".to_string(),
        });

        // Build up conversation with several reasoning blocks
        let _ = f.step("First message").await;
        let _ = f.step("Second message").await;
        let _ = f.step("Third message").await;

        // Now use slash command to compact to 1 reasoning block
        let events = f.step("/compact reasoning 1").await;

        // Should receive a system message confirming the compaction
        let system_message = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) => match msg.sender {
                    tycode_core::chat::events::MessageSender::System => Some(msg),
                    _ => None,
                },
                _ => None,
            })
            .last();

        assert!(
            system_message.is_some(),
            "Expected system message after compact command"
        );

        let msg = system_message.unwrap();
        assert!(
            msg.content.contains("pruning") || msg.content.contains("Compacted"),
            "System message should confirm pruning: got '{}'",
            msg.content
        );

        // Verify reasoning was actually pruned by checking next AI request
        f.clear_captured_requests();
        let _ = f.step("Verify pruning worked").await;
        let request = f.get_last_ai_request().expect("Expected AI request");

        let reasoning_count: usize = request
            .messages
            .iter()
            .map(|m| {
                m.content
                    .blocks()
                    .iter()
                    .filter(|b| matches!(b, ContentBlock::ReasoningContent(_)))
                    .count()
            })
            .sum();

        // Should have at most 1 reasoning block (the retained count)
        assert!(
            reasoning_count <= 1,
            "Expected at most 1 reasoning block after /compact reasoning 1, got {}",
            reasoning_count
        );
    });
}

#[test]
fn test_compact_reasoning_command_no_pruning_needed() {
    fixture::run(|mut f: fixture::Fixture| async move {
        // Configure mock to return reasoning content
        f.set_mock_behavior(fixture::MockBehavior::ReasoningContentThenSuccess {
            remaining_reasonings: 5,
            reasoning_text: "Single reasoning block".to_string(),
        });

        // Build up conversation with only a few reasoning blocks
        let _ = f.step("First message").await;

        // Try to compact to 5 blocks (more than we have)
        let events = f.step("/compact reasoning 5").await;

        // Should receive system message saying no pruning needed
        let system_message = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) => match msg.sender {
                    tycode_core::chat::events::MessageSender::System => Some(msg),
                    _ => None,
                },
                _ => None,
            })
            .last();

        assert!(system_message.is_some(), "Expected system message");

        let msg = system_message.unwrap();
        assert!(
            msg.content.contains("No pruning needed") || msg.content.contains("pruning needed"),
            "System message should indicate no pruning needed: got '{}'",
            msg.content
        );
    });
}

#[test]
fn test_compact_reasoning_command_missing_argument() {
    fixture::run(|mut f: fixture::Fixture| async move {
        let events = f.step("/compact").await;

        let system_message = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) => match msg.sender {
                    tycode_core::chat::events::MessageSender::System => Some(msg),
                    _ => None,
                },
                _ => None,
            })
            .last();

        assert!(
            system_message.is_some(),
            "Expected system message after /compact"
        );

        let msg = system_message.unwrap();
        // Should indicate compaction occurred or no conversation to compact
        assert!(
            msg.content.contains("Compaction") || msg.content.contains("No conversation"),
            "Should indicate compaction result: got '{}'",
            msg.content
        );
    });
}

#[test]
fn test_compact_reasoning_command_invalid_count() {
    fixture::run(|mut f: fixture::Fixture| async move {
        // Test invalid count (not a number)
        let events = f.step("/compact reasoning abc").await;

        let system_message = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) => match msg.sender {
                    tycode_core::chat::events::MessageSender::System => Some(msg),
                    _ => None,
                },
                _ => None,
            })
            .last();

        assert!(
            system_message.is_some(),
            "Expected system message for invalid count"
        );

        let msg = system_message.unwrap();
        assert!(
            msg.content.contains("positive number") || msg.content.contains("specify"),
            "Should show error for invalid count: got '{}'",
            msg.content
        );
    });
}

#[test]
fn test_compact_reasoning_command_unknown_subcommand() {
    fixture::run(|mut f: fixture::Fixture| async move {
        // Test unknown subcommand
        let events = f.step("/compact something_else 10").await;

        let system_message = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) => match msg.sender {
                    tycode_core::chat::events::MessageSender::System => Some(msg),
                    _ => None,
                },
                _ => None,
            })
            .last();

        assert!(
            system_message.is_some(),
            "Expected system message for unknown subcommand"
        );

        let msg = system_message.unwrap();
        assert!(
            msg.content.contains("Unknown") || msg.content.contains("reasoning"),
            "Should show error for unknown subcommand: got '{}'",
            msg.content
        );
    });
}

#[test]
fn test_compact_reasoning_command_zero_count() {
    fixture::run(|mut f: fixture::Fixture| async move {
        // Test zero count (should error)
        let events = f.step("/compact reasoning 0").await;

        let system_message = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) => match msg.sender {
                    tycode_core::chat::events::MessageSender::System => Some(msg),
                    _ => None,
                },
                _ => None,
            })
            .last();

        assert!(
            system_message.is_some(),
            "Expected system message for zero count"
        );

        let msg = system_message.unwrap();
        assert!(
            msg.content.contains("positive number"),
            "Should show error for zero count: got '{}'",
            msg.content
        );
    });
}

#[test]
fn test_compact_reasoning_command_no_reasoning_blocks() {
    fixture::run(|mut f: fixture::Fixture| async move {
        // Use mock that never returns reasoning
        f.set_mock_behavior(fixture::MockBehavior::Success);

        // Exchange without reasoning
        let _ = f.step("Message without reasoning").await;

        // Try to compact
        let events = f.step("/compact reasoning 5").await;

        let system_message = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) => match msg.sender {
                    tycode_core::chat::events::MessageSender::System => Some(msg),
                    _ => None,
                },
                _ => None,
            })
            .last();

        assert!(
            system_message.is_some(),
            "Expected system message when no reasoning blocks"
        );

        let msg = system_message.unwrap();
        assert!(
            msg.content.contains("No reasoning blocks"),
            "Should indicate no reasoning blocks found: got '{}'",
            msg.content
        );
    });
}
