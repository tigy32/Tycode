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
fn test_coder_cannot_spawn_itself() {
    use tycode_core::chat::ChatEvent;

    fixture::run_with_agent("coder", |mut fixture| async move {
        // Coder tries to spawn another coder - should fail
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "spawn_agent".to_string(),
            tool_arguments: r#"{"agent_type": "coder", "task": "do something"}"#.to_string(),
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
    });
}
