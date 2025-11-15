use tycode_core::chat::events::{ChatEvent, MessageSender};

mod fixture;

#[test]
fn test_input_too_long_triggers_compaction() {
    fixture::run(|mut fixture| async move {
        use tycode_core::ai::mock::MockBehavior;

        // First establish a conversation
        let events = fixture.step("Hello").await;
        assert!(!events.is_empty());

        // Configure mock to return InputTooLong once, then succeed
        fixture.set_mock_behavior(MockBehavior::InputTooLongThenSuccess {
            remaining_errors: 1,
        });

        // Send a message that will trigger InputTooLong
        let events = fixture.step("Continue conversation").await;

        // Verify we got an assistant response (proves compaction worked and conversation continued)
        assert!(
            events.iter().any(|e| {
                matches!(
                    e,
                    ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Assistant { .. })
                )
            }),
            "Should receive assistant message after compaction"
        );
    });
}

#[test]
fn test_compaction_clears_tracked_files() {
    fixture::run(|mut fixture| async move {
        use tycode_core::ai::mock::MockBehavior;

        // Establish initial conversation with tracked files potentially in context
        let events = fixture.step("Hello").await;
        assert!(!events.is_empty());

        // Configure mock to return InputTooLong once, then succeed
        // When compaction occurs, tracked files should be cleared to reduce context
        fixture.set_mock_behavior(MockBehavior::InputTooLongThenSuccess {
            remaining_errors: 1,
        });

        // Send a message that triggers InputTooLong and forces compaction
        let events = fixture
            .step("Message that triggers compaction and clears tracked files")
            .await;

        // Verify conversation continues after compaction (tracked files cleared)
        assert!(
            events.iter().any(|e| {
                matches!(
                    e,
                    ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Assistant { .. })
                )
            }),
            "Should receive assistant message after compaction clears tracked files"
        );

        // Verify we can continue the conversation after compaction
        let events = fixture.step("Continue after compaction").await;
        assert!(
            events.iter().any(|e| {
                matches!(
                    e,
                    ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Assistant { .. })
                )
            }),
            "Conversation should continue normally after compaction and file clearing"
        );
    });
}

#[test]
fn test_compaction_with_tool_blocks_in_history() {
    fixture::run(|mut fixture| async move {
        use tycode_core::ai::mock::MockBehavior;

        // Step 1: Create a conversation with ToolUse and ToolResult blocks
        // Use ToolUseThenSuccess to trigger a tool use, which will add both
        // ToolUse (from AI) and ToolResult (from tool execution) to the conversation
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "set_tracked_files".to_string(),
            tool_arguments: r#"{"file_paths": ["test.rs"]}"#.to_string(),
        });

        // Send a message that will trigger tool use
        let events = fixture.step("Show me the test file").await;
        assert!(!events.is_empty(), "Should have events from tool use");

        // Step 2: Now trigger compaction while the conversation has tool blocks in history
        // This should expose the bug: compaction creates a summary request with tools: vec![]
        // but the conversation history contains ToolUse and ToolResult blocks,
        // which causes Bedrock to require toolConfig to be present
        fixture.set_mock_behavior(MockBehavior::InputTooLongThenSuccess {
            remaining_errors: 1,
        });

        // This step should trigger compaction and currently FAIL with toolConfig validation error
        let events = fixture.step("Continue conversation after tool use").await;

        // This assertion will FAIL until the bug is fixed
        // Expected failure: toolConfig validation error from Bedrock during compaction
        assert!(
            events.iter().any(|e| {
                matches!(
                    e,
                    ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Assistant { .. })
                )
            }),
            "Should receive assistant message after compaction (currently fails with toolConfig error)"
        );
    });
}
