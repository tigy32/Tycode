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

#[test]
fn test_pin_file_appears_in_context() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();
        std::fs::write(workspace_path.join("pinned.txt"), "pinned content here").unwrap();

        fixture.set_mock_behavior(MockBehavior::Success);
        fixture.step("/@ pinned.txt").await;

        fixture.clear_captured_requests();
        fixture.set_mock_behavior(MockBehavior::Success);
        fixture.step("What's in the pinned file?").await;

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
            "Should have Tracked Files section. Content: {}",
            user_content
        );
        assert!(
            user_content.contains("pinned.txt"),
            "Should contain pinned.txt. Content: {}",
            user_content
        );
        assert!(
            user_content.contains("pinned content here"),
            "Should contain file contents. Content: {}",
            user_content
        );
    });
}

#[test]
fn test_pinned_files_persist_after_ai_clears_tracked() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();
        std::fs::write(workspace_path.join("persistent.txt"), "persistent content").unwrap();

        fixture.set_mock_behavior(MockBehavior::Success);
        fixture.step("/@ persistent.txt").await;

        fixture.clear_captured_requests();
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "set_tracked_files".to_string(),
            tool_arguments: r#"{"file_paths": []}"#.to_string(),
        });
        fixture.step("Clear all tracked files").await;

        fixture.clear_captured_requests();
        fixture.set_mock_behavior(MockBehavior::Success);
        fixture.step("What files do we have?").await;

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
            user_content.contains("persistent.txt"),
            "Pinned file should persist after AI clear. Content: {}",
            user_content
        );
        assert!(
            user_content.contains("persistent content"),
            "Pinned file contents should persist. Content: {}",
            user_content
        );
    });
}

#[test]
fn test_pin_all_files() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();
        std::fs::write(workspace_path.join("file1.txt"), "content one").unwrap();
        std::fs::write(workspace_path.join("file2.txt"), "content two").unwrap();

        fixture.set_mock_behavior(MockBehavior::Success);
        fixture.step("/@ all").await;

        fixture.clear_captured_requests();
        fixture.set_mock_behavior(MockBehavior::Success);
        fixture.step("What files are pinned?").await;

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
            user_content.contains("file1.txt"),
            "Should contain file1.txt. Content: {}",
            user_content
        );
        assert!(
            user_content.contains("file2.txt"),
            "Should contain file2.txt. Content: {}",
            user_content
        );
        assert!(
            user_content.contains("content one"),
            "Should contain file1 contents. Content: {}",
            user_content
        );
        assert!(
            user_content.contains("content two"),
            "Should contain file2 contents. Content: {}",
            user_content
        );
    });
}

#[test]
fn test_pin_clear_removes_pinned() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();
        std::fs::write(workspace_path.join("clearme.txt"), "will be cleared").unwrap();

        fixture.set_mock_behavior(MockBehavior::Success);
        fixture.step("/@ clearme.txt").await;

        fixture.set_mock_behavior(MockBehavior::Success);
        fixture.step("/@ clear").await;

        fixture.clear_captured_requests();
        fixture.set_mock_behavior(MockBehavior::Success);
        fixture.step("What files are tracked?").await;

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
            !user_content.contains("Tracked Files:"),
            "Should not have Tracked Files section after clear. Content: {}",
            user_content
        );
    });
}

#[test]
fn test_pin_list_shows_pinned_files() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();
        std::fs::write(workspace_path.join("listed1.txt"), "content").unwrap();
        std::fs::write(workspace_path.join("listed2.txt"), "content").unwrap();

        fixture.set_mock_behavior(MockBehavior::Success);
        fixture.step("/@ listed1.txt").await;
        fixture.set_mock_behavior(MockBehavior::Success);
        fixture.step("/@ listed2.txt").await;

        let events = fixture.step("/@ list").await;

        let system_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                tycode_core::chat::events::ChatEvent::MessageAdded(msg)
                    if msg.sender == tycode_core::chat::events::MessageSender::System =>
                {
                    Some(msg.content.clone())
                }
                _ => None,
            })
            .collect();

        let combined = system_messages.join(" ");
        assert!(
            combined.contains("listed1.txt"),
            "List should show listed1.txt. Messages: {}",
            combined
        );
        assert!(
            combined.contains("listed2.txt"),
            "List should show listed2.txt. Messages: {}",
            combined
        );
    });
}

#[test]
fn test_ai_tracking_pinned_file_no_duplicate() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();
        std::fs::write(workspace_path.join("shared.txt"), "shared content").unwrap();

        fixture.set_mock_behavior(MockBehavior::Success);
        fixture.step("/@ shared.txt").await;

        fixture.clear_captured_requests();
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "set_tracked_files".to_string(),
            tool_arguments: r#"{"file_paths": ["shared.txt"]}"#.to_string(),
        });
        fixture.step("Track shared.txt").await;

        fixture.clear_captured_requests();
        fixture.set_mock_behavior(MockBehavior::Success);
        fixture.step("Show me the files").await;

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

        let count = user_content.matches("shared content").count();
        assert!(
            count == 1,
            "File content should appear exactly once, found {} times. Content: {}",
            count,
            user_content
        );
    });
}

#[test]
fn test_pin_nonexistent_file_returns_error() {
    fixture::run(|mut fixture| async move {
        let events = fixture.step("/@ nonexistent.txt").await;

        let all_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                tycode_core::chat::events::ChatEvent::MessageAdded(msg) => {
                    Some(msg.content.clone())
                }
                _ => None,
            })
            .collect();

        let combined = all_messages.join(" ");
        assert!(
            combined.contains("not found")
                || combined.contains("Not found")
                || combined.contains("error")
                || combined.contains("Error"),
            "Should return error for nonexistent file. Messages: {}",
            combined
        );
    });
}
