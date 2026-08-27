//! Agent-stack simulation tests.
//!
//! Covers sub-agent lifecycle behaviour that survives without structured
//! orchestration events: progress-message suppression, discarding live
//! sub-agents on reset, and root-agent selection.

#[path = "../fixture.rs"]
mod fixture;

use fixture::MockBehavior;
use tycode_core::chat::events::ChatEvent;

fn complete_task(result: &str) -> MockBehavior {
    MockBehavior::ToolUse {
        tool_name: "complete_task".to_string(),
        tool_arguments: serde_json::json!({ "result": result, "success": true }).to_string(),
    }
}

fn all_event_text(events: &[ChatEvent]) -> String {
    events
        .iter()
        .filter_map(|event| match event {
            ChatEvent::MessageAdded(message) => Some(message.content.clone()),
            ChatEvent::Error(message) => Some(message.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn test_progress_messages_can_be_suppressed() {
    fixture::run_with_agent("coordinator", |mut fixture| async move {
        fixture
            .update_settings(|settings| {
                settings.orchestration_progress_messages = false;
            })
            .await;
        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                MockBehavior::ToolUse {
                    tool_name: "spawn_agent".to_string(),
                    tool_arguments: r#"{"agent_type": "coder", "task": "add a widget"}"#
                        .to_string(),
                },
                complete_task("implemented"),
                MockBehavior::Success,
            ],
        });

        let events = fixture.step("add a widget").await;

        let text = all_event_text(&events);
        assert!(
            !text.contains("🔄") && !text.contains("✅") && !text.contains("🔍"),
            "progress strings must be suppressed. Events: {text}"
        );
    });
}

/// A sub-agent left live on the stack must not survive /clear: the next turn
/// has to run under the root agent again.
#[test]
fn test_clear_discards_live_sub_agents() {
    fixture::run_with_agent("tycode", |mut fixture| async move {
        // The spawned one_shot asks the user a question, which stops the turn
        // without completing the agent, leaving it on the stack.
        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                MockBehavior::ToolUse {
                    tool_name: "spawn_agent".to_string(),
                    tool_arguments: r#"{"agent_type": "one_shot", "task": "hold the stack"}"#
                        .to_string(),
                },
                MockBehavior::ToolUse {
                    tool_name: "ask_user_question".to_string(),
                    tool_arguments: r#"{"question": "should I proceed?"}"#.to_string(),
                },
                MockBehavior::Success,
            ],
        });
        fixture.step("delegate and pause").await;

        let events = fixture.step("/clear").await;
        assert!(
            events
                .iter()
                .any(|event| matches!(event, ChatEvent::ConversationCleared)),
            "/clear must emit ConversationCleared"
        );

        fixture.clear_captured_requests();
        fixture.set_mock_behavior(MockBehavior::Success);
        fixture.step("who is running now?").await;

        let requests = fixture.get_all_ai_requests();
        let system_prompt = &requests
            .first()
            .expect("the post-clear turn must issue a request")
            .system_prompt;
        assert!(
            system_prompt.contains("You are the Tycode agent"),
            "the turn after /clear must run under the root agent, not the \
             discarded one_shot sub-agent"
        );
    });
}

/// Drains events after a non-step actor message (e.g. SetRootAgent) until
/// the turn's TypingStatusChanged(false), mirroring `Session::step`.
async fn drain_turn(fixture: &mut fixture::Session) -> Vec<ChatEvent> {
    let mut events = Vec::new();
    while let Some(event) = fixture.event_rx.recv().await {
        if matches!(event, ChatEvent::TypingStatusChanged(false)) {
            break;
        }
        if !matches!(event, ChatEvent::TypingStatusChanged(_)) {
            events.push(event);
        }
    }
    events
}

#[test]
fn test_set_root_agent_applies_before_first_message() {
    // Tyde's flow: connect (SessionStarted), pick the orchestration mode via
    // the typed SetRootAgent command, then send the first user message. The
    // first turn must run under the selected agent.
    fixture::run_with_agent("tycode", |mut fixture| async move {
        fixture.set_mock_behavior(MockBehavior::Success);

        fixture
            .actor
            .set_root_agent("coordinator".to_string())
            .unwrap();
        let events = drain_turn(&mut fixture).await;
        assert!(
            events.iter().any(|event| matches!(
                event,
                ChatEvent::RootAgentChanged { agent } if agent == "coordinator"
            )),
            "the switch must be acknowledged with a typed event. Events: {events:?}"
        );

        fixture.step("add a widget").await;
        let requests = fixture.get_all_ai_requests();
        assert!(
            requests[0].system_prompt.contains("primary coordinator"),
            "the first user turn must run under the newly selected root agent"
        );
    });
}

#[test]
fn test_set_root_agent_rejects_unknown_agent() {
    fixture::run_with_agent("tycode", |mut fixture| async move {
        fixture.set_mock_behavior(MockBehavior::Success);

        fixture
            .actor
            .set_root_agent("does_not_exist".to_string())
            .unwrap();
        let events = drain_turn(&mut fixture).await;
        assert!(
            events.iter().any(|event| matches!(
                event,
                ChatEvent::Error(message) if message.contains("Unknown agent type 'does_not_exist'")
            )),
            "unknown agents must be rejected with a typed error. Events: {events:?}"
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, ChatEvent::RootAgentChanged { .. })),
            "a rejected switch must not acknowledge a root change"
        );

        // The session still works under the original root.
        fixture.clear_captured_requests();
        fixture.step("hello").await;
        let requests = fixture.get_all_ai_requests();
        assert!(
            requests
                .first()
                .is_some_and(|request| request.system_prompt.contains("You are the Tycode agent")),
            "the original conversational root must remain active"
        );
    });
}

#[test]
fn test_set_root_agent_preserves_conversation() {
    fixture::run_with_agent("tycode", |mut fixture| async move {
        fixture.set_mock_behavior(MockBehavior::Success);
        fixture.step("remember the magic word is xyzzy").await;

        fixture
            .actor
            .set_root_agent("one_shot".to_string())
            .unwrap();
        drain_turn(&mut fixture).await;

        fixture.step("what was the magic word?").await;
        let request = fixture
            .get_last_ai_request()
            .expect("the switched root converses");
        let history: String = request
            .messages
            .iter()
            .map(|m| m.content.text())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            history.contains("xyzzy"),
            "the root conversation must survive the agent switch"
        );
    });
}
