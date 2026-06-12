use std::collections::HashSet;
use std::pin::Pin;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};

use tycode_core::ai::mock::MockBehavior;
use tycode_core::ai::model::Model;
use tycode_core::ai::{
    AiError, AiProvider, Content, ConversationRequest, ConversationResponse, Cost, StopReason,
    StreamEvent, TokenUsage,
};
use tycode_core::chat::actor::ChatActorBuilder;
use tycode_core::chat::events::{ChatEvent, MessageSender};

mod fixture;

use tokio::time::Duration;
use tokio_stream::Stream;

#[derive(Clone, Default)]
struct SlowStreamingProvider {
    call_count: Arc<AtomicUsize>,
}

impl SlowStreamingProvider {
    fn response(text: &str) -> ConversationResponse {
        ConversationResponse {
            content: Content::text_only(text.to_string()),
            usage: TokenUsage::new(10, 10),
            stop_reason: StopReason::EndTurn,
        }
    }
}

#[async_trait::async_trait]
impl AiProvider for SlowStreamingProvider {
    fn name(&self) -> &'static str {
        "slow_streaming"
    }

    fn supported_models(&self) -> HashSet<Model> {
        HashSet::from([Model::None])
    }

    async fn converse(
        &self,
        _request: ConversationRequest,
    ) -> Result<ConversationResponse, AiError> {
        Ok(Self::response("response"))
    }

    fn get_cost(&self, _model: &Model) -> Cost {
        Cost::new(0.0, 0.0, 0.0, 0.0)
    }

    async fn converse_stream(
        &self,
        _request: ConversationRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, AiError>> + Send>>, AiError> {
        let call_index = self.call_count.fetch_add(1, Ordering::SeqCst);
        let stream = async_stream::stream! {
            if call_index == 0 {
                yield Ok(StreamEvent::TextDelta {
                    text: "partial slow response".to_string(),
                });
                tokio::time::sleep(Duration::from_secs(5)).await;
                yield Ok(StreamEvent::MessageComplete {
                    response: Self::response("slow response completed after cancellation"),
                });
            } else {
                yield Ok(StreamEvent::MessageComplete {
                    response: Self::response("response after cancellation"),
                });
            }
        };

        Ok(Box::pin(stream))
    }
}

#[test]
fn test_cancel_during_ai_streaming_response_is_handled_promptly() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");
    let local = tokio::task::LocalSet::new();

    runtime.block_on(local.run_until(async {
        let temp_dir = tempfile::TempDir::new().unwrap();
        let workspace_path = temp_dir.path().join("workspace");
        let root_dir = temp_dir.path().join(".tycode");
        std::fs::create_dir_all(&workspace_path).unwrap();
        std::fs::create_dir_all(&root_dir).unwrap();

        let provider = Arc::new(SlowStreamingProvider::default());
        let (actor, mut event_rx) =
            ChatActorBuilder::tycode(vec![workspace_path], Some(root_dir), None)
                .unwrap()
                .provider(provider)
                .ephemeral()
                .build()
                .unwrap();

        actor
            .send_message("start a slow streaming response".to_string())
            .unwrap();

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                match event_rx.recv().await {
                    Some(ChatEvent::StreamDelta { .. }) => break,
                    Some(_) => continue,
                    None => panic!("event channel closed before stream delta"),
                }
            }
        })
        .await
        .expect("slow provider should start streaming before cancellation");

        actor.cancel().unwrap();

        let cancellation_events = tokio::time::timeout(Duration::from_millis(700), async {
            let mut events = Vec::new();
            loop {
                match event_rx.recv().await {
                    Some(event) => {
                        let done = matches!(event, ChatEvent::TypingStatusChanged(false));
                        events.push(event);
                        if done {
                            break events;
                        }
                    }
                    None => panic!("event channel closed before cancellation completed"),
                }
            }
        })
        .await
        .expect("cancellation should complete promptly while an AI response stream is open");

        let cancellation_count = cancellation_events
            .iter()
            .filter(|event| matches!(event, ChatEvent::OperationCancelled { .. }))
            .count();
        assert_eq!(
            cancellation_count, 1,
            "expected exactly one OperationCancelled after cancelling an in-progress AI stream; events={cancellation_events:#?}"
        );

        actor
            .send_message("confirm actor still responds after cancellation".to_string())
            .unwrap();

        let follow_up_events = tokio::time::timeout(Duration::from_secs(1), async {
            let mut events = Vec::new();
            loop {
                match event_rx.recv().await {
                    Some(event) => {
                        let done = matches!(event, ChatEvent::TypingStatusChanged(false));
                        events.push(event);
                        if done {
                            break events;
                        }
                    }
                    None => panic!("event channel closed before follow-up completed"),
                }
            }
        })
        .await
        .expect("actor should accept a follow-up message after stream cancellation");

        assert!(
            follow_up_events.iter().any(|event| matches!(
                event,
                ChatEvent::StreamEnd { message }
                    if matches!(message.sender, MessageSender::Assistant { .. })
                        && message.content.contains("response after cancellation")
            )),
            "expected assistant response to follow-up after cancellation; events={follow_up_events:#?}"
        );
    }));
}

#[test]
fn test_cancel_with_pending_tool_preserves_conversation() {
    fixture::run(|mut fixture| async move {
        // This test validates the core bug fix:
        // When we cancel while a tool might be pending, the conversation should remain valid

        let workspace_path = fixture.workspace_path();
        let workspace_name = workspace_path.file_name().unwrap().to_str().unwrap();
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "bash".to_string(),
            tool_arguments: format!(
                r#"{{"command": "sleep 30", "timeout_seconds": 30, "working_directory": "/{}"}}"#,
                workspace_name
            ),
        });

        // Send message
        fixture.send_message("Run a long test");

        // Let the actor task run
        tokio::task::yield_now().await;

        // Wait for ToolRequest to guarantee tool execution has started
        let requested_tool = loop {
            match fixture.event_rx.recv().await {
                Some(ChatEvent::ToolRequest(request)) => {
                    // Tool execution has started - cancel immediately
                    let requested_tool = (request.tool_call_id, request.tool_name);
                    fixture.actor.cancel().unwrap();
                    break requested_tool;
                }
                Some(_) => continue,
                None => panic!("Event channel closed before ToolRequest"),
            }
        };

        // Collect all remaining events until typing stops
        let mut events = Vec::new();
        loop {
            match fixture.event_rx.recv().await {
                Some(event) => {
                    let done = matches!(event, ChatEvent::TypingStatusChanged(false));
                    events.push(event);
                    if done {
                        break;
                    }
                }
                None => break,
            }
        }

        let cancellation_count = events
            .iter()
            .filter(|e| matches!(e, ChatEvent::OperationCancelled { .. }))
            .count();
        assert_eq!(
            cancellation_count, 1,
            "Should receive exactly one OperationCancelled event"
        );
        assert!(
            events.iter().any(|event| matches!(
                event,
                ChatEvent::ToolExecutionCompleted {
                    tool_call_id,
                    tool_name,
                    ..
                } if tool_call_id == &requested_tool.0 && tool_name == &requested_tool.1
            )),
            "cancelled ToolRequest should be followed by ToolExecutionCompleted; requested={requested_tool:?}, events={events:#?}"
        );

        // KEY TEST: Send another message - this MUST work without 4xx error
        // BEFORE fix: Would get 4xx error if tools were pending
        // AFTER fix: Works because error results were added for any pending tools
        fixture.set_mock_behavior(MockBehavior::Success);
        fixture.send_message("What's 2+2?");

        // Collect events until assistant responds
        let mut got_assistant_response = false;
        loop {
            match fixture.event_rx.recv().await {
                Some(ChatEvent::StreamEnd { message })
                    if matches!(message.sender, MessageSender::Assistant { .. }) =>
                {
                    got_assistant_response = true;
                }
                Some(ChatEvent::TypingStatusChanged(false)) => break,
                Some(_) => continue,
                None => break,
            }
        }

        assert!(
            got_assistant_response,
            "Should successfully continue conversation after cancellation (proves no 4xx error)"
        );
    });
}

#[test]
fn test_multiple_cancellations_preserve_conversation() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();
        let workspace_name = workspace_path.file_name().unwrap().to_str().unwrap();

        // First cancellation
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "bash".to_string(),
            tool_arguments: format!(
                r#"{{"command": "sleep 30", "timeout_seconds": 30, "working_directory": "/{}"}}"#,
                workspace_name
            ),
        });
        fixture.send_message("First request");
        tokio::task::yield_now().await;

        // Wait for ToolRequest
        loop {
            match fixture.event_rx.recv().await {
                Some(ChatEvent::ToolRequest(_)) => {
                    fixture.actor.cancel().unwrap();
                    break;
                }
                Some(_) => continue,
                None => panic!("Event channel closed"),
            }
        }

        // Wait for typing to stop
        loop {
            match fixture.event_rx.recv().await {
                Some(ChatEvent::TypingStatusChanged(false)) => break,
                Some(_) => continue,
                None => break,
            }
        }

        // Second cancellation
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "bash".to_string(),
            tool_arguments: format!(
                r#"{{"command": "sleep 30", "timeout_seconds": 30, "working_directory": "/{}"}}"#,
                workspace_name
            ),
        });
        fixture.send_message("Second request");
        tokio::task::yield_now().await;

        // Wait for ToolRequest
        loop {
            match fixture.event_rx.recv().await {
                Some(ChatEvent::ToolRequest(_)) => {
                    fixture.actor.cancel().unwrap();
                    break;
                }
                Some(_) => continue,
                None => panic!("Event channel closed"),
            }
        }

        // Wait for typing to stop
        loop {
            match fixture.event_rx.recv().await {
                Some(ChatEvent::TypingStatusChanged(false)) => break,
                Some(_) => continue,
                None => break,
            }
        }

        // Final message should work
        fixture.set_mock_behavior(MockBehavior::Success);
        fixture.send_message("Final message");

        // Collect events until assistant responds
        let mut got_assistant_response = false;
        loop {
            match fixture.event_rx.recv().await {
                Some(ChatEvent::StreamEnd { message })
                    if matches!(message.sender, MessageSender::Assistant { .. }) =>
                {
                    got_assistant_response = true;
                }
                Some(ChatEvent::TypingStatusChanged(false)) => break,
                Some(_) => continue,
                None => break,
            }
        }

        assert!(
            got_assistant_response,
            "Should handle multiple cancellations and maintain valid conversation"
        );
    });
}

#[test]
fn test_cancel_without_pending_tools() {
    fixture::run(|mut fixture| async move {
        // Test that cancelling without any tool uses doesn't break anything
        // This tests the edge case where we cancel after a message has completed
        fixture.set_mock_behavior(MockBehavior::Success);

        // Send a message that completes successfully
        let mut seen_first_response = false;
        fixture.send_message("Hello");

        loop {
            match fixture.event_rx.recv().await {
                Some(ChatEvent::StreamEnd { message })
                    if matches!(message.sender, MessageSender::Assistant { .. }) =>
                {
                    seen_first_response = true;
                }
                Some(ChatEvent::TypingStatusChanged(false)) => break,
                Some(_) => continue,
                None => break,
            }
        }

        assert!(
            seen_first_response,
            "First message should complete successfully"
        );

        // Cancel while idle should be a no-op; there is no active turn protocol to drop.
        fixture.actor.cancel().unwrap();

        // Drain any incidental events.
        tokio::time::timeout(Duration::from_millis(100), async {
            while let Some(_) = fixture.event_rx.recv().await {}
        })
        .await
        .ok();

        // The key test: verify the actor still works after cancelling while idle
        fixture.send_message("Continue");
        tokio::task::yield_now().await;

        let mut seen_second_response = false;
        let start = tokio::time::Instant::now();
        while start.elapsed() < Duration::from_millis(500) {
            if let Ok(Some(event)) =
                tokio::time::timeout(Duration::from_millis(100), fixture.event_rx.recv()).await
            {
                if let ChatEvent::StreamEnd { message } = event {
                    if matches!(message.sender, MessageSender::Assistant { .. }) {
                        seen_second_response = true;
                        break;
                    }
                }
            }
        }

        assert!(
            seen_second_response,
            "Should continue conversation even when cancelling without tools"
        );
    });
}

#[test]
fn test_cancel_error_results_mention_cancellation() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();
        let workspace_name = workspace_path.file_name().unwrap().to_str().unwrap();
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "bash".to_string(),
            tool_arguments: format!(
                r#"{{"command": "sleep 30", "timeout_seconds": 30, "working_directory": "/{}"}}"#,
                workspace_name
            ),
        });

        fixture.send_message("Run test");
        tokio::task::yield_now().await;

        // Wait for ToolRequest
        loop {
            match fixture.event_rx.recv().await {
                Some(ChatEvent::ToolRequest(_)) => {
                    fixture.actor.cancel().unwrap();
                    break;
                }
                Some(_) => continue,
                None => panic!("Event channel closed"),
            }
        }

        // Collect events until typing stops
        let mut events = Vec::new();
        loop {
            match fixture.event_rx.recv().await {
                Some(event) => {
                    let done = matches!(event, ChatEvent::TypingStatusChanged(false));
                    events.push(event);
                    if done {
                        break;
                    }
                }
                None => break,
            }
        }

        // Find any cancellation error messages
        let cancellation_errors: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::ToolExecutionCompleted { success, error, .. } if !success => {
                    error.as_ref()
                }
                _ => None,
            })
            .collect();

        // If there were any error completions, verify they mention cancellation
        for error_msg in cancellation_errors {
            assert!(
                error_msg.to_lowercase().contains("cancel"),
                "Error message should mention cancellation: {}",
                error_msg
            );
        }

        // Most importantly: conversation should still work
        fixture.set_mock_behavior(MockBehavior::Success);
        fixture.send_message("Continue");

        // Collect events until assistant responds
        let mut got_assistant_response = false;
        loop {
            match fixture.event_rx.recv().await {
                Some(ChatEvent::StreamEnd { message })
                    if matches!(message.sender, MessageSender::Assistant { .. }) =>
                {
                    got_assistant_response = true;
                }
                Some(ChatEvent::TypingStatusChanged(false)) => break,
                Some(_) => continue,
                None => break,
            }
        }

        assert!(got_assistant_response, "Conversation should remain valid");
    });
}
