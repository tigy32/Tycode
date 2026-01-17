//! Memory module simulation tests.
//!
//! Tests for `src/modules/memory/`
//!
//! These tests validate the actual user value of the memory system:
//! 1. Can the AI store memories via the append_memory tool?
//! 2. Do stored memories appear in the AI's context in subsequent sessions?

#[path = "../fixture.rs"]
mod fixture;

use std::time::Duration;

use fixture::{MockBehavior, Workspace};

use tycode_core::ai::types::ContentBlock;
use tycode_core::settings::manager::SettingsManager;

/// Helper to enable memory in workspace settings before spawning session.
fn enable_memory_in_workspace(workspace: &Workspace) {
    let settings_path = workspace.tycode_dir().join("settings.toml");
    let settings_manager = SettingsManager::from_path(settings_path).unwrap();
    let mut settings = settings_manager.settings(); // Read existing settings to preserve other fields
    settings.memory.enabled = true;
    settings_manager.save_settings(settings).unwrap();
}

#[test]
fn append_memory_stores_to_disk() {
    use tokio::time::timeout;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let local = tokio::task::LocalSet::new();

    runtime.block_on(local.run_until(async {
        timeout(Duration::from_secs(30), async {
            let workspace = Workspace::new();
            enable_memory_in_workspace(&workspace);

            let behavior = MockBehavior::ToolUseThenSuccess {
                tool_name: "append_memory".to_string(),
                tool_arguments: r#"{"content": "user prefers vim keybindings"}"#.to_string(),
            };

            let mut session = workspace.spawn_session("one_shot", behavior);
            session.step("I like vim keybindings").await;
            drop(session);

            let memory_file = workspace.tycode_dir().join("memory/memories_log.json");
            let content = std::fs::read_to_string(&memory_file).expect("Memory file should exist");

            assert!(
                content.contains("user prefers vim keybindings"),
                "Memory content should be stored. File: {}",
                content
            );
        })
        .await
        .expect("Test timed out");
    }));
}

#[test]
fn memory_appears_in_ai_context() {
    use tokio::time::timeout;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let local = tokio::task::LocalSet::new();

    runtime.block_on(local.run_until(async {
        timeout(Duration::from_secs(30), async {
            let workspace = Workspace::new();
            enable_memory_in_workspace(&workspace);

            // Session 1: Store a memory
            let behavior1 = MockBehavior::ToolUseThenSuccess {
                tool_name: "append_memory".to_string(),
                tool_arguments: r#"{"content": "user prefers dark mode theme"}"#.to_string(),
            };
            let mut session1 = workspace.spawn_session("one_shot", behavior1);
            session1.step("I prefer dark mode").await;
            drop(session1);

            // Re-enable memory for next session
            enable_memory_in_workspace(&workspace);

            // Session 2: New conversation - memory should be in AI context
            let behavior2 = MockBehavior::Success;
            let mut session2 = workspace.spawn_session("one_shot", behavior2);
            session2.step("Hello").await;

            // Get the AI request and check if memory appears in context
            let request = session2
                .get_last_ai_request()
                .expect("Should have captured AI request");

            // Build full context string from system prompt and messages
            let mut context = request.system_prompt.clone();
            for msg in &request.messages {
                for block in msg.content.blocks() {
                    if let ContentBlock::Text(text) = block {
                        context.push_str(text);
                    }
                }
            }

            assert!(
                context.contains("dark mode"),
                "Memory should appear in AI context. Context length: {} chars",
                context.len()
            );
        })
        .await
        .expect("Test timed out");
    }));
}

#[test]
fn background_manager_stores_memories() {
    use tokio::time::timeout;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let local = tokio::task::LocalSet::new();

    runtime.block_on(local.run_until(async {
        timeout(Duration::from_secs(30), async {
            let workspace = Workspace::new();
            enable_memory_in_workspace(&workspace);

            // Queue behaviors:
            // 1. Main agent responds to user message
            // 2. Background memory manager uses append_memory
            let behavior = MockBehavior::BehaviorQueue {
                behaviors: vec![
                    MockBehavior::Success,
                    MockBehavior::ToolUseThenSuccess {
                        tool_name: "append_memory".to_string(),
                        tool_arguments:
                            r#"{"content": "BGMEM_TEST_7f3a9b: user prefers tabs over spaces"}"#
                                .to_string(),
                    },
                ],
            };

            let mut session = workspace.spawn_session("one_shot", behavior);

            // Send message - actor spawns background memory manager internally
            session
                .step("Always use tabs not spaces. Remember this.")
                .await;

            // Wait for background task to complete
            tokio::time::sleep(Duration::from_millis(500)).await;
            drop(session);

            // Re-enable memory for next session
            enable_memory_in_workspace(&workspace);

            // New session - memory should appear in AI context
            let behavior2 = MockBehavior::Success;
            let mut session2 = workspace.spawn_session("one_shot", behavior2);
            session2.step("What formatting do I prefer?").await;

            // Verify memory appears in context
            let request = session2
                .get_last_ai_request()
                .expect("Should have captured AI request");

            let mut context = request.system_prompt.clone();
            for msg in &request.messages {
                for block in msg.content.blocks() {
                    if let ContentBlock::Text(text) = block {
                        context.push_str(text);
                    }
                }
            }

            assert!(
                context.contains("BGMEM_TEST_7f3a9b: user prefers tabs over spaces"),
                "Background manager memory should appear in AI context. Context: {}",
                context
            );
        })
        .await
        .expect("Test timed out");
    }));
}

#[test]
fn memory_compact_creates_compaction_file() {
    use tokio::time::timeout;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let local = tokio::task::LocalSet::new();

    runtime.block_on(local.run_until(async {
        timeout(Duration::from_secs(30), async {
            let workspace = Workspace::new();
            enable_memory_in_workspace(&workspace);

            // Session 1: Store memory via actor
            let store_behavior = MockBehavior::ToolUseThenSuccess {
                tool_name: "append_memory".to_string(),
                tool_arguments: r#"{"content": "COMPACTION_FILE_TEST_9x2k: user prefers dark mode"}"#.to_string(),
            };
            let mut session1 = workspace.spawn_session("one_shot", store_behavior);
            session1.step("Remember I prefer dark mode").await;
            drop(session1);

            enable_memory_in_workspace(&workspace);

            // Session 2: Run /memory compact - mock the summarizer agent calling complete_task
            let compact_behavior = MockBehavior::ToolUseThenSuccess {
                tool_name: "complete_task".to_string(),
                tool_arguments: r#"{"success": true, "result": "COMPACTION_SUMMARY_3j7m: User prefers dark mode theme"}"#.to_string(),
            };
            let mut session2 = workspace.spawn_session("one_shot", compact_behavior);
            session2.step("/memory compact").await;
            drop(session2);

            // Verify compaction file was created
            let memory_dir = workspace.tycode_dir().join("memory");
            let compaction_files: Vec<_> = std::fs::read_dir(&memory_dir)
                .expect("Memory directory should exist")
                .filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().starts_with("compaction_"))
                .collect();

            assert!(
                !compaction_files.is_empty(),
                "Compaction file should be created. Files in memory dir: {:?}",
                std::fs::read_dir(&memory_dir)
                    .map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name()).collect::<Vec<_>>())
            );

            // Verify file contains the summary
            let compaction_file = &compaction_files[0].path();
            let content = std::fs::read_to_string(compaction_file)
                .expect("Should be able to read compaction file");
            assert!(
                content.contains("COMPACTION_SUMMARY_3j7m"),
                "Compaction file should contain summary. Content: {}",
                content
            );
        })
        .await
        .expect("Test timed out");
    }));
}

#[test]
fn compaction_summary_appears_in_system_prompt() {
    use tokio::time::timeout;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let local = tokio::task::LocalSet::new();

    runtime.block_on(local.run_until(async {
        timeout(Duration::from_secs(30), async {
            let workspace = Workspace::new();
            enable_memory_in_workspace(&workspace);

            // Session 1: Store memory via actor
            let store_behavior = MockBehavior::ToolUseThenSuccess {
                tool_name: "append_memory".to_string(),
                tool_arguments: r#"{"content": "PROMPT_MEM_4k9x: user likes rust"}"#.to_string(),
            };
            let mut session1 = workspace.spawn_session("one_shot", store_behavior);
            session1.step("Remember I like Rust").await;
            drop(session1);

            enable_memory_in_workspace(&workspace);

            // Session 2: Run /memory compact
            let compact_behavior = MockBehavior::ToolUseThenSuccess {
                tool_name: "complete_task".to_string(),
                tool_arguments: r#"{"success": true, "result": "PROMPT_SUMMARY_8x7z: User prefers Rust programming language"}"#.to_string(),
            };
            let mut session2 = workspace.spawn_session("one_shot", compact_behavior);
            session2.step("/memory compact").await;
            drop(session2);

            enable_memory_in_workspace(&workspace);

            // Session 3: New session - compaction summary should be in system prompt
            let behavior3 = MockBehavior::Success;
            let mut session3 = workspace.spawn_session("one_shot", behavior3);
            session3.step("Hello").await;

            // Get ALL AI requests and check the FIRST one (the main agent's request)
            // The last request is from the background memory manager, which doesn't include compaction
            let requests = session3.get_all_ai_requests();
            assert!(!requests.is_empty(), "Should have at least one AI request");
            let request = &requests[0];

            // Build full context string from system prompt and messages
            let mut context = request.system_prompt.clone();
            for msg in &request.messages {
                for block in msg.content.blocks() {
                    if let ContentBlock::Text(text) = block {
                        context.push_str(text);
                    }
                }
            }

            assert!(
                context.contains("PROMPT_SUMMARY_8x7z"),
                "Compaction summary should appear in AI context. Context length: {} chars",
                context.len()
            );
        })
        .await
        .expect("Test timed out");
    }));
}

#[test]
fn memory_compact_with_no_new_memories_succeeds() {
    use tokio::time::timeout;

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let local = tokio::task::LocalSet::new();

    runtime.block_on(local.run_until(async {
        timeout(Duration::from_secs(30), async {
            let workspace = Workspace::new();
            enable_memory_in_workspace(&workspace);

            // Session 1: Store memory via actor
            let store_behavior = MockBehavior::ToolUseThenSuccess {
                tool_name: "append_memory".to_string(),
                tool_arguments: r#"{"content": "NO_NEW_TEST_5m2p: some memory"}"#.to_string(),
            };
            let mut session1 = workspace.spawn_session("one_shot", store_behavior);
            session1.step("Remember this").await;
            drop(session1);

            enable_memory_in_workspace(&workspace);

            // Session 2: First compaction
            let compact_behavior1 = MockBehavior::ToolUseThenSuccess {
                tool_name: "complete_task".to_string(),
                tool_arguments:
                    r#"{"success": true, "result": "FIRST_COMPACT_6n3q: Initial summary"}"#
                        .to_string(),
            };
            let mut session2 = workspace.spawn_session("one_shot", compact_behavior1);
            session2.step("/memory compact").await;
            drop(session2);

            // Verify first compaction created a file
            let memory_dir = workspace.tycode_dir().join("memory");
            let compaction_files: Vec<_> = std::fs::read_dir(&memory_dir)
                .expect("Memory directory should exist")
                .filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().starts_with("compaction_"))
                .collect();

            assert_eq!(
                compaction_files.len(),
                1,
                "First compaction should create exactly 1 file"
            );

            enable_memory_in_workspace(&workspace);

            // Session 3: Second compaction with NO new memories - should succeed without error
            // but doesn't need to create a new file (no new memories to summarize)
            let compact_behavior2 = MockBehavior::Success;
            let mut session3 = workspace.spawn_session("one_shot", compact_behavior2);
            let events = session3.step("/memory compact").await;
            drop(session3);

            // Verify the command completed without error (no Error messages)
            let has_error = events.iter().any(|e| {
                if let tycode_core::chat::events::ChatEvent::MessageAdded(message) = e {
                    matches!(
                        message.sender,
                        tycode_core::chat::events::MessageSender::Error
                    )
                } else {
                    false
                }
            });

            assert!(
                !has_error,
                "Second compact with no new memories should succeed without error"
            );
        })
        .await
        .expect("Test timed out");
    }));
}
