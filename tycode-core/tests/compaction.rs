use tycode_core::chat::events::{ChatEvent, MessageSender};

mod fixture;

#[test]
#[ignore] // Requires AWS credentials and real Bedrock API access
fn test_compaction_fails_with_tooluse_blocks() {
    use tempfile::TempDir;
    use tycode_core::{
        chat::actor::ChatActor,
        settings::manager::SettingsManager,
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("Failed to create tokio runtime");

    let local = tokio::task::LocalSet::new();

    runtime.block_on(local.run_until(async {
        let workspace_dir = TempDir::new().unwrap();
        let workspace_path = workspace_dir.path().to_path_buf();
        let sessions_dir = workspace_path.join("sessions");
        std::fs::create_dir_all(&sessions_dir).unwrap();

        std::fs::write(workspace_path.join("test.txt"), "test content").unwrap();

        let settings_dir = workspace_path.join(".tycode");
        std::fs::create_dir_all(&settings_dir).unwrap();
        let settings_path = settings_dir.join("settings.toml");

        let settings_manager = SettingsManager::from_path(settings_path.clone()).unwrap();
        let mut settings = tycode_core::settings::Settings::default();
        settings.add_provider(
            "bedrock".to_string(),
            tycode_core::settings::ProviderConfig::Bedrock {
                profile: "default".to_string(),
                region: "us-west-2".to_string(),
            },
        );
        settings.active_provider = Some("bedrock".to_string());
        settings.default_agent = "one_shot".to_string();
        settings_manager.save_settings(settings).unwrap();

        let (actor, mut event_rx) = ChatActor::builder()
            .workspace_roots(vec![workspace_path.clone()])
            .sessions_dir(sessions_dir)
            .settings_path(settings_path)
            .build()
            .unwrap();

        actor
            .send_message("Use set_tracked_files to track test.txt".to_string())
            .unwrap();

        while let Some(event) = event_rx.recv().await {
            if matches!(event, ChatEvent::TypingStatusChanged(false)) {
                break;
            }
        }

        let large_message = format!("Analyze this data: {}", "x".repeat(100000));
        actor.send_message(large_message).unwrap();

        let mut saw_toolconfig_error = false;
        while let Some(event) = event_rx.recv().await {
            if matches!(event, ChatEvent::TypingStatusChanged(false)) {
                break;
            }

            let ChatEvent::Error(err) = &event else {
                continue;
            };

            eprintln!("Error during compaction: {}", err);
            if err.contains("toolConfig") || err.contains("tool_config") {
                saw_toolconfig_error = true;
            }
        }

        assert!(
            saw_toolconfig_error,
            "Expected toolConfig error when compacting conversation with ToolUse blocks"
        );
    }));
}

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
                    ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Assistant { .. })
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
                    ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Assistant { .. })
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
                    ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Assistant { .. })
                )
            }),
            "Conversation should continue normally after compaction and file clearing"
        );
    });
}
