#[path = "../fixture.rs"]
mod fixture;

use fixture::MockBehavior;
use tycode_core::ai::ContentBlock;
use tycode_core::chat::events::ChatEvent;

fn complete_task_behavior() -> MockBehavior {
    MockBehavior::ToolUseThenSuccess {
        tool_name: "complete_task".to_string(),
        tool_arguments: r#"{"result": "done", "success": true}"#.to_string(),
    }
}

fn extract_context_content(request: &tycode_core::ai::ConversationRequest) -> String {
    request
        .messages
        .iter()
        .rev()
        .find_map(|msg| {
            msg.content.blocks().iter().find_map(|block| {
                if let ContentBlock::Text(text) = block {
                    if text.contains("Task List:") {
                        return Some(text.clone());
                    }
                }
                None
            })
        })
        .unwrap_or_default()
}

fn extract_task_descriptions(request: &tycode_core::ai::ConversationRequest) -> Vec<String> {
    let context = extract_context_content(request);

    context
        .lines()
        .filter(|line| line.starts_with("  - [") && line.contains("Task "))
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(2, ": ").collect();
            if parts.len() == 2 {
                Some(parts[1].to_string())
            } else {
                None
            }
        })
        .collect()
}

fn has_task_with_description(request: &tycode_core::ai::ConversationRequest, desc: &str) -> bool {
    let tasks = extract_task_descriptions(request);
    tasks.iter().any(|t| t.contains(desc))
}

fn count_task_updates(events: &[ChatEvent]) -> usize {
    events
        .iter()
        .filter(|e| matches!(e, ChatEvent::TaskUpdate(_)))
        .count()
}

#[test]
fn test_spawned_agent_has_independent_task_list() {
    fixture::run_with_agent("coordinator", |mut fixture| async move {
        fixture.set_mock_behavior(complete_task_behavior());
        fixture.step("Hello").await;

        let parent_first_request = fixture
            .get_last_ai_request()
            .expect("Should have initial parent request");

        let parent_initial_tasks = extract_task_descriptions(&parent_first_request);
        assert!(
            !parent_initial_tasks.is_empty(),
            "Parent should have initial tasks in context. Context: {:?}",
            extract_context_content(&parent_first_request)
        );

        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                MockBehavior::ToolUse {
                    tool_name: "spawn_agent".to_string(),
                    tool_arguments: r#"{"agent_type": "coder", "task": "test child work", "initial_task_list": ["Child task A", "Child task B"]}"#.to_string(),
                },
                MockBehavior::ToolUse {
                    tool_name: "complete_task".to_string(),
                    tool_arguments: r#"{"result": "child done", "success": true}"#.to_string(),
                },
            ],
        });

        fixture.step("Spawn coder").await;

        let all_requests = fixture.get_all_ai_requests();

        let child_requests: Vec<_> = all_requests
            .iter()
            .filter(|req| has_task_with_description(req, "Child task A"))
            .collect();

        assert!(
            !child_requests.is_empty(),
            "Should have at least one request with child tasks. Got {} requests.\n\
             All contexts: {:?}",
            child_requests.len(),
            all_requests
                .iter()
                .map(|r| extract_context_content(r))
                .collect::<Vec<_>>()
        );

        assert!(
            has_task_with_description(child_requests[0], "Child task A"),
            "Child task list should contain 'Child task A'"
        );
        assert!(
            has_task_with_description(child_requests[0], "Child task B"),
            "Child task list should contain 'Child task B'"
        );

        let parent_requests: Vec<_> = all_requests
            .iter()
            .filter(|req| !has_task_with_description(req, "Child task A"))
            .collect();

        assert!(!parent_requests.is_empty(), "Should have parent requests");

        for req in &parent_requests {
            assert!(
                has_task_with_description(req, "Await user request"),
                "Parent request should retain its original tasks. Context: {:?}",
                extract_context_content(req)
            );
            assert!(
                !has_task_with_description(req, "Child task"),
                "Parent should NOT have child tasks. Context: {:?}",
                extract_context_content(req)
            );
        }
    });
}

#[test]
fn test_child_manage_task_list_does_not_affect_parent() {
    fixture::run_with_agent("coordinator", |mut fixture| async move {
        fixture.set_mock_behavior(complete_task_behavior());
        fixture.step("Hello").await;

        let parent_initial = fixture
            .get_last_ai_request()
            .expect("Should have parent request");
        let parent_initial_tasks = extract_task_descriptions(&parent_initial);

        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                MockBehavior::ToolUse {
                    tool_name: "spawn_agent".to_string(),
                    tool_arguments: r#"{"agent_type": "coder", "task": "test", "initial_task_list": ["Child task"]}"#.to_string(),
                },
                MockBehavior::ToolUse {
                    tool_name: "manage_task_list".to_string(),
                    tool_arguments: r#"{"title": "Child Updated", "tasks": [{"description": "Completely New Child Task", "status": "completed"}]}"#.to_string(),
                },
                MockBehavior::ToolUse {
                    tool_name: "complete_task".to_string(),
                    tool_arguments: r#"{"result": "child done", "success": true}"#.to_string(),
                },
                MockBehavior::ToolUse {
                    tool_name: "complete_task".to_string(),
                    tool_arguments: r#"{"result": "all done", "success": true}"#.to_string(),
                },
            ],
        });

        fixture.step("Spawn child and modify tasks").await;

        let all_requests = fixture.get_all_ai_requests();

        let child_requests: Vec<_> = all_requests
            .iter()
            .filter(|req| has_task_with_description(req, "Completely New Child Task"))
            .collect();

        assert!(
            !child_requests.is_empty(),
            "Child should have updated its task list with 'Completely New Child Task'"
        );

        let final_request = all_requests.last().expect("Should have final request");
        let final_context = extract_context_content(final_request);

        assert!(
            !has_task_with_description(final_request, "Completely New Child Task"),
            "Parent should NOT have child's modified task after child completes. Final context: {:?}",
            final_context
        );

        let final_parent_tasks = extract_task_descriptions(final_request);

        assert!(
            !final_parent_tasks.is_empty(),
            "Parent should still have tasks after child completes. Before: {:?}, After: {:?}",
            parent_initial_tasks,
            final_parent_tasks
        );
    });
}

#[test]
fn test_spawned_agent_receives_initial_task_list() {
    fixture::run_with_agent("coordinator", |mut fixture| async move {
        fixture.set_mock_behavior(complete_task_behavior());
        fixture.step("Hello").await;

        let parent_request = fixture
            .get_last_ai_request()
            .expect("Should have parent request");
        let parent_context = extract_context_content(&parent_request);

        fixture.clear_captured_requests();
        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                MockBehavior::ToolUse {
                    tool_name: "spawn_agent".to_string(),
                    tool_arguments: r#"{"agent_type": "coder", "task": "test", "initial_task_list": ["Task specific to child"]}"#.to_string(),
                },
                MockBehavior::ToolUse {
                    tool_name: "complete_task".to_string(),
                    tool_arguments: r#"{"result": "done", "success": true}"#.to_string(),
                },
            ],
        });

        fixture.step("Spawn coder").await;

        let all_requests = fixture.get_all_ai_requests();

        let child_request = all_requests
            .iter()
            .find(|req| has_task_with_description(req, "Task specific to child"))
            .expect("Should have child request with initial_task_list tasks");

        let child_context = extract_context_content(child_request);

        assert!(
            has_task_with_description(child_request, "Task specific to child"),
            "Child should have its initial_task_list tasks in context. Context: {:?}",
            child_context
        );

        assert!(
            !parent_context.contains("Task specific to child"),
            "Parent should NOT have child's task. Parent context: {:?}",
            parent_context
        );

        assert_ne!(
            parent_context, child_context,
            "Child context should be different from parent context"
        );
    });
}

#[test]
fn test_spawn_agent_has_initial_task_list_parameter() {
    fixture::run_with_agent("coordinator", |mut fixture| async move {
        fixture.set_mock_behavior(complete_task_behavior());
        fixture.step("Hello").await;

        let request = fixture
            .get_last_ai_request()
            .expect("Should have captured AI request");

        let spawn_tool = request.tools.iter().find(|t| t.name == "spawn_agent");
        assert!(
            spawn_tool.is_some(),
            "Coordinator should have spawn_agent tool"
        );

        let tool = spawn_tool.unwrap();
        let schema = &tool.input_schema;
        let schema_str = serde_json::to_string(&tool.input_schema).unwrap();
        assert!(
            schema_str.contains("initial_task_list"),
            "spawn_agent should have initial_task_list parameter in schema. Got: {}",
            schema_str
        );
        assert!(
            !schema_str.contains("\"status\""),
            "initial_task_list schema should not require or define status. Got: {}",
            schema_str
        );
        let items_type = schema
            .get("properties")
            .and_then(|v| v.get("initial_task_list"))
            .and_then(|v| v.get("items"))
            .and_then(|v| v.get("type"))
            .and_then(|v| v.as_str());
        assert_eq!(
            items_type,
            Some("string"),
            "initial_task_list items should be strings. Schema: {}",
            schema_str
        );
    });
}

#[test]
fn test_coordinator_has_manage_task_list_tool() {
    fixture::run_with_agent("coordinator", |mut fixture| async move {
        fixture.set_mock_behavior(complete_task_behavior());
        fixture.step("Hello").await;

        let request = fixture
            .get_last_ai_request()
            .expect("Should have captured AI request");

        let has_task_list = request.tools.iter().any(|t| t.name == "manage_task_list");
        assert!(
            has_task_list,
            "Coordinator should have manage_task_list tool"
        );
    });
}

#[test]
fn test_coder_has_manage_task_list_tool() {
    fixture::run_with_agent("coder", |mut fixture| async move {
        fixture.set_mock_behavior(complete_task_behavior());
        fixture.step("Hello").await;

        let request = fixture
            .get_last_ai_request()
            .expect("Should have captured AI request");

        let has_task_list = request.tools.iter().any(|t| t.name == "manage_task_list");
        assert!(has_task_list, "Coder should have manage_task_list tool");
    });
}

#[test]
fn test_initial_task_list_uses_spawn_task_as_title() {
    fixture::run_with_agent("coordinator", |mut fixture| async move {
        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                MockBehavior::ToolUse {
                    tool_name: "spawn_agent".to_string(),
                    tool_arguments: r#"{"agent_type": "coder", "task": "Implement parser", "initial_task_list": ["Child task title test"]}"#.to_string(),
                },
                MockBehavior::ToolUse {
                    tool_name: "complete_task".to_string(),
                    tool_arguments: r#"{"result": "child done", "success": true}"#.to_string(),
                },
                MockBehavior::ToolUse {
                    tool_name: "complete_task".to_string(),
                    tool_arguments: r#"{"result": "parent done", "success": true}"#.to_string(),
                },
            ],
        });

        fixture.step("Spawn child").await;

        let all_requests = fixture.get_all_ai_requests();
        let child_request = all_requests
            .iter()
            .find(|req| has_task_with_description(req, "Child task title test"))
            .expect("Should have child request with initial_task_list task");
        let child_context = extract_context_content(child_request);

        assert!(
            child_context.contains("Task List: Implement parser"),
            "Child task list title should use spawn task. Context: {:?}",
            child_context
        );
        assert!(
            child_context.contains("[Pending] Task"),
            "Spawned initial_task_list entries should default to Pending. Context: {:?}",
            child_context
        );
    });
}

#[test]
fn test_spawn_and_pop_emit_task_updates() {
    fixture::run_with_agent("coordinator", |mut fixture| async move {
        fixture.set_mock_behavior(complete_task_behavior());
        fixture.step("Hello").await;

        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                MockBehavior::ToolUse {
                    tool_name: "spawn_agent".to_string(),
                    tool_arguments: r#"{"agent_type": "coder", "task": "task update test", "initial_task_list": ["Child update task"]}"#.to_string(),
                },
                MockBehavior::ToolUse {
                    tool_name: "complete_task".to_string(),
                    tool_arguments: r#"{"result": "child done", "success": true}"#.to_string(),
                },
                MockBehavior::ToolUse {
                    tool_name: "complete_task".to_string(),
                    tool_arguments: r#"{"result": "parent done", "success": true}"#.to_string(),
                },
            ],
        });

        let events = fixture.step("Spawn child and return").await;
        let update_count = count_task_updates(&events);

        assert!(
            update_count >= 2,
            "Expected task updates for child spawn and parent restore, got {}. Events: {:?}",
            update_count,
            events
        );
    });
}

#[test]
fn test_review_agent_keeps_child_task_list_until_parent_restored() {
    fixture::run_with_agent("coordinator", |mut fixture| async move {
        fixture
            .update_settings(|settings| {
                settings.review_level = tycode_core::settings::config::ReviewLevel::Task;
            })
            .await;

        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                MockBehavior::ToolUse {
                    tool_name: "spawn_agent".to_string(),
                    tool_arguments: r#"{"agent_type": "coder", "task": "review pipeline task", "initial_task_list": ["Coder scoped task"]}"#.to_string(),
                },
                MockBehavior::ToolUse {
                    tool_name: "complete_task".to_string(),
                    tool_arguments: r#"{"result": "coder done", "success": true}"#.to_string(),
                },
                MockBehavior::ToolUse {
                    tool_name: "complete_task".to_string(),
                    tool_arguments: r#"{"result": "review approved", "success": true}"#.to_string(),
                },
                MockBehavior::ToolUse {
                    tool_name: "complete_task".to_string(),
                    tool_arguments: r#"{"result": "parent done", "success": true}"#.to_string(),
                },
            ],
        });

        fixture.step("Run review flow").await;

        let all_requests = fixture.get_all_ai_requests();
        let review_request = all_requests
            .iter()
            .find(|req| {
                req.messages
                    .iter()
                    .any(|msg| msg.content.text().contains("You are a code review agent"))
            })
            .expect("Expected a review-agent request");
        let review_context = extract_context_content(review_request);

        assert!(
            review_context.contains("Coder scoped task"),
            "Review agent should still see coder task list context. Context: {:?}",
            review_context
        );
        assert!(
            !review_context.contains("Await user request"),
            "Review agent context should not be restored to parent task list early. Context: {:?}",
            review_context
        );

        let final_request = all_requests
            .last()
            .expect("Should have final parent request");
        let final_context = extract_context_content(final_request);
        assert!(
            final_context.contains("Await user request"),
            "Parent task list should be restored after review/coder pop. Final context: {:?}",
            final_context
        );
        assert!(
            !final_context.contains("Coder scoped task"),
            "Parent context should not retain child task list after restoration. Final context: {:?}",
            final_context
        );
    });
}

#[test]
fn test_parent_task_list_preserved_after_child_completes() {
    fixture::run_with_agent("coordinator", |mut fixture| async move {
        fixture.set_mock_behavior(complete_task_behavior());
        fixture.step("Hello").await;

        let parent_before = fixture
            .get_last_ai_request()
            .expect("Should have parent request");
        let tasks_before = extract_task_descriptions(&parent_before);

        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                MockBehavior::ToolUse {
                    tool_name: "spawn_agent".to_string(),
                    tool_arguments: r#"{"agent_type": "coder", "task": "child task"}"#.to_string(),
                },
                MockBehavior::ToolUse {
                    tool_name: "complete_task".to_string(),
                    tool_arguments: r#"{"result": "child done", "success": true}"#.to_string(),
                },
                MockBehavior::ToolUse {
                    tool_name: "manage_task_list".to_string(),
                    tool_arguments: r#"{"title": "Same Tasks", "tasks": [{"description": "Parent task updated", "status": "in_progress"}]}"#.to_string(),
                },
                MockBehavior::ToolUse {
                    tool_name: "complete_task".to_string(),
                    tool_arguments: r#"{"result": "done", "success": true}"#.to_string(),
                },
            ],
        });

        fixture.clear_captured_requests();
        fixture.step("Spawn child and return").await;

        let all_requests = fixture.get_all_ai_requests();
        let parent_after = all_requests
            .iter()
            .find(|req| req.tools.iter().any(|t| t.name == "manage_task_list"))
            .expect("Parent should still have manage_task_list after child completes");

        let tasks_after = extract_task_descriptions(parent_after);

        assert!(
            !tasks_after.is_empty(),
            "Parent should still have tasks after child completes. Before: {:?}, After: {:?}",
            tasks_before,
            tasks_after
        );

        let has_manage = parent_after
            .tools
            .iter()
            .any(|t| t.name == "manage_task_list");
        assert!(
            has_manage,
            "Parent should retain manage_task_list tool after child completes"
        );
    });
}
