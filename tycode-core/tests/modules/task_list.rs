//! End-to-end tests for the TaskListModule.
//!
//! Tests verify:
//! - Initial TaskUpdate event on new session
//! - Task list renders in context messages with default state
//! - TaskUpdate events are emitted
//!
//! Note: ProposeTaskListTool and UpdateTaskListTool are not directly testable
//! through the Fixture because the default coder agent doesn't have them in
//! its allowed tools list. Those tools would need agent-level changes to test.

#[path = "../fixture.rs"]
mod fixture;

use fixture::{run, MockBehavior};
use tycode_core::ai::types::MessageRole;
use tycode_core::chat::events::ChatEvent;
use tycode_core::chat::events::EventSender;
use tycode_core::module::Module;
use tycode_core::modules::task_list::{TaskList, TaskListModule, TaskStatus, TaskWithStatus};

/// Helper to find the first TaskUpdate event in a list of events
fn find_task_update(events: &[ChatEvent]) -> Option<&TaskList> {
    events.iter().find_map(|e| match e {
        ChatEvent::TaskUpdate(task_list) => Some(task_list),
        _ => None,
    })
}

/// Count TaskUpdate events in event list
fn count_task_updates(events: &[ChatEvent]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, ChatEvent::TaskUpdate(_)))
        .count()
}

#[test]
fn test_initial_task_update_on_new_session() {
    run(|mut fixture| async move {
        // Send a simple message to trigger event collection
        fixture.set_mock_behavior(MockBehavior::Success);
        let events = fixture.step("hello").await;

        // The initial TaskUpdate is emitted in TaskListModule::new() before
        // any messages are processed. We verify the default state exists by checking
        // the AI request context contains the task list.
        let last_request = fixture
            .get_last_ai_request()
            .expect("Should have AI request");
        let context = last_request
            .messages
            .iter()
            .find(|m| m.role == MessageRole::User)
            .map(|m| m.content.text())
            .expect("Should have context");

        assert!(
            context.contains("Task List:"),
            "Context should contain task list. Got: {}",
            context
        );
        assert!(
            context.contains("Await user request"),
            "Context should contain default task. Got: {}",
            context
        );

        // Verify TaskUpdate event was emitted
        let task_update = find_task_update(&events);
        assert!(task_update.is_some(), "Should emit TaskUpdate event");
    })
}

#[test]
fn test_task_list_renders_in_context() {
    run(|mut fixture| async move {
        fixture.set_mock_behavior(MockBehavior::Success);
        fixture.step("What are my tasks?").await;

        let last_request = fixture
            .get_last_ai_request()
            .expect("Should have AI request");

        let context = last_request
            .messages
            .iter()
            .filter(|m| m.role == MessageRole::User)
            .last()
            .map(|m| m.content.text())
            .expect("Should have context");

        // Verify task list header, default task, and status markers
        assert!(
            context.contains("Task List:"),
            "Missing header. Got: {}",
            context
        );
        assert!(
            context.contains("Await user request"),
            "Missing default task. Got: {}",
            context
        );
        assert!(
            context.contains("[InProgress]"),
            "Missing InProgress marker. Got: {}",
            context
        );
        assert!(
            context.contains("[Pending]"),
            "Missing Pending marker. Got: {}",
            context
        );
    })
}

#[test]
fn test_manage_task_list_tool_replaces_task_list() {
    run(|mut fixture| async move {
        // Use manage_task_list to replace the entire task list
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "manage_task_list".to_string(),
            tool_arguments: serde_json::json!({
                "title": "Test Implementation",
                "tasks": [
                    { "description": "Write tests", "status": "completed" },
                    { "description": "Fix bugs", "status": "in_progress" },
                    { "description": "Review code", "status": "pending" }
                ]
            })
            .to_string(),
        });
        let events = fixture.step("Set up my task list").await;

        // Count TaskUpdate events:
        // - 1st: Initial TaskUpdate from TaskListModule::new()
        // - 2nd: TaskUpdate after manage_task_list tool replaces task list
        // If tool is broken/no-op, only 1 TaskUpdate will exist
        let update_count = count_task_updates(&events);
        assert!(
            update_count >= 2,
            "Should have at least 2 TaskUpdate events (initial + after tool). Got: {}",
            update_count
        );

        // Verify the new task list appears in the context of the second AI call
        // (after tool execution, the AI is called again with updated context)
        let last_request = fixture
            .get_last_ai_request()
            .expect("Should have AI request");
        let context = last_request
            .messages
            .iter()
            .filter(|m| m.role == MessageRole::User)
            .last()
            .map(|m| m.content.text())
            .expect("Should have context");

        // Verify new title and tasks appear in context
        assert!(
            context.contains("Test Implementation"),
            "Context should contain new title. Got: {}",
            context
        );
        assert!(
            context.contains("Write tests"),
            "Context should contain first task. Got: {}",
            context
        );
        assert!(
            context.contains("[Completed]"),
            "Context should show completed status. Got: {}",
            context
        );
    })
}

#[test]
fn test_session_state_save_load_roundtrip() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        // Create module and replace with custom task list
        let (event_sender, mut rx) = EventSender::new();
        let module = TaskListModule::new(event_sender);

        // Drain initial TaskUpdate event
        let _ = rx.recv().await;

        module.replace(
            "Test Session".to_string(),
            vec![
                TaskWithStatus {
                    description: "First task".to_string(),
                    status: TaskStatus::Completed,
                },
                TaskWithStatus {
                    description: "Second task".to_string(),
                    status: TaskStatus::InProgress,
                },
            ],
        );

        // Save state
        let session_state = module.session_state().expect("Should have session state");
        assert_eq!(session_state.key(), "task_list");
        let saved = session_state.save();

        // Create new module and load saved state
        let (event_sender2, _rx2) = EventSender::new();
        let module2 = TaskListModule::new(event_sender2);
        let session_state2 = module2.session_state().expect("Should have session state");
        session_state2.load(saved).expect("Load should succeed");

        // Verify state was restored
        let restored = module2.get();
        assert_eq!(restored.title, "Test Session");
        assert_eq!(restored.tasks.len(), 2);
        assert_eq!(restored.tasks[0].description, "First task");
        assert_eq!(restored.tasks[0].status, TaskStatus::Completed);
        assert_eq!(restored.tasks[1].description, "Second task");
        assert_eq!(restored.tasks[1].status, TaskStatus::InProgress);
    });
}

#[test]
fn test_empty_task_list_rejected() {
    run(|mut fixture| async move {
        // Try to set an empty task list
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "manage_task_list".to_string(),
            tool_arguments: serde_json::json!({
                "title": "Empty List",
                "tasks": []
            })
            .to_string(),
        });
        let events = fixture.step("Set empty task list").await;

        // Tool should have failed validation - check we still have only 1 TaskUpdate
        // (the initial one, not a second one from successful tool execution)
        let update_count = count_task_updates(&events);
        assert_eq!(
            update_count, 1,
            "Should only have initial TaskUpdate (empty list rejected). Got: {}",
            update_count
        );

        // Verify context still shows default task list, not the empty one
        let last_request = fixture
            .get_last_ai_request()
            .expect("Should have AI request");
        let context = last_request
            .messages
            .iter()
            .filter(|m| m.role == MessageRole::User)
            .last()
            .map(|m| m.content.text())
            .expect("Should have context");

        assert!(
            context.contains("Await user request"),
            "Context should still show default task (empty list rejected). Got: {}",
            context
        );
        assert!(
            !context.contains("Empty List"),
            "Context should NOT contain rejected title. Got: {}",
            context
        );
    })
}
