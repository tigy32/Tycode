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
            tool_arguments: r#"{"file_path": "new_file.txt", "content": "Hello, World!"}"#
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
            tool_arguments: r#"{"file_path": "to_delete.txt"}"#.to_string(),
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
            tool_arguments: r#"{"file_path": "modify_me.txt", "diff": [{"search": "line 2", "replace": "modified line"}]}"#.to_string(),
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
            tool_arguments: r#"{"file_path": "overwrite.txt", "content": "new content"}"#
                .to_string(),
        });
        fixture.step("Overwrite the file").await;

        let content = std::fs::read_to_string(&test_file).unwrap();
        assert_eq!(content, "new content", "File should have new content");
    });
}

#[test]
fn test_cline_format_modification_after_settings_change() {
    use tycode_core::file::config::File;
    use tycode_core::settings::config::FileModificationApi;

    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();
        let test_file = workspace_path.join("cline_test.txt");

        std::fs::write(&test_file, "first line\nsecond line\nthird line\n").unwrap();

        fixture
            .update_settings(|settings| {
                let file_config = File {
                    file_modification_api: FileModificationApi::ClineSearchReplace,
                    ..Default::default()
                };
                let value = serde_json::to_value(&file_config).unwrap();
                settings.modules.insert("file".to_string(), value);
            })
            .await;

        let cline_diff =
            "------- SEARCH\nsecond line\n=======\nmodified second line\n+++++++ REPLACE";

        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "modify_file".to_string(),
            tool_arguments: serde_json::json!({
                "path": "cline_test.txt",
                "diff": cline_diff
            })
            .to_string(),
        });
        fixture.step("Modify using Cline format").await;

        let content = std::fs::read_to_string(&test_file).unwrap();
        assert!(
            content.contains("modified second line"),
            "File should contain Cline-modified content. Content: {}",
            content
        );
        assert!(
            !content.contains("\nsecond line\n"),
            "File should not contain original second line. Content: {}",
            content
        );
    });
}

#[test]
fn test_cline_format_multiple_search_replace_blocks() {
    use tycode_core::file::config::File;
    use tycode_core::settings::config::FileModificationApi;

    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();
        let test_file = workspace_path.join("multi_block.txt");

        std::fs::write(&test_file, "alpha\nbeta\ngamma\ndelta\n").unwrap();

        fixture
            .update_settings(|settings| {
                let file_config = File {
                    file_modification_api: FileModificationApi::ClineSearchReplace,
                    ..Default::default()
                };
                let value = serde_json::to_value(&file_config).unwrap();
                settings.modules.insert("file".to_string(), value);
            })
            .await;

        let cline_diff = "------- SEARCH\nalpha\n=======\nALPHA\n+++++++ REPLACE\n------- SEARCH\ngamma\n=======\nGAMMA\n+++++++ REPLACE";

        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "modify_file".to_string(),
            tool_arguments: serde_json::json!({
                "path": "multi_block.txt",
                "diff": cline_diff
            })
            .to_string(),
        });
        fixture.step("Apply multiple Cline blocks").await;

        let content = std::fs::read_to_string(&test_file).unwrap();
        assert!(
            content.contains("ALPHA"),
            "First block should be applied. Content: {}",
            content
        );
        assert!(
            content.contains("GAMMA"),
            "Second block should be applied. Content: {}",
            content
        );
        assert!(
            content.contains("beta") && content.contains("delta"),
            "Unchanged lines should remain. Content: {}",
            content
        );
    });
}

#[test]
fn test_cline_format_with_vfs_absolute_path() {
    use tycode_core::file::config::File;
    use tycode_core::settings::config::FileModificationApi;

    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();
        let test_file = workspace_path.join("vfs_test.txt");

        std::fs::write(&test_file, "original content here\\n").unwrap();

        fixture
            .update_settings(|settings| {
                let file_config = File {
                    file_modification_api: FileModificationApi::ClineSearchReplace,
                    ..Default::default()
                };
                let value = serde_json::to_value(&file_config).unwrap();
                settings.modules.insert("file".to_string(), value);
            })
            .await;

        let workspace_name = workspace_path.file_name().unwrap().to_str().unwrap();
        let vfs_path = format!("/{}/vfs_test.txt", workspace_name);

        let cline_diff =
            "------- SEARCH\noriginal content here\n=======\nVFS resolved content\n+++++++ REPLACE";

        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "modify_file".to_string(),
            tool_arguments: serde_json::json!({
                "path": vfs_path,
                "diff": cline_diff
            })
            .to_string(),
        });
        fixture.step("Modify via VFS path").await;

        let content = std::fs::read_to_string(&test_file).unwrap();
        assert!(
            content.contains("VFS resolved content"),
            "VFS path should resolve and apply modification. Content: {}",
            content
        );
    });
}
