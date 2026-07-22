use std::path::Path;

use serde_json::json;
use tycode_core::ai::types::ContentBlock;
use tycode_core::chat::events::{ChatEvent, MessageSender};
use tycode_core::modules::execution::config::{CommandExecutionMode, ExecutionConfig};

#[path = "../fixture.rs"]
mod fixture;

fn tool_results_from_last_request(fixture: &fixture::Fixture) -> Vec<String> {
    fixture
        .get_last_ai_request()
        .expect("Should have AI request")
        .messages
        .iter()
        .flat_map(|message| message.content.blocks())
        .filter_map(|block| match block {
            ContentBlock::ToolResult(result) => Some(result.content.clone()),
            _ => None,
        })
        .collect()
}

fn bash_args(command: &str, workspace_path: &Path) -> String {
    json!({
        "command": command,
        "working_directory": workspace_path,
        "timeout_seconds": 10
    })
    .to_string()
}

#[test]
fn test_bash_returns_output_in_tool_result() {
    fixture::run(|mut fixture| async move {
        use tycode_core::ai::mock::MockBehavior;

        let workspace_path = fixture.workspace_path();
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "bash".to_string(),
            tool_arguments: bash_args("echo hello", &workspace_path),
        });

        let events = fixture.step("Run a command").await;

        assert!(
            events.iter().any(|event| {
                matches!(
                    event,
                    ChatEvent::StreamEnd { message }
                        if matches!(message.sender, MessageSender::Assistant { .. })
                )
            }),
            "Should receive assistant message after tool execution"
        );

        let results = tool_results_from_last_request(&fixture);
        assert_eq!(results.len(), 1);
        assert!(results[0].contains("hello"), "Captured: {}", results[0]);
        assert!(results[0].contains("stdout"), "Captured: {}", results[0]);
    });
}

#[test]
fn test_multiple_commands_return_separate_tool_results() {
    fixture::run(|mut fixture| async move {
        use tycode_core::ai::mock::MockBehavior;

        let workspace_path = fixture.workspace_path();
        fixture.set_mock_behavior(MockBehavior::MultipleToolUses {
            tool_uses: vec![
                (
                    "bash".to_string(),
                    bash_args("echo first_output", &workspace_path),
                ),
                (
                    "bash".to_string(),
                    bash_args("echo second_output", &workspace_path),
                ),
            ],
        });

        fixture.step("Run multiple commands").await;

        let results = tool_results_from_last_request(&fixture);
        assert_eq!(results.len(), 2);
        assert!(results.iter().any(|result| result.contains("first_output")));
        assert!(results
            .iter()
            .any(|result| result.contains("second_output")));
    });
}

#[test]
fn test_bash_quoted_arguments() {
    fixture::run(|mut fixture| async move {
        use tycode_core::ai::mock::MockBehavior;

        let workspace_path = fixture.workspace_path();
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "bash".to_string(),
            tool_arguments: bash_args("echo \"hello world\"", &workspace_path),
        });

        fixture.step("Run command with quoted arguments").await;

        let results = tool_results_from_last_request(&fixture);
        assert!(results.iter().any(|result| result.contains("hello world")));
    });
}

#[test]
fn test_bash_bash_mode() {
    fixture::run(|mut fixture| async move {
        use tycode_core::ai::mock::MockBehavior;

        let workspace_path = fixture.workspace_path();
        fixture
            .update_settings(|settings| {
                let mut config: ExecutionConfig = settings.get_module_config("execution");
                config.execution_mode = CommandExecutionMode::Bash;
                settings.set_module_config("execution", config);
            })
            .await;

        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "bash".to_string(),
            tool_arguments: bash_args("echo hello_bash | cat", &workspace_path),
        });

        fixture.step("Run command with shell features").await;

        let results = tool_results_from_last_request(&fixture);
        assert!(results.iter().any(|result| result.contains("hello_bash")));
    });
}

#[test]
fn test_large_output_compaction() {
    fixture::run(|mut fixture| async move {
        use tycode_core::ai::mock::MockBehavior;

        let workspace_path = fixture.workspace_path();
        fixture
            .update_settings(|settings| {
                let mut config: ExecutionConfig = settings.get_module_config("execution");
                config.max_output_bytes = Some(100);
                settings.set_module_config("execution", config);
            })
            .await;

        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "bash".to_string(),
            tool_arguments: bash_args("seq 1 1000", &workspace_path),
        });

        fixture.step("Generate large output").await;

        let results = tool_results_from_last_request(&fixture);
        assert_eq!(results.len(), 1);
        let result = &results[0];
        assert!(result.contains("truncated"), "Captured: {result}");
        assert!(
            result.contains("1\\n2") || result.contains("1\n2"),
            "Captured: {result}"
        );
        assert!(result.contains("1000"), "Captured: {result}");
    });
}

#[test]
fn test_command_output_remains_in_conversation() {
    fixture::run(|mut fixture| async move {
        use tycode_core::ai::mock::MockBehavior;

        let workspace_path = fixture.workspace_path();
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "bash".to_string(),
            tool_arguments: bash_args("echo persistent_output", &workspace_path),
        });
        fixture.step("Run a command").await;

        fixture.set_mock_behavior(MockBehavior::Success);
        fixture.step("Respond normally").await;

        let results = tool_results_from_last_request(&fixture);
        assert!(
            results
                .iter()
                .any(|result| result.contains("persistent_output")),
            "Command output should remain in conversation history"
        );
    });
}
