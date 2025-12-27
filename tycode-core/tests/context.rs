use tycode_core::chat::events::ChatEvent;

mod fixture;

/// Regression test: context should not hang when workspace directory is deleted
/// after the ChatActor is initialized. This simulates VSCode multi-workspace
/// scenarios where a folder is removed from disk while still referenced.
#[test]
fn test_deleted_workspace_directory_does_not_hang() {
    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::time::{timeout, Duration};
    use tycode_core::{
        ai::mock::{MockBehavior, MockProvider},
        chat::actor::ChatActorBuilder,
        settings::{manager::SettingsManager, Settings},
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");

    let local = tokio::task::LocalSet::new();

    runtime.block_on(local.run_until(async {
        let workspace1 = TempDir::new().unwrap();
        let workspace2 = TempDir::new().unwrap();

        let workspace1_path = workspace1.path().to_path_buf();
        let workspace2_path = workspace2.path().to_path_buf();

        std::fs::write(workspace1_path.join("file1.txt"), "content1").unwrap();
        std::fs::write(workspace2_path.join("file2.txt"), "content2").unwrap();

        let tycode_dir = workspace1_path.join(".tycode");
        std::fs::create_dir_all(&tycode_dir).unwrap();
        let settings_path = tycode_dir.join("settings.toml");
        let settings_manager = SettingsManager::from_path(settings_path.clone()).unwrap();

        let mut default_settings = Settings::default();
        default_settings.add_provider(
            "mock".to_string(),
            tycode_core::settings::ProviderConfig::Mock {
                behavior: MockBehavior::Success,
            },
        );
        default_settings.active_provider = Some("mock".to_string());
        settings_manager.save_settings(default_settings).unwrap();

        let mock_provider = MockProvider::new(MockBehavior::Success);

        let (actor, mut event_rx) = ChatActorBuilder::new(
            vec![workspace1_path.clone(), workspace2_path.clone()],
            tycode_dir,
        )
        .provider(Arc::new(mock_provider))
        .build()
        .unwrap();

        // Ensure actor initialization completes before deleting workspace
        actor.send_message("hello".to_string()).unwrap();
        while let Some(event) = event_rx.recv().await {
            if matches!(event, ChatEvent::TypingStatusChanged(false)) {
                break;
            }
        }

        drop(workspace2);

        actor.send_message("/context".to_string()).unwrap();
        let result = timeout(Duration::from_secs(5), async {
            let mut has_response = false;
            while let Some(event) = event_rx.recv().await {
                if matches!(event, ChatEvent::MessageAdded(_)) {
                    has_response = true;
                }
                if matches!(event, ChatEvent::TypingStatusChanged(false)) {
                    break;
                }
            }
            has_response
        })
        .await;

        match result {
            Ok(has_response) => {
                assert!(has_response, "Should receive a response message, not hang");
            }
            Err(_) => {
                panic!("Test timed out after 5 seconds - context command is hanging!");
            }
        }
    }));
}

#[test]
fn test_git_ignore_rules_respected() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();

        std::process::Command::new("git")
            .arg("init")
            .current_dir(&workspace_path)
            .output()
            .expect("Failed to init git repo");

        std::fs::write(workspace_path.join(".gitignore"), "*.log\ntarget/\n.env\n").unwrap();

        std::fs::write(workspace_path.join("debug.log"), "log content").unwrap();
        std::fs::write(workspace_path.join(".env"), "SECRET=123").unwrap();
        std::fs::create_dir_all(workspace_path.join("target")).unwrap();
        std::fs::write(workspace_path.join("target/output.txt"), "output").unwrap();

        std::fs::write(workspace_path.join("src.rs"), "fn main() {}").unwrap();
        std::fs::write(workspace_path.join("README.md"), "# Project").unwrap();

        let events = fixture.step("/context").await;

        let response_text: String = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) => Some(msg.content.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            !response_text.contains("debug.log"),
            "Ignored file debug.log should not appear in response"
        );
        assert!(
            !response_text.contains(".env"),
            "Ignored file .env should not appear in response"
        );
        assert!(
            !response_text.contains("target/output.txt") && !response_text.contains("target/"),
            "Ignored directory target/ should not appear in response"
        );

        assert!(
            response_text.contains("src.rs"),
            "Non-ignored file src.rs should appear in response. Response: {}",
            response_text
        );
        assert!(
            response_text.contains("README.md"),
            "Non-ignored file README.md should appear in response. Response: {}",
            response_text
        );
    });
}

#[test]
fn test_non_git_repo_shows_all_files() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();

        std::fs::write(workspace_path.join(".gitignore"), "*.log\n").unwrap();

        std::fs::write(workspace_path.join("debug.log"), "log content").unwrap();
        std::fs::write(workspace_path.join("main.rs"), "fn main() {}").unwrap();

        let events = fixture.step("/context").await;

        let response_text: String = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) => Some(msg.content.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            response_text.contains("debug.log"),
            "Without git repo, .gitignore rules should not apply. File debug.log should appear. Response: {}",
            response_text
        );
        assert!(
            response_text.contains("main.rs"),
            "File main.rs should appear in response. Response: {}",
            response_text
        );
    });
}

#[test]
fn test_nested_gitignore_patterns() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();

        std::process::Command::new("git")
            .arg("init")
            .current_dir(&workspace_path)
            .output()
            .expect("Failed to init git repo");

        std::fs::write(workspace_path.join(".gitignore"), "*.tmp\n").unwrap();

        std::fs::create_dir_all(workspace_path.join("src")).unwrap();
        std::fs::write(workspace_path.join("src/lib.rs"), "// lib").unwrap();
        std::fs::write(workspace_path.join("src/cache.tmp"), "cache").unwrap();
        std::fs::write(workspace_path.join("root.tmp"), "root cache").unwrap();

        let events = fixture.step("/context").await;

        let response_text: String = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) => Some(msg.content.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            !response_text.contains("cache.tmp"),
            "Ignored nested file cache.tmp should not appear. Response: {}",
            response_text
        );
        assert!(
            !response_text.contains("root.tmp"),
            "Ignored root file root.tmp should not appear. Response: {}",
            response_text
        );
        assert!(
            response_text.contains("lib.rs"),
            "Non-ignored file lib.rs should appear. Response: {}",
            response_text
        );
    });
}

#[test]
fn test_large_file_list_truncation() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();

        // Create many files to exceed 20KB threshold for file list.
        // File paths themselves contribute to the byte count, so we create
        // approximately 2400 files with moderately long paths (~50-60 bytes each)
        // to ensure we exceed the 20,000 byte (20KB) threshold.
        for i in 0..2400 {
            let dir = format!("directory_{:02}", i / 100);
            let filename = format!("file_with_long_name_for_testing_{:03}.rs", i);
            let path = workspace_path.join(&dir).join(&filename);

            std::fs::create_dir_all(path.parent().unwrap()).unwrap();

            // Write file content (small enough to not matter)
            std::fs::write(&path, "// test\n").unwrap();
        }

        let events = fixture.step("/context").await;

        let response_text: String = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) => Some(msg.content.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        // The file list should be included (not disabled)
        let has_file_list = response_text.contains("file_with_long_name_for_testing");
        assert!(
            has_file_list,
            "File list should be included even when large. Response: {}",
            response_text
        );

        // Count how many files are actually listed
        let file_count = response_text
            .matches("file_with_long_name_for_testing")
            .count();

        // The file list should be truncated (not all 2400 files should be listed)
        assert!(
            file_count < 2400,
            "File list should be truncated to fit byte limit. Found {} files but created 2400",
            file_count
        );

        // Should have at least some files (BFS should collect some before hitting limit)
        assert!(
            file_count > 0,
            "File list should contain at least some files. Found {} files",
            file_count
        );
    });
}

#[test]
fn test_multiple_git_repos_with_separate_gitignores() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();

        let project_a_path = workspace_path.join("project_a");
        let project_b_path = workspace_path.join("project_b");

        std::fs::create_dir_all(&project_a_path).unwrap();
        std::fs::create_dir_all(&project_b_path).unwrap();

        std::process::Command::new("git")
            .arg("init")
            .current_dir(&project_a_path)
            .output()
            .expect("Failed to init git repo for project_a");

        std::process::Command::new("git")
            .arg("init")
            .current_dir(&project_b_path)
            .output()
            .expect("Failed to init git repo for project_b");

        std::fs::write(project_a_path.join(".gitignore"), "*.log\n").unwrap();
        std::fs::write(project_a_path.join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(project_a_path.join("debug.log"), "debug logs").unwrap();

        std::fs::write(project_b_path.join(".gitignore"), "*.tmp\n").unwrap();
        std::fs::write(project_b_path.join("lib.rs"), "pub fn lib() {}").unwrap();
        std::fs::write(project_b_path.join("cache.tmp"), "cache data").unwrap();

        let events = fixture.step("/context").await;

        let response_text: String = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) => Some(msg.content.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            response_text.contains("main.rs"),
            "Non-ignored file project_a/main.rs should appear. Response: {}",
            response_text
        );
        assert!(
            !response_text.contains("debug.log"),
            "File project_a/debug.log should be ignored by project_a's .gitignore. Response: {}",
            response_text
        );

        assert!(
            response_text.contains("lib.rs"),
            "Non-ignored file project_b/lib.rs should appear. Response: {}",
            response_text
        );
        assert!(
            !response_text.contains("cache.tmp"),
            "File project_b/cache.tmp should be ignored by project_b's .gitignore. Response: {}",
            response_text
        );

        assert!(
            !response_text.contains(".git/"),
            ".git directories must never appear in context (models are not allowed to touch .git). Response: {}",
            response_text
        );
    });
}

#[test]
fn test_gitignore_when_workspace_is_parent_of_git_repo() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();

        // Create a subdirectory that will be the actual git repo
        let git_repo_path = workspace_path.join("my_project");
        std::fs::create_dir_all(&git_repo_path).unwrap();

        // Initialize git repo in the subdirectory (NOT in workspace_path)
        std::process::Command::new("git")
            .arg("init")
            .current_dir(&git_repo_path)
            .output()
            .expect("Failed to init git repo");

        // Create .gitignore in the git repo subdirectory
        std::fs::write(
            git_repo_path.join(".gitignore"),
            "node_modules/\n*.log\ntarget/\n",
        )
        .unwrap();

        // Create some files that should be ignored
        std::fs::create_dir_all(git_repo_path.join("node_modules")).unwrap();
        std::fs::write(
            git_repo_path.join("node_modules/package.json"),
            r#"{"name": "test"}"#,
        )
        .unwrap();
        std::fs::write(
            git_repo_path.join("node_modules/index.js"),
            "module.exports = {}",
        )
        .unwrap();

        std::fs::create_dir_all(git_repo_path.join("target")).unwrap();
        std::fs::write(git_repo_path.join("target/build.out"), "build output").unwrap();

        std::fs::write(git_repo_path.join("debug.log"), "debug logs").unwrap();

        // Create some files that should NOT be ignored
        std::fs::create_dir_all(git_repo_path.join("src")).unwrap();
        std::fs::write(git_repo_path.join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(git_repo_path.join("Cargo.toml"), "[package]").unwrap();
        std::fs::write(git_repo_path.join("README.md"), "# Project").unwrap();

        // Also create a file in the workspace root (parent of git repo)
        std::fs::write(workspace_path.join("notes.txt"), "some notes").unwrap();

        let events = fixture.step("/context").await;

        let response_text: String = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) => Some(msg.content.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        // Files inside node_modules/ should be ignored
        assert!(
            !response_text.contains("node_modules"),
            "Ignored directory node_modules/ should not appear in response. Response: {}",
            response_text
        );
        assert!(
            !response_text.contains("package.json") || !response_text.contains("my_project"),
            "Ignored file node_modules/package.json should not appear. Response: {}",
            response_text
        );
        assert!(
            !response_text.contains("index.js") || !response_text.contains("my_project"),
            "Ignored file node_modules/index.js should not appear. Response: {}",
            response_text
        );

        // Files in target/ should be ignored
        assert!(
            !response_text.contains("target/"),
            "Ignored directory target/ should not appear. Response: {}",
            response_text
        );
        assert!(
            !response_text.contains("build.out"),
            "Ignored file target/build.out should not appear. Response: {}",
            response_text
        );

        // *.log files should be ignored
        assert!(
            !response_text.contains("debug.log"),
            "Ignored file debug.log should not appear. Response: {}",
            response_text
        );

        // Non-ignored files should appear
        assert!(
            response_text.contains("main.rs"),
            "Non-ignored file src/main.rs should appear. Response: {}",
            response_text
        );
        assert!(
            response_text.contains("Cargo.toml"),
            "Non-ignored file Cargo.toml should appear. Response: {}",
            response_text
        );
        assert!(
            response_text.contains("README.md"),
            "Non-ignored file README.md should appear. Response: {}",
            response_text
        );

        // File in parent workspace should appear (not part of git repo)
        assert!(
            response_text.contains("notes.txt"),
            "File in workspace root (parent of git repo) should appear. Response: {}",
            response_text
        );

        // .git directory should never appear
        assert!(
            !response_text.contains(".git/"),
            ".git directory should never appear. Response: {}",
            response_text
        );
    });
}

#[test]
fn test_nested_gitignore_files_in_subdirectories() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();

        std::process::Command::new("git")
            .arg("init")
            .current_dir(&workspace_path)
            .output()
            .expect("Failed to init git repo");

        // Root .gitignore
        std::fs::write(workspace_path.join(".gitignore"), "*.log\n").unwrap();

        // Nested .gitignore in src/
        std::fs::create_dir_all(workspace_path.join("src")).unwrap();
        std::fs::write(workspace_path.join("src/.gitignore"), "*.tmp\n*.cache\n").unwrap();

        // Nested .gitignore in docs/
        std::fs::create_dir_all(workspace_path.join("docs")).unwrap();
        std::fs::write(workspace_path.join("docs/.gitignore"), "draft/\n").unwrap();

        // Create files that should be ignored by root .gitignore
        std::fs::write(workspace_path.join("debug.log"), "debug").unwrap();
        std::fs::write(workspace_path.join("src/build.log"), "build log").unwrap();

        // Create files that should be ignored by src/.gitignore
        std::fs::write(workspace_path.join("src/cache.tmp"), "cache").unwrap();
        std::fs::write(workspace_path.join("src/data.cache"), "data").unwrap();

        // Create files that should be ignored by docs/.gitignore
        std::fs::create_dir_all(workspace_path.join("docs/draft")).unwrap();
        std::fs::write(workspace_path.join("docs/draft/notes.md"), "draft notes").unwrap();

        // Create files that should NOT be ignored
        std::fs::write(workspace_path.join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(workspace_path.join("docs/README.md"), "# Docs").unwrap();
        std::fs::write(workspace_path.join("Cargo.toml"), "[package]").unwrap();

        let events = fixture.step("/context").await;

        let response_text: String = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) => Some(msg.content.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        // Files ignored by root .gitignore
        assert!(
            !response_text.contains("debug.log"),
            "Root-level debug.log should be ignored by root .gitignore. Response: {}",
            response_text
        );
        assert!(
            !response_text.contains("build.log"),
            "src/build.log should be ignored by root .gitignore (*.log). Response: {}",
            response_text
        );

        // Files ignored by src/.gitignore
        assert!(
            !response_text.contains("cache.tmp"),
            "src/cache.tmp should be ignored by src/.gitignore. Response: {}",
            response_text
        );
        assert!(
            !response_text.contains("data.cache"),
            "src/data.cache should be ignored by src/.gitignore. Response: {}",
            response_text
        );

        // Files ignored by docs/.gitignore
        assert!(
            !response_text.contains("draft/") && !response_text.contains("draft"),
            "docs/draft/ should be ignored by docs/.gitignore. Response: {}",
            response_text
        );
        assert!(
            !response_text.contains("notes.md") || !response_text.contains("draft"),
            "docs/draft/notes.md should be ignored by docs/.gitignore. Response: {}",
            response_text
        );

        // Non-ignored files should appear
        assert!(
            response_text.contains("main.rs"),
            "src/main.rs should not be ignored. Response: {}",
            response_text
        );
        assert!(
            response_text.contains("README.md"),
            "docs/README.md should not be ignored. Response: {}",
            response_text
        );
        assert!(
            response_text.contains("Cargo.toml"),
            "Cargo.toml should not be ignored. Response: {}",
            response_text
        );
    });
}

#[test]
fn test_nested_gitignore_with_parent_workspace_bfs_path() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();

        // Workspace root is NOT a git repo (forces BFS path)
        // Git repo is in a subdirectory
        let git_repo_path = workspace_path.join("project");
        std::fs::create_dir_all(&git_repo_path).unwrap();

        std::process::Command::new("git")
            .arg("init")
            .current_dir(&git_repo_path)
            .output()
            .expect("Failed to init git repo");

        // Root .gitignore
        std::fs::write(git_repo_path.join(".gitignore"), "*.log\n").unwrap();

        // Nested .gitignore in src/
        std::fs::create_dir_all(git_repo_path.join("src")).unwrap();
        std::fs::write(git_repo_path.join("src/.gitignore"), "*.tmp\n*.cache\n").unwrap();

        // Create files that should be ignored by root .gitignore
        std::fs::write(git_repo_path.join("debug.log"), "debug").unwrap();
        std::fs::write(git_repo_path.join("src/build.log"), "build log").unwrap();

        // Create files that should be ignored by src/.gitignore
        std::fs::write(git_repo_path.join("src/cache.tmp"), "cache").unwrap();
        std::fs::write(git_repo_path.join("src/data.cache"), "data").unwrap();

        // Create files that should NOT be ignored
        std::fs::write(git_repo_path.join("src/main.rs"), "fn main() {}").unwrap();
        std::fs::write(git_repo_path.join("Cargo.toml"), "[package]").unwrap();

        let events = fixture.step("/context").await;

        let response_text: String = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) => Some(msg.content.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        // Files ignored by root .gitignore
        assert!(
            !response_text.contains("debug.log"),
            "debug.log should be ignored by root .gitignore. Response: {}",
            response_text
        );
        assert!(
            !response_text.contains("build.log"),
            "src/build.log should be ignored by root .gitignore (*.log). Response: {}",
            response_text
        );

        // Files ignored by src/.gitignore - THIS WILL LIKELY FAIL
        assert!(
            !response_text.contains("cache.tmp"),
            "src/cache.tmp should be ignored by nested src/.gitignore. Response: {}",
            response_text
        );
        assert!(
            !response_text.contains("data.cache"),
            "src/data.cache should be ignored by nested src/.gitignore. Response: {}",
            response_text
        );

        // Non-ignored files should appear
        assert!(
            response_text.contains("main.rs"),
            "src/main.rs should not be ignored. Response: {}",
            response_text
        );
        assert!(
            response_text.contains("Cargo.toml"),
            "Cargo.toml should not be ignored. Response: {}",
            response_text
        );
    });
}

#[test]
fn test_hidden_files_visibility() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();

        std::process::Command::new("git")
            .arg("init")
            .current_dir(&workspace_path)
            .output()
            .expect("Failed to init git repo");

        std::fs::write(
            workspace_path.join(".gitignore"),
            ".tycode/\n.brazil/\n*.log\n",
        )
        .unwrap();

        // Create hidden files that should be visible (NOT in .gitignore)
        std::fs::create_dir_all(workspace_path.join(".github/workflows")).unwrap();
        std::fs::write(
            workspace_path.join(".github/workflows/ci.yml"),
            "name: CI\non: [push]\n",
        )
        .unwrap();

        // Create hidden files that should NOT be visible (in .gitignore)
        std::fs::create_dir_all(workspace_path.join(".tycode")).unwrap();
        std::fs::write(workspace_path.join(".tycode/config.toml"), "test config").unwrap();

        std::fs::create_dir_all(workspace_path.join(".brazil")).unwrap();
        std::fs::write(workspace_path.join(".brazil/settings"), "brazil settings").unwrap();

        // Create regular files that should be visible
        std::fs::write(workspace_path.join("README.md"), "# Project").unwrap();
        std::fs::write(workspace_path.join("main.rs"), "fn main() {}").unwrap();

        // Create files that should be ignored by pattern
        std::fs::write(workspace_path.join("debug.log"), "log content").unwrap();

        let events = fixture.step("/context").await;

        let response_text: String = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) => Some(msg.content.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        // Hidden files NOT in .gitignore should appear
        assert!(
            response_text.contains(".github"),
            "Hidden directory .github should appear in response. Response: {}",
            response_text
        );
        assert!(
            response_text.contains("ci.yml") || response_text.contains("workflows"),
            "File .github/workflows/ci.yml should appear in response. Response: {}",
            response_text
        );

        // Hidden files in .gitignore should NOT appear
        assert!(
            !response_text.contains(".tycode"),
            "Ignored hidden directory .tycode should not appear in response. Response: {}",
            response_text
        );
        assert!(
            !response_text.contains("config.toml") || !response_text.contains(".tycode"),
            "Ignored file .tycode/config.toml should not appear in response. Response: {}",
            response_text
        );

        assert!(
            !response_text.contains(".brazil"),
            "Ignored hidden directory .brazil should not appear in response. Response: {}",
            response_text
        );

        // .git directory should NEVER appear
        assert!(
            !response_text.contains(".git/"),
            ".git directory must never appear in context. Response: {}",
            response_text
        );

        // Regular files should appear
        assert!(
            response_text.contains("README.md"),
            "Regular file README.md should appear. Response: {}",
            response_text
        );
        assert!(
            response_text.contains("main.rs"),
            "Regular file main.rs should appear. Response: {}",
            response_text
        );

        // Files matching .gitignore patterns should not appear
        assert!(
            !response_text.contains("debug.log"),
            "Ignored file debug.log should not appear. Response: {}",
            response_text
        );
    });
}

#[test]
fn test_workspace_is_git_repo_with_nested_git_repos() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();

        // Workspace root IS a git repo
        std::process::Command::new("git")
            .arg("init")
            .current_dir(&workspace_path)
            .output()
            .expect("Failed to init git repo in workspace");

        // Root .gitignore
        std::fs::write(workspace_path.join(".gitignore"), "*.log\n").unwrap();

        // Create a nested git repo (not a submodule)
        let nested_repo = workspace_path.join("nested_project");
        std::fs::create_dir_all(&nested_repo).unwrap();
        std::process::Command::new("git")
            .arg("init")
            .current_dir(&nested_repo)
            .output()
            .expect("Failed to init nested git repo");

        // Nested repo has its own .gitignore
        std::fs::write(nested_repo.join(".gitignore"), "*.tmp\n").unwrap();

        // Files in root that should be ignored
        std::fs::write(workspace_path.join("root.log"), "root log").unwrap();

        // Files in nested repo that should be ignored by its .gitignore
        std::fs::write(nested_repo.join("data.tmp"), "temp data").unwrap();

        // Files in nested repo that should be ignored by root .gitignore
        std::fs::write(nested_repo.join("debug.log"), "debug log").unwrap();

        // Files that should NOT be ignored
        std::fs::write(workspace_path.join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(nested_repo.join("lib.rs"), "pub fn lib() {}").unwrap();

        let events = fixture.step("/context").await;

        let response_text: String = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) => Some(msg.content.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        // Root .log file should be ignored
        assert!(
            !response_text.contains("root.log"),
            "root.log should be ignored by root .gitignore. Response: {}",
            response_text
        );

        // Nested repo files should NOT appear because Git treats nested git repos
        // (that are not submodules) as opaque directories and doesn't recurse into them
        assert!(
            !response_text.contains("data.tmp"),
            "nested_project/data.tmp should not appear (nested git repo is ignored). Response: {}",
            response_text
        );

        assert!(
            !response_text.contains("debug.log"),
            "nested_project/debug.log should not appear (nested git repo is ignored). Response: {}",
            response_text
        );

        assert!(
            !response_text.contains("lib.rs") || !response_text.contains("nested"),
            "nested_project/lib.rs should not appear (nested git repo is ignored). Response: {}",
            response_text
        );

        // Non-ignored files in the root should appear
        assert!(
            response_text.contains("main.rs"),
            "main.rs should not be ignored. Response: {}",
            response_text
        );

        // The nested_project directory itself might appear, but files inside it should not
        // This matches Git's behavior: git ls-files shows "nested/" but not "nested/lib.rs"
    });
}

#[test]
fn test_set_tracked_files_contents_appear_in_context() {
    use tycode_core::ai::mock::MockBehavior;

    fixture::run(|mut fixture| async move {
        // Step 1: Have model call set_tracked_files
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "set_tracked_files".to_string(),
            tool_arguments: r#"{"file_paths": ["example.txt"]}"#.to_string(),
        });
        fixture.step("Track example.txt").await;

        // Step 2: Clear and send another message
        fixture.clear_captured_requests();
        fixture.set_mock_behavior(MockBehavior::Success);
        fixture.step("What's in the file?").await;

        // Step 3: Verify file contents appear in TRACKED FILES section
        // Context is appended to the LAST USER MESSAGE, not to system_prompt
        // (see request.rs - context_content is appended to last user message)
        let request = fixture
            .get_last_ai_request()
            .expect("should have captured request");

        // Get the last user message content
        let last_user_msg = request
            .messages
            .iter()
            .filter(|m| m.role == tycode_core::ai::MessageRole::User)
            .last()
            .expect("should have user message");
        let user_content = last_user_msg.content.text();

        // Check for the tracked files section header
        assert!(
            user_content.contains("Tracked Files:"),
            "Should have Tracked Files section in user message. Content: {}",
            user_content
        );

        // Check for the file marker format used by TrackedFilesManager
        assert!(
            user_content.contains("=== ") && user_content.contains("example.txt ==="),
            "Should have file marker for example.txt. Content: {}",
            user_content
        );

        // Check that file contents appear after the marker
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
