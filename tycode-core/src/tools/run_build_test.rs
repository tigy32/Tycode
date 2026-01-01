use crate::chat::events::{ToolExecutionResult, ToolRequest as ToolRequestEvent, ToolRequestType};
use crate::cmd::run_cmd;
use crate::context::command_outputs::CommandOutputsManager;
use crate::file::access::FileAccessManager;
use crate::settings::config::RunBuildTestOutputMode;
use crate::settings::SettingsManager;
use crate::tools::r#trait::{
    ContinuationPreference, ToolCallHandle, ToolCategory, ToolExecutor, ToolOutput, ToolRequest,
};
use crate::tools::ToolName;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const BLOCKED_COMMANDS: &[&str] = &["rm", "rmdir", "dd", "shred", "mkfs", "fdisk", "parted"];

#[derive(Clone)]
pub struct RunBuildTestTool {
    access: FileAccessManager,
    command_outputs_manager: Arc<CommandOutputsManager>,
    settings: SettingsManager,
}

impl RunBuildTestTool {
    pub fn tool_name() -> ToolName {
        ToolName::new("run_build_test")
    }

    pub fn new(workspace_roots: Vec<PathBuf>, settings: SettingsManager) -> anyhow::Result<Self> {
        Ok(Self {
            access: FileAccessManager::new(workspace_roots)?,
            command_outputs_manager: Arc::new(CommandOutputsManager::new(10)),
            settings,
        })
    }

    /// Get the context component for command outputs visibility
    pub fn context_component(&self) -> Arc<dyn crate::context::ContextComponent + Send + Sync> {
        self.command_outputs_manager.clone()
    }
}

/// Handle for run_build_test tool execution
struct RunBuildTestHandle {
    command: String,
    working_directory: PathBuf,
    timeout_seconds: u64,
    tool_use_id: String,
    command_outputs_manager: Arc<CommandOutputsManager>,
    output_mode: RunBuildTestOutputMode,
}

#[async_trait::async_trait(?Send)]
impl ToolCallHandle for RunBuildTestHandle {
    fn tool_request(&self) -> ToolRequestEvent {
        ToolRequestEvent {
            tool_call_id: self.tool_use_id.clone(),
            tool_name: "run_build_test".to_string(),
            tool_type: ToolRequestType::RunCommand {
                command: self.command.clone(),
                working_directory: self.working_directory.to_string_lossy().to_string(),
            },
        }
    }

    async fn execute(self: Box<Self>) -> ToolOutput {
        let timeout = Duration::from_secs(self.timeout_seconds);

        let result = match run_cmd(
            self.working_directory.clone(),
            self.command.clone(),
            timeout,
        )
        .await
        {
            Ok(r) => r,
            Err(e) => {
                let error_msg = format!("Command execution failed: {e:?}");
                return ToolOutput::Result {
                    content: error_msg.clone(),
                    is_error: true,
                    continuation: ContinuationPreference::Continue,
                    ui_result: ToolExecutionResult::Error {
                        short_message: "Command failed".to_string(),
                        detailed_message: error_msg,
                    },
                };
            }
        };

        let combined_output = if result.err.is_empty() {
            result.out.clone()
        } else if result.out.is_empty() {
            result.err.clone()
        } else {
            format!("{}\n{}", result.out, result.err)
        };

        self.command_outputs_manager.add_output(
            self.command.clone(),
            combined_output,
            Some(result.code),
        );

        let is_error = result.code != 0;
        let content = match (&self.output_mode, is_error) {
            (RunBuildTestOutputMode::ToolResponse, _) => json!({
                "exit_code": result.code,
                "stdout": result.out,
                "stderr": result.err,
            })
            .to_string(),
            (RunBuildTestOutputMode::Context, true) => json!({
                "exit_code": result.code,
                "status": "failed",
                "message": "Command failed. See context section for output."
            })
            .to_string(),
            (RunBuildTestOutputMode::Context, false) => json!({
                "exit_code": result.code,
                "status": "success",
                "message": "Command executed. See context section for output."
            })
            .to_string(),
        };

        ToolOutput::Result {
            content,
            is_error,
            continuation: ContinuationPreference::Continue,
            ui_result: ToolExecutionResult::RunCommand {
                exit_code: result.code,
                stdout: result.out,
                stderr: result.err,
            },
        }
    }
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for RunBuildTestTool {
    fn name(&self) -> &'static str {
        "run_build_test"
    }

    fn description(&self) -> &'static str {
        "Run build, test, or execution commands (cargo build, npm test, python main.py) - NOT for file operations (no cat/ls/grep/find) or shell features (no pipes/redirects); use dedicated file tools instead."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The command to execute"
                },
                "working_directory": {
                    "type": "string",
                    "description": "The directory to run the command in. Must be within a workspace root. Must be an absolute path."
                },
                "timeout_seconds": {
                    "type": "integer",
                    "description": "Maximum seconds to wait for command completion",
                    "minimum": 1,
                    "maximum": 300
                }
            },
            "required": ["command", "timeout_seconds", "working_directory"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Execution
    }

    async fn process(&self, request: &ToolRequest) -> Result<Box<dyn ToolCallHandle>> {
        let command_str = request
            .arguments
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'command' argument"))?;

        let timeout_seconds = request
            .arguments
            .get("timeout_seconds")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow!("Missing 'timeout_seconds' argument"))?;

        let working_directory = request
            .arguments
            .get("working_directory")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow!("Missing 'working_directory' argument"))?;
        let resolved_working_directory = self.access.resolve(working_directory)?;

        let parts: Vec<&str> = command_str.split_whitespace().collect();
        if parts.is_empty() {
            return Err(anyhow!("Empty command"));
        }

        let cmd = parts[0];
        if BLOCKED_COMMANDS.contains(&cmd) || cmd.starts_with("mkfs.") {
            let msg = if cmd == "rm" || cmd == "rmdir" {
                format!("Command '{cmd}' is blocked for safety. Use the delete_file tool instead.")
            } else {
                format!("Command '{cmd}' is blocked for safety.")
            };
            return Err(anyhow!(msg));
        }

        let output_mode = self.settings.settings().run_build_test_output_mode.clone();

        Ok(Box::new(RunBuildTestHandle {
            command: command_str.to_string(),
            working_directory: resolved_working_directory,
            timeout_seconds,
            tool_use_id: request.tool_use_id.clone(),
            command_outputs_manager: self.command_outputs_manager.clone(),
            output_mode,
        }))
    }
}
