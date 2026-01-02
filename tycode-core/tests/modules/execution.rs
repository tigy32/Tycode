use tycode_core::chat::events::{ChatEvent, MessageSender};
use tycode_core::settings::config::{CommandExecutionMode, RunBuildTestOutputMode};

#[path = "../fixture.rs"]
mod fixture;

fn get_context_from_last_request(fixture: &fixture::Fixture) -> String {
    fixture
        .get_last_ai_request()
        .and_then(|req| req.messages.last().map(|msg| msg.content.text()))
        .expect("Should have AI request with messages")
}

#[test]
fn test_run_build_test_tool_response_mode() {
    fixture::run(|mut fixture| async move {
        use tycode_core::ai::mock::MockBehavior;

        let workspace_path = fixture.workspace_path();
        let workspace_name = workspace_path.file_name().unwrap().to_str().unwrap();

        // Configure mock to call run_build_test
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "run_build_test".to_string(),
            tool_arguments: format!(
                r#"{{"command": "echo hello", "working_directory": "/{}", "timeout_seconds": 10}}"#,
                workspace_name
            ),
        });

        let events = fixture.step("Run a command").await;

        // Verify that the conversation completed successfully with an assistant message
        assert!(
            events.iter().any(|e| {
                matches!(
                    e,
                    ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Assistant { .. })
                )
            }),
            "Should receive assistant message after tool execution"
        );
    });
}

#[test]
fn test_run_build_test_context_mode() {
    fixture::run(|mut fixture| async move {
        use tycode_core::ai::mock::MockBehavior;

        let workspace_path = fixture.workspace_path();

        // First, update settings to use Context mode
        fixture
            .update_settings(|settings| {
                settings.run_build_test_output_mode = RunBuildTestOutputMode::Context;
            })
            .await;

        let workspace_name = workspace_path.file_name().unwrap().to_str().unwrap();
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "run_build_test".to_string(),
            tool_arguments: format!(
                r#"{{"command": "echo test", "working_directory": "/{}", "timeout_seconds": 10}}"#,
                workspace_name
            ),
        });

        let events = fixture.step("Run a command").await;

        // Verify that the conversation completed successfully with an assistant message
        assert!(
            events.iter().any(|e| {
                matches!(
                    e,
                    ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Assistant { .. })
                )
            }),
            "Should receive assistant message after tool execution in Context mode"
        );
    });
}

#[test]
fn test_run_build_test_context_mode_multiple_commands() {
    fixture::run(|mut fixture| async move {
        use tycode_core::ai::mock::MockBehavior;

        let workspace_path = fixture.workspace_path();

        // Update settings to use Context mode
        fixture
            .update_settings(|settings| {
                settings.run_build_test_output_mode = RunBuildTestOutputMode::Context;
            })
            .await;

        let workspace_name = workspace_path.file_name().unwrap().to_str().unwrap();

        // First command
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "run_build_test".to_string(),
            tool_arguments: format!(
                r#"{{"command": "echo first", "working_directory": "/{}", "timeout_seconds": 10}}"#,
                workspace_name
            ),
        });

        let events1 = fixture.step("Run first command").await;

        assert!(
            events1.iter().any(|e| {
                matches!(
                    e,
                    ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Assistant { .. })
                )
            }),
            "Should receive assistant message after first command"
        );

        // Second command - this should replace the first command's output in state
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "run_build_test".to_string(),
            tool_arguments: format!(
                r#"{{"command": "echo second", "working_directory": "/{}", "timeout_seconds": 10}}"#,
                workspace_name
            ),
        });

        let events2 = fixture.step("Run second command").await;

        assert!(
            events2.iter().any(|e| {
                matches!(
                    e,
                    ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Assistant { .. })
                )
            }),
            "Should receive assistant message after second command"
        );
    });
}

// Context mode should include stdout/stderr in the ToolExecutionCompleted event
#[test]
fn test_context_mode_includes_stdout_stderr_in_event() {
    fixture::run(|mut fixture| async move {
        use tycode_core::ai::mock::MockBehavior;

        let workspace_path = fixture.workspace_path();

        fixture
            .update_settings(|settings| {
                settings.run_build_test_output_mode = RunBuildTestOutputMode::Context;
            })
            .await;

        let workspace_name = workspace_path.file_name().unwrap().to_str().unwrap();
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "run_build_test".to_string(),
            tool_arguments: format!(
                r#"{{"command": "echo test_output", "working_directory": "/{}", "timeout_seconds": 10}}"#,
                workspace_name
            ),
        });

        let events = fixture.step("Run a command that produces output").await;

        assert!(
            events.iter().any(|e| {
                matches!(
                    e,
                    ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Assistant { .. })
                )
            }),
            "Should receive assistant message after tool execution in Context mode"
        );

        // Check context immediately after tool execution (it's in the last AI request)
        let context_content = get_context_from_last_request(&fixture);

        assert!(
            context_content.contains("test_output"),
            "Context sent to AI should include stdout/stderr from the command execution. Captured: {}",
            context_content
        );
    });
}

// Last Command Output should show the command that was run
#[test]
fn test_last_command_output_shows_command() {
    fixture::run(|mut fixture| async move {
        use tycode_core::ai::mock::MockBehavior;

        let workspace_path = fixture.workspace_path();

        fixture
            .update_settings(|settings| {
                settings.run_build_test_output_mode = RunBuildTestOutputMode::Context;
            })
            .await;

        let workspace_name = workspace_path.file_name().unwrap().to_str().unwrap();
        let test_command = "echo test_command_bug_2";
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "run_build_test".to_string(),
            tool_arguments: format!(
                r#"{{"command": "{}", "working_directory": "/{}", "timeout_seconds": 10}}"#,
                test_command, workspace_name
            ),
        });

        let _events1 = fixture.step("Run a command").await;

        // Check context immediately after tool execution (it's in the last AI request)
        let context_content = get_context_from_last_request(&fixture);

        assert!(
            !context_content.is_empty(),
            "Should have context in messages"
        );

        assert!(
            context_content.contains(test_command),
            "Context sent to AI should show the command that was run: {}. Captured: {}",
            test_command,
            context_content
        );
    });
}

// Multiple commands in a single response should all appear in context
#[test]
fn test_multiple_commands_in_single_response_all_appear_in_context() {
    fixture::run(|mut fixture| async move {
        use tycode_core::ai::mock::MockBehavior;

        let workspace_path = fixture.workspace_path();

        fixture
            .update_settings(|settings| {
                settings.run_build_test_output_mode = RunBuildTestOutputMode::Context;
            })
            .await;

        let workspace_name = workspace_path.file_name().unwrap().to_str().unwrap();

        // Use MultipleToolUses to execute two run_build_test commands in a single AI response
        fixture.set_mock_behavior(MockBehavior::MultipleToolUses {
            tool_uses: vec![
                (
                    "run_build_test".to_string(),
                    format!(
                        r#"{{"command": "echo first_output", "working_directory": "/{}", "timeout_seconds": 10}}"#,
                        workspace_name
                    ),
                ),
                (
                    "run_build_test".to_string(),
                    format!(
                        r#"{{"command": "echo second_output", "working_directory": "/{}", "timeout_seconds": 10}}"#,
                        workspace_name
                    ),
                ),
            ],
        });

        let _events = fixture.step("Run multiple commands").await;

        let context_content = get_context_from_last_request(&fixture);

        assert!(
            context_content.contains("first_output"),
            "Context should contain output from first command. Captured: {}",
            context_content
        );

        assert!(
            context_content.contains("second_output"),
            "Context should contain output from second command. Captured: {}",
            context_content
        );
    });
}

// Regression test: quoted arguments with spaces should be parsed correctly
// Before fix: `echo "hello world"` was split into ["echo", "\"hello", "world\""]
// After fix: correctly parsed as ["echo", "hello world"]
#[test]
fn test_run_build_test_quoted_arguments() {
    fixture::run(|mut fixture| async move {
        use tycode_core::ai::mock::MockBehavior;

        let workspace_path = fixture.workspace_path();

        fixture
            .update_settings(|settings| {
                settings.run_build_test_output_mode = RunBuildTestOutputMode::Context;
            })
            .await;

        let workspace_name = workspace_path.file_name().unwrap().to_str().unwrap();
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "run_build_test".to_string(),
            tool_arguments: format!(
                r#"{{"command": "echo \"hello world\"", "working_directory": "/{}", "timeout_seconds": 10}}"#,
                workspace_name
            ),
        });

        let _events = fixture.step("Run command with quoted arguments").await;

        let context_content = get_context_from_last_request(&fixture);

        assert!(
            context_content.contains("hello world"),
            "Quoted arguments should be parsed correctly - expected 'hello world' in output. Captured: {}",
            context_content
        );
    });
}

// Bash execution mode should allow shell features like pipes
#[test]
fn test_run_build_test_bash_mode() {
    fixture::run(|mut fixture| async move {
        use tycode_core::ai::mock::MockBehavior;

        let workspace_path = fixture.workspace_path();

        fixture
            .update_settings(|settings| {
                settings.command_execution_mode = CommandExecutionMode::Bash;
                settings.run_build_test_output_mode = RunBuildTestOutputMode::Context;
            })
            .await;

        let workspace_name = workspace_path.file_name().unwrap().to_str().unwrap();

        // Use a pipe command that requires bash mode to work correctly
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "run_build_test".to_string(),
            tool_arguments: format!(
                r#"{{"command": "echo hello_bash | cat", "working_directory": "/{}", "timeout_seconds": 10}}"#,
                workspace_name
            ),
        });

        let _events = fixture.step("Run command with shell features").await;

        let context_content = get_context_from_last_request(&fixture);

        assert!(
            context_content.contains("hello_bash"),
            "Bash mode should execute pipe command successfully. Captured: {}",
            context_content
        );
    });
}

// Last command output should get cleared after the next AI response
#[test]
fn test_last_command_output_cleared_after_ai_response() {
    fixture::run(|mut fixture| async move {
        use tycode_core::ai::mock::MockBehavior;

        let workspace_path = fixture.workspace_path();

        fixture
            .update_settings(|settings| {
                settings.run_build_test_output_mode = RunBuildTestOutputMode::Context;
            })
            .await;

        let workspace_name = workspace_path.file_name().unwrap().to_str().unwrap();
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "run_build_test".to_string(),
            tool_arguments: format!(
                r#"{{"command": "echo first_command", "working_directory": "/{}", "timeout_seconds": 10}}"#,
                workspace_name
            ),
        });

        let _events1 = fixture.step("Run first command").await;

        fixture.set_mock_behavior(MockBehavior::Success);
        let _events2 = fixture.step("Just respond normally").await;

        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "run_build_test".to_string(),
            tool_arguments: format!(
                r#"{{"command": "echo second_command", "working_directory": "/{}", "timeout_seconds": 10}}"#,
                workspace_name
            ),
        });

        let _events3 = fixture.step("Run second command").await;

        let context_content = get_context_from_last_request(&fixture);

        assert!(
            !context_content.is_empty(),
            "Should have context in messages"
        );

        assert!(
            context_content.contains("second_command"),
            "Context sent to AI should contain output from second command. Captured: {}",
            context_content
        );

        assert!(
            !context_content.contains("first_command"),
            "Context sent to AI should NOT contain output from first command (should have been cleared after AI response). Captured: {}",
            context_content
        )
    });
}
