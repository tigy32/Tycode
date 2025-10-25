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
