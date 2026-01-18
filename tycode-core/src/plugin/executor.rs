//! Hook executor for running shell commands.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::time::timeout;
use tracing::{debug, warn};

use super::hooks::{
    ConfiguredHook, HookDispatcher, HookEvent, HookInput, HookOutput, HookResult, PluginHooks,
};
use super::manifest::HooksConfig;

/// Executes hook shell commands.
pub struct HookExecutor {
    dispatcher: HookDispatcher,
}

impl HookExecutor {
    /// Creates a new HookExecutor with the given dispatcher.
    pub fn new(dispatcher: HookDispatcher) -> Self {
        Self { dispatcher }
    }

    /// Dispatches an event to all relevant hooks and returns the combined result.
    pub async fn dispatch(&self, event: HookEvent, input: HookInput) -> HookResult {
        if !self.dispatcher.has_hooks(event) {
            return HookResult::Continue;
        }

        let hooks = self.dispatcher.get_hooks(event);

        for hook in hooks {
            if !hook.matches(&input) {
                continue;
            }

            debug!(
                plugin = %hook.plugin_name,
                event = ?event,
                "Executing hook"
            );

            match self.execute_hook(hook, &input).await {
                Ok(output) => {
                    // Process the output
                    if !output.r#continue {
                        let reason = output
                            .reason
                            .unwrap_or_else(|| "Hook blocked execution".to_string());
                        return HookResult::Blocked(reason);
                    }

                    if let Some(decision) = &output.decision {
                        match decision {
                            super::hooks::HookDecision::Deny => {
                                let reason = output
                                    .reason
                                    .unwrap_or_else(|| "Hook denied execution".to_string());
                                return HookResult::Denied(reason);
                            }
                            super::hooks::HookDecision::Block => {
                                let reason = output
                                    .reason
                                    .unwrap_or_else(|| "Hook blocked execution".to_string());
                                return HookResult::Blocked(reason);
                            }
                            super::hooks::HookDecision::Allow => {}
                            super::hooks::HookDecision::Ask => {
                                // For now, treat "ask" as allow
                            }
                        }
                    }

                    // Check for modified input
                    if let Some(specific) = &output.hook_specific_output {
                        if let Some(updated) = &specific.updated_input {
                            return HookResult::ContinueModified(updated.clone());
                        }
                    }
                }
                Err(e) => {
                    warn!(
                        plugin = %hook.plugin_name,
                        error = %e,
                        "Hook execution failed"
                    );
                    // Non-blocking error - continue with other hooks
                }
            }
        }

        HookResult::Continue
    }

    /// Executes a single hook command.
    async fn execute_hook(&self, hook: &ConfiguredHook, input: &HookInput) -> Result<HookOutput> {
        let command = hook.expanded_command();
        let timeout_duration = Duration::from_millis(hook.definition.timeout);

        // Serialize input to JSON
        let input_json =
            serde_json::to_string(input).context("Failed to serialize hook input")?;

        debug!(command = %command, timeout = ?timeout_duration, "Running hook command");

        // Spawn the process with kill_on_drop to ensure cleanup on timeout
        let mut child = Command::new("sh")
            .args(["-c", &command])
            .current_dir(&hook.plugin_root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .context("Failed to spawn hook process")?;

        // Write input to stdin
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(input_json.as_bytes())
                .await
                .context("Failed to write to hook stdin")?;
        }

        // Wait for completion with timeout
        let output = timeout(timeout_duration, child.wait_with_output())
            .await
            .context("Hook execution timed out")?
            .context("Failed to wait for hook process")?;

        // Parse output based on exit code
        match output.status.code() {
            Some(0) => {
                // Success - parse JSON output
                let stdout = String::from_utf8_lossy(&output.stdout);
                if stdout.trim().is_empty() {
                    Ok(HookOutput::allow())
                } else {
                    serde_json::from_str(stdout.trim())
                        .context("Failed to parse hook output JSON")
                }
            }
            Some(2) => {
                // Exit code 2 = blocking error
                let stderr = String::from_utf8_lossy(&output.stderr);
                Err(anyhow::anyhow!("Hook blocked: {}", stderr.trim()))
            }
            Some(code) => {
                // Other non-zero = non-blocking error
                let stderr = String::from_utf8_lossy(&output.stderr);
                warn!(
                    exit_code = code,
                    stderr = %stderr.trim(),
                    "Hook exited with non-zero status"
                );
                Ok(HookOutput::allow())
            }
            None => {
                // Process was terminated by signal
                warn!("Hook process was terminated by signal");
                Ok(HookOutput::allow())
            }
        }
    }
}

/// Loads hooks from a hooks.json file.
pub fn load_hooks_from_file(
    path: &Path,
    plugin_root: PathBuf,
    plugin_name: &str,
) -> Result<PluginHooks> {
    let config = HooksConfig::load(path)?;
    let mut plugin_hooks = PluginHooks::new();

    for hook_def in config.hooks {
        if let Some(event) = HookEvent::parse(&hook_def.event) {
            let configured = ConfiguredHook {
                definition: hook_def,
                plugin_root: plugin_root.clone(),
                plugin_name: plugin_name.to_string(),
            };
            plugin_hooks.add_hook(event, configured);
        } else {
            warn!(
                plugin = plugin_name,
                event = hook_def.event,
                "Unknown hook event type"
            );
        }
    }

    Ok(plugin_hooks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_hook_script(dir: &Path, name: &str, content: &str) -> PathBuf {
        let script_path = dir.join(name);
        fs::write(&script_path, content).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&script_path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&script_path, perms).unwrap();
        }

        script_path
    }

    #[tokio::test]
    async fn test_execute_simple_hook() {
        let temp = TempDir::new().unwrap();

        // Create a simple script that outputs JSON
        let script = r#"#!/bin/sh
echo '{"continue": true, "decision": "allow"}'
"#;
        let script_path = create_test_hook_script(temp.path(), "test.sh", script);

        let hook = ConfiguredHook {
            definition: super::super::manifest::HookDefinition {
                event: "PreToolUse".to_string(),
                matchers: vec![],
                command: script_path.display().to_string(),
                timeout: 5000,
            },
            plugin_root: temp.path().to_path_buf(),
            plugin_name: "test-plugin".to_string(),
        };

        let input = HookInput::pre_tool_use(
            "session-123",
            "/workspace",
            "/transcript",
            "write_file",
            serde_json::json!({}),
        );

        let mut dispatcher = HookDispatcher::new();
        let mut hooks = PluginHooks::new();
        hooks.add_hook(HookEvent::PreToolUse, hook);
        dispatcher.register_hooks(hooks);

        let executor = HookExecutor::new(dispatcher);
        let result = executor.dispatch(HookEvent::PreToolUse, input).await;

        assert!(matches!(result, HookResult::Continue));
    }

    #[tokio::test]
    async fn test_hook_deny() {
        let temp = TempDir::new().unwrap();

        let script = r#"#!/bin/sh
echo '{"continue": true, "decision": "deny", "reason": "Not allowed"}'
"#;
        let script_path = create_test_hook_script(temp.path(), "deny.sh", script);

        let hook = ConfiguredHook {
            definition: super::super::manifest::HookDefinition {
                event: "PreToolUse".to_string(),
                matchers: vec![],
                command: script_path.display().to_string(),
                timeout: 5000,
            },
            plugin_root: temp.path().to_path_buf(),
            plugin_name: "test-plugin".to_string(),
        };

        let input = HookInput::pre_tool_use(
            "session-123",
            "/workspace",
            "/transcript",
            "write_file",
            serde_json::json!({}),
        );

        let mut dispatcher = HookDispatcher::new();
        let mut hooks = PluginHooks::new();
        hooks.add_hook(HookEvent::PreToolUse, hook);
        dispatcher.register_hooks(hooks);

        let executor = HookExecutor::new(dispatcher);
        let result = executor.dispatch(HookEvent::PreToolUse, input).await;

        assert!(matches!(result, HookResult::Denied(_)));
    }

    #[tokio::test]
    async fn test_hook_block() {
        let temp = TempDir::new().unwrap();

        let script = r#"#!/bin/sh
echo '{"continue": false, "decision": "block", "reason": "Blocked!"}'
"#;
        let script_path = create_test_hook_script(temp.path(), "block.sh", script);

        let hook = ConfiguredHook {
            definition: super::super::manifest::HookDefinition {
                event: "PreToolUse".to_string(),
                matchers: vec![],
                command: script_path.display().to_string(),
                timeout: 5000,
            },
            plugin_root: temp.path().to_path_buf(),
            plugin_name: "test-plugin".to_string(),
        };

        let input = HookInput::pre_tool_use(
            "session-123",
            "/workspace",
            "/transcript",
            "write_file",
            serde_json::json!({}),
        );

        let mut dispatcher = HookDispatcher::new();
        let mut hooks = PluginHooks::new();
        hooks.add_hook(HookEvent::PreToolUse, hook);
        dispatcher.register_hooks(hooks);

        let executor = HookExecutor::new(dispatcher);
        let result = executor.dispatch(HookEvent::PreToolUse, input).await;

        assert!(matches!(result, HookResult::Blocked(_)));
    }

    #[test]
    fn test_load_hooks_from_file() {
        let temp = TempDir::new().unwrap();
        let hooks_path = temp.path().join("hooks.json");

        let hooks_content = r#"{
            "hooks": [
                {
                    "event": "PreToolUse",
                    "matchers": [
                        {
                            "type": "tool_name",
                            "tool_names": ["write_file"]
                        }
                    ],
                    "command": "echo 'hello'",
                    "timeout": 5000
                },
                {
                    "event": "SessionStart",
                    "matchers": [],
                    "command": "echo 'started'",
                    "timeout": 1000
                }
            ]
        }"#;

        fs::write(&hooks_path, hooks_content).unwrap();

        let hooks =
            load_hooks_from_file(&hooks_path, temp.path().to_path_buf(), "test-plugin").unwrap();

        assert!(hooks.has_hooks(HookEvent::PreToolUse));
        assert!(hooks.has_hooks(HookEvent::SessionStart));
        assert!(!hooks.has_hooks(HookEvent::SessionEnd));
    }
}
