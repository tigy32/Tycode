//! Read-only file access module simulation tests.
//!
//! Tests for `src/file/read_only.rs`

#[path = "../../fixture.rs"]
mod fixture;

use fixture::MockBehavior;
use tycode_core::ai::MessageRole;

#[test]
fn test_set_tracked_files_contents_appear_in_context() {
    fixture::run(|mut fixture| async move {
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "set_tracked_files".to_string(),
            tool_arguments: r#"{"file_paths": ["example.txt"]}"#.to_string(),
        });
        fixture.step("Track example.txt").await;

        fixture.clear_captured_requests();
        fixture.set_mock_behavior(MockBehavior::Success);
        fixture.step("What's in the file?").await;

        // Context is appended to the LAST USER MESSAGE, not to system_prompt
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
            user_content.contains("Tracked Files:"),
            "Should have Tracked Files section in user message. Content: {}",
            user_content
        );

        assert!(
            user_content.contains("=== ") && user_content.contains("example.txt ==="),
            "Should have file marker for example.txt. Content: {}",
            user_content
        );

        let tracked_section = user_content
            .split("Tracked Files:")
            .nth(1)
            .expect("should have tracked files section");
        assert!(
            tracked_section.contains("test content"),
            "Tracked files section should contain file contents. Section: {}",
            tracked_section
        );
    });
}

#[test]
fn test_file_tree_appears_in_context() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();

        std::fs::create_dir_all(workspace_path.join("src")).unwrap();
        std::fs::write(workspace_path.join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(workspace_path.join("Cargo.toml"), "[package]").unwrap();

        fixture.set_mock_behavior(MockBehavior::Success);
        fixture.step("Hello").await;

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
            user_content.contains("Project Files:"),
            "Should have Project Files section in context. Content: {}",
            user_content
        );

        assert!(
            user_content.contains("main.rs"),
            "File tree should contain main.rs. Content: {}",
            user_content
        );
        assert!(
            user_content.contains("Cargo.toml"),
            "File tree should contain Cargo.toml. Content: {}",
            user_content
        );
    });
}

#[test]
fn test_file_tree_respects_gitignore() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();

        std::process::Command::new("git")
            .arg("init")
            .current_dir(&workspace_path)
            .output()
            .expect("Failed to init git repo");

        std::fs::write(workspace_path.join(".gitignore"), "*.log\ntarget/\n").unwrap();

        std::fs::write(workspace_path.join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(workspace_path.join("debug.log"), "logs").unwrap();
        std::fs::create_dir_all(workspace_path.join("target")).unwrap();
        std::fs::write(workspace_path.join("target/build.out"), "output").unwrap();

        fixture.set_mock_behavior(MockBehavior::Success);
        fixture.step("Hello").await;

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
            user_content.contains("main.rs"),
            "Non-ignored file main.rs should appear. Content: {}",
            user_content
        );

        assert!(
            !user_content.contains("debug.log"),
            "Ignored file debug.log should not appear. Content: {}",
            user_content
        );
        assert!(
            !user_content.contains("build.out"),
            "Ignored file target/build.out should not appear. Content: {}",
            user_content
        );
    });
}
