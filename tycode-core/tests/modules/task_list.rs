//! End-to-end tests for the TaskListModule.
//!
//! Tests verify:
//! - Initial TaskUpdate event on new session
//! - Empty task lists are omitted from context
//! - TaskUpdate events are emitted
//!
//! Note: ProposeTaskListTool and UpdateTaskListTool are not directly testable
//! through the Fixture because the default coder agent doesn't have them in
//! its allowed tools list. Those tools would need agent-level changes to test.

#[path = "../fixture.rs"]
mod fixture;

use fixture::{run, MockBehavior};
use tycode_core::ai::types::{ContentBlock, MessageRole};
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
        // any messages are processed, but it must not invent work for the model.
        let last_request = fixture
            .get_last_ai_request()
            .expect("Should have AI request");
        let has_task_context = last_request
            .messages
            .iter()
            .any(|m| m.content.text().contains("Task List:"));
        assert!(!has_task_context, "empty task list must be omitted");

        // Verify TaskUpdate event was emitted
        let task_update = find_task_update(&events).expect("Should emit TaskUpdate event");
        assert!(task_update.title.is_empty());
        assert!(task_update.tasks.is_empty());
    })
}

#[test]
fn test_default_task_list_is_neutral() {
    let task_list = TaskList::default();
    assert!(task_list.title.is_empty());
    assert!(task_list.tasks.is_empty());
}

#[test]
fn test_context_refresh_precedes_trailing_tool_exchange() {
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
            .find_map(|m| {
                let text = m.content.text();
                text.contains("Task List: Test Implementation")
                    .then_some(text)
            })
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

        let message_kinds: Vec<_> = last_request
            .messages
            .iter()
            .map(|message| {
                if message
                    .content
                    .blocks()
                    .iter()
                    .any(|block| matches!(block, ContentBlock::ToolUse(_)))
                {
                    "tool_use"
                } else if message
                    .content
                    .blocks()
                    .iter()
                    .any(|block| matches!(block, ContentBlock::ToolResult(_)))
                {
                    "tool_result"
                } else if message
                    .content
                    .text()
                    .contains("Task List: Test Implementation")
                {
                    "context"
                } else {
                    "other"
                }
            })
            .collect();
        let context_index = message_kinds
            .iter()
            .position(|kind| *kind == "context")
            .expect("request should contain refreshed context");
        assert_eq!(
            &message_kinds[context_index..],
            &["context", "tool_use", "tool_result"],
            "refreshed context must precede the trailing tool exchange"
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

        // Verify neither rejected state nor invented default work is injected.
        let last_request = fixture
            .get_last_ai_request()
            .expect("Should have AI request");
        let request_text = last_request
            .messages
            .iter()
            .map(|m| m.content.text())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            !request_text.contains("Await user request"),
            "Request should not contain invented default work. Got: {}",
            request_text
        );
        assert!(
            !request_text.contains("Empty List"),
            "Context should NOT contain rejected title. Got: {}",
            request_text
        );
    })
}
