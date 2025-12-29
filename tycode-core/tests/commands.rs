use tycode_core::chat::events::{ChatEvent, MessageSender};

mod fixture;

#[test]
fn test_debug_ui_command_works() {
    fixture::run(|mut fixture| async move {
        let events = fixture.step("/debug_ui").await;

        assert!(!events.is_empty(), "Should receive events from debug_ui");

        let has_test_events = events.iter().any(|e| {
            matches!(
                e,
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System)
            )
        });

        assert!(has_test_events, "debug_ui should return test events");
    });
}

/// Regression test for /provider add bedrock panic when profile is missing.
/// Bug: `parts[4]` was accessed without bounds check after only validating `parts.len() >= 4`.
/// The command `/provider add test_alias bedrock` has 4 parts but needs 5 for the profile.
#[test]
fn test_provider_add_bedrock_missing_profile_returns_error_not_panic() {
    fixture::run(|mut fixture| async move {
        // This command would panic with the buggy code because it accesses parts[4]
        // without checking if it exists (parts.len() is 4, valid indices are 0-3)
        let events = fixture.step("/provider add test_alias bedrock").await;

        let messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) => Some((msg.sender.clone(), msg.content.clone())),
                _ => None,
            })
            .collect();

        // Should get an error message about missing profile, not a panic
        let has_usage_error = messages.iter().any(|(sender, content)| {
            matches!(sender, MessageSender::Error) && content.contains("Usage:")
        });

        assert!(
            has_usage_error,
            "Should return usage error when profile is missing. Got messages: {:?}",
            messages
        );
    });
}

/// Regression test for spawn_coder failing with empty AgentCatalog.
/// Bug: tools.rs passed `Arc::new(AgentCatalog::new())` (empty) to ToolRegistry
/// instead of `state.agent_catalog.clone()` (populated with registered agents).
/// This caused spawn_coder to fail with "Failed to create coder agent".
#[test]
fn test_spawn_coder_with_populated_catalog() {
    use tokio::time::{timeout, Duration};

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");

    let local = tokio::task::LocalSet::new();

    runtime.block_on(local.run_until(async {
        // Use BehaviorQueue with ToolUse (not ToolUseThenSuccess) to preserve the queue.
        // ToolUseThenSuccess calls set_behavior which destroys the queue.
        let mut fixture = fixture::Fixture::with_agent_and_behavior(
            "coordinator",
            fixture::MockBehavior::BehaviorQueue {
                behaviors: vec![
                    // Coordinator's AI call - spawn_coder
                    fixture::MockBehavior::ToolUse {
                        tool_name: "spawn_coder".to_string(),
                        tool_arguments: r#"{"task": "test task"}"#.to_string(),
                    },
                    // Coder's AI call - complete_task to finish immediately
                    fixture::MockBehavior::ToolUse {
                        tool_name: "complete_task".to_string(),
                        tool_arguments: r#"{"success": true, "result": "done"}"#.to_string(),
                    },
                    // Coordinator after coder completes - complete_task to finish
                    fixture::MockBehavior::ToolUse {
                        tool_name: "complete_task".to_string(),
                        tool_arguments: r#"{"success": true, "result": "all done"}"#.to_string(),
                    },
                ],
            },
        );

        let events = timeout(Duration::from_secs(30), fixture.step("Do something"))
            .await
            .expect("Test timed out");

        // Check that we don't get the "Failed to create coder agent" error
        let has_catalog_error = events.iter().any(|e| match e {
            ChatEvent::MessageAdded(msg) => msg.content.contains("Failed to create coder agent"),
            ChatEvent::ToolExecutionCompleted { error, .. } => error
                .as_ref()
                .map_or(false, |e| e.contains("Failed to create coder agent")),
            _ => false,
        });

        assert!(
            !has_catalog_error,
            "spawn_coder should not fail with 'Failed to create coder agent'. \
             This error indicates the AgentCatalog is empty."
        );
    }));
}

#[test]
fn test_debug_ui_not_in_help() {
    fixture::run(|mut fixture| async move {
        let events = fixture.step("/help").await;

        let help_content = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            !help_content.contains("debug_ui"),
            "Help output should not contain debug_ui command. Found: {}",
            help_content
        );
    });
}
