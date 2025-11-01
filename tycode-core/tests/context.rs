use tycode_core::chat::events::{ChatEvent, MessageSender};

mod fixture;

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
fn test_large_file_list_warning() {
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

        let events = fixture.step("Show context").await;

        let has_warning = events.iter().any(|e| {
            matches!(
                e,
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Warning)
                    && msg.content.to_lowercase().contains("warning")
                    && (msg.content.to_lowercase().contains("file")
                        || msg.content.to_lowercase().contains("large"))
            )
        });

        assert!(
            has_warning,
            "Should send system warning about large file list when > 20KB. Events: {:#?}",
            events
        );

        let has_response = events.iter().any(|e| {
            matches!(
                e,
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Assistant { .. })
            )
        });

        assert!(
            has_response,
            "Should still receive assistant response even after large file warning. Events: {:#?}",
            events
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
