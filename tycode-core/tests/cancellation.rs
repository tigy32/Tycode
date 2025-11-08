use tycode_core::ai::mock::MockBehavior;
use tycode_core::chat::events::{ChatEvent, MessageSender};
use tycode_core::security::SecurityMode;

mod fixture;

use tokio::time::Duration;

#[test]
fn test_cancel_with_pending_tool_preserves_conversation() {
    fixture::run(|mut fixture| async move {
        // This test validates the core bug fix:
        // When we cancel while a tool might be pending, the conversation should remain valid

        // Enable all security modes to allow tool execution
        fixture
            .update_settings(|settings| {
                settings.security.mode = SecurityMode::All;
            })
            .await;

        let workspace_path = fixture.workspace_path();
        let workspace_name = workspace_path.file_name().unwrap().to_str().unwrap();
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "run_build_test".to_string(),
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
        loop {
            match fixture.event_rx.recv().await {
                Some(ChatEvent::ToolRequest(_)) => {
                    // Tool execution has started - cancel immediately
                    fixture.actor.cancel().unwrap();
                    break;
                }
                Some(_) => continue,
                None => panic!("Event channel closed before ToolRequest"),
            }
        }

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

        // Verify we got OperationCancelled
        assert!(
            events
                .iter()
                .any(|e| matches!(e, ChatEvent::OperationCancelled { .. })),
            "Should receive OperationCancelled event"
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
                Some(ChatEvent::MessageAdded(msg))
                    if matches!(msg.sender, MessageSender::Assistant { .. }) =>
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
        // Enable all security modes to allow tool execution
        fixture
            .update_settings(|settings| {
                settings.security.mode = SecurityMode::All;
            })
            .await;

        let workspace_path = fixture.workspace_path();
        let workspace_name = workspace_path.file_name().unwrap().to_str().unwrap();

        // First cancellation
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "run_build_test".to_string(),
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
            tool_name: "run_build_test".to_string(),
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
                Some(ChatEvent::MessageAdded(msg))
                    if matches!(msg.sender, MessageSender::Assistant { .. }) =>
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
                Some(ChatEvent::MessageAdded(msg))
                    if matches!(msg.sender, MessageSender::Assistant { .. }) =>
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

        // Cancel while idle (should just send OperationCancelled event)
        fixture.actor.cancel().unwrap();

        // Drain the cancellation event
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
                if let ChatEvent::MessageAdded(msg) = event {
                    if matches!(msg.sender, MessageSender::Assistant { .. }) {
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
        // Enable all security modes to allow tool execution
        fixture
            .update_settings(|settings| {
                settings.security.mode = SecurityMode::All;
            })
            .await;

        let workspace_path = fixture.workspace_path();
        let workspace_name = workspace_path.file_name().unwrap().to_str().unwrap();
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "run_build_test".to_string(),
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
                Some(ChatEvent::MessageAdded(msg))
                    if matches!(msg.sender, MessageSender::Assistant { .. }) =>
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
