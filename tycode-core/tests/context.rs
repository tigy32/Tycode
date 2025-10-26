use tycode_core::chat::events::{ChatEvent, MessageSender};

mod fixture;

#[test]
fn test_large_file_list_warning() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();

        // Create many files to exceed 20KB threshold for file list.
        // File paths themselves contribute to the byte count, so we create
        // approximately 600 files with moderately long paths (~50-60 bytes each)
        // to ensure we exceed the 20,000 byte (20KB) threshold.
        for i in 0..600 {
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
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System)
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
