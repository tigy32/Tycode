//! File search module simulation tests.
//!
//! Tests for `src/file/search.rs`

#[path = "../../fixture.rs"]
mod fixture;

use fixture::MockBehavior;
use tycode_core::ai::MessageRole;

#[test]
fn test_search_files_finds_pattern() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();

        std::fs::create_dir_all(workspace_path.join("src")).unwrap();
        std::fs::write(
            workspace_path.join("src/main.rs"),
            "fn main() {\n    println!(\"hello\");\n}",
        )
        .unwrap();
        std::fs::write(
            workspace_path.join("src/lib.rs"),
            "pub fn greet() {\n    println!(\"hello\");\n}",
        )
        .unwrap();

        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "search_files".to_string(),
            tool_arguments: r#"{"directory_path": "src", "pattern": "println", "file_pattern": "*.rs", "max_results": 10, "include_context": false, "context_lines": 0}"#.to_string(),
        });
        fixture.step("Search for println in src").await;

        let request = fixture
            .get_last_ai_request()
            .expect("should have captured request");

        let last_user_msg = request
            .messages
            .iter()
            .filter(|m| m.role == MessageRole::User)
            .last()
            .expect("should have user message");
        let user_content = last_user_msg.content.text();

        assert!(
            user_content.contains("main.rs") || user_content.contains("lib.rs"),
            "Search results should mention files containing pattern. Content: {}",
            user_content
        );
    });
}

#[test]
fn test_list_files_shows_directory_contents() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();

        std::fs::create_dir_all(workspace_path.join("mydir")).unwrap();
        std::fs::write(workspace_path.join("mydir/file1.txt"), "content1").unwrap();
        std::fs::write(workspace_path.join("mydir/file2.txt"), "content2").unwrap();

        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "list_files".to_string(),
            tool_arguments: r#"{"directory_path": "mydir"}"#.to_string(),
        });
        fixture.step("List files in mydir").await;

        let request = fixture
            .get_last_ai_request()
            .expect("should have captured request");

        let last_user_msg = request
            .messages
            .iter()
            .filter(|m| m.role == MessageRole::User)
            .last()
            .expect("should have user message");
        let user_content = last_user_msg.content.text();

        assert!(
            user_content.contains("file1.txt") && user_content.contains("file2.txt"),
            "List should show both files. Content: {}",
            user_content
        );
    });
}
