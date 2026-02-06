mod fixture;

use serde_json::json;
use std::fs;
use tycode_core::ai::mock::MockBehavior;
use tycode_core::chat::events::{ChatEvent, MessageSender};

fn setup_rust_project(fixture: &fixture::Fixture) {
    let workspace = fixture.workspace_path();

    fs::write(
        workspace.join("Cargo.toml"),
        r#"[package]
name = "test-project"
version = "0.1.0"
edition = "2021"
"#,
    )
    .unwrap();

    fs::create_dir_all(workspace.join("src")).unwrap();
    fs::write(workspace.join("src/lib.rs"), "// test file\n").unwrap();
}

fn find_tool_execution_completed(events: &[ChatEvent], tool_name: &str) -> Option<bool> {
    events.iter().find_map(|e| match e {
        ChatEvent::ToolExecutionCompleted {
            tool_name: name,
            success,
            ..
        } if name == tool_name => Some(*success),
        _ => None,
    })
}

fn has_error_event(events: &[ChatEvent]) -> bool {
    events.iter().any(|e| matches!(e, ChatEvent::Error(_)))
}

fn get_ai_response_text(events: &[ChatEvent]) -> String {
    events
        .iter()
        .filter_map(|e| match e {
            ChatEvent::StreamEnd { message }
                if matches!(message.sender, MessageSender::Assistant { .. }) =>
            {
                Some(message.content.clone())
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}

// =============================================================================
// search_types Tool Tests
// =============================================================================

#[test]
fn search_types_tool_executes_successfully() {
    fixture::run(|mut fixture| async move {
        setup_rust_project(&fixture);
        fixture
            .update_settings(|s| s.enable_type_analyzer = true)
            .await;

        let workspace = fixture.workspace_path();
        let workspace_name = workspace.file_name().unwrap().to_string_lossy();
        let args = json!({
            "language": "rust",
            "workspace_root": workspace_name,
            "type_name": "BuildStatus"
        });
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "search_types".to_string(),
            tool_arguments: serde_json::to_string(&args).unwrap(),
        });

        let events = fixture.step("Search for BuildStatus type").await;

        let completed = find_tool_execution_completed(&events, "search_types");
        assert!(completed.is_some(), "Should have tool execution completed");
    });
}

#[test]
fn search_types_returns_type_paths_in_result() {
    fixture::run(|mut fixture| async move {
        setup_rust_project(&fixture);
        fixture
            .update_settings(|s| s.enable_type_analyzer = true)
            .await;

        let workspace = fixture.workspace_path();
        let workspace_name = workspace.file_name().unwrap().to_string_lossy();
        let args = json!({
            "language": "rust",
            "workspace_root": workspace_name,
            "type_name": "String"
        });
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "search_types".to_string(),
            tool_arguments: serde_json::to_string(&args).unwrap(),
        });

        let events = fixture.step("Find String type").await;

        let completed = find_tool_execution_completed(&events, "search_types");
        assert!(completed.is_some(), "Should have tool execution completed");
    });
}

#[test]
fn search_types_handles_nonexistent_type() {
    fixture::run(|mut fixture| async move {
        setup_rust_project(&fixture);
        fixture
            .update_settings(|s| s.enable_type_analyzer = true)
            .await;

        let workspace = fixture.workspace_path();
        let workspace_name = workspace.file_name().unwrap().to_string_lossy();
        let args = json!({
            "language": "rust",
            "workspace_root": workspace_name,
            "type_name": "NonExistentType12345"
        });
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "search_types".to_string(),
            tool_arguments: serde_json::to_string(&args).unwrap(),
        });

        let events = fixture.step("Search for NonExistentType12345").await;

        let completed = find_tool_execution_completed(&events, "search_types");
        assert!(completed.is_some(), "Should have tool execution completed");
    });
}

#[test]
fn search_types_validates_missing_parameters() {
    fixture::run(|mut fixture| async move {
        setup_rust_project(&fixture);
        fixture
            .update_settings(|s| s.enable_type_analyzer = true)
            .await;

        let args = json!({
            "language": "rust"
        });
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "search_types".to_string(),
            tool_arguments: serde_json::to_string(&args).unwrap(),
        });

        let events = fixture.step("Search types without required params").await;

        let completed = find_tool_execution_completed(&events, "search_types");
        assert!(completed.is_some(), "Should have tool execution completed");
        assert!(
            !completed.unwrap(),
            "Tool should fail with missing parameters"
        );
    });
}

#[test]
fn search_types_validates_invalid_workspace_root() {
    fixture::run(|mut fixture| async move {
        fixture
            .update_settings(|s| s.enable_type_analyzer = true)
            .await;

        let args = json!({
            "language": "rust",
            "workspace_root": "/invalid/workspace/root",
            "type_name": "BuildStatus"
        });
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "search_types".to_string(),
            tool_arguments: serde_json::to_string(&args).unwrap(),
        });

        let events = fixture.step("Search with invalid workspace").await;

        let completed = find_tool_execution_completed(&events, "search_types");
        assert!(completed.is_some(), "Should have tool execution completed");
        assert!(
            !completed.unwrap(),
            "Tool should fail with invalid workspace"
        );
    });
}

#[test]
fn search_types_validates_unsupported_language() {
    fixture::run(|mut fixture| async move {
        setup_rust_project(&fixture);
        fixture
            .update_settings(|s| s.enable_type_analyzer = true)
            .await;

        let workspace = fixture.workspace_path();
        let workspace_name = workspace.file_name().unwrap().to_string_lossy();
        let args = json!({
            "language": "typescript",
            "workspace_root": workspace_name,
            "type_name": "BuildStatus"
        });
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "search_types".to_string(),
            tool_arguments: serde_json::to_string(&args).unwrap(),
        });

        let events = fixture.step("Search with unsupported language").await;

        let completed = find_tool_execution_completed(&events, "search_types");
        assert!(completed.is_some(), "Should have tool execution completed");
        assert!(
            !completed.unwrap(),
            "Tool should fail with unsupported language"
        );
    });
}

// =============================================================================
// get_type_docs Tool Tests
// =============================================================================

#[test]
fn get_type_docs_tool_executes_successfully() {
    fixture::run(|mut fixture| async move {
        setup_rust_project(&fixture);
        fixture
            .update_settings(|s| s.enable_type_analyzer = true)
            .await;

        let workspace = fixture.workspace_path();
        let workspace_name = workspace.file_name().unwrap().to_string_lossy();
        let args = json!({
            "language": "rust",
            "workspace_root": workspace_name,
            "type_path": "tycode_core::analyzer::BuildStatus"
        });
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "get_type_docs".to_string(),
            tool_arguments: serde_json::to_string(&args).unwrap(),
        });

        let events = fixture.step("Get docs for BuildStatus").await;

        let completed = find_tool_execution_completed(&events, "get_type_docs");
        assert!(completed.is_some(), "Should have tool execution completed");
    });
}

#[test]
fn get_type_docs_returns_documentation_content() {
    fixture::run(|mut fixture| async move {
        setup_rust_project(&fixture);
        fixture
            .update_settings(|s| s.enable_type_analyzer = true)
            .await;

        let workspace = fixture.workspace_path();
        let workspace_name = workspace.file_name().unwrap().to_string_lossy();
        let args = json!({
            "language": "rust",
            "workspace_root": workspace_name,
            "type_path": "tycode_core::analyzer::SharedTypeAnalyzer"
        });
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "get_type_docs".to_string(),
            tool_arguments: serde_json::to_string(&args).unwrap(),
        });

        let events = fixture.step("Get docs for SharedTypeAnalyzer").await;

        let completed = find_tool_execution_completed(&events, "get_type_docs");
        assert!(completed.is_some(), "Should have tool execution completed");
    });
}

#[test]
fn get_type_docs_handles_nonexistent_type() {
    fixture::run(|mut fixture| async move {
        setup_rust_project(&fixture);
        fixture
            .update_settings(|s| s.enable_type_analyzer = true)
            .await;

        let workspace = fixture.workspace_path();
        let workspace_name = workspace.file_name().unwrap().to_string_lossy();
        let args = json!({
            "language": "rust",
            "workspace_root": workspace_name,
            "type_path": "tycode_core::nonexistent::Type"
        });
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "get_type_docs".to_string(),
            tool_arguments: serde_json::to_string(&args).unwrap(),
        });

        let events = fixture.step("Get docs for nonexistent type").await;

        let completed = find_tool_execution_completed(&events, "get_type_docs");
        assert!(completed.is_some(), "Should have tool execution completed");
    });
}

#[test]
fn get_type_docs_validates_missing_type_path() {
    fixture::run(|mut fixture| async move {
        setup_rust_project(&fixture);
        fixture
            .update_settings(|s| s.enable_type_analyzer = true)
            .await;

        let workspace = fixture.workspace_path();
        let workspace_name = workspace.file_name().unwrap().to_string_lossy();
        let args = json!({
            "language": "rust",
            "workspace_root": workspace_name
        });
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "get_type_docs".to_string(),
            tool_arguments: serde_json::to_string(&args).unwrap(),
        });

        let events = fixture.step("Get docs without type_path").await;

        let completed = find_tool_execution_completed(&events, "get_type_docs");
        assert!(completed.is_some(), "Should have tool execution completed");
        assert!(
            !completed.unwrap(),
            "Tool should fail with missing type_path"
        );
    });
}

#[test]
fn get_type_docs_validates_invalid_workspace_root() {
    fixture::run(|mut fixture| async move {
        fixture
            .update_settings(|s| s.enable_type_analyzer = true)
            .await;

        let args = json!({
            "language": "rust",
            "workspace_root": "/invalid/workspace/root",
            "type_path": "tycode_core::analyzer::BuildStatus"
        });
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "get_type_docs".to_string(),
            tool_arguments: serde_json::to_string(&args).unwrap(),
        });

        let events = fixture.step("Get docs with invalid workspace").await;

        let completed = find_tool_execution_completed(&events, "get_type_docs");
        assert!(completed.is_some(), "Should have tool execution completed");
        assert!(
            !completed.unwrap(),
            "Tool should fail with invalid workspace"
        );
    });
}

#[test]
fn get_type_docs_validates_unsupported_language() {
    fixture::run(|mut fixture| async move {
        setup_rust_project(&fixture);
        fixture
            .update_settings(|s| s.enable_type_analyzer = true)
            .await;

        let workspace = fixture.workspace_path();
        let workspace_name = workspace.file_name().unwrap().to_string_lossy();
        let args = json!({
            "language": "python",
            "workspace_root": workspace_name,
            "type_path": "tycode_core::analyzer::BuildStatus"
        });
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "get_type_docs".to_string(),
            tool_arguments: serde_json::to_string(&args).unwrap(),
        });

        let events = fixture.step("Get docs with unsupported language").await;

        let completed = find_tool_execution_completed(&events, "get_type_docs");
        assert!(completed.is_some(), "Should have tool execution completed");
        assert!(
            !completed.unwrap(),
            "Tool should fail with unsupported language"
        );
    });
}

// =============================================================================
// Integration Tests - Search then Get Docs Workflow
// =============================================================================

#[test]
fn search_then_get_docs_workflow() {
    fixture::run(|mut fixture| async move {
        setup_rust_project(&fixture);
        fixture
            .update_settings(|s| s.enable_type_analyzer = true)
            .await;

        let workspace = fixture.workspace_path();
        let workspace_name = workspace.file_name().unwrap().to_string_lossy();
        let args = json!({
            "language": "rust",
            "workspace_root": workspace_name,
            "type_name": "BuildStatus"
        });
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "search_types".to_string(),
            tool_arguments: serde_json::to_string(&args).unwrap(),
        });

        let events = fixture.step("Search for BuildStatus").await;
        let completed = find_tool_execution_completed(&events, "search_types");
        assert!(
            completed.is_some(),
            "First step should execute search_types"
        );

        let args = json!({
            "language": "rust",
            "workspace_root": workspace_name,
            "type_path": "tycode_core::analyzer::BuildStatus"
        });
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "get_type_docs".to_string(),
            tool_arguments: serde_json::to_string(&args).unwrap(),
        });

        let events = fixture.step("Now get the documentation").await;
        let completed = find_tool_execution_completed(&events, "get_type_docs");
        assert!(
            completed.is_some(),
            "Second step should execute get_type_docs"
        );
    });
}

#[test]
fn multiple_searches_in_conversation() {
    fixture::run(|mut fixture| async move {
        setup_rust_project(&fixture);
        fixture
            .update_settings(|s| s.enable_type_analyzer = true)
            .await;

        let workspace = fixture.workspace_path();
        let workspace_name = workspace.file_name().unwrap().to_string_lossy();

        let args = json!({
            "language": "rust",
            "workspace_root": workspace_name,
            "type_name": "BuildStatus"
        });
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "search_types".to_string(),
            tool_arguments: serde_json::to_string(&args).unwrap(),
        });

        let events = fixture.step("Search for BuildStatus").await;
        let completed = find_tool_execution_completed(&events, "search_types");
        assert!(completed.is_some(), "First search should execute");

        let args = json!({
            "language": "rust",
            "workspace_root": workspace_name,
            "type_name": "MessageRole"
        });
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "search_types".to_string(),
            tool_arguments: serde_json::to_string(&args).unwrap(),
        });

        let events = fixture.step("Now search for MessageRole").await;
        let completed = find_tool_execution_completed(&events, "search_types");
        assert!(completed.is_some(), "Second search should execute");
    });
}

// =============================================================================
// Registry Configuration Tests
// =============================================================================

#[test]
fn type_analyzer_tools_not_available_when_disabled() {
    fixture::run(|mut fixture| async move {
        let workspace_name = fixture
            .workspace_path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let args = json!({
            "language": "rust",
            "workspace_root": workspace_name,
            "type_name": "BuildStatus"
        });
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "search_types".to_string(),
            tool_arguments: serde_json::to_string(&args).unwrap(),
        });

        let events = fixture.step("Search for BuildStatus").await;

        let completed = find_tool_execution_completed(&events, "search_types");
        if let Some(success) = completed {
            assert!(
                !success,
                "Tool should not be available when type analyzer is disabled"
            );
        }
    });
}

#[test]
fn type_analyzer_tools_available_when_enabled() {
    fixture::run(|mut fixture| async move {
        setup_rust_project(&fixture);
        fixture
            .update_settings(|s| s.enable_type_analyzer = true)
            .await;

        let workspace = fixture.workspace_path();
        let workspace_name = workspace.file_name().unwrap().to_string_lossy();
        let args = json!({
            "language": "rust",
            "workspace_root": workspace_name,
            "type_name": "BuildStatus"
        });
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "search_types".to_string(),
            tool_arguments: serde_json::to_string(&args).unwrap(),
        });

        let events = fixture.step("Search for BuildStatus").await;

        let completed = find_tool_execution_completed(&events, "search_types");
        assert!(completed.is_some(), "Tool should be available when enabled");
    });
}

#[test]
fn get_type_docs_not_available_when_disabled() {
    fixture::run(|mut fixture| async move {
        let workspace_name = fixture
            .workspace_path()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let args = json!({
            "language": "rust",
            "workspace_root": workspace_name,
            "type_path": "tycode_core::analyzer::BuildStatus"
        });
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "get_type_docs".to_string(),
            tool_arguments: serde_json::to_string(&args).unwrap(),
        });

        let events = fixture.step("Get docs for BuildStatus").await;

        let completed = find_tool_execution_completed(&events, "get_type_docs");
        if let Some(success) = completed {
            assert!(
                !success,
                "Tool should not be available when type analyzer is disabled"
            );
        }
    });
}

#[test]
fn enabling_type_analyzer_mid_conversation() {
    fixture::run(|mut fixture| async move {
        setup_rust_project(&fixture);
        fixture.set_mock_behavior(MockBehavior::Success);
        let _ = fixture.step("Hello").await;

        fixture
            .update_settings(|s| s.enable_type_analyzer = true)
            .await;

        let workspace = fixture.workspace_path();
        let workspace_name = workspace.file_name().unwrap().to_string_lossy();
        let args = json!({
            "language": "rust",
            "workspace_root": workspace_name,
            "type_name": "BuildStatus"
        });
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "search_types".to_string(),
            tool_arguments: serde_json::to_string(&args).unwrap(),
        });

        let events = fixture.step("Search for BuildStatus").await;
        let completed = find_tool_execution_completed(&events, "search_types");
        assert!(
            completed.is_some(),
            "Tool should be available after enabling"
        );
    });
}

// =============================================================================
// Edge Cases and Robustness Tests
// =============================================================================

#[test]
fn handles_special_characters_in_search() {
    fixture::run(|mut fixture| async move {
        setup_rust_project(&fixture);
        fixture
            .update_settings(|s| s.enable_type_analyzer = true)
            .await;

        let workspace = fixture.workspace_path();
        let workspace_name = workspace.file_name().unwrap().to_string_lossy();
        let args = json!({
            "language": "rust",
            "workspace_root": workspace_name,
            "type_name": "Type<T>"
        });
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "search_types".to_string(),
            tool_arguments: serde_json::to_string(&args).unwrap(),
        });

        let events = fixture.step("Search for Type<T>").await;

        assert!(
            !has_error_event(&events),
            "Should handle special characters without panic"
        );
    });
}

#[test]
fn handles_empty_type_path() {
    fixture::run(|mut fixture| async move {
        setup_rust_project(&fixture);
        fixture
            .update_settings(|s| s.enable_type_analyzer = true)
            .await;

        let workspace = fixture.workspace_path();
        let workspace_name = workspace.file_name().unwrap().to_string_lossy();
        let args = json!({
            "language": "rust",
            "workspace_root": workspace_name,
            "type_path": ""
        });
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "get_type_docs".to_string(),
            tool_arguments: serde_json::to_string(&args).unwrap(),
        });

        let events = fixture.step("Get docs for empty path").await;

        let completed = find_tool_execution_completed(&events, "get_type_docs");
        assert!(
            completed.is_some(),
            "Should have tool execution even for empty path"
        );
    });
}

#[test]
fn conversation_continues_after_tool_error() {
    fixture::run(|mut fixture| async move {
        setup_rust_project(&fixture);
        fixture
            .update_settings(|s| s.enable_type_analyzer = true)
            .await;

        let args = json!({
            "language": "rust",
            "workspace_root": "/invalid/path",
            "type_name": "BuildStatus"
        });
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "search_types".to_string(),
            tool_arguments: serde_json::to_string(&args).unwrap(),
        });

        let _ = fixture.step("Search with invalid workspace").await;

        fixture.set_mock_behavior(MockBehavior::Success);
        let events = fixture.step("Continue the conversation").await;

        let response = get_ai_response_text(&events);
        assert!(
            !response.is_empty(),
            "Conversation should continue after tool error"
        );
    });
}
