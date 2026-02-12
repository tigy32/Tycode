pub mod config;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use std::{env, process::Stdio};

use anyhow::{anyhow, Result};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::process::Command;

use std::collections::VecDeque;

use crate::chat::events::{ToolExecutionResult, ToolRequest as ToolRequestEvent, ToolRequestType};
use crate::file::access::FileAccessManager;
use crate::module::Module;
use crate::module::PromptComponent;
use crate::module::{ContextComponent, ContextComponentId};
use crate::settings::SettingsManager;
use crate::tools::r#trait::{
    ContinuationPreference, ToolCallHandle, ToolCategory, ToolExecutor, ToolOutput, ToolRequest,
};
use crate::tools::ToolName;

use config::{CommandExecutionMode, ExecutionConfig, RunBuildTestOutputMode};

const BLOCKED_COMMANDS: &[&str] = &["rm", "rmdir", "dd", "shred", "mkfs", "fdisk", "parted"];

// === Command Outputs Context Component ===

pub const COMMAND_OUTPUTS_ID: ContextComponentId = ContextComponentId("command_outputs");

/// A stored command output with its command and result.
#[derive(Debug, Clone)]
pub struct CommandOutput {
    pub command: String,
    pub output: String,
    pub exit_code: Option<i32>,
}

/// Manages command output history and provides context rendering.
/// Stores a fixed-size buffer of recent command outputs.
pub struct CommandOutputsManager {
    outputs: std::sync::RwLock<VecDeque<CommandOutput>>,
    max_outputs: usize,
}

impl CommandOutputsManager {
    pub fn new(max_outputs: usize) -> Self {
        Self {
            outputs: std::sync::RwLock::new(VecDeque::with_capacity(max_outputs)),
            max_outputs,
        }
    }

    /// Add a command output to the buffer.
    /// If buffer is full, oldest output is removed.
    pub fn add_output(&self, command: String, output: String, exit_code: Option<i32>) {
        let mut outputs = self.outputs.write().unwrap();
        if outputs.len() >= self.max_outputs {
            outputs.pop_front();
        }
        outputs.push_back(CommandOutput {
            command,
            output,
            exit_code,
        });
    }

    /// Clear all stored outputs.
    pub fn clear(&self) {
        self.outputs.write().unwrap().clear();
    }

    /// Get the number of stored outputs.
    pub fn len(&self) -> usize {
        self.outputs.read().unwrap().len()
    }

    /// Check if buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.outputs.read().unwrap().is_empty()
    }
}

#[async_trait::async_trait(?Send)]
impl ContextComponent for CommandOutputsManager {
    fn id(&self) -> ContextComponentId {
        COMMAND_OUTPUTS_ID
    }

    async fn build_context_section(&self) -> Option<String> {
        let outputs: Vec<CommandOutput> = {
            let mut guard = self.outputs.write().unwrap();
            guard.drain(..).collect()
        };

        if outputs.is_empty() {
            return None;
        }

        let mut result = String::from("Recent Command Outputs:\n");
        for output in outputs.iter() {
            result.push_str(&format!("\n$ {}\n", output.command));
            if let Some(code) = output.exit_code {
                result.push_str(&format!("Exit code: {}\n", code));
            }
            if !output.output.is_empty() {
                result.push_str(&output.output);
                if !output.output.ends_with('\n') {
                    result.push('\n');
                }
            }
        }
        Some(result)
    }
}

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
    command_outputs_manager: Arc<CommandOutputsManager>,
    access: FileAccessManager,
    settings: SettingsManager,
}

impl ExecutionModule {
    pub fn new(workspace_roots: Vec<PathBuf>, settings: SettingsManager) -> Result<Self> {
        let inner = Arc::new(ExecutionModuleInner {
            command_outputs_manager: Arc::new(CommandOutputsManager::new(10)),
            access: FileAccessManager::new(workspace_roots)?,
            settings,
        });
        Ok(Self { inner })
    }
}

impl Module for ExecutionModule {
    fn prompt_components(&self) -> Vec<Arc<dyn PromptComponent>> {
        vec![]
    }

    fn context_components(&self) -> Vec<Arc<dyn ContextComponent>> {
        vec![self.inner.command_outputs_manager.clone()]
    }

    fn tools(&self) -> Vec<Arc<dyn ToolExecutor>> {
        vec![Arc::new(RunBuildTestTool {
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

pub struct RunBuildTestTool {
    inner: Arc<ExecutionModuleInner>,
}

impl RunBuildTestTool {
    pub fn tool_name() -> ToolName {
        ToolName::new("run_build_test")
    }
}

struct RunBuildTestHandle {
    command: String,
    working_directory: PathBuf,
    timeout_seconds: u64,
    tool_use_id: String,
    command_outputs_manager: Arc<CommandOutputsManager>,
    output_mode: RunBuildTestOutputMode,
    execution_mode: CommandExecutionMode,
    max_output_bytes: Option<usize>,
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
    base_dir: &Path,
    vfs_display_path: &str,
) -> Result<(String, PathBuf)> {
    let persist_dir = base_dir.join(".tycode").join("tool-calls");
    tokio::fs::create_dir_all(&persist_dir).await?;

    let persist_path = persist_dir.join(tool_call_id);
    tokio::fs::write(&persist_path, output).await?;

    let mut truncated = compact_output(output, max_bytes);
    truncated.push_str(&format!(
        "\n\n[Full output saved to: {}. Use head/tail/grep to inspect.]",
        vfs_display_path
    ));

    Ok((truncated, persist_path))
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

        let combined_output = if result.err.is_empty() {
            result.out.clone()
        } else if result.out.is_empty() {
            result.err.clone()
        } else {
            format!("{}\n{}", result.out, result.err)
        };

        // Truncate for Context mode only — ToolResponse mode leaves truncation to tools.rs
        let display_output = match (&self.output_mode, &self.max_output_bytes) {
            (RunBuildTestOutputMode::Context, Some(max)) if combined_output.len() > *max => {
                compact_output(&combined_output, *max)
            }
            _ => combined_output.clone(),
        };

        self.command_outputs_manager.add_output(
            self.command.clone(),
            display_output,
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
    fn name(&self) -> String {
        "run_build_test".to_string()
    }

    fn description(&self) -> String {
        let config: ExecutionConfig = self.inner.settings.get_module_config("execution");
        match config.execution_mode {
            CommandExecutionMode::Direct => {
                "Run build, test, or execution commands (cargo build, npm test, python main.py) - NOT for file operations (no cat/ls/grep/find); use dedicated file tools instead. Shell features like pipes (cmd | grep) and redirects (cmd > file) will fail or behave unexpectedly.".to_string()
            }
            CommandExecutionMode::Bash => {
                "Run build, test, or execution commands (cargo build, npm test, python main.py) - NOT for file operations (no cat/ls/grep/find); use dedicated file tools instead.".to_string()
            }
        }
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
        let resolved_working_directory = self.inner.access.resolve(working_directory)?;

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

        let config: ExecutionConfig = self.inner.settings.get_module_config("execution");
        let output_mode = config.output_mode.clone();
        let execution_mode = config.execution_mode.clone();

        Ok(Box::new(RunBuildTestHandle {
            command: command_str.to_string(),
            working_directory: resolved_working_directory,
            timeout_seconds,
            tool_use_id: request.tool_use_id.clone(),
            command_outputs_manager: self.inner.command_outputs_manager.clone(),
            output_mode,
            execution_mode,
            max_output_bytes: config.max_output_bytes,
        }))
    }
}
