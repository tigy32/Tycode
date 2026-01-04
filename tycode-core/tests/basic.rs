use tycode_core::chat::events::{ChatEvent, MessageSender};

mod fixture;

#[test]
fn test_fixture() {
    fixture::run(|mut fixture| async move {
        use tycode_core::ai::mock::MockBehavior;

        // First message: send "Hello" and verify assistant response
        let events = fixture.step("Hello").await;

        assert!(!events.is_empty(), "Should receive events");
        assert!(
            events.iter().any(|e| {
                matches!(
                    e,
                    ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Assistant { .. })
                )
            }),
            "Should receive assistant message"
        );

        // Second message: reconfigure mock to return a tool use and verify
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "set_tracked_files".to_string(),
            tool_arguments: r#"{"file_paths": ["src/main.rs"]}"#.to_string(),
        });

        let events = fixture.step("Set tracked files").await;

        assert!(
            events.iter().any(|e| {
                matches!(
                    e,
                    ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Assistant { .. })
                )
            }),
            "Should receive assistant message with tool use"
        );
    });
}

#[test]
fn test_invalid_tool_calls_continue_conversation() {
    fixture::run(|mut fixture| async move {
        use tycode_core::ai::mock::MockBehavior;

        // Configure mock to return an invalid tool call (nonexistent tool), then a valid one
        // This simulates the AI hallucinating a tool that doesn't exist, then recovering
        fixture.set_mock_behavior(MockBehavior::ToolUseThenToolUse {
            first_tool_name: "nonexistent_tool".to_string(),
            first_tool_arguments: r#"{"foo": "bar"}"#.to_string(),
            second_tool_name: "set_tracked_files".to_string(),
            second_tool_arguments: r#"{"file_paths": []}"#.to_string(),
        });

        let events = fixture.step("Use a tool").await;

        // Count assistant messages - should have at least 2:
        // 1. Initial message with invalid tool use
        // 2. Follow-up message after seeing the error (proves continuation worked)
        let assistant_message_count = events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Assistant { .. })
                )
            })
            .count();

        assert!(
            assistant_message_count >= 2,
            "Expected at least 2 assistant messages (initial tool use + continuation after error), got {}. \
             This indicates the conversation stopped instead of continuing after the invalid tool call.",
            assistant_message_count
        );
    });
}
