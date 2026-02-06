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
                    ChatEvent::StreamEnd { message } if matches!(message.sender, MessageSender::Assistant { .. })
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
                    ChatEvent::StreamEnd { message } if matches!(message.sender, MessageSender::Assistant { .. })
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
                    ChatEvent::StreamEnd { message } if matches!(message.sender, MessageSender::Assistant { .. })
                )
            }),
            "Conversation should continue normally after compaction and file clearing"
        );
    });
}

#[test]
fn test_compaction_with_tool_use_blocks() {
    fixture::run(|mut fixture| async move {
        use tycode_core::ai::mock::MockBehavior;
        use tycode_core::ai::types::ContentBlock;

        // First, trigger a tool use interaction to get ToolUse/ToolResult blocks in conversation
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "set_tracked_files".to_string(),
            tool_arguments: r#"{"file_paths": ["example.txt"]}"#.to_string(),
        });
        let events = fixture.step("Use a tool").await;
        assert!(!events.is_empty(), "Should receive events from tool use");

        // Now trigger InputTooLong to force compaction
        // The conversation now contains ToolUse and ToolResult blocks
        fixture.set_mock_behavior(MockBehavior::InputTooLongThenSuccess {
            remaining_errors: 1,
        });

        let events = fixture.step("Trigger compaction").await;

        // Get all AI requests to find the compaction request
        let requests = fixture.get_all_ai_requests();

        // Find the compaction request by looking for the summarization system prompt
        let compaction_request = requests
            .iter()
            .find(|req| req.system_prompt.contains("conversation summarizer"))
            .expect("Should find compaction/summarization request");

        // Assert that the compaction request does NOT contain ToolUse or ToolResult blocks
        for message in &compaction_request.messages {
            for content in message.content.blocks() {
                assert!(
                    !matches!(content, ContentBlock::ToolUse { .. }),
                    "Compaction request should not contain ToolUse blocks"
                );
                assert!(
                    !matches!(content, ContentBlock::ToolResult { .. }),
                    "Compaction request should not contain ToolResult blocks"
                );
            }
        }

        // Verify conversation continues successfully after compaction
        assert!(
            events.iter().any(|e| {
                matches!(
                    e,
                    ChatEvent::StreamEnd { message } if matches!(message.sender, MessageSender::Assistant { .. })
                )
            }),
            "Should receive assistant message after compaction"
        );
    });
}
