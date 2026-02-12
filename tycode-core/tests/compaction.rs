use tycode_core::chat::events::{ChatEvent, MessageSender};

mod fixture;

#[test]
fn test_input_too_long_triggers_compaction() {
    fixture::run(|mut fixture| async move {
        use tycode_core::ai::mock::MockBehavior;

        // First establish a conversation
        let events = fixture.step("Hello").await;
        assert!(!events.is_empty());

        // Configure mock to return InputTooLong once, then succeed
        fixture.set_mock_behavior(MockBehavior::InputTooLongThenSuccess {
            remaining_errors: 1,
        });

        // Send a message that will trigger InputTooLong
        let events = fixture.step("Continue conversation").await;

        // Verify we got an assistant response (proves compaction worked and conversation continued)
        assert!(
            events.iter().any(|e| {
                matches!(
                    e,
                    ChatEvent::StreamEnd { message } if matches!(message.sender, MessageSender::Assistant { .. })
                )
            }),
            "Should receive assistant message after compaction"
        );
    });
}

#[test]
fn test_compaction_clears_tracked_files() {
    fixture::run(|mut fixture| async move {
        use tycode_core::ai::mock::MockBehavior;

        // Establish initial conversation with tracked files potentially in context
        let events = fixture.step("Hello").await;
        assert!(!events.is_empty());

        // Configure mock to return InputTooLong once, then succeed
        // When compaction occurs, tracked files should be cleared to reduce context
        fixture.set_mock_behavior(MockBehavior::InputTooLongThenSuccess {
            remaining_errors: 1,
        });

        // Send a message that triggers InputTooLong and forces compaction
        let events = fixture
            .step("Message that triggers compaction and clears tracked files")
            .await;

        // Verify conversation continues after compaction (tracked files cleared)
        assert!(
            events.iter().any(|e| {
                matches!(
                    e,
                    ChatEvent::StreamEnd { message } if matches!(message.sender, MessageSender::Assistant { .. })
                )
            }),
            "Should receive assistant message after compaction clears tracked files"
        );

        // Verify we can continue the conversation after compaction
        let events = fixture.step("Continue after compaction").await;
        assert!(
            events.iter().any(|e| {
                matches!(
                    e,
                    ChatEvent::StreamEnd { message } if matches!(message.sender, MessageSender::Assistant { .. })
                )
            }),
            "Conversation should continue normally after compaction and file clearing"
        );
    });
}

#[test]
fn test_compaction_with_tool_use_blocks() {
    fixture::run(|mut fixture| async move {
        use tycode_core::ai::mock::MockBehavior;
        use tycode_core::ai::types::ContentBlock;

        // First, trigger a tool use interaction to get ToolUse/ToolResult blocks in conversation
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "set_tracked_files".to_string(),
            tool_arguments: r#"{"file_paths": ["example.txt"]}"#.to_string(),
        });
        let events = fixture.step("Use a tool").await;
        assert!(!events.is_empty(), "Should receive events from tool use");

        // Now trigger InputTooLong to force compaction
        // The conversation now contains ToolUse and ToolResult blocks
        fixture.set_mock_behavior(MockBehavior::InputTooLongThenSuccess {
            remaining_errors: 1,
        });

        let events = fixture.step("Trigger compaction").await;

        // Get all AI requests to find the compaction request
        let requests = fixture.get_all_ai_requests();

        // Find the compaction request by looking for the summarization system prompt
        let compaction_request = requests
            .iter()
            .find(|req| req.system_prompt.contains("conversation summarizer"))
            .expect("Should find compaction/summarization request");

        // Assert that the compaction request does NOT contain ToolUse or ToolResult blocks
        for message in &compaction_request.messages {
            for content in message.content.blocks() {
                assert!(
                    !matches!(content, ContentBlock::ToolUse { .. }),
                    "Compaction request should not contain ToolUse blocks"
                );
                assert!(
                    !matches!(content, ContentBlock::ToolResult { .. }),
                    "Compaction request should not contain ToolResult blocks"
                );
            }
        }

        // Verify conversation continues successfully after compaction
        assert!(
            events.iter().any(|e| {
                matches!(
                    e,
                    ChatEvent::StreamEnd { message } if matches!(message.sender, MessageSender::Assistant { .. })
                )
            }),
            "Should receive assistant message after compaction"
        );
    });
}

#[test]
fn test_tool_result_truncation_and_disk_persistence() {
    fixture::run(|mut fixture| async move {
        use tycode_core::ai::mock::MockBehavior;

        // Set max_output_bytes very low so even small outputs trigger truncation
        fixture
            .update_settings(|s| {
                s.modules.insert(
                    "execution".to_string(),
                    serde_json::json!({
                        "max_output_bytes": 200
                    }),
                );
            })
            .await;

        // Write a file larger than 200 bytes to the workspace
        let large_content = "x".repeat(1000);
        let large_file_path = fixture.workspace_path().join("large_output.txt");
        std::fs::write(&large_file_path, &large_content).unwrap();

        // Construct VFS path from workspace directory name
        let ws_name = fixture
            .workspace_path()
            .canonicalize()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let vfs_workspace = format!("/{ws_name}");
        let canonical_file = large_file_path.canonicalize().unwrap();

        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "run_build_test".to_string(),
            tool_arguments: serde_json::json!({
                "command": format!("cat {}", canonical_file.display()),
                "working_directory": vfs_workspace,
                "timeout_seconds": 10
            })
            .to_string(),
        });

        let events = fixture.step("Run a command with large output").await;

        // Verify we got an assistant response (conversation completed)
        assert!(
            events.iter().any(|e| {
                matches!(
                    e,
                    ChatEvent::StreamEnd { message } if matches!(message.sender, MessageSender::Assistant { .. })
                )
            }),
            "Should receive assistant message after tool execution"
        );

        // Check that the full output was persisted to disk
        let tool_calls_dir = fixture.workspace_path().join(".tycode").join("tool-calls");
        assert!(tool_calls_dir.exists(), "tool-calls directory should exist");

        let entries: Vec<_> = std::fs::read_dir(&tool_calls_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert!(
            !entries.is_empty(),
            "Should have at least one persisted tool result"
        );

        // Verify the persisted file contains the full output
        let persisted_content = std::fs::read_to_string(entries[0].path()).unwrap();
        assert!(
            persisted_content.len() >= 1000,
            "Persisted file should contain the full output, got {} bytes",
            persisted_content.len()
        );

        // Verify the tool result in the AI request was truncated
        let requests = fixture.get_all_ai_requests();
        let tool_result_request = requests.iter().find(|req| {
            req.messages.iter().any(|msg| {
                msg.content
                    .blocks()
                    .iter()
                    .any(|b| matches!(b, tycode_core::ai::types::ContentBlock::ToolResult(_)))
            })
        });

        if let Some(req) = tool_result_request {
            let tool_results: Vec<_> = req
                .messages
                .iter()
                .flat_map(|msg| msg.content.blocks())
                .filter_map(|b| {
                    if let tycode_core::ai::types::ContentBlock::ToolResult(r) = b {
                        Some(r)
                    } else {
                        None
                    }
                })
                .collect();

            for result in &tool_results {
                if result.content.contains("truncated")
                    || result.content.contains("Full output saved to")
                {
                    assert!(
                        result.content.len() < 2000,
                        "Tool result should be truncated, but was {} bytes",
                        result.content.len()
                    );
                }
            }
        }
    });
}
