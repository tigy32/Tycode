//! Hook system for plugin event handling.
//!
//! This module defines hook events compatible with Claude Code's hook system.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;

use super::manifest::HookDefinition;

/// All supported hook events (Claude Code compatible).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum HookEvent {
    /// Fired before a tool is executed
    PreToolUse,
    /// Fired after successful tool execution
    PostToolUse,
    /// Fired after tool execution fails
    PostToolUseFailure,
    /// Fired when a permission dialog is shown
    PermissionRequest,
    /// Fired when the user submits a prompt
    UserPromptSubmit,
    /// Fired when notifications are sent
    Notification,
    /// Fired when an agent finishes
    Stop,
    /// Fired when a subagent starts
    SubagentStart,
    /// Fired when a subagent stops
    SubagentStop,
    /// Fired at session start
    SessionStart,
    /// Fired at session end
    SessionEnd,
    /// Fired before context compaction
    PreCompact,
}

impl HookEvent {
    /// Returns the event name as used in hook configurations.
    pub fn as_str(&self) -> &'static str {
        match self {
            HookEvent::PreToolUse => "PreToolUse",
            HookEvent::PostToolUse => "PostToolUse",
            HookEvent::PostToolUseFailure => "PostToolUseFailure",
            HookEvent::PermissionRequest => "PermissionRequest",
            HookEvent::UserPromptSubmit => "UserPromptSubmit",
            HookEvent::Notification => "Notification",
            HookEvent::Stop => "Stop",
            HookEvent::SubagentStart => "SubagentStart",
            HookEvent::SubagentStop => "SubagentStop",
            HookEvent::SessionStart => "SessionStart",
            HookEvent::SessionEnd => "SessionEnd",
            HookEvent::PreCompact => "PreCompact",
        }
    }

    /// Parses a hook event from a string.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "PreToolUse" => Some(HookEvent::PreToolUse),
            "PostToolUse" => Some(HookEvent::PostToolUse),
            "PostToolUseFailure" => Some(HookEvent::PostToolUseFailure),
            "PermissionRequest" => Some(HookEvent::PermissionRequest),
            "UserPromptSubmit" => Some(HookEvent::UserPromptSubmit),
            "Notification" => Some(HookEvent::Notification),
            "Stop" => Some(HookEvent::Stop),
            "SubagentStart" => Some(HookEvent::SubagentStart),
            "SubagentStop" => Some(HookEvent::SubagentStop),
            "SessionStart" => Some(HookEvent::SessionStart),
            "SessionEnd" => Some(HookEvent::SessionEnd),
            "PreCompact" => Some(HookEvent::PreCompact),
            _ => None,
        }
    }
}

/// Input data passed to hooks via stdin (Claude Code compatible).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookInput {
    /// Current session ID
    pub session_id: String,

    /// Path to the transcript file
    pub transcript_path: String,

    /// Current working directory
    pub cwd: String,

    /// The hook event name
    pub hook_event_name: String,

    /// Tool name (for tool-related hooks)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,

    /// Tool input arguments (for tool-related hooks)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_input: Option<Value>,

    /// Tool output (for PostToolUse)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_output: Option<String>,

    /// Error message (for PostToolUseFailure)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// User prompt content (for UserPromptSubmit)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,

    /// Agent name (for agent-related hooks)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,

    /// Agent task (for SubagentStart)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task: Option<String>,

    /// Notification content (for Notification hooks)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notification: Option<String>,
}

impl HookInput {
    /// Creates a new HookInput for a session start event.
    pub fn session_start(session_id: &str, cwd: &str, transcript_path: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            transcript_path: transcript_path.to_string(),
            cwd: cwd.to_string(),
            hook_event_name: HookEvent::SessionStart.as_str().to_string(),
            tool_name: None,
            tool_input: None,
            tool_output: None,
            error: None,
            prompt: None,
            agent_name: None,
            task: None,
            notification: None,
        }
    }

    /// Creates a new HookInput for a pre-tool-use event.
    pub fn pre_tool_use(
        session_id: &str,
        cwd: &str,
        transcript_path: &str,
        tool_name: &str,
        tool_input: Value,
    ) -> Self {
        Self {
            session_id: session_id.to_string(),
            transcript_path: transcript_path.to_string(),
            cwd: cwd.to_string(),
            hook_event_name: HookEvent::PreToolUse.as_str().to_string(),
            tool_name: Some(tool_name.to_string()),
            tool_input: Some(tool_input),
            tool_output: None,
            error: None,
            prompt: None,
            agent_name: None,
            task: None,
            notification: None,
        }
    }

    /// Creates a new HookInput for a post-tool-use event.
    pub fn post_tool_use(
        session_id: &str,
        cwd: &str,
        transcript_path: &str,
        tool_name: &str,
        tool_input: Value,
        tool_output: &str,
    ) -> Self {
        Self {
            session_id: session_id.to_string(),
            transcript_path: transcript_path.to_string(),
            cwd: cwd.to_string(),
            hook_event_name: HookEvent::PostToolUse.as_str().to_string(),
            tool_name: Some(tool_name.to_string()),
            tool_input: Some(tool_input),
            tool_output: Some(tool_output.to_string()),
            error: None,
            prompt: None,
            agent_name: None,
            task: None,
            notification: None,
        }
    }

    /// Creates a new HookInput for a post-tool-use-failure event.
    pub fn post_tool_use_failure(
        session_id: &str,
        cwd: &str,
        transcript_path: &str,
        tool_name: &str,
        tool_input: Value,
        tool_output: &str,
        error: &str,
    ) -> Self {
        Self {
            session_id: session_id.to_string(),
            transcript_path: transcript_path.to_string(),
            cwd: cwd.to_string(),
            hook_event_name: HookEvent::PostToolUseFailure.as_str().to_string(),
            tool_name: Some(tool_name.to_string()),
            tool_input: Some(tool_input),
            tool_output: Some(tool_output.to_string()),
            error: Some(error.to_string()),
            prompt: None,
            agent_name: None,
            task: None,
            notification: None,
        }
    }

    /// Creates a new HookInput for a user prompt submit event.
    pub fn user_prompt_submit(session_id: &str, cwd: &str, transcript_path: &str, prompt: &str) -> Self {
        Self {
            session_id: session_id.to_string(),
            transcript_path: transcript_path.to_string(),
            cwd: cwd.to_string(),
            hook_event_name: HookEvent::UserPromptSubmit.as_str().to_string(),
            tool_name: None,
            tool_input: None,
            tool_output: None,
            error: None,
            prompt: Some(prompt.to_string()),
            agent_name: None,
            task: None,
            notification: None,
        }
    }

    /// Creates a new HookInput for a subagent start event.
    pub fn subagent_start(
        session_id: &str,
        cwd: &str,
        transcript_path: &str,
        agent_name: &str,
        task: &str,
    ) -> Self {
        Self {
            session_id: session_id.to_string(),
            transcript_path: transcript_path.to_string(),
            cwd: cwd.to_string(),
            hook_event_name: HookEvent::SubagentStart.as_str().to_string(),
            tool_name: None,
            tool_input: None,
            tool_output: None,
            error: None,
            prompt: None,
            agent_name: Some(agent_name.to_string()),
            task: Some(task.to_string()),
            notification: None,
        }
    }
}

/// Output from a hook (Claude Code compatible).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookOutput {
    /// Whether to continue execution
    #[serde(default = "default_continue")]
    pub r#continue: bool,

    /// Decision for PreToolUse: allow, deny, or block
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision: Option<HookDecision>,

    /// Reason for the decision
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,

    /// Whether to suppress output
    #[serde(default)]
    pub suppress_output: bool,

    /// Hook-specific output data
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hook_specific_output: Option<HookSpecificOutput>,
}

fn default_continue() -> bool {
    true
}

impl Default for HookOutput {
    fn default() -> Self {
        Self {
            r#continue: true,
            decision: None,
            reason: None,
            suppress_output: false,
            hook_specific_output: None,
        }
    }
}

/// Decision values for hooks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HookDecision {
    Allow,
    Deny,
    Block,
    Ask,
}

/// Hook-specific output data.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookSpecificOutput {
    /// The hook event name this output is for
    pub hook_event_name: String,

    /// Permission decision for PreToolUse
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_decision: Option<HookDecision>,

    /// Updated tool input (for modifying tool arguments)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_input: Option<Value>,

    /// Additional context to add to the conversation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub additional_context: Option<String>,
}

impl HookOutput {
    /// Creates a default "continue" output.
    pub fn allow() -> Self {
        Self {
            r#continue: true,
            decision: Some(HookDecision::Allow),
            ..Default::default()
        }
    }

    /// Creates a "deny" output that prevents the action but continues conversation.
    pub fn deny(reason: String) -> Self {
        Self {
            r#continue: true,
            decision: Some(HookDecision::Deny),
            reason: Some(reason),
            ..Default::default()
        }
    }

    /// Creates a "block" output that stops execution entirely.
    pub fn block(reason: String) -> Self {
        Self {
            r#continue: false,
            decision: Some(HookDecision::Block),
            reason: Some(reason),
            ..Default::default()
        }
    }

    /// Creates an output with modified tool input.
    pub fn with_modified_input(mut self, input: Value) -> Self {
        self.hook_specific_output = Some(HookSpecificOutput {
            hook_event_name: HookEvent::PreToolUse.as_str().to_string(),
            permission_decision: Some(HookDecision::Allow),
            updated_input: Some(input),
            additional_context: None,
        });
        self
    }
}

/// Result of hook execution.
#[derive(Debug, Clone, Default)]
pub enum HookResult {
    /// Continue with execution (no hooks blocked)
    #[default]
    Continue,
    /// Continue but with modified input
    ContinueModified(Value),
    /// Tool was denied (returns error to agent)
    Denied(String),
    /// Execution was blocked entirely
    Blocked(String),
}

/// Configured hook for a specific event.
#[derive(Debug, Clone)]
pub struct ConfiguredHook {
    /// The hook definition
    pub definition: HookDefinition,
    /// Plugin root path for variable expansion
    pub plugin_root: PathBuf,
    /// Plugin name for logging
    pub plugin_name: String,
}

impl ConfiguredHook {
    /// Returns the expanded command with variables replaced.
    pub fn expanded_command(&self) -> String {
        self.definition
            .command
            .replace("${CLAUDE_PLUGIN_ROOT}", &self.plugin_root.display().to_string())
    }

    /// Checks if this hook matches the given input.
    pub fn matches(&self, input: &HookInput) -> bool {
        if self.definition.matchers.is_empty() {
            return true;
        }

        for matcher in &self.definition.matchers {
            match matcher.matcher_type.as_str() {
                "tool_name" => {
                    if let Some(tool_name) = &input.tool_name {
                        if !matcher.tool_names.is_empty()
                            && !matcher.tool_names.contains(tool_name)
                        {
                            return false;
                        }
                    }
                }
                "pattern" => {
                    if let (Some(pattern), Some(prompt)) = (&matcher.pattern, &input.prompt) {
                        if !prompt.contains(pattern) {
                            return false;
                        }
                    }
                }
                _ => {}
            }
        }

        true
    }
}

/// Collection of hooks for a plugin, organized by event.
#[derive(Debug, Clone, Default)]
pub struct PluginHooks {
    /// Hooks indexed by event type
    pub hooks: HashMap<HookEvent, Vec<ConfiguredHook>>,
}

impl PluginHooks {
    /// Creates a new empty PluginHooks.
    pub fn new() -> Self {
        Self {
            hooks: HashMap::new(),
        }
    }

    /// Adds a hook for a specific event.
    pub fn add_hook(&mut self, event: HookEvent, hook: ConfiguredHook) {
        self.hooks.entry(event).or_default().push(hook);
    }

    /// Gets all hooks for a specific event.
    pub fn get_hooks(&self, event: HookEvent) -> &[ConfiguredHook] {
        self.hooks.get(&event).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Returns true if there are any hooks for the given event.
    pub fn has_hooks(&self, event: HookEvent) -> bool {
        self.hooks.get(&event).map(|v| !v.is_empty()).unwrap_or(false)
    }

    /// Merges another PluginHooks into this one.
    pub fn merge(&mut self, other: PluginHooks) {
        for (event, hooks) in other.hooks {
            self.hooks.entry(event).or_default().extend(hooks);
        }
    }
}

/// Dispatches hook events to all registered plugins.
#[derive(Debug, Default)]
pub struct HookDispatcher {
    /// All configured hooks from all plugins
    pub(crate) all_hooks: PluginHooks,
}

impl HookDispatcher {
    /// Creates a new HookDispatcher.
    pub fn new() -> Self {
        Self {
            all_hooks: PluginHooks::new(),
        }
    }

    /// Registers hooks from a plugin.
    pub fn register_hooks(&mut self, hooks: PluginHooks) {
        self.all_hooks.merge(hooks);
    }

    /// Returns true if there are any hooks for the given event.
    pub fn has_hooks(&self, event: HookEvent) -> bool {
        self.all_hooks.has_hooks(event)
    }

    /// Gets all hooks for a specific event.
    pub fn get_hooks(&self, event: HookEvent) -> &[ConfiguredHook] {
        self.all_hooks.get_hooks(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hook_event_parsing() {
        assert_eq!(HookEvent::parse("PreToolUse"), Some(HookEvent::PreToolUse));
        assert_eq!(HookEvent::parse("SessionStart"), Some(HookEvent::SessionStart));
        assert_eq!(HookEvent::parse("Invalid"), None);
    }

    #[test]
    fn test_hook_input_serialization() {
        let input = HookInput::pre_tool_use(
            "session-123",
            "/workspace",
            "/path/to/transcript",
            "write_file",
            serde_json::json!({ "path": "/test.txt", "content": "hello" }),
        );

        let json = serde_json::to_string(&input).unwrap();
        assert!(json.contains("PreToolUse"));
        assert!(json.contains("write_file"));
    }

    #[test]
    fn test_hook_output_defaults() {
        let output = HookOutput::default();
        assert!(output.r#continue);
        assert!(output.decision.is_none());
    }

    #[test]
    fn test_hook_output_deny() {
        let output = HookOutput::deny("Not allowed".to_string());
        assert!(output.r#continue);
        assert_eq!(output.decision, Some(HookDecision::Deny));
        assert_eq!(output.reason.as_deref(), Some("Not allowed"));
    }

    #[test]
    fn test_hook_output_block() {
        let output = HookOutput::block("Blocked".to_string());
        assert!(!output.r#continue);
        assert_eq!(output.decision, Some(HookDecision::Block));
    }

    #[test]
    fn test_plugin_hooks_merge() {
        let mut hooks1 = PluginHooks::new();
        let mut hooks2 = PluginHooks::new();

        let hook1 = ConfiguredHook {
            definition: HookDefinition {
                event: "PreToolUse".to_string(),
                matchers: vec![],
                command: "echo hook1".to_string(),
                timeout: 5000,
            },
            plugin_root: PathBuf::from("/plugin1"),
            plugin_name: "plugin1".to_string(),
        };

        let hook2 = ConfiguredHook {
            definition: HookDefinition {
                event: "PreToolUse".to_string(),
                matchers: vec![],
                command: "echo hook2".to_string(),
                timeout: 5000,
            },
            plugin_root: PathBuf::from("/plugin2"),
            plugin_name: "plugin2".to_string(),
        };

        hooks1.add_hook(HookEvent::PreToolUse, hook1);
        hooks2.add_hook(HookEvent::PreToolUse, hook2);

        hooks1.merge(hooks2);
        assert_eq!(hooks1.get_hooks(HookEvent::PreToolUse).len(), 2);
    }
}
