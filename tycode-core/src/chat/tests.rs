#[cfg(test)]
#[allow(dead_code)]
mod tests {
    use crate::agents::catalog::AgentCatalog;
    use crate::ai::mock::{MockBehavior, MockProvider};
    use crate::ai::model::Model;
    use crate::ai::types::*;
    use crate::chat::actor::ActorState;
    use crate::chat::events::EventSender;
    use crate::chat::tools::execute_tool_calls;
    use crate::settings::SettingsManager;
    use anyhow::Result;
    use std::collections::HashSet;
    use std::path::PathBuf;
    use std::sync::Once;

    static TRACING_INIT: Once = Once::new();

    fn setup_tracing() {
        TRACING_INIT.call_once(|| {
            let _ = tracing_subscriber::fmt()
                .with_test_writer()
                .with_max_level(tracing::Level::DEBUG)
                .try_init();
        });
    }

    struct TestFixture {
        state: ActorState,
        _event_rx: tokio::sync::mpsc::UnboundedReceiver<crate::chat::events::ChatEvent>,
        workspace: PathBuf,
        _settings_path: PathBuf,
    }

    impl TestFixture {
        fn new(mock_behavior: MockBehavior, agent_type: &str) -> Self {
            setup_tracing();

            let test_workspace = std::env::temp_dir().join("tycode_test_workspace");
            let settings_path = std::env::temp_dir().join("tycode_test_settings.toml");
            std::fs::create_dir_all(&test_workspace).expect("Failed to create test workspace");
            std::fs::create_dir_all(test_workspace.join(".git"))
                .expect("Failed to create .git directory");

            let mock_provider = MockProvider::new(mock_behavior);
            let (event_sender, event_rx) = EventSender::new();
            let root_agent =
                AgentCatalog::create_agent(agent_type).expect("Failed to create root agent");

            let state = ActorState {
                event_sender,
                provider: Box::new(mock_provider),
                agent_stack: vec![crate::agents::agent::ActiveAgent::new(root_agent)],
                workspace_roots: vec![test_workspace.clone()],
                settings: SettingsManager::from_path(settings_path.clone())
                    .expect("Failed to create settings manager"),
                tracked_files: HashSet::new(),
                session_token_usage: TokenUsage::empty(),
                session_cost: 0.0,
                mcp_manager: None,
                task_list: None,
            };

            TestFixture {
                state,
                _event_rx: event_rx,
                workspace: test_workspace,
                _settings_path: settings_path,
            }
        }
    }

    #[tokio::test]
    async fn test_spawn_agent_single_tool_result() -> Result<()> {
        let mut fixture = TestFixture::new(
            MockBehavior::ToolUse {
                tool_name: "spawn_agent".to_string(),
                tool_arguments: serde_json::json!({
                    "agent_type": "coder",
                    "task": "Write hello world"
                })
                .to_string(),
            },
            "coordinator",
        );
        let state = &mut fixture.state;

        let spawn_tool_calls = vec![ToolUseData {
            id: "spawn_1".to_string(),
            name: "spawn_agent".to_string(),
            arguments: serde_json::json!({
                "agent_type": "coder",
                "task": "Write hello world"
            }),
        }];

        let result = execute_tool_calls(state, spawn_tool_calls, Model::ClaudeSonnet45).await?;

        assert!(
            result.continue_conversation,
            "spawn_agent should allow conversation to continue"
        );

        assert_eq!(
            state.agent_stack.len(),
            2,
            "Expected 2 agents in stack (root + spawned)"
        );

        let complete_tool_calls = vec![ToolUseData {
            id: "complete_1".to_string(),
            name: "complete_task".to_string(),
            arguments: serde_json::json!({
                "result": "Task completed successfully",
                "success": true
            }),
        }];

        let _complete_result =
            execute_tool_calls(state, complete_tool_calls, Model::ClaudeSonnet45).await?;

        assert_eq!(
            state.agent_stack.len(),
            1,
            "Expected 1 agent in stack after complete_task"
        );

        let parent_conversation = &state.agent_stack[0].conversation;
        let spawn_tool_result_count = parent_conversation
            .iter()
            .flat_map(|msg| msg.content.blocks())
            .filter(|block| {
                if let ContentBlock::ToolResult(tr) = block {
                    tr.tool_use_id == "spawn_1"
                } else {
                    false
                }
            })
            .count();

        assert_eq!(
            spawn_tool_result_count,
            1,
            "Parent conversation should have exactly 1 ToolResult for spawn_agent (the acknowledgment)"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_spawn_agent_with_set_tracked_files() -> Result<()> {
        let mut fixture = TestFixture::new(
            MockBehavior::ToolUse {
                tool_name: "spawn_agent".to_string(),
                tool_arguments: serde_json::json!({
                    "agent_type": "coder",
                    "task": "Write hello world"
                })
                .to_string(),
            },
            "coordinator",
        );
        let state = &mut fixture.state;

        let test_file = fixture.workspace.join("test.rs");
        std::fs::write(&test_file, "fn main() {}").expect("Failed to create test file");

        let spawn_call = vec![ToolUseData {
            id: "spawn_1".to_string(),
            name: "spawn_agent".to_string(),
            arguments: serde_json::json!({
                "agent_type": "coder",
                "task": "Write hello world"
            }),
        }];

        let result = execute_tool_calls(state, spawn_call, Model::ClaudeSonnet45).await?;

        assert!(
            result.continue_conversation,
            "spawn_agent should allow conversation to continue"
        );

        assert_eq!(
            state.agent_stack.len(),
            2,
            "Expected 2 agents in stack after spawn_agent"
        );

        let track_call = vec![ToolUseData {
            id: "track_1".to_string(),
            name: "set_tracked_files".to_string(),
            arguments: serde_json::json!({
                "file_paths": ["tycode_test_workspace/test.rs"]
            }),
        }];

        let track_result = execute_tool_calls(state, track_call, Model::ClaudeSonnet45).await?;

        assert!(
            track_result.continue_conversation,
            "set_tracked_files should allow conversation to continue"
        );

        assert_eq!(
            state.tracked_files.len(),
            1,
            "Expected 1 tracked file after set_tracked_files"
        );
        assert!(
            state
                .tracked_files
                .contains(&PathBuf::from("tycode_test_workspace/test.rs")),
            "Tracked files should contain the test file"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_complete_task_with_set_tracked_files() -> Result<()> {
        let mut fixture = TestFixture::new(
            MockBehavior::ToolUse {
                tool_name: "complete_task".to_string(),
                tool_arguments: serde_json::json!({
                    "result": "Task completed",
                    "success": true
                })
                .to_string(),
            },
            "coordinator",
        );
        let state = &mut fixture.state;

        let test_file = fixture.workspace.join("test.rs");
        std::fs::write(&test_file, "fn main() {}").expect("Failed to create test file");

        let spawn_call = vec![ToolUseData {
            id: "spawn_1".to_string(),
            name: "spawn_agent".to_string(),
            arguments: serde_json::json!({
                "agent_type": "coder",
                "task": "Write hello world"
            }),
        }];

        execute_tool_calls(state, spawn_call, Model::ClaudeSonnet45).await?;

        assert_eq!(state.agent_stack.len(), 2, "Agent should be spawned");

        let track_call = vec![ToolUseData {
            id: "track_1".to_string(),
            name: "set_tracked_files".to_string(),
            arguments: serde_json::json!({
                "file_paths": ["tycode_test_workspace/test.rs"]
            }),
        }];

        let track_result = execute_tool_calls(state, track_call, Model::ClaudeSonnet45).await?;

        assert!(
            track_result.continue_conversation,
            "set_tracked_files should allow conversation to continue"
        );

        assert_eq!(
            state.tracked_files.len(),
            1,
            "Expected 1 tracked file after set_tracked_files"
        );

        let complete_call = vec![ToolUseData {
            id: "complete_1".to_string(),
            name: "complete_task".to_string(),
            arguments: serde_json::json!({
                "result": "Task completed successfully",
                "success": true
            }),
        }];

        let result = execute_tool_calls(state, complete_call, Model::ClaudeSonnet45).await?;

        assert!(
            result.continue_conversation,
            "complete_task should allow conversation to continue"
        );

        assert_eq!(
            state.agent_stack.len(),
            1,
            "Expected 1 agent in stack after complete_task"
        );

        assert!(
            state
                .tracked_files
                .contains(&PathBuf::from("tycode_test_workspace/test.rs")),
            "Tracked files should still contain the test file"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_pop_agent_adds_result_to_parent_conversation() -> Result<()> {
        let mut fixture = TestFixture::new(
            MockBehavior::ToolUse {
                tool_name: "spawn_agent".to_string(),
                tool_arguments: serde_json::json!({
                    "agent_type": "coder",
                    "task": "Write hello world"
                })
                .to_string(),
            },
            "coordinator",
        );
        let state = &mut fixture.state;

        let spawn_tool_calls = vec![ToolUseData {
            id: "spawn_1".to_string(),
            name: "spawn_agent".to_string(),
            arguments: serde_json::json!({
                "agent_type": "coder",
                "task": "Write hello world"
            }),
        }];

        let result = execute_tool_calls(state, spawn_tool_calls, Model::ClaudeSonnet45).await?;

        assert!(
            result.continue_conversation,
            "spawn_agent should allow conversation to continue"
        );

        assert_eq!(
            state.agent_stack.len(),
            2,
            "Expected 2 agents in stack (root + spawned)"
        );

        let complete_tool_calls = vec![ToolUseData {
            id: "complete_1".to_string(),
            name: "complete_task".to_string(),
            arguments: serde_json::json!({
                "result": "Task completed successfully",
                "success": true
            }),
        }];

        let _complete_result =
            execute_tool_calls(state, complete_tool_calls, Model::ClaudeSonnet45).await?;

        assert_eq!(
            state.agent_stack.len(),
            1,
            "Expected 1 agent in stack after complete_task"
        );

        let parent_conversation = &state.agent_stack[0].conversation;
        let last_msg = parent_conversation
            .last()
            .expect("Conversation should not be empty");
        assert_eq!(
            last_msg.role,
            MessageRole::User,
            "Last message should be User role"
        );
        let text = last_msg.content.text();
        assert!(
            text.contains("Sub-agent completed"),
            "Last message should contain sub-agent completed text"
        );
        assert!(
            text.contains("[success=true]"),
            "Last message should contain success=true"
        );
        assert!(
            text.contains("Task completed successfully"),
            "Last message should contain the result text"
        );

        let spawn_tool_result_count = parent_conversation
            .iter()
            .flat_map(|msg| msg.content.blocks())
            .filter(|block| {
                if let ContentBlock::ToolResult(tr) = block {
                    tr.tool_use_id == "spawn_1"
                } else {
                    false
                }
            })
            .count();

        assert_eq!(
            spawn_tool_result_count, 1,
            "Parent conversation should have exactly 1 ToolResult for spawn_agent"
        );

        Ok(())
    }
}
