use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::PathBuf;
use std::process::Stdio;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::task::JoinHandle;

use crate::ai::error::AiError;
use crate::ai::model::Model;
use crate::ai::provider::AiProvider;
use crate::ai::tweaks::ModelTweaks;
use crate::ai::types::*;
use crate::settings::config::ToolCallStyle;

/// Provider that proxies requests through the local `codex` CLI in JSONL mode.
#[derive(Clone)]
pub struct CodexCliProvider {
    command: PathBuf,
    additional_args: Vec<String>,
    env: HashMap<String, String>,
}

impl CodexCliProvider {
    /// Returns the default mapping from Model enum to Codex CLI model IDs.
    fn default_model_mappings() -> HashMap<Model, String> {
        let mut mappings = HashMap::new();
        mappings.insert(Model::Gpt53Codex, "gpt-5.3-codex".to_string());
        mappings.insert(Model::Gpt51CodexMax, "gpt-5.1-codex-max".to_string());
        mappings.insert(Model::Gpt52, "gpt-5.2-codex".to_string());
        mappings
    }

    pub fn new(
        command: PathBuf,
        additional_args: Vec<String>,
        env: HashMap<String, String>,
    ) -> Self {
        Self {
            command,
            additional_args,
            env,
        }
    }

    fn resolve_model(&self, requested: &Model) -> String {
        Self::default_model_mappings()
            .get(requested)
            .cloned()
            .unwrap_or_else(|| "gpt-5.3-codex".to_string())
    }

    fn build_prompt(&self, request: &ConversationRequest) -> Result<String, AiError> {
        if request.messages.is_empty() {
            return Err(AiError::Terminal(anyhow::anyhow!(
                "Codex CLI provider requires at least one message"
            )));
        }

        let mut sections = Vec::new();

        if !request.system_prompt.trim().is_empty() {
            sections.push(format!(
                "<system_prompt>\n{}\n</system_prompt>",
                request.system_prompt.trim()
            ));
        }

        if !request.tools.is_empty() {
            let tool_catalog = request
                .tools
                .iter()
                .map(render_tool_catalog_line)
                .collect::<Vec<_>>()
                .join("\n");

            sections.push(format!(
                "<available_tools>\n{}\n</available_tools>",
                tool_catalog
            ));
        }

        if !request.stop_sequences.is_empty() {
            sections.push(format!(
                "<stop_sequences>\n{}\n</stop_sequences>",
                request
                    .stop_sequences
                    .iter()
                    .map(|s| format!("- {s}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }

        let history_start = self.history_start_index(&request.messages);
        let history = &request.messages[history_start..];

        if history_start > 0 {
            sections.push(format!(
                "<history_notice>\nOnly the most recent {} messages are included to control prompt size.\n</history_notice>",
                history.len()
            ));
        }

        let mut transcript = Vec::new();
        for (idx, message) in history.iter().enumerate() {
            transcript.push(self.render_message(idx + 1, message));
        }
        sections.push(format!(
            "<conversation>\n{}\n</conversation>",
            transcript.join("\n\n")
        ));

        sections.push(
            "Respond as the assistant to the most recent user request.\n\
Do not execute shell commands or external tools directly.\n\
If tool usage is required, include tool calls in the `tool_calls` field and keep `assistant_message` concise.\n\
For each tool call, set `arguments` to a JSON object encoded as a string (example: \"{\\\"path\\\":\\\"README.md\\\"}\").\n\
Use the exact parameter names listed in <available_tools>."
                .to_string(),
        );

        Ok(sections.join("\n\n"))
    }

    fn render_message(&self, index: usize, message: &Message) -> String {
        let role = match message.role {
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
        };

        let mut lines = Vec::new();
        for block in message.content.blocks() {
            match block {
                ContentBlock::Text(text) => {
                    if !text.trim().is_empty() {
                        lines.push(text.trim().to_string());
                    }
                }
                ContentBlock::ReasoningContent(reasoning) => {
                    if !reasoning.text.trim().is_empty() {
                        lines.push(format!("[reasoning] {}", reasoning.text.trim()));
                    }
                }
                ContentBlock::ToolUse(tool_use) => {
                    lines.push(format!(
                        "[tool_use id={} name={}] {}",
                        tool_use.id,
                        tool_use.name,
                        serde_json::to_string(&tool_use.arguments)
                            .unwrap_or_else(|_| "{}".to_string())
                    ));
                }
                ContentBlock::ToolResult(tool_result) => {
                    lines.push(format!(
                        "[tool_result tool_use_id={} is_error={}] {}",
                        tool_result.tool_use_id,
                        tool_result.is_error,
                        tool_result.content.trim()
                    ));
                }
                ContentBlock::Image(image) => {
                    lines.push(format!(
                        "[image media_type={} bytes_base64={}]",
                        image.media_type,
                        image.data.len()
                    ));
                }
            }
        }

        if lines.is_empty() {
            lines.push("<empty>".to_string());
        }

        format!(
            "<message index=\"{}\" role=\"{}\">\n{}\n</message>",
            index,
            role,
            lines.join("\n")
        )
    }

    fn history_start_index(&self, messages: &[Message]) -> usize {
        const MAX_PROMPT_MESSAGES: usize = 8;
        const MAX_PROMPT_CHARS: usize = 60_000;

        let mut included = 0usize;
        let mut chars = 0usize;
        let mut start_idx = messages.len().saturating_sub(1);

        for idx in (0..messages.len()).rev() {
            let estimated = estimate_message_chars(&messages[idx]);
            if included > 0
                && (included >= MAX_PROMPT_MESSAGES || chars + estimated > MAX_PROMPT_CHARS)
            {
                break;
            }
            chars += estimated;
            included += 1;
            start_idx = idx;
        }

        start_idx
    }

    fn build_output_schema(&self, tools: &[ToolDefinition]) -> Value {
        if tools.is_empty() {
            return serde_json::json!({
                "$schema": "http://json-schema.org/draft-07/schema#",
                "type": "object",
                "properties": {
                    "assistant_message": { "type": "string" }
                },
                "required": ["assistant_message"],
                "additionalProperties": false
            });
        }

        let tool_names: Vec<String> = tools.iter().map(|tool| tool.name.clone()).collect();

        serde_json::json!({
            "$schema": "http://json-schema.org/draft-07/schema#",
            "type": "object",
            "properties": {
                "assistant_message": { "type": "string" },
                "tool_calls": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string", "enum": tool_names },
                            "arguments": { "type": "string" }
                        },
                        "required": ["name", "arguments"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["assistant_message", "tool_calls"],
            "additionalProperties": false
        })
    }

    fn write_output_schema_file(&self, tools: &[ToolDefinition]) -> Result<PathBuf, AiError> {
        let schema = self.build_output_schema(tools);
        let schema_bytes = serde_json::to_vec_pretty(&schema).map_err(|err| {
            AiError::Terminal(anyhow::anyhow!(
                "Failed serializing Codex output schema: {err}"
            ))
        })?;

        let path = std::env::temp_dir().join(format!(
            "tycode-codex-output-schema-{}.json",
            uuid::Uuid::new_v4()
        ));

        fs::write(&path, schema_bytes).map_err(|err| {
            AiError::Terminal(anyhow::anyhow!(
                "Failed writing Codex output schema file '{}': {err}",
                path.display()
            ))
        })?;

        Ok(path)
    }

    async fn invoke_cli(
        &self,
        prompt: &str,
        model: &str,
        tools: &[ToolDefinition],
    ) -> Result<(Vec<ContentBlock>, TokenUsage, StopReason), AiError> {
        let output_schema_path = self.write_output_schema_file(tools)?;
        let mut command = Command::new(&self.command);
        command
            .arg("exec")
            .arg("--json")
            .arg("--skip-git-repo-check")
            .arg("--sandbox")
            .arg("read-only")
            .arg("--disable")
            .arg("shell_tool")
            .arg("--model")
            .arg(model)
            .arg("--output-schema")
            .arg(&output_schema_path);

        for arg in &self.additional_args {
            command.arg(arg);
        }

        command.arg("-");
        command.env("NO_COLOR", "1");

        for (key, value) in &self.env {
            command.env(key, value);
        }

        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|err| {
                let _ = fs::remove_file(&output_schema_path);
                AiError::Terminal(anyhow::anyhow!(
                    "Failed to spawn Codex CLI '{}': {err}",
                    self.command.display()
                ))
            })?;

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| AiError::Terminal(anyhow::anyhow!("Codex CLI stdin is unavailable")))?;
        stdin.write_all(prompt.as_bytes()).await.map_err(|err| {
            AiError::Terminal(anyhow::anyhow!(
                "Failed writing prompt to Codex stdin: {err}"
            ))
        })?;
        stdin.flush().await.map_err(|err| {
            AiError::Terminal(anyhow::anyhow!("Failed to flush Codex stdin: {err}"))
        })?;
        drop(stdin);

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| AiError::Terminal(anyhow::anyhow!("Codex CLI stdout is unavailable")))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| AiError::Terminal(anyhow::anyhow!("Codex CLI stderr is unavailable")))?;

        let mut stdout_reader = BufReader::new(stdout).lines();
        let stderr_handle: JoinHandle<Result<String, std::io::Error>> = tokio::spawn(async move {
            let mut buf = String::new();
            let mut reader = BufReader::new(stderr);
            reader.read_to_string(&mut buf).await?;
            Ok(buf)
        });

        let mut state = CodexRunState::default();

        while let Some(line) = stdout_reader.next_line().await.map_err(|err| {
            AiError::Retryable(anyhow::anyhow!("Failed reading Codex CLI stdout: {err}"))
        })? {
            if line.trim().is_empty() {
                continue;
            }

            let Ok(value) = serde_json::from_str::<Value>(&line) else {
                tracing::debug!("Ignoring non-JSON Codex stdout line: {line}");
                continue;
            };

            state.handle_event(value);
        }

        let status = child.wait().await.map_err(|err| {
            AiError::Retryable(anyhow::anyhow!("Failed waiting for Codex CLI: {err}"))
        })?;

        let stderr_output = match stderr_handle.await {
            Ok(Ok(text)) => text,
            Ok(Err(err)) => {
                tracing::warn!("Failed reading Codex CLI stderr: {err}");
                String::new()
            }
            Err(err) => {
                tracing::warn!("Failed awaiting Codex CLI stderr: {err}");
                String::new()
            }
        };

        let _ = fs::remove_file(&output_schema_path);

        if let Some(error_message) = state.error_message.clone() {
            return Err(map_codex_error(error_message));
        }

        if !status.success() {
            let message = stderr_output.trim();
            if message.is_empty() {
                return Err(AiError::Terminal(anyhow::anyhow!(
                    "Codex CLI exited with status {}",
                    status
                )));
            }
            return Err(map_codex_error(message.to_string()));
        }

        Ok(state.finish())
    }
}

#[async_trait::async_trait]
impl AiProvider for CodexCliProvider {
    fn name(&self) -> &'static str {
        "Codex"
    }

    fn supported_models(&self) -> HashSet<Model> {
        HashSet::from([Model::Gpt53Codex, Model::Gpt52, Model::Gpt51CodexMax])
    }

    async fn converse(
        &self,
        request: ConversationRequest,
    ) -> Result<ConversationResponse, AiError> {
        let model_id = self.resolve_model(&request.model.model);
        let prompt = self.build_prompt(&request)?;
        let (content_blocks, usage, stop_reason) =
            self.invoke_cli(&prompt, &model_id, &request.tools).await?;

        Ok(ConversationResponse {
            content: Content::from(content_blocks),
            usage,
            stop_reason,
        })
    }

    fn get_cost(&self, model: &Model) -> Cost {
        match model {
            Model::Gpt53Codex => Cost::new(1.75, 14.0, 0.0, 0.0),
            Model::Gpt52 => Cost::new(1.75, 14.0, 0.0, 0.0),
            Model::Gpt51CodexMax => Cost::new(1.25, 10.0, 0.0, 0.0),
            _ => Cost::new(0.0, 0.0, 0.0, 0.0),
        }
    }

    fn tweaks(&self) -> ModelTweaks {
        ModelTweaks {
            tool_call_style: Some(ToolCallStyle::Json),
            ..Default::default()
        }
    }
}

#[derive(Default)]
struct CodexRunState {
    content_blocks: Vec<ContentBlock>,
    usage: Option<TokenUsage>,
    error_message: Option<String>,
}

impl CodexRunState {
    fn handle_event(&mut self, value: Value) {
        let Some(event_type) = value.get("type").and_then(Value::as_str) else {
            return;
        };

        match event_type {
            "item.completed" => {
                let Some(item) = value.get("item") else {
                    return;
                };

                let Some(item_type) = item.get("type").and_then(Value::as_str) else {
                    return;
                };

                match item_type {
                    "reasoning" => {
                        let text = extract_item_text(item);
                        if !text.is_empty() {
                            self.content_blocks.push(ContentBlock::ReasoningContent(
                                ReasoningData {
                                    text,
                                    signature: None,
                                    blob: None,
                                    raw_json: None,
                                },
                            ));
                        }
                    }
                    "agent_message" | "assistant_message" | "output_text" | "message" => {
                        let text = extract_item_text(item);
                        if let Some((assistant_message, tool_calls)) =
                            parse_structured_agent_message(&text)
                        {
                            if !assistant_message.is_empty() {
                                self.content_blocks
                                    .push(ContentBlock::Text(assistant_message));
                            }
                            for tool_call in tool_calls {
                                self.content_blocks.push(ContentBlock::ToolUse(tool_call));
                            }
                        } else if !text.is_empty() {
                            self.content_blocks.push(ContentBlock::Text(text));
                        }
                    }
                    _ => {}
                }
            }
            "turn.completed" => {
                self.usage = value.get("usage").and_then(parse_usage);
            }
            "turn.failed" => {
                self.error_message = value
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
            }
            "error" => {
                self.error_message = value
                    .get("message")
                    .and_then(Value::as_str)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
            }
            _ => {}
        }
    }

    fn finish(self) -> (Vec<ContentBlock>, TokenUsage, StopReason) {
        let mut content_blocks = self.content_blocks;

        let has_text = content_blocks
            .iter()
            .any(|block| matches!(block, ContentBlock::Text(text) if !text.trim().is_empty()));

        if !has_text {
            let fallback = content_blocks.iter().rev().find_map(|block| match block {
                ContentBlock::ReasoningContent(reasoning) if !reasoning.text.trim().is_empty() => {
                    Some(reasoning_fallback_text(&reasoning.text))
                }
                _ => None,
            });

            if let Some(text) = fallback {
                content_blocks.push(ContentBlock::Text(text));
            }
        }

        (
            content_blocks,
            self.usage.unwrap_or_else(TokenUsage::empty),
            StopReason::EndTurn,
        )
    }
}

fn extract_item_text(item: &Value) -> String {
    if let Some(text) = item.get("text").and_then(Value::as_str) {
        let text = text.trim();
        if !text.is_empty() {
            return text.to_string();
        }
    }

    let from_content: String = item
        .get("content")
        .and_then(Value::as_array)
        .map(|parts| extract_text_from_content_parts(parts))
        .unwrap_or_default();
    if !from_content.is_empty() {
        return from_content;
    }

    item.get("message")
        .and_then(|message| {
            message
                .get("content")
                .and_then(Value::as_array)
                .map(|parts| extract_text_from_content_parts(parts))
        })
        .unwrap_or_default()
}

fn extract_text_from_content_parts(parts: &[Value]) -> String {
    parts
        .iter()
        .filter_map(|part| part.get("text").and_then(Value::as_str))
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn reasoning_fallback_text(reasoning_text: &str) -> String {
    let trimmed = reasoning_text.trim();
    if let Some((first, rest)) = trimmed.split_once("\n\n") {
        let first = first.trim();
        let rest = rest.trim();
        if first.starts_with("**") && first.ends_with("**") && !rest.is_empty() {
            return rest.to_string();
        }
    }
    trimmed.to_string()
}

fn estimate_message_chars(message: &Message) -> usize {
    message
        .content
        .blocks()
        .iter()
        .map(|block| match block {
            ContentBlock::Text(text) => text.len(),
            ContentBlock::ReasoningContent(reasoning) => reasoning.text.len(),
            ContentBlock::ToolUse(tool_use) => {
                tool_use.name.len()
                    + serde_json::to_string(&tool_use.arguments)
                        .map(|s| s.len())
                        .unwrap_or(64)
            }
            ContentBlock::ToolResult(tool_result) => tool_result.content.len() + 32,
            ContentBlock::Image(image) => image.data.len().min(512) + image.media_type.len(),
        })
        .sum::<usize>()
}

fn render_tool_catalog_line(tool: &ToolDefinition) -> String {
    let description = tool
        .description
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let arg_names = tool
        .input_schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|props| {
            props
                .keys()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "<none>".to_string());
    let required = tool
        .input_schema
        .get("required")
        .and_then(Value::as_array)
        .map(|vals| {
            vals.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "<none>".to_string());

    format!(
        "- {}: {} (args: {}; required: {})",
        tool.name, description, arg_names, required
    )
}

fn parse_structured_agent_message(text: &str) -> Option<(String, Vec<ToolUseData>)> {
    let value: Value = serde_json::from_str(text).ok()?;
    let obj = value.as_object()?;

    let assistant_message = obj
        .get("assistant_message")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string();

    let mut tool_calls = Vec::new();
    if let Some(calls) = obj.get("tool_calls").and_then(Value::as_array) {
        for call in calls {
            let Some(name) = call.get("name").and_then(Value::as_str) else {
                continue;
            };

            let arguments = call
                .get("arguments")
                .and_then(parse_tool_arguments_value)
                .unwrap_or_else(|| serde_json::json!({}));

            let id = call
                .get("id")
                .and_then(Value::as_str)
                .map(|s| s.to_string())
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

            tool_calls.push(ToolUseData {
                id,
                name: name.to_string(),
                arguments,
            });
        }
    }

    Some((assistant_message, tool_calls))
}

fn parse_tool_arguments_value(value: &Value) -> Option<Value> {
    match value {
        Value::Object(_) => Some(value.clone()),
        Value::String(raw) => {
            let parsed: Value = serde_json::from_str(raw).ok()?;
            Some(parsed)
        }
        _ => Some(value.clone()),
    }
}

fn parse_usage(value: &Value) -> Option<TokenUsage> {
    let input_tokens = value.get("input_tokens")?.as_u64()? as u32;
    let output_tokens = value.get("output_tokens")?.as_u64()? as u32;
    let cached_prompt_tokens = value
        .get("cached_input_tokens")
        .and_then(Value::as_u64)
        .map(|v| v as u32);

    Some(TokenUsage {
        input_tokens,
        output_tokens,
        total_tokens: input_tokens + output_tokens,
        cached_prompt_tokens,
        cache_creation_input_tokens: None,
        reasoning_tokens: None,
    })
}

fn map_codex_error(message: String) -> AiError {
    let lower = message.to_lowercase();
    if lower.contains("context") && lower.contains("length")
        || lower.contains("too long")
        || lower.contains("maximum context")
    {
        return AiError::InputTooLong(anyhow::anyhow!("Codex CLI error: {}", message));
    }

    AiError::Terminal(anyhow::anyhow!("Codex CLI error: {}", message))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider_for_unit_tests() -> CodexCliProvider {
        CodexCliProvider::new(PathBuf::from("codex"), Vec::new(), HashMap::new())
    }

    #[test]
    fn build_prompt_includes_system_tools_and_history() {
        let provider = provider_for_unit_tests();
        let request = ConversationRequest {
            messages: vec![
                Message::user(Content::text_only("read README".to_string())),
                Message::assistant(Content::new(vec![ContentBlock::Text(
                    "I can do that".to_string(),
                )])),
            ],
            model: Model::Gpt51CodexMax.default_settings(),
            system_prompt: "You are strict about tool formatting.".to_string(),
            stop_sequences: vec!["<stop>".to_string()],
            tools: vec![ToolDefinition {
                name: "read_file".to_string(),
                description: "Read files".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": { "path": { "type": "string" } },
                    "required": ["path"],
                }),
            }],
        };

        let prompt = provider
            .build_prompt(&request)
            .expect("prompt should build");
        assert!(prompt.contains("<system_prompt>"));
        assert!(prompt.contains("<available_tools>"));
        assert!(prompt.contains("read_file"));
        assert!(prompt.contains("<conversation>"));
        assert!(prompt.contains("read README"));
        assert!(prompt.contains("I can do that"));
        assert!(prompt.contains("<stop>"));
    }

    #[test]
    fn parses_structured_agent_message_with_tool_calls() {
        let raw = r#"{"assistant_message":"loading readme","tool_calls":[{"name":"set_tracked_files","arguments":{"file_paths":["README.md"]}}]}"#;
        let parsed = parse_structured_agent_message(raw).expect("should parse structured output");
        assert_eq!(parsed.0, "loading readme");
        assert_eq!(parsed.1.len(), 1);
        assert_eq!(parsed.1[0].name, "set_tracked_files");
        assert_eq!(parsed.1[0].arguments["file_paths"][0], "README.md");
    }

    #[test]
    fn parses_structured_agent_message_with_json_string_arguments() {
        let raw = r#"{"assistant_message":"loading readme","tool_calls":[{"name":"set_tracked_files","arguments":"{\"file_paths\":[\"README.md\"]}"}]}"#;
        let parsed = parse_structured_agent_message(raw).expect("should parse structured output");
        assert_eq!(parsed.0, "loading readme");
        assert_eq!(parsed.1.len(), 1);
        assert_eq!(parsed.1[0].name, "set_tracked_files");
        assert_eq!(parsed.1[0].arguments["file_paths"][0], "README.md");
    }

    #[test]
    fn history_window_limits_older_messages() {
        let provider = provider_for_unit_tests();
        let messages = (0..20)
            .map(|i| {
                let role = if i % 2 == 0 {
                    MessageRole::User
                } else {
                    MessageRole::Assistant
                };
                Message::new(role, Content::text_only(format!("message-{i}")))
            })
            .collect::<Vec<_>>();

        let start = provider.history_start_index(&messages);
        let kept = &messages[start..];
        assert!(kept.len() <= 8);
        assert!(kept.iter().all(|m| !m.content.text().contains("message-0")));
        assert!(kept
            .last()
            .expect("kept must not be empty")
            .content
            .text()
            .contains("message-19"));
    }

    #[test]
    fn resolves_new_gpt53_codex_model_id() {
        let provider = provider_for_unit_tests();
        let model_id = provider.resolve_model(&Model::Gpt53Codex);
        assert_eq!(model_id, "gpt-5.3-codex");
    }

    #[test]
    fn output_schema_uses_enum_tool_names_with_string_type() {
        let provider = provider_for_unit_tests();
        let schema = provider.build_output_schema(&[ToolDefinition {
            name: "set_tracked_files".to_string(),
            description: "Track files".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": { "file_paths": { "type": "array", "items": { "type": "string" } } },
                "required": ["file_paths"]
            }),
        }]);

        let name_schema = &schema["properties"]["tool_calls"]["items"]["properties"]["name"];
        assert_eq!(name_schema["type"], "string");
        assert_eq!(name_schema["enum"][0], "set_tracked_files");

        let arguments_schema =
            &schema["properties"]["tool_calls"]["items"]["properties"]["arguments"];
        assert_eq!(arguments_schema["type"], "string");
    }

    #[test]
    fn run_state_collects_reasoning_text_and_usage() {
        let mut state = CodexRunState::default();
        state.handle_event(serde_json::json!({
            "type": "item.completed",
            "item": { "type": "reasoning", "text": "thinking" }
        }));
        state.handle_event(serde_json::json!({
            "type": "item.completed",
            "item": { "type": "agent_message", "text": "final answer" }
        }));
        state.handle_event(serde_json::json!({
            "type": "turn.completed",
            "usage": { "input_tokens": 10, "cached_input_tokens": 3, "output_tokens": 7 }
        }));

        let (blocks, usage, stop_reason) = state.finish();
        assert!(matches!(stop_reason, StopReason::EndTurn));
        assert_eq!(usage.input_tokens, 10);
        assert_eq!(usage.output_tokens, 7);
        assert_eq!(usage.total_tokens, 17);
        assert_eq!(usage.cached_prompt_tokens, Some(3));
        assert_eq!(blocks.len(), 2);

        match &blocks[0] {
            ContentBlock::ReasoningContent(r) => assert_eq!(r.text, "thinking"),
            _ => panic!("first block should be reasoning"),
        }

        match &blocks[1] {
            ContentBlock::Text(t) => assert_eq!(t, "final answer"),
            _ => panic!("second block should be text"),
        }
    }

    #[test]
    fn run_state_supports_assistant_message_item_type() {
        let mut state = CodexRunState::default();
        state.handle_event(serde_json::json!({
            "type": "item.completed",
            "item": { "type": "assistant_message", "text": "hello from assistant_message" }
        }));

        let (blocks, _, _) = state.finish();
        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            ContentBlock::Text(text) => assert_eq!(text, "hello from assistant_message"),
            _ => panic!("expected text block"),
        }
    }

    #[test]
    fn run_state_uses_reasoning_as_fallback_when_no_text_exists() {
        let mut state = CodexRunState::default();
        state.handle_event(serde_json::json!({
            "type": "item.completed",
            "item": { "type": "reasoning", "text": "**Plan**\n\nHere is the answer." }
        }));

        let (blocks, _, _) = state.finish();
        assert_eq!(blocks.len(), 2);
        match &blocks[1] {
            ContentBlock::Text(text) => assert_eq!(text, "Here is the answer."),
            _ => panic!("expected fallback text block"),
        }
    }

    #[test]
    fn map_codex_error_classifies_input_too_long() {
        let error = map_codex_error("Maximum context length exceeded".to_string());
        assert!(matches!(error, AiError::InputTooLong(_)));
    }
}
