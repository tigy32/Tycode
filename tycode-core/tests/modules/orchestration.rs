//! Orchestration workflow simulation tests.
//!
//! Drives the mechanical builder and swarm agents end-to-end through the chat
//! actor with a scripted mock provider, validating that workflow phase
//! transitions produce the expected agent sequence.

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

fn fail_task(result: &str) -> MockBehavior {
    MockBehavior::ToolUse {
        tool_name: "complete_task".to_string(),
        tool_arguments: serde_json::json!({ "result": result, "success": false }).to_string(),
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
fn test_builder_runs_plan_implement_review_pipeline() {
    fixture::run_with_agent("builder", |mut fixture| async move {
        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                complete_task("THE PLAN: add a widget to widgets.rs"),
                complete_task("implemented the widget"),
                complete_task("review approved"),
                MockBehavior::Success,
            ],
        });

        let events = fixture.step("add a widget").await;

        let requests = fixture.get_all_ai_requests();
        assert_eq!(
            requests.len(),
            3,
            "expected exactly planner + coder + review requests, got {}",
            requests.len()
        );

        assert!(
            requests[0].system_prompt.contains("PLANNER"),
            "first request should go to the planner"
        );
        assert!(
            requests[1]
                .system_prompt
                .contains("executing assigned coding tasks"),
            "second request should go to the coder"
        );
        let coder_input: String = requests[1]
            .messages
            .iter()
            .map(|m| m.content.text())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            coder_input.contains("THE PLAN"),
            "coder must receive the plan. Input: {coder_input}"
        );
        assert!(
            requests[2].system_prompt.contains("review sub-agent"),
            "third request should go to the review agent"
        );

        let text = all_event_text(&events);
        assert!(
            text.contains("Task completed [success=true]"),
            "builder completion should cascade to the root. Events: {text}"
        );
        assert!(
            text.contains("implemented the widget") && text.contains("review approved"),
            "final result should carry implementation and review. Events: {text}"
        );
    });
}

#[test]
fn test_builder_planning_failure_fails_fast() {
    fixture::run_with_agent("builder", |mut fixture| async move {
        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                fail_task("cannot plan: repository is empty"),
                MockBehavior::Success,
            ],
        });

        let events = fixture.step("add a widget").await;

        let requests = fixture.get_all_ai_requests();
        assert_eq!(
            requests.len(),
            1,
            "planning failure must not spawn a coder, got {} requests",
            requests.len()
        );

        let text = all_event_text(&events);
        assert!(
            text.contains("Task completed [success=false]") && text.contains("Planning failed"),
            "builder should fail fast on planning failure. Events: {text}"
        );
    });
}

const SWARM_PLAN: &str = r#"## Plan
Split the change across two files.

```json
{"assignments": [
  {"file": "src/a.rs", "instructions": "define the struct", "shared_surfaces": ["pub struct X { y: u32 }"]},
  {"file": "src/b.rs", "instructions": "consume the struct", "shared_surfaces": ["pub struct X { y: u32 }"]}
]}
```"#;

#[test]
fn test_swarm_fans_out_workers_and_integration_review() {
    fixture::run_with_agent("swarm", |mut fixture| async move {
        // Queue: planner, then 4 fan-out completions (2 workers + 2 pair
        // reviews, order nondeterministic under concurrency so all
        // identical), then the integration review.
        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                complete_task(SWARM_PLAN),
                complete_task("worker done"),
                complete_task("worker done"),
                complete_task("worker done"),
                complete_task("worker done"),
                complete_task("integration review approved"),
                MockBehavior::Success,
            ],
        });

        let events = fixture.step("make the wide change").await;

        let requests = fixture.get_all_ai_requests();
        assert_eq!(
            requests.len(),
            6,
            "expected planner + 2 workers + 2 pair reviews + integration review, got {}",
            requests.len()
        );

        let text = all_event_text(&events);
        assert!(
            text.contains("Fan-out: launching 2 worker(s)"),
            "swarm should fan out two workers. Events: {text}"
        );
        assert!(
            text.contains("Task completed [success=true]") && text.contains("Swarm complete"),
            "swarm should complete after integration approval. Events: {text}"
        );
    });
}

#[test]
fn test_swarm_degrades_to_sequential_coder_without_assignments() {
    fixture::run_with_agent("swarm", |mut fixture| async move {
        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                complete_task("a plan with no assignment block"),
                complete_task("implemented sequentially"),
                MockBehavior::Success,
            ],
        });

        let events = fixture.step("small change").await;

        let requests = fixture.get_all_ai_requests();
        assert_eq!(
            requests.len(),
            2,
            "unparseable plan should degrade to planner + single coder, got {}",
            requests.len()
        );
        assert!(
            requests[1]
                .system_prompt
                .contains("executing assigned coding tasks"),
            "degraded path should use the coder agent"
        );

        let text = all_event_text(&events);
        assert!(
            text.contains("Task completed [success=true]"),
            "degraded swarm should still complete. Events: {text}"
        );
    });
}
