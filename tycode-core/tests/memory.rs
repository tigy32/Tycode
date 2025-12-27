mod fixture;

use fixture::{MockBehavior, Workspace};

/// Reproduces the MemoryLog race condition bug where a second ChatActor instance
/// would overwrite memories from the first instance.
///
/// Bug mechanism (now fixed):
/// 1. Actor 1 stores a memory, file contains: [memory1]
/// 2. Actor 2 starts with MemoryLog::new() which created EMPTY in-memory state
/// 3. Actor 2 appends a memory, triggering save() which wrote [memory2] to disk
/// 4. memory1 is lost forever
///
/// Fix: MemoryLog now loads from disk on every operation (load-on-demand).
#[test]
fn test_memory_persists_across_concurrent_actors() {
    use tokio::time::{timeout, Duration};

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();

    let local = tokio::task::LocalSet::new();

    runtime.block_on(local.run_until(async {
        timeout(Duration::from_secs(30), async {
            let workspace = Workspace::new();

            // Actor 1 stores a memory
            let behavior1 = MockBehavior::ToolUseThenSuccess {
                tool_name: "append_memory".to_string(),
                tool_arguments: r#"{"content": "Memory from actor 1"}"#.to_string(),
            };

            let mut session1 = workspace.spawn_session("one_shot", behavior1, true);
            let _ = session1.step("Store a memory").await;
            drop(session1);

            // Actor 2 stores another memory
            // With the old bug, this would wipe actor 1's memory
            let behavior2 = MockBehavior::ToolUseThenSuccess {
                tool_name: "append_memory".to_string(),
                tool_arguments: r#"{"content": "Memory from actor 2"}"#.to_string(),
            };

            let mut session2 = workspace.spawn_session("one_shot", behavior2, true);
            let _ = session2.step("Store another memory").await;
            drop(session2);

            // Verify both memories exist
            let memory_file = workspace.memory_dir().join("memories_log.json");
            let content = std::fs::read_to_string(&memory_file).expect("Memory file should exist");

            assert!(
                content.contains("Memory from actor 1"),
                "First actor's memory should be preserved. File contents: {}",
                content
            );
            assert!(
                content.contains("Memory from actor 2"),
                "Second actor's memory should exist. File contents: {}",
                content
            );
        })
        .await
        .expect("Test timed out");
    }));
}
