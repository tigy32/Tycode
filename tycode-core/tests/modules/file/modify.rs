//! File modification module simulation tests.
//!
//! Tests for `src/file/modify.rs`

#[path = "../../fixture.rs"]
mod fixture;

use fixture::MockBehavior;

#[test]
fn test_write_file_creates_new_file() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();
        let test_file = workspace_path.join("new_file.txt");

        assert!(!test_file.exists(), "File should not exist before test");

        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "write_file".to_string(),
            tool_arguments: serde_json::json!({
                "file_path": test_file.display().to_string(),
                "content": "Hello, World!"
            })
            .to_string(),
        });
        fixture.step("Create a new file").await;

        assert!(test_file.exists(), "File should exist after write_file");
        let content = std::fs::read_to_string(&test_file).unwrap();
        assert_eq!(content, "Hello, World!", "File content should match");
    });
}

#[test]
fn test_delete_file_removes_file() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();
        let test_file = workspace_path.join("to_delete.txt");

        std::fs::write(&test_file, "delete me").unwrap();
        assert!(test_file.exists(), "File should exist before deletion");

        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "delete_file".to_string(),
            tool_arguments: serde_json::json!({
                "file_path": test_file.display().to_string()
            })
            .to_string(),
        });
        fixture.step("Delete the file").await;

        assert!(
            !test_file.exists(),
            "File should not exist after delete_file"
        );
    });
}

#[test]
fn test_modify_file_applies_changes() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();
        let test_file = workspace_path.join("modify_me.txt");

        std::fs::write(&test_file, "line 1\nline 2\nline 3\n").unwrap();

        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "modify_file".to_string(),
            tool_arguments: serde_json::json!({
                "file_path": test_file.display().to_string(),
                "diff": [{"search": "line 2", "replace": "modified line"}]
            })
            .to_string(),
        });
        fixture.step("Modify line 2").await;

        let content = std::fs::read_to_string(&test_file).unwrap();
        assert!(
            content.contains("modified line"),
            "File should contain modified content. Content: {}",
            content
        );
        assert!(
            !content.contains("line 2"),
            "File should not contain original line 2. Content: {}",
            content
        );
    });
}

#[test]
fn test_write_file_overwrites_existing() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();
        let test_file = workspace_path.join("overwrite.txt");

        std::fs::write(&test_file, "original content").unwrap();

        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "write_file".to_string(),
            tool_arguments: serde_json::json!({
                "file_path": test_file.display().to_string(),
                "content": "new content"
            })
            .to_string(),
        });
        fixture.step("Overwrite the file").await;

        let content = std::fs::read_to_string(&test_file).unwrap();
        assert_eq!(content, "new content", "File should have new content");
    });
}

#[test]
fn test_modify_file_with_real_absolute_path() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();
        let test_file = workspace_path.join("real_path_test.txt");

        std::fs::write(&test_file, "original content here\n").unwrap();

        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "modify_file".to_string(),
            tool_arguments: serde_json::json!({
                "file_path": test_file.display().to_string(),
                "diff": [{"search": "original content here", "replace": "real path content"}]
            })
            .to_string(),
        });
        fixture.step("Modify via real path").await;

        let content = std::fs::read_to_string(&test_file).unwrap();
        assert!(
            content.contains("real path content"),
            "real path should resolve and apply modification. Content: {}",
            content
        );
    });
}
