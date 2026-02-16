//! Review module simulation tests.
//!
//! Tests for `/review deep` command pipeline:
//! 1. Hunk reviewers launch and complete with results
//! 2. Consolidation agent receives all hunk results
//! 3. Final consolidated review is returned

#[path = "../fixture.rs"]
mod fixture;

use fixture::MockBehavior;
use std::process::Command;
use tycode_core::chat::events::ChatEvent;

fn complete_task_review(review: &str) -> MockBehavior {
    MockBehavior::ToolUse {
        tool_name: "complete_task".to_string(),
        tool_arguments: format!(
            r#"{{"result": "{}", "success": true}}"#,
            review.replace('"', r#"\""#)
        ),
    }
}

fn init_git_repo(workspace: &std::path::Path) {
    let run = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(workspace)
            .output()
            .expect("git command failed");
    };
    run(&["init"]);
    run(&["config", "user.email", "test@test.com"]);
    run(&["config", "user.name", "Test"]);
}

fn create_two_hunk_diff(workspace: &std::path::Path) {
    // deep_review uses -U15 context, so changes must be 31+ lines apart
    // to produce 2 separate hunks. 50 lines with changes at 2 and 49.
    let lines: Vec<String> = (1..=50).map(|i| format!("line_{i:02}")).collect();
    let initial = lines.join("\n");
    let file = workspace.join("test.rs");
    std::fs::write(&file, &initial).unwrap();

    let run = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(workspace)
            .output()
            .unwrap();
    };
    run(&["add", "."]);
    run(&["commit", "-m", "initial"]);

    // Lines 2 and 49: 47 lines apart, well beyond -U15 context → 2 hunks
    let modified = initial
        .replace("line_02", "modified_line_02")
        .replace("line_49", "modified_line_49");
    std::fs::write(&file, &modified).unwrap();
}

#[test]
fn test_deep_review_full_pipeline() {
    fixture::run_with_agent("tycode", |mut fixture| async move {
        let workspace = fixture.workspace_path();
        init_git_repo(&workspace);
        create_two_hunk_diff(&workspace);

        // Queue: 2 hunk reviewers + 1 consolidation + 1 fallback for main agent
        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                complete_task_review("Hunk 1: variable naming could be improved"),
                complete_task_review("Hunk 2: logic looks correct"),
                complete_task_review(
                    "CONSOLIDATED: Overall acceptable with minor naming suggestions",
                ),
                MockBehavior::Success, // fallback if main agent processes result
            ],
        });

        let events = fixture.step("/review deep").await;

        // Collect all text from events
        let all_text: String = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) => Some(msg.content.clone()),
                ChatEvent::Error(msg) => Some(msg.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        // Pipeline should produce output
        assert!(
            !all_text.is_empty(),
            "Deep review should produce output. Events: {events:?}"
        );

        // Consolidated result should appear
        assert!(
            all_text.contains("CONSOLIDATED"),
            "Output should contain consolidated review result. Got: {all_text}"
        );

        // Verify AI requests: at least 3 (2 hunk reviewers + 1 consolidation)
        let all_requests = fixture.get_all_ai_requests();
        assert!(
            all_requests.len() >= 3,
            "Expected at least 3 AI requests (2 hunk + 1 consolidation), got {}",
            all_requests.len()
        );

        // Verify consolidation request received hunk review results
        // Consolidation is the last request (after hunk reviewers)
        let consolidation_request = all_requests.last().unwrap();
        let consolidation_input: String = consolidation_request
            .messages
            .iter()
            .map(|m| m.content.text())
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            consolidation_input.contains("Hunk 1") && consolidation_input.contains("Hunk 2"),
            "Consolidation agent should receive both hunk review results. Input: {consolidation_input}"
        );
    });
}

#[test]
fn test_deep_review_no_changes() {
    fixture::run_with_agent("tycode", |mut fixture| async move {
        let workspace = fixture.workspace_path();
        init_git_repo(&workspace);

        // Commit the example.txt that fixture creates, with no unstaged changes
        Command::new("git")
            .args(["add", "."])
            .current_dir(&workspace)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(&workspace)
            .output()
            .unwrap();

        // No modifications — git diff should be empty
        let events = fixture.step("/review deep").await;

        let all_text: String = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) => Some(msg.content.clone()),
                ChatEvent::Error(msg) => Some(msg.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        // Should indicate no changes or empty diff
        assert!(
            all_text.to_lowercase().contains("no")
                || all_text.to_lowercase().contains("empty")
                || all_text.to_lowercase().contains("nothing"),
            "Deep review with no changes should indicate nothing to review. Got: {all_text}"
        );

        // No AI requests should be made for empty diff
        let all_requests = fixture.get_all_ai_requests();
        assert!(
            all_requests.is_empty(),
            "No AI requests expected for empty diff, got {}",
            all_requests.len()
        );
    });
}
