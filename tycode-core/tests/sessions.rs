mod fixture;

use fixture::Fixture;
use tycode_core::ai::mock::MockBehavior;
use tycode_core::ai::types::{Content, Message, MessageRole};
use tycode_core::chat::events::{ChatEvent, MessageSender};

use tycode_core::persistence::{session::SessionData, storage};

fn is_assistant_message(event: &ChatEvent) -> bool {
    let ChatEvent::StreamEnd { message } = event else {
        return false;
    };
    matches!(message.sender, MessageSender::Assistant { .. })
}

#[test]
fn test_session_auto_save_after_ai_response() {
    fixture::run(|mut fixture| async move {
        let events = fixture.step("Hello test").await;

        let got_assistant_response = events.iter().any(|event| is_assistant_message(event));
        assert!(got_assistant_response, "Should receive assistant response");

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let sessions_dir = fixture.sessions_dir();
        let sessions = storage::list_sessions(Some(&sessions_dir)).unwrap();
        assert_eq!(
            sessions.len(),
            1,
            "Expected exactly one session to be saved"
        );

        let session_data = storage::load_session(&sessions[0].id, Some(&sessions_dir)).unwrap();
        assert!(
            session_data.messages.len() >= 2,
            "Expected at least user and assistant messages"
        );
    });
}

#[test]
fn test_sessions_list_command() {
    fixture::run(|mut fixture| async move {
        let sessions_dir = fixture.sessions_dir();

        let mut session1 = SessionData::new(
            "session_001".to_string(),
            vec![Message {
                role: MessageRole::User,
                content: Content::text_only("Test 1".to_string()),
            }],
            vec![],
        );
        session1.module_state.insert(
            "task_list".to_string(),
            serde_json::json!({
                "title": "Understand user requirements",
                "tasks": []
            }),
        );
        storage::save_session(&session1, Some(&sessions_dir)).unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

        let session2 = SessionData::new(
            "session_002".to_string(),
            vec![Message {
                role: MessageRole::User,
                content: Content::text_only("Te st 2".to_string()),
            }],
            vec![],
        );
        storage::save_session(&session2, Some(&sessions_dir)).unwrap();

        let events = fixture.step("/sessions list").await;

        let mut response_text = String::new();
        for event in &events {
            let ChatEvent::MessageAdded(msg) = event else {
                continue;
            };
            if !matches!(msg.sender, MessageSender::System) {
                continue;
            };
            response_text = msg.content.clone();
        }

        if response_text.is_empty() {
            panic!("No System message found. Events received: {:?}", events);
        }

        assert!(
            response_text.contains("session_001"),
            "Response should contain session_001. Actual response: {}",
            response_text
        );
        assert!(
            response_text.contains("session_002"),
            "Response should contain session_002"
        );

        let session_001_pos = response_text.find("session_001").unwrap();
        let session_002_pos = response_text.find("session_002").unwrap();
        assert!(
            session_001_pos < session_002_pos,
            "session_001 should appear first (older)"
        );

        assert!(
            response_text.contains("Task List: Understand user requirements"),
            "Response should show task list title. Actual: {}",
            response_text
        );
    });
}

#[test]
fn test_sessions_resume_command() {
    fixture::run(|mut fixture| async move {
        let sessions_dir = fixture.sessions_dir();

        let test_messages = vec![
            Message {
                role: MessageRole::User,
                content: Content::text_only("Original message".to_string()),
            },
            Message {
                role: MessageRole::Assistant,
                content: Content::text_only("Original response".to_string()),
            },
        ];

        let session = SessionData::new("test_session".to_string(), test_messages.clone(), vec![]);
        storage::save_session(&session, Some(&sessions_dir)).unwrap();

        let events = fixture.step("/sessions resume test_session").await;

        let mut got_success = false;
        for event in events {
            let ChatEvent::MessageAdded(msg) = &event else {
                continue;
            };
            if !matches!(msg.sender, MessageSender::System) {
                continue;
            };
            if !(msg.content.contains("resumed") || msg.content.contains("Resumed")) {
                continue;
            };
            got_success = true;
            break;
        }
        assert!(got_success, "Should receive session resumed confirmation");
    });
}

#[test]
fn test_sessions_delete_command() {
    fixture::run(|mut fixture| async move {
        let sessions_dir = fixture.sessions_dir();

        let session = SessionData::new(
            "test_delete".to_string(),
            vec![Message {
                role: MessageRole::User,
                content: Content::text_only("Test".to_string()),
            }],
            vec![],
        );
        storage::save_session(&session, Some(&sessions_dir)).unwrap();

        let sessions_before = storage::list_sessions(Some(&sessions_dir)).unwrap();
        assert_eq!(
            sessions_before.len(),
            1,
            "Should have one session before delete"
        );

        let events = fixture.step("/sessions delete test_delete").await;

        let mut got_success = false;
        for event in events {
            let ChatEvent::MessageAdded(msg) = &event else {
                continue;
            };
            if !matches!(msg.sender, MessageSender::System) {
                continue;
            };
            if !msg.content.contains("deleted") {
                continue;
            };
            got_success = true;
            break;
        }
        assert!(got_success, "Should receive delete confirmation");

        let sessions_after = storage::list_sessions(Some(&sessions_dir)).unwrap();
        assert_eq!(
            sessions_after.len(),
            0,
            "Should have no sessions after delete"
        );
    });
}

#[test]
fn test_session_isolation() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");

    let local = tokio::task::LocalSet::new();

    runtime.block_on(local.run_until(async {
        let mut fixture1 = Fixture::new();
        let mut fixture2 = Fixture::new();

        fixture1.send_message("Actor 1 message");
        fixture2.send_message("Actor 2 message");

        let mut got_response1 = false;
        let mut got_response2 = false;
        let start = tokio::time::Instant::now();
        let timeout = tokio::time::Duration::from_secs(5);

        while (!got_response1 || !got_response2) && start.elapsed() < timeout {
            tokio::select! {
                Some(event) = fixture1.event_rx.recv() => {
                    got_response1 = got_response1 || is_assistant_message(&event);
                }
                Some(event) = fixture2.event_rx.recv() => {
                    got_response2 = got_response2 || is_assistant_message(&event);
                }
            }
        }

        assert!(got_response1, "Actor 1 should receive response");
        assert!(got_response2, "Actor 2 should receive response");

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let sessions_dir1 = fixture1.sessions_dir();
        let sessions_dir2 = fixture2.sessions_dir();

        let sessions1 = storage::list_sessions(Some(&sessions_dir1)).unwrap();
        let sessions2 = storage::list_sessions(Some(&sessions_dir2)).unwrap();

        assert_eq!(
            sessions1.len(),
            1,
            "Actor 1 should have exactly one session"
        );
        assert_eq!(
            sessions2.len(),
            1,
            "Actor 2 should have exactly one session"
        );

        assert_ne!(
            sessions1[0].id, sessions2[0].id,
            "Session IDs should be different"
        );
    }));
}

#[test]
fn test_session_replay_with_tool_events() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();
        let test_file_path = workspace_path.join("test.rs");
        std::fs::write(&test_file_path, "// test file\n").unwrap();

        let root_name = workspace_path.file_name().unwrap().to_str().unwrap();
        let virtual_path = format!("{}/test.rs", root_name);

        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "set_tracked_files".to_string(),
            tool_arguments: format!(r#"{{"file_paths": ["{}"]}}"#, virtual_path),
        });

        let events = fixture.step("Please help me").await;

        let mut has_tool_request = false;
        let mut has_tool_execution_completed = false;
        let mut has_message_added = false;

        for event in &events {
            match event {
                ChatEvent::ToolRequest(_) => has_tool_request = true,
                ChatEvent::ToolExecutionCompleted { .. } => has_tool_execution_completed = true,
                ChatEvent::MessageAdded(_) => has_message_added = true,
                _ => {}
            }
        }

        assert!(has_message_added, "Should have MessageAdded events");
        assert!(has_tool_request, "Should have ToolRequest event");
        assert!(
            has_tool_execution_completed,
            "Should have ToolExecutionCompleted event"
        );

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let sessions_dir = fixture.sessions_dir();
        let sessions = storage::list_sessions(Some(&sessions_dir)).unwrap();
        assert_eq!(
            sessions.len(),
            1,
            "Expected exactly one session to be saved"
        );

        let session_data = storage::load_session(&sessions[0].id, Some(&sessions_dir)).unwrap();

        let mut has_tool_request_saved = false;
        let mut has_tool_execution_completed_saved = false;
        let mut has_message_added_saved = false;

        for event in &session_data.events {
            match event {
                ChatEvent::ToolRequest(_) => has_tool_request_saved = true,
                ChatEvent::ToolExecutionCompleted { .. } => {
                    has_tool_execution_completed_saved = true
                }
                ChatEvent::MessageAdded(_) => has_message_added_saved = true,
                _ => {}
            }
        }

        assert!(
            session_data.events.len() > 0,
            "Total event count should be greater than 0"
        );
        assert!(
            has_message_added_saved,
            "Should have MessageAdded events in storage"
        );
        assert!(
            has_tool_request_saved,
            "Should have ToolRequest event in storage"
        );
        assert!(
            has_tool_execution_completed_saved,
            "Should have ToolExecutionCompleted event in storage"
        );
    });
}
