use tycode_core::ai::mock::MockBehavior;
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
                    ChatEvent::StreamEnd { message } if matches!(message.sender, MessageSender::Assistant { .. })
                )
            }),
            "Should receive assistant message"
        );

        // Second message: reconfigure mock to return a tool use and verify
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "bash".to_string(),
            tool_arguments: r#"{"command": "echo ok"}"#.to_string(),
        });

        let events = fixture.step("Run a command").await;

        assert!(
            events.iter().any(|e| {
                matches!(
                    e,
                    ChatEvent::StreamEnd { message } if matches!(message.sender, MessageSender::Assistant { .. })
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
            second_tool_name: "bash".to_string(),
            second_tool_arguments: r#"{"command": "echo recovered"}"#.to_string(),
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
                    ChatEvent::StreamEnd { message } if matches!(message.sender, MessageSender::Assistant { .. })
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

#[test]
fn tool_use_without_text_emits_stream_start_before_stream_end() {
    fixture::run(|mut fixture| async move {
        fixture.set_mock_behavior(MockBehavior::ToolUseNoTextThenSuccess {
            tool_name: "bash".to_string(),
            tool_arguments: r#"{"command": "echo no-text"}"#.to_string(),
        });

        let events = fixture.step("Use a tool").await;

        let mut open = false;
        let mut pairs = 0u32;
        for event in &events {
            match event {
                ChatEvent::StreamStart { .. } => {
                    assert!(
                        !open,
                        "StreamStart while previous stream still open; events={events:?}"
                    );
                    open = true;
                }
                ChatEvent::StreamEnd { .. } => {
                    assert!(
                        open,
                        "StreamEnd arrived without preceding StreamStart; events={events:?}"
                    );
                    open = false;
                    pairs += 1;
                }
                _ => {}
            }
        }

        assert!(
            pairs >= 1,
            "expected at least one StreamStart/StreamEnd pair; events={events:?}"
        );
        assert!(
            !open,
            "stream left open (StreamStart without StreamEnd); events={events:?}"
        );
    });
}

/// The VSCode extension spawns the subprocess with zero workspace roots when
/// no folder is open (e.g. to load settings). The actor must build and answer
/// protocol requests instead of panicking in ExecutionModule construction.
#[test]
fn test_actor_builds_and_serves_settings_without_workspace_roots() {
    use std::sync::Arc;
    use tycode_core::ai::mock::MockProvider;
    use tycode_core::chat::actor::ChatActorBuilder;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");
    let local = tokio::task::LocalSet::new();

    runtime.block_on(local.run_until(async {
        let tycode_dir = tempfile::TempDir::new().unwrap();

        let (actor, mut event_rx) =
            ChatActorBuilder::tycode(Vec::new(), Some(tycode_dir.path().to_path_buf()), None)
                .expect("builder must tolerate zero workspace roots")
                .provider(Arc::new(MockProvider::new(MockBehavior::Success)))
                .ephemeral()
                .build()
                .expect("actor must build without workspace roots");

        actor.get_settings().unwrap();

        let settings = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while let Some(event) = event_rx.recv().await {
                if let ChatEvent::Settings(settings) = event {
                    return Some(settings);
                }
            }
            None
        })
        .await
        .expect("settings must arrive before the timeout");
        assert!(
            settings.is_some(),
            "a rootless actor must still serve GetSettings"
        );
    }));
}
