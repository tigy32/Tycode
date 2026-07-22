pub mod config;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use std::{env, process::Stdio};

use anyhow::{anyhow, Result};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::process::Command;

use crate::chat::events::{ToolExecutionResult, ToolRequest as ToolRequestEvent, ToolRequestType};
use crate::file::access::FileAccessManager;
use crate::module::{ContextComponent, Module};
use crate::module::PromptComponent;
use crate::settings::SettingsManager;
use crate::tools::r#trait::{
    ContinuationPreference, SharedTool, ToolCallHandle, ToolCategory, ToolExecutor, ToolOutput,
    ToolRequest,
};
use crate::tools::ToolName;

use config::{CommandExecutionMode, ExecutionConfig};

#[derive(Debug, Clone, Serialize)]
pub struct CommandResult {
    pub command: String,
    pub code: i32,
    pub out: String,
    pub err: String,
}

pub async fn run_cmd(
    dir: PathBuf,
    cmd: String,
    timeout: Duration,
    execution_mode: CommandExecutionMode,
) -> Result<CommandResult> {
    let path = env::var("PATH")?;
    tracing::info!(?path, ?dir, ?cmd, ?execution_mode, "Attempting to run_cmd");

    let child = match execution_mode {
        CommandExecutionMode::Direct => {
            let parts = shell_words::split(&cmd)
                .map_err(|e| anyhow::anyhow!("Failed to parse command: {e:?}"))?;
            if parts.is_empty() {
                return Err(anyhow::anyhow!("Empty command"));
            }
            let program = &parts[0];
            let args: Vec<&str> = parts[1..].iter().map(|s| s.as_str()).collect();

            Command::new(program)
                .args(args)
                .current_dir(&dir)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()?
        }
        CommandExecutionMode::Bash => Command::new("bash")
            .args(["-c", &cmd])
            .current_dir(&dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?,
    };

    let output = tokio::time::timeout(timeout, async {
        let output = child.wait_with_output().await?;
        Ok::<_, std::io::Error>(output)
    })
    .await??;

    let code = output.status.code().unwrap_or(1);
    let out = String::from_utf8_lossy(&output.stdout).to_string();
    let err = String::from_utf8_lossy(&output.stderr).to_string();

    Ok(CommandResult {
        command: cmd,
        code,
        out,
        err,
    })
}

pub struct ExecutionModule {
    inner: Arc<ExecutionModuleInner>,
}

struct ExecutionModuleInner {
    access: FileAccessManager,
    default_working_directory: PathBuf,
    settings: SettingsManager,
}

impl ExecutionModule {
    pub fn new(workspace_roots: Vec<PathBuf>, settings: SettingsManager) -> Result<Self> {
        let access = FileAccessManager::new(workspace_roots)?;
        // No workspace roots is a legitimate state (e.g. the VSCode extension
        // with no folder open, loading settings); commands without an
        // explicit working_directory then run from the home directory, like
        // a fresh shell.
        let default_working_directory = match access.roots.first() {
            Some(default_workspace) => default_workspace.clone(),
            None => dirs::home_dir().unwrap_or_else(std::env::temp_dir),
        };

        let inner = Arc::new(ExecutionModuleInner {
            access,
            default_working_directory,
            settings,
        });
        Ok(Self { inner })
    }
}

#[async_trait::async_trait(?Send)]
impl Module for ExecutionModule {
    fn prompt_components(&self) -> Vec<Arc<dyn PromptComponent>> {
        vec![]
    }

    fn context_components(&self) -> Vec<Arc<dyn ContextComponent>> {
        vec![]
    }

    async fn tools(&self) -> Vec<SharedTool> {
        vec![Arc::new(BashTool {
            inner: self.inner.clone(),
        })]
    }

    fn session_state(&self) -> Option<Arc<dyn crate::module::SessionStateComponent>> {
        None
    }

    fn settings_namespace(&self) -> Option<&'static str> {
        Some("execution")
    }

    fn settings_json_schema(&self) -> Option<schemars::schema::RootSchema> {
        Some(schemars::schema_for!(ExecutionConfig))
    }
}

pub struct BashTool {
    inner: Arc<ExecutionModuleInner>,
}

impl BashTool {
    pub fn tool_name() -> ToolName {
        ToolName::new("bash")
    }
}

struct BashHandle {
    command: String,
    working_directory: PathBuf,
    timeout_seconds: u64,
    tool_use_id: String,
    execution_mode: CommandExecutionMode,
}

/// Compact output by keeping first half and last half with truncation marker.
pub fn compact_output(output: &str, max_bytes: usize) -> String {
    if output.len() <= max_bytes {
        return output.to_string();
    }

    let half = max_bytes / 2;
    let start_end = output.floor_char_boundary(half);
    let end_start_target = output.len().saturating_sub(half);
    let end_start = output.ceil_char_boundary(end_start_target);

    let start = &output[..start_end];
    let end = &output[end_start..];
    let omitted = output.len() - start.len() - end.len();

    format!(
        "{}\\n... [output truncated: {} bytes omitted] ...\\n{}",
        start, omitted, end
    )
}

/// Truncate large output and persist the full result to disk.
/// Returns the truncated content (with a note about the persisted file) and the file path.
pub async fn truncate_and_persist(
    output: &str,
    tool_call_id: &str,
    max_bytes: usize,
    persist_dir: &Path,
    display_path: &str,
) -> Result<(String, PathBuf)> {
    tokio::fs::create_dir_all(persist_dir).await?;

    let persist_path = persist_dir.join(tool_call_id);
    tokio::fs::write(&persist_path, output).await?;

    let mut truncated = compact_output(output, max_bytes);
    truncated.push_str(&format!(
        "\n\n[Full output saved to: {}. Use head/tail/grep to inspect.]",
        display_path
    ));

    Ok((truncated, persist_path))
}

#[async_trait::async_trait(?Send)]
impl ToolCallHandle for BashHandle {
    fn tool_request(&self) -> ToolRequestEvent {
        ToolRequestEvent {
            tool_call_id: self.tool_use_id.clone(),
            tool_name: "bash".to_string(),
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
            self.execution_mode.clone(),
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

        let is_error = result.code != 0;
        let content = json!({
            "exit_code": result.code,
            "stdout": result.out,
            "stderr": result.err,
        })
        .to_string();

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
impl ToolExecutor for BashTool {
    fn name(&self) -> String {
        "bash".to_string()
    }

    fn description(&self) -> String {
        "Run a Bash command in the workspace. Use this for inspecting files, searching, building, testing, and running project commands.".to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The Bash command to execute"
                },
                "working_directory": {
                    "type": "string",
                    "description": "Absolute directory to run the command in. Defaults to the first workspace root. Must be inside a configured workspace root."
                },
                "timeout_seconds": {
                    "type": "integer",
                    "description": "Maximum seconds to wait for command completion. Defaults to 60.",
                    "minimum": 1,
                    "maximum": 300
                }
            },
            "required": ["command"]
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
            .unwrap_or(60);

        let resolved_working_directory = request
            .arguments
            .get("working_directory")
            .and_then(|v| v.as_str())
            .map(|dir| self.inner.access.resolve(dir))
            .transpose()?
            .unwrap_or_else(|| self.inner.default_working_directory.clone());

        let config: ExecutionConfig = self.inner.settings.get_module_config("execution");
        let execution_mode = config.execution_mode.clone();

        Ok(Box::new(BashHandle {
            command: command_str.to_string(),
            working_directory: resolved_working_directory,
            timeout_seconds,
            tool_use_id: request.tool_use_id.clone(),
            execution_mode,
        }))
    }
}
