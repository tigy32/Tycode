use tycode_core::{
    ai::mock::MockBehavior,
    chat::events::{ChatEvent, MessageSender},
};

mod fixture;

#[test]
fn test_coder_agent_requires_tool_use() {
    fixture::run_with_agent("coder", |mut fixture| async move {
        fixture.set_mock_behavior(MockBehavior::TextOnlyThenToolUse {
            remaining_text_responses: 2,
            tool_name: "complete_task".to_string(),
            tool_arguments: r#"{"success": true, "result": "Task completed"}"#.to_string(),
        });

        let events = fixture.step("Write a hello world function").await;

        let assistant_message_count = events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    ChatEvent::StreamEnd { message } if matches!(message.sender, MessageSender::Assistant { .. })
                )
            })
            .count();

        assert!(
            assistant_message_count >= 3,
            "Expected at least 3 assistant messages (2 text-only + 1 with tool use), got {}",
            assistant_message_count
        );
    });
}
