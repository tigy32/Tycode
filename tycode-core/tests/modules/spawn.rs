//! Spawn module simulation tests.
//!
//! Tests for `src/spawn/`
//!
//! These tests validate the spawn_agent tool:
//! 1. Coordinator and Coder have access to spawn_agent
//! 2. Recon (leaf agent) does not have spawn_agent
//! 3. spawn_agent description includes available agent types

#[path = "../fixture.rs"]
mod fixture;

use fixture::MockBehavior;

fn complete_task_behavior() -> MockBehavior {
    MockBehavior::ToolUseThenSuccess {
        tool_name: "complete_task".to_string(),
        tool_arguments: r#"{"result": "done", "success": true}"#.to_string(),
    }
}

#[test]
fn test_coordinator_has_spawn_agent_tool() {
    fixture::run_with_agent("coordinator", |mut fixture| async move {
        fixture.set_mock_behavior(complete_task_behavior());
        fixture.step("Hello").await;

        let request = fixture
            .get_last_ai_request()
            .expect("Should have captured AI request");

        let has_spawn_agent = request.tools.iter().any(|t| t.name == "spawn_agent");
        assert!(has_spawn_agent, "Coordinator should have spawn_agent tool");
    });
}

#[test]
fn test_coder_has_spawn_agent_tool() {
    fixture::run_with_agent("coder", |mut fixture| async move {
        fixture.set_mock_behavior(complete_task_behavior());
        fixture.step("Hello").await;

        let request = fixture
            .get_last_ai_request()
            .expect("Should have captured AI request");

        let has_spawn_agent = request.tools.iter().any(|t| t.name == "spawn_agent");
        assert!(has_spawn_agent, "Coder should have spawn_agent tool");
    });
}

#[test]
fn test_recon_has_no_spawn_agent_tool() {
    fixture::run_with_agent("recon", |mut fixture| async move {
        fixture.set_mock_behavior(complete_task_behavior());
        fixture.step("Hello").await;

        let request = fixture
            .get_last_ai_request()
            .expect("Should have captured AI request");

        let has_spawn_agent = request.tools.iter().any(|t| t.name == "spawn_agent");
        assert!(
            !has_spawn_agent,
            "Recon should NOT have spawn_agent tool (leaf agent)"
        );
    });
}

#[test]
fn test_spawn_agent_description_includes_allowed_agents() {
    fixture::run_with_agent("coordinator", |mut fixture| async move {
        fixture.set_mock_behavior(complete_task_behavior());
        fixture.step("Hello").await;

        let request = fixture
            .get_last_ai_request()
            .expect("Should have captured AI request");

        let spawn_tool = request.tools.iter().find(|t| t.name == "spawn_agent");
        let description = spawn_tool.map(|t| t.description.as_str()).unwrap_or("");

        assert!(
            description.contains("coder") && description.contains("recon"),
            "spawn_agent description should list allowed agent types. Got: {}",
            description
        );
    });
}

#[test]
fn test_spawned_agent_receives_orientation_message() {
    fixture::run_with_agent("coordinator", |mut fixture| async move {
        // Coordinator spawns a coder - should include orientation message
        // Use BehaviorQueue: coordinator spawns coder, coder completes immediately
        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                // Coordinator spawns coder
                MockBehavior::ToolUse {
                    tool_name: "spawn_agent".to_string(),
                    tool_arguments: r#"{"agent_type": "coder", "task": "Write a test file"}"#
                        .to_string(),
                },
                // Spawned coder completes immediately
                MockBehavior::ToolUse {
                    tool_name: "complete_task".to_string(),
                    tool_arguments: r#"{"result": "done", "success": true}"#.to_string(),
                },
                // Back to coordinator
                MockBehavior::Success,
            ],
        });
        fixture.step("I need help writing tests").await;

        let all_requests = fixture.get_all_ai_requests();
        // First request is coordinator, second is the spawned coder
        assert!(
            all_requests.len() >= 2,
            "Expected at least 2 AI requests (coordinator + spawned coder), got {}",
            all_requests.len()
        );

        // The spawned coder's request should contain the orientation message
        let coder_request = &all_requests[1];
        let messages_text: String = coder_request
            .messages
            .iter()
            .map(|m| m.content.text())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            messages_text.contains("AGENT TRANSITION"),
            "Spawned agent should receive orientation message with AGENT TRANSITION marker. Messages: {}",
            messages_text
        );

        assert!(
            messages_text.contains("sub-agent"),
            "Orientation message should mention sub-agent role. Messages: {}",
            messages_text
        );

        assert!(
            messages_text.contains("complete_task"),
            "Orientation message should mention complete_task. Messages: {}",
            messages_text
        );

        // Verify the task comes after the orientation
        let orientation_pos = messages_text.find("AGENT TRANSITION");
        let task_pos = messages_text.find("Write a test file");
        assert!(
            orientation_pos < task_pos,
            "Orientation message should appear before the task. Orientation at {:?}, task at {:?}",
            orientation_pos,
            task_pos
        );
    });
}

#[test]
fn test_coder_cannot_spawn_itself() {
    use tycode_core::ai::ContentBlock;
    use tycode_core::chat::ChatEvent;

    fixture::run_with_agent("coder", |mut fixture| async move {
        // Coder tries to spawn another coder - should fail
        // Use BehaviorQueue: coder tries to spawn (fails), then completes after error feedback
        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                // Coder tries to spawn coder (will fail)
                MockBehavior::ToolUse {
                    tool_name: "spawn_agent".to_string(),
                    tool_arguments: r#"{"agent_type": "coder", "task": "do something"}"#
                        .to_string(),
                },
                // After error feedback, coder completes
                MockBehavior::ToolUse {
                    tool_name: "complete_task".to_string(),
                    tool_arguments: r#"{"result": "failed to spawn", "success": false}"#
                        .to_string(),
                },
            ],
        });
        let events = fixture.step("I need help with a task").await;

        // Check that the spawn was rejected with an error
        let has_self_spawn_error = events.iter().any(|e| {
            if let ChatEvent::ToolExecutionCompleted {
                tool_name,
                success,
                error,
                ..
            } = e
            {
                tool_name == "spawn_agent" && (!success || error.is_some())
            } else {
                false
            }
        });

        assert!(
            has_self_spawn_error,
            "Coder should not be able to spawn itself. Events: {:?}",
            events
        );

        // Verify the actor continued and the error was fed back to the model.
        // If an anyhow error crashed the actor, there won't be a second AI request.
        let all_requests = fixture.get_all_ai_requests();
        assert!(
            all_requests.len() >= 2,
            "Expected at least 2 AI requests (tool use + tool result), got {}",
            all_requests.len()
        );

        // The second request should contain the tool error result fed back to the model
        let second_request = &all_requests[1];
        let has_error_tool_result = second_request.messages.iter().any(|msg| {
            msg.content.blocks().iter().any(|block| {
                if let ContentBlock::ToolResult(result) = block {
                    result.is_error
                } else {
                    false
                }
            })
        });

        assert!(
            has_error_tool_result,
            "Second AI request should contain tool_result with is_error=true. Messages: {:?}",
            second_request.messages
        );
    });
}
