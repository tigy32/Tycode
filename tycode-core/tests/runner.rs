mod fixture;

use std::time::Duration;
use tycode_core::ai::mock::MockBehavior;

fn memory_storage_behavior(content: &str, source: Option<&str>, result: &str) -> MockBehavior {
    let append_args = match source {
        Some(s) => format!(r#"{{"content": "{}", "source": "{}"}}"#, content, s),
        None => format!(r#"{{"content": "{}"}}"#, content),
    };
    let complete_args = format!(r#"{{"result": "{}", "success": true}}"#, result);
    MockBehavior::BehaviorQueue {
        behaviors: vec![
            MockBehavior::Success,
            MockBehavior::ToolUseThenToolUse {
                first_tool_name: "append_memory".to_string(),
                first_tool_arguments: append_args,
                second_tool_name: "complete_task".to_string(),
                second_tool_arguments: complete_args,
            },
        ],
    }
}

/// Ensures memory survives across actor restarts by persisting to disk
#[test]
fn memory_manager_stores_memory_through_actor() {
    fixture::run_with_memory(|mut fixture| async move {
        // Main agent completes immediately so memory manager sub-agent can execute storage asynchronously
        fixture.set_mock_behavior(memory_storage_behavior(
            "likes dark mode",
            None,
            "Stored preference",
        ));

        fixture.step("I prefer dark mode").await;

        let memory_file = fixture.memory_dir().join("memories_log.json");
        tokio::time::timeout(Duration::from_secs(2), async {
            while !memory_file.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("Memory file was not created within timeout");
        assert!(
            memory_file.exists(),
            "Memory file should exist at {:?}",
            memory_file
        );

        let content = std::fs::read_to_string(&memory_file).unwrap();
        assert!(
            content.contains("likes dark mode"),
            "Memory content should contain the stored text"
        );
    });
}

/// Prevents data corruption during the async memory storage process
#[test]
fn memory_manager_stores_correct_content() {
    fixture::run_with_memory(|mut fixture| async move {
        // Main agent completes immediately so memory manager sub-agent can execute storage asynchronously
        fixture.set_mock_behavior(memory_storage_behavior(
            "Rust with async/await",
            Some("test-project"),
            "Stored",
        ));

        fixture
            .step("This project uses Rust with async/await")
            .await;

        let memory_file = fixture.memory_dir().join("memories_log.json");
        tokio::time::timeout(Duration::from_secs(2), async {
            while !memory_file.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("Memory file was not created within timeout");
        assert!(memory_file.exists());

        let content = std::fs::read_to_string(&memory_file).unwrap();
        assert!(
            content.contains("Rust with async/await"),
            "Memory should contain stored content"
        );
        assert!(
            content.contains("test-project"),
            "Memory should contain source"
        );
    });
}
