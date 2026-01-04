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
use tycode_core::settings::Settings;

/// Helper to enable memory in workspace settings before spawning session.
fn enable_memory_in_workspace(workspace: &Workspace) {
    let settings_path = workspace.tycode_dir().join("settings.toml");
    let settings_manager = SettingsManager::from_path(settings_path).unwrap();
    let mut settings = Settings::default();
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
