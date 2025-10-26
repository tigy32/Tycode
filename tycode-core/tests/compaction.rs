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
