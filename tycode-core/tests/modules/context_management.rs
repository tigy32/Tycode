//! End-to-end tests for the context management module.
//!
//! Tests the `/compact reasoning` slash command.

use tycode_core::ai::types::ContentBlock;
use tycode_core::chat::events::ChatEvent;

#[path = "../fixture.rs"]
mod fixture;

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
