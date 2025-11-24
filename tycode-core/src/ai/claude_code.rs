use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::process::Stdio;

use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::task::JoinHandle;

use crate::ai::error::AiError;
use crate::ai::model::Model;
use crate::ai::provider::AiProvider;
use crate::ai::types::*;

/// Provider that proxies requests through the local `claude` CLI in structured JSON mode.
#[derive(Clone)]
pub struct ClaudeCodeProvider {
    command: PathBuf,
    additional_args: Vec<String>,
    env: HashMap<String, String>,
}

impl ClaudeCodeProvider {
    /// Returns the default mapping from Model enum to Claude CLI model IDs
    fn default_model_mappings() -> HashMap<Model, String> {
        let mut mappings = HashMap::new();
        mappings.insert(
            Model::ClaudeSonnet45,
            "claude-sonnet-4-5-20250929".to_string(),
        );
        mappings.insert(
            Model::ClaudeHaiku45,
            "claude-haiku-4-5-20251001".to_string(),
        );
        mappings.insert(Model::ClaudeOpus45, "claude-opus-4-5-20251101".to_string());
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
            .unwrap_or_else(|| "claude-sonnet-4-5-20250929".to_string())
    }

    fn format_system_prompt(&self, user_prompt: &str) -> Option<String> {
        let user_prompt = user_prompt.trim();
        if user_prompt.is_empty() {
            return None;
        }
        Some(user_prompt.to_string())
    }

    fn build_messages(&self, messages: &[Message]) -> Result<Vec<ClaudeMessage>, AiError> {
        let mut converted = Vec::new();

        for message in messages {
            let role = match message.role {
                MessageRole::User => "user",
                MessageRole::Assistant => "assistant",
            };

            let mut content = Vec::new();
            for block in message.content.blocks() {
                match block {
                    ContentBlock::Text(text) => {
                        if !text.trim().is_empty() {
                            content.push(ClaudeContentBlock::Text {
                                text: text.trim().to_string(),
                            });
                        }
                    }
                    ContentBlock::ReasoningContent(reasoning) => {
                        if !reasoning.text.trim().is_empty() {
                            content.push(ClaudeContentBlock::Thinking {
                                text: reasoning.text.trim().to_string(),
                            });
                        }
                    }
                    ContentBlock::ToolUse(tool_use) => {
                        content.push(ClaudeContentBlock::ToolUse {
                            id: tool_use.id.clone(),
                            name: tool_use.name.clone(),
                            input: tool_use.arguments.clone(),
                        });
                    }
                    ContentBlock::ToolResult(tool_result) => {
                        if !tool_result.content.trim().is_empty() {
                            content.push(ClaudeContentBlock::ToolResult {
                                tool_use_id: tool_result.tool_use_id.clone(),
                                is_error: tool_result.is_error.then_some(true),
                                content: vec![ClaudeToolResultContent::OutputText {
                                    text: tool_result.content.trim().to_string(),
                                }],
                            });
                        }
                    }
                }
            }

            if content.is_empty() {
                continue;
            }

            converted.push(ClaudeMessage {
                role: role.to_string(),
                content,
            });
        }

        Ok(converted)
    }

    fn build_tools(&self, tools: &[ToolDefinition]) -> Option<Vec<ClaudeToolDefinition>> {
        if tools.is_empty() {
            return None;
        }

        Some(
            tools
                .iter()
                .map(|tool| ClaudeToolDefinition {
                    name: tool.name.clone(),
                    description: tool.description.clone(),
                    input_schema: tool.input_schema.clone(),
                })
                .collect(),
        )
    }

    fn build_thinking(&self, reasoning_budget: &ReasoningBudget) -> Option<ClaudeThinking> {
        reasoning_budget
            .get_max_tokens()
            .map(|budget_tokens| ClaudeThinking {
                thinking_type: "enabled".to_string(),
                budget_tokens,
            })
    }

    async fn invoke_cli(
        &self,
        messages: &[ClaudeMessage],
        model: &str,
        system_prompt: &str,
        thinking_budget: Option<ClaudeThinking>,
        tools: Option<Vec<ClaudeToolDefinition>>,
        max_tokens: Option<u32>,
    ) -> Result<(Vec<ContentBlock>, TokenUsage, StopReason), AiError> {
        let mut command = Command::new(&self.command);
        command.arg("chat").arg("--print").arg("--model").arg(model);

        if !system_prompt.is_empty() {
            command.arg("--system-prompt").arg(system_prompt);
        }

        command
            .arg("--output-format")
            .arg("stream-json")
            .arg("--verbose")
            .arg("--max-turns")
            .arg("1")
            .arg("--disallowed-tools")
            .arg("Bash,Edit,Read,WebSearch,Grep,Glob,Task,Write,NotebookEdit,WebFetch,BashOutput,KillShell,Skill,SlashCommand,TodoWrite,ExitPlanMode");

        // Trim shell color codes from CLI output when possible
        command.env("NO_COLOR", "1");

        // It seems like we would want to set this, i don't think we can reuse
        // the cache, but wildly this breaks oauth?
        // command.env("DISABLE_PROMPT_CACHING", "1");

        // Set thinking budget via environment variable
        if let Some(thinking) = thinking_budget.as_ref() {
            command.env("MAX_THINKING_TOKENS", thinking.budget_tokens.to_string());
        }

        for arg in &self.additional_args {
            command.arg(arg);
        }

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
                AiError::Terminal(anyhow::anyhow!(
                    "Failed to spawn Claude Code CLI '{}': {err}",
                    self.command.display()
                ))
            })?;

        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| AiError::Terminal(anyhow::anyhow!("Claude CLI stdin is unavailable")))?;

        // Send complete API request as JSON
        let mut request = serde_json::json!({
            "model": model,
            "messages": messages,
        });

        if let Some(max_tokens) = max_tokens {
            request["max_tokens"] = serde_json::json!(max_tokens);
        }

        if let Some(tools) = tools {
            request["tools"] = serde_json::json!(tools);
        }

        if let Some(thinking) = &thinking_budget {
            request["thinking"] = serde_json::json!(thinking);
        }

        let request_json = serde_json::to_vec(&request).map_err(|err| {
            AiError::Terminal(anyhow::anyhow!("Failed to serialize request: {err}"))
        })?;

        tracing::debug!(
            "Sending to Claude CLI: {}",
            String::from_utf8_lossy(&request_json)
        );

        stdin.write_all(&request_json).await.map_err(|err| {
            AiError::Terminal(anyhow::anyhow!("Failed to write to Claude stdin: {err}"))
        })?;

        stdin.flush().await.map_err(|err| {
            AiError::Terminal(anyhow::anyhow!("Failed to flush Claude stdin: {err}"))
        })?;
        drop(stdin);

        let stdout = child.stdout.take().ok_or_else(|| {
            AiError::Terminal(anyhow::anyhow!("Claude CLI stdout is unavailable"))
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            AiError::Terminal(anyhow::anyhow!("Claude CLI stderr is unavailable"))
        })?;

        let mut stdout_reader = BufReader::new(stdout).lines();
        let stderr_handle: JoinHandle<Result<String, std::io::Error>> = tokio::spawn(async move {
            let mut buf = String::new();
            let mut reader = BufReader::new(stderr);
            reader.read_to_string(&mut buf).await?;
            Ok(buf)
        });

        let mut stream_state = StreamState::default();

        while let Some(line) = stdout_reader.next_line().await.map_err(|err| {
            AiError::Retryable(anyhow::anyhow!("Failed reading Claude CLI stdout: {err}"))
        })? {
            if line.trim().is_empty() {
                continue;
            }

            tracing::debug!("claude_cli_event" = line);

            let value: Value = serde_json::from_str(&line).map_err(|err| {
                AiError::Terminal(anyhow::anyhow!(
                    "Failed to parse Claude CLI output as JSON: {err}. Line: {line}"
                ))
            })?;

            let event = ParsedEvent::from_value(value).map_err(|err| {
                AiError::Terminal(anyhow::anyhow!(
                    "Failed to interpret Claude CLI event: {err}"
                ))
            })?;

            if stream_state.handle_event(event).map_err(|err| {
                AiError::Terminal(anyhow::anyhow!(
                    "Error while processing Claude CLI event: {err}"
                ))
            })? {
                break;
            }
        }

        let status = child.wait().await.map_err(|err| {
            AiError::Retryable(anyhow::anyhow!("Failed waiting for Claude CLI: {err}"))
        })?;

        let stderr_output = match stderr_handle.await {
            Ok(Ok(text)) => text,
            Ok(Err(err)) => {
                tracing::warn!("Failed reading Claude CLI stderr: {err}");
                String::new()
            }
            Err(err) => {
                tracing::warn!("Failed awaiting Claude CLI stderr: {err}");
                String::new()
            }
        };

        if !status.success() {
            let error_message = stream_state
                .error_message
                .as_deref()
                .unwrap_or_else(|| stderr_output.trim());

            if error_message.contains("too long") {
                return Err(AiError::InputTooLong(anyhow::anyhow!(
                    "Claude CLI error: {}",
                    error_message
                )));
            }

            return Err(AiError::Terminal(anyhow::anyhow!(
                "Claude CLI error: {}",
                error_message
            )));
        }

        let (content_blocks, usage, stop_reason) =
            stream_state.finish().map_err(AiError::Terminal)?;
        tracing::info!(?usage, ?stop_reason, "Claude CLI completed");

        Ok((content_blocks, usage, stop_reason))
    }
}

#[async_trait::async_trait]
impl AiProvider for ClaudeCodeProvider {
    fn name(&self) -> &'static str {
        "ClaudeCode"
    }

    fn supported_models(&self) -> HashSet<Model> {
        HashSet::from([
            Model::ClaudeSonnet45,
            Model::ClaudeOpus45,
            Model::ClaudeHaiku45,
        ])
    }

    async fn converse(
        &self,
        request: ConversationRequest,
    ) -> Result<ConversationResponse, AiError> {
        let model_id = self.resolve_model(&request.model.model);
        let messages = self.build_messages(&request.messages)?;
        let system_prompt = self
            .format_system_prompt(&request.system_prompt)
            .unwrap_or_else(|| String::new());
        let thinking_budget = self.build_thinking(&request.model.reasoning_budget);
        let tools = self.build_tools(&request.tools);

        let (content_blocks, usage, stop_reason) = self
            .invoke_cli(
                &messages,
                &model_id,
                &system_prompt,
                thinking_budget,
                tools,
                request.model.max_tokens,
            )
            .await?;

        Ok(ConversationResponse {
            content: Content::from(content_blocks),
            usage,
            stop_reason,
        })
    }

    fn get_cost(&self, model: &Model) -> Cost {
        match model {
            Model::ClaudeOpus45 => Cost::new(5.0, 25.0, 6.25, 0.5),
            Model::ClaudeSonnet45 => Cost::new(3.0, 15.0, 3.75, 0.3),
            Model::ClaudeHaiku45 => Cost::new(1.0, 5.0, 1.25, 0.1),
            _ => Cost::new(0.0, 0.0, 0.0, 0.0),
        }
    }
}

#[derive(Debug, Serialize)]
struct ClaudeMessage {
    role: String,
    content: Vec<ClaudeContentBlock>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum ClaudeContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "thinking")]
    Thinking { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: Vec<ClaudeToolResultContent>,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type")]
enum ClaudeToolResultContent {
    #[serde(rename = "output_text")]
    OutputText { text: String },
}

#[derive(Debug, Serialize)]
struct ClaudeToolDefinition {
    name: String,
    description: String,
    input_schema: Value,
}

#[derive(Debug, Serialize)]
struct ClaudeThinking {
    #[serde(rename = "type")]
    thinking_type: String,
    budget_tokens: u32,
}

#[derive(Default)]
struct StreamState {
    content_blocks: Vec<ContentBlock>,
    pending_blocks: HashMap<usize, PendingBlock>,
    usage: Option<TokenUsage>,
    stop_reason: Option<StopReason>,
    pending_stop_sequence: Option<String>,
    error_message: Option<String>,
}

impl StreamState {
    fn merge_usage(&mut self, new_usage: TokenUsage) {
        if let Some(existing) = &mut self.usage {
            existing.input_tokens += new_usage.input_tokens;
            existing.output_tokens += new_usage.output_tokens;
            existing.total_tokens += new_usage.total_tokens;
            if let Some(new_cached) = new_usage.cached_prompt_tokens {
                existing.cached_prompt_tokens =
                    Some(existing.cached_prompt_tokens.unwrap_or(0) + new_cached);
            }
            if let Some(new_cache_creation) = new_usage.cache_creation_input_tokens {
                existing.cache_creation_input_tokens =
                    Some(existing.cache_creation_input_tokens.unwrap_or(0) + new_cache_creation);
            }
            if let Some(new_reasoning) = new_usage.reasoning_tokens {
                existing.reasoning_tokens =
                    Some(existing.reasoning_tokens.unwrap_or(0) + new_reasoning);
            }
        } else {
            self.usage = Some(new_usage);
        }
    }

    fn handle_event(&mut self, event: ParsedEvent) -> Result<bool, anyhow::Error> {
        match event {
            ParsedEvent::Assistant(data) => {
                // Handle complete assistant message from CLI
                for block in data.message.content {
                    match block {
                        AssistantContentBlock::Text { text } => {
                            if !text.trim().is_empty() {
                                self.content_blocks.push(ContentBlock::Text(text));
                            }
                        }
                        AssistantContentBlock::Thinking { text } => {
                            if !text.trim().is_empty() {
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
                        AssistantContentBlock::ToolUse { id, name, input } => {
                            self.content_blocks.push(ContentBlock::ToolUse(ToolUseData {
                                id,
                                name,
                                arguments: input,
                            }));
                            if self.stop_reason.is_none() {
                                self.stop_reason = Some(StopReason::ToolUse);
                            }
                        }
                    }
                }

                if let Some(usage) = data.message.usage {
                    self.merge_usage(usage.into());
                }

                if let Some(stop_reason) = data.message.stop_reason {
                    self.stop_reason =
                        Some(map_stop_reason(&stop_reason, data.message.stop_sequence));
                }
            }
            ParsedEvent::MessageStart(data) => {
                if let Some(usage) = data.message.and_then(|m| m.usage) {
                    self.merge_usage(usage.into());
                }
            }
            ParsedEvent::MessageDelta(delta) => {
                if let Some(usage) = delta.delta.usage {
                    self.merge_usage(usage.into());
                }
                if let Some(stop_reason) = delta.delta.stop_reason {
                    let stop_sequence = delta.delta.stop_sequence;
                    self.stop_reason = Some(map_stop_reason(&stop_reason, stop_sequence.clone()));
                    self.pending_stop_sequence = stop_sequence;
                }
            }
            ParsedEvent::MessageStop(data) => {
                if let Some(usage) = data
                    .usage
                    .or_else(|| data.message.as_ref().and_then(|m| m.usage.clone()))
                {
                    self.merge_usage(usage.into());
                }

                if let Some(reason) = data
                    .stop_reason
                    .or_else(|| data.message.as_ref().and_then(|m| m.stop_reason.clone()))
                {
                    self.stop_reason = Some(map_stop_reason(
                        &reason,
                        data.stop_sequence.or_else(|| {
                            data.message.as_ref().and_then(|m| m.stop_sequence.clone())
                        }),
                    ));
                }

                return Ok(true);
            }
            ParsedEvent::ContentBlockStart(start) => {
                let index = start.index;
                match start.content_block.r#type.as_str() {
                    "text" => {
                        self.pending_blocks
                            .insert(index, PendingBlock::Text(String::new()));
                    }
                    "thinking" | "reasoning" => {
                        self.pending_blocks
                            .insert(index, PendingBlock::Thinking(String::new()));
                    }
                    "tool_use" => {
                        let id = start
                            .content_block
                            .id
                            .clone()
                            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
                        let name = start
                            .content_block
                            .name
                            .clone()
                            .unwrap_or_else(|| "unknown_tool".to_string());

                        // Don't pre-populate buffer from input field as deltas will provide the JSON
                        let buffer = String::new();

                        self.pending_blocks
                            .insert(index, PendingBlock::ToolUse { id, name, buffer });
                    }
                    other => {
                        tracing::warn!("Unsupported content block type from Claude CLI: {other}");
                    }
                }
            }
            ParsedEvent::ContentBlockDelta(delta) => {
                if let Some(entry) = self.pending_blocks.get_mut(&delta.index) {
                    match (entry, delta.delta) {
                        (PendingBlock::Text(buffer), BlockDelta::TextDelta { text }) => {
                            buffer.push_str(&text);
                        }
                        (PendingBlock::Thinking(buffer), BlockDelta::ThinkingDelta { text }) => {
                            buffer.push_str(&text);
                        }
                        (
                            PendingBlock::ToolUse { buffer, .. },
                            BlockDelta::InputJsonDelta { partial_json },
                        ) => {
                            buffer.push_str(&partial_json);
                        }
                        (entry, BlockDelta::Other(value)) => {
                            tracing::warn!("Unhandled content_block_delta: {:?}", value);
                            if let PendingBlock::ToolUse { buffer, .. } = entry {
                                if let Ok(fragment) = serde_json::to_string(&value) {
                                    buffer.push_str(&fragment);
                                }
                            }
                        }
                        _ => {
                            tracing::warn!("Ignoring unsupported delta for block");
                        }
                    }
                }
            }
            ParsedEvent::ContentBlockStop(stop) => {
                if let Some(entry) = self.pending_blocks.remove(&stop.index) {
                    match entry {
                        PendingBlock::Text(buffer) => {
                            if !buffer.trim().is_empty() {
                                self.content_blocks
                                    .push(ContentBlock::Text(buffer.trim().to_string()));
                            }
                        }
                        PendingBlock::Thinking(buffer) => {
                            if !buffer.trim().is_empty() {
                                self.content_blocks.push(ContentBlock::ReasoningContent(
                                    ReasoningData {
                                        text: buffer.trim().to_string(),
                                        signature: None,
                                        blob: None,
                                        raw_json: None,
                                    },
                                ));
                            }
                        }
                        PendingBlock::ToolUse { id, name, buffer } => {
                            let arguments = if buffer.trim().is_empty() {
                                Value::Null
                            } else {
                                serde_json::from_str(&buffer).with_context(|| {
                                    format!("Failed to parse tool_use input JSON: {buffer}")
                                })?
                            };
                            self.content_blocks.push(ContentBlock::ToolUse(ToolUseData {
                                id,
                                name,
                                arguments,
                            }));
                            if self.stop_reason.is_none() {
                                self.stop_reason = Some(StopReason::ToolUse);
                            }
                        }
                    }
                }
            }
            ParsedEvent::Result(result) => {
                if let Some(usage) = result.usage {
                    self.merge_usage(usage.into());
                }
                if result.is_error.unwrap_or(false) {
                    if let Some(error_msg) = result.result {
                        self.error_message = Some(error_msg);
                    }
                }
                return Ok(true);
            }
            ParsedEvent::Error(error) => {
                return Err(anyhow::anyhow!(
                    "Claude CLI reported error: {}",
                    error.message
                ));
            }
            ParsedEvent::Ping => {}
        }

        Ok(false)
    }

    fn finish(mut self) -> Result<(Vec<ContentBlock>, TokenUsage, StopReason), anyhow::Error> {
        if !self.pending_blocks.is_empty() {
            for (_, entry) in self.pending_blocks.drain() {
                match entry {
                    PendingBlock::Text(buffer) if !buffer.is_empty() => {
                        self.content_blocks
                            .push(ContentBlock::Text(buffer.trim().to_string()));
                    }
                    PendingBlock::Thinking(buffer) if !buffer.is_empty() => {
                        self.content_blocks
                            .push(ContentBlock::ReasoningContent(ReasoningData {
                                text: buffer.trim().to_string(),
                                signature: None,
                                blob: None,
                                raw_json: None,
                            }));
                    }
                    PendingBlock::ToolUse { .. } => {
                        // Tool use blocks should always terminate properly; log if they do not
                        tracing::warn!("Incomplete tool_use block from Claude CLI");
                    }
                    _ => {}
                }
            }
        }

        let usage = self.usage.unwrap_or_else(TokenUsage::empty);
        let stop_reason = self.stop_reason.unwrap_or_else(|| {
            if let Some(stop_sequence) = self.pending_stop_sequence {
                StopReason::StopSequence(stop_sequence)
            } else {
                StopReason::EndTurn
            }
        });

        Ok((self.content_blocks, usage, stop_reason))
    }
}

#[derive(Debug)]
enum ParsedEvent {
    MessageStart(MessageStartEvent),
    MessageDelta(MessageDeltaEvent),
    MessageStop(MessageStopEvent),
    ContentBlockStart(ContentBlockStartEvent),
    ContentBlockDelta(ContentBlockDeltaEvent),
    ContentBlockStop(ContentBlockStopEvent),
    Assistant(AssistantMessageEvent),
    Result(ResultEvent),
    Error(ClaudeErrorEvent),
    Ping,
}

impl ParsedEvent {
    fn from_value(value: Value) -> Result<Self, anyhow::Error> {
        if let Some(event_type) = value.get("type").and_then(|v| v.as_str()) {
            match event_type {
                "event" => {
                    let event_name = value
                        .get("event")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let data = value.get("data").cloned().unwrap_or(Value::Null);
                    ParsedEvent::from_event_name(&event_name, data)
                }
                other => ParsedEvent::from_event_name(other, value.clone()),
            }
        } else {
            Err(anyhow::anyhow!("Claude CLI event missing 'type' field"))
        }
    }

    fn from_event_name(event: &str, data: Value) -> Result<Self, anyhow::Error> {
        match event {
            "message_start" => Ok(ParsedEvent::MessageStart(serde_json::from_value(data)?)),
            "message_delta" => Ok(ParsedEvent::MessageDelta(serde_json::from_value(data)?)),
            "message_stop" => Ok(ParsedEvent::MessageStop(serde_json::from_value(data)?)),
            "content_block_start" => Ok(ParsedEvent::ContentBlockStart(serde_json::from_value(
                data,
            )?)),
            "content_block_delta" => Ok(ParsedEvent::ContentBlockDelta(serde_json::from_value(
                data,
            )?)),
            "content_block_stop" => {
                Ok(ParsedEvent::ContentBlockStop(serde_json::from_value(data)?))
            }
            "assistant" => Ok(ParsedEvent::Assistant(serde_json::from_value(data)?)),
            "result" => Ok(ParsedEvent::Result(serde_json::from_value(data)?)),
            "error" => Ok(ParsedEvent::Error(serde_json::from_value(data)?)),
            "ping" => Ok(ParsedEvent::Ping),
            "system" => Ok(ParsedEvent::Ping), // Ignore system init messages
            other => {
                tracing::warn!("Unhandled Claude CLI event type: {other}");
                Ok(ParsedEvent::Ping)
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct MessageStartEvent {
    message: Option<MessageStartData>,
}

#[derive(Debug, Deserialize)]
struct MessageStartData {
    #[serde(default)]
    usage: Option<ClaudeUsage>,
}

#[derive(Debug, Deserialize)]
struct MessageDeltaEvent {
    delta: MessageDeltaData,
}

#[derive(Debug, Deserialize)]
struct MessageDeltaData {
    #[serde(default)]
    usage: Option<ClaudeUsage>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    stop_sequence: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MessageStopEvent {
    #[serde(default)]
    message: Option<MessageStopData>,
    #[serde(default)]
    usage: Option<ClaudeUsage>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    stop_sequence: Option<String>,
}

#[derive(Debug, Deserialize)]
struct MessageStopData {
    #[serde(default)]
    usage: Option<ClaudeUsage>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    stop_sequence: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AssistantMessageEvent {
    message: AssistantMessage,
}

#[derive(Debug, Deserialize)]
struct AssistantMessage {
    content: Vec<AssistantContentBlock>,
    #[serde(default)]
    usage: Option<ClaudeUsage>,
    #[serde(default)]
    stop_reason: Option<String>,
    #[serde(default)]
    stop_sequence: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum AssistantContentBlock {
    #[serde(rename = "text")]
    Text {
        #[serde(default)]
        text: String,
    },
    #[serde(rename = "thinking")]
    Thinking {
        #[serde(default, rename = "thinking")]
        text: String,
    },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
}

#[derive(Debug, Deserialize)]
struct ContentBlockStartEvent {
    index: usize,
    content_block: ContentBlockPayload,
}

#[derive(Debug, Deserialize, Clone)]
struct ContentBlockPayload {
    #[serde(rename = "type")]
    r#type: String,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ContentBlockDeltaEvent {
    index: usize,
    delta: BlockDelta,
}

#[derive(Debug)]
enum BlockDelta {
    TextDelta { text: String },
    ThinkingDelta { text: String },
    InputJsonDelta { partial_json: String },
    Other(Value),
}

impl<'de> Deserialize<'de> for BlockDelta {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        let delta_type = value.get("type").and_then(Value::as_str).unwrap_or("");

        match delta_type {
            "text_delta" => {
                let text = value
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                Ok(BlockDelta::TextDelta { text })
            }
            "thinking_delta" | "reasoning_delta" => {
                let text = value
                    .get("text")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                Ok(BlockDelta::ThinkingDelta { text })
            }
            "input_json_delta" | "output_json_delta" => {
                let partial_json = value
                    .get("partial_json")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                Ok(BlockDelta::InputJsonDelta { partial_json })
            }
            _ => Ok(BlockDelta::Other(value)),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ContentBlockStopEvent {
    index: usize,
}

#[derive(Debug, Deserialize)]
struct ResultEvent {
    #[serde(default)]
    usage: Option<ClaudeUsage>,
    #[serde(default)]
    is_error: Option<bool>,
    #[serde(default)]
    result: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ClaudeErrorEvent {
    message: String,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct ClaudeUsage {
    #[serde(default)]
    input_tokens: u32,
    #[serde(default)]
    output_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: Option<u32>,
    #[serde(default)]
    cache_creation_input_tokens: Option<u32>,
    #[serde(default, alias = "thinking_tokens")]
    reasoning_tokens: Option<u32>,
}

impl From<ClaudeUsage> for TokenUsage {
    fn from(usage: ClaudeUsage) -> Self {
        let reasoning = usage.reasoning_tokens;
        TokenUsage {
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            total_tokens: usage.input_tokens + usage.output_tokens + reasoning.unwrap_or(0),
            cached_prompt_tokens: usage.cache_read_input_tokens,
            cache_creation_input_tokens: usage.cache_creation_input_tokens,
            reasoning_tokens: reasoning,
        }
    }
}

enum PendingBlock {
    Text(String),
    Thinking(String),
    ToolUse {
        id: String,
        name: String,
        buffer: String,
    },
}

fn map_stop_reason(reason: &str, stop_sequence: Option<String>) -> StopReason {
    match reason {
        "end_turn" => StopReason::EndTurn,
        "max_tokens" => StopReason::MaxTokens,
        "stop_sequence" => {
            let seq = stop_sequence.unwrap_or_else(|| "".to_string());
            StopReason::StopSequence(seq)
        }
        "tool_use" => StopReason::ToolUse,
        other => {
            tracing::warn!("Unknown Claude stop_reason: {other}");
            StopReason::EndTurn
        }
    }
}

#[cfg(test)]
mod tests {
    use tracing::debug;

    use super::*;
    use crate::ai::tests::{
        test_hello_world, test_reasoning_conversation, test_reasoning_with_tools, test_tool_usage,
    };
    use std::collections::HashMap;
    use std::path::PathBuf;

    async fn create_claude_provider() -> anyhow::Result<ClaudeCodeProvider> {
        Ok(ClaudeCodeProvider::new(
            PathBuf::from("claude"),
            Vec::new(),
            HashMap::new(),
        ))
    }

    #[tokio::test]
    #[ignore = "requires local Claude CLI"]
    async fn test_claude_hello_world() {
        let provider = match create_claude_provider().await {
            Ok(provider) => provider,
            Err(e) => {
                debug!(?e, "Failed to create Claude Code provider");
                panic!("Failed to create Claude Code provider: {e:?}");
            }
        };

        if let Err(e) = test_hello_world(provider.clone()).await {
            debug!(?e, "Claude Code hello world test failed");
            panic!("Claude Code hello world test failed: {e:?}");
        }
    }

    #[tokio::test]
    #[ignore = "requires local Claude CLI"]
    async fn test_claude_reasoning_conversation() {
        let provider = match create_claude_provider().await {
            Ok(provider) => provider,
            Err(e) => {
                debug!(?e, "Failed to create Claude Code provider");
                panic!("Failed to create Claude Code provider: {e:?}");
            }
        };

        if let Err(e) = test_reasoning_conversation(provider.clone()).await {
            debug!(?e, "Claude Code reasoning conversation test failed");
            panic!("Claude Code reasoning conversation test failed: {e:?}");
        }
    }

    #[tokio::test]
    #[ignore = "requires local Claude CLI"]
    async fn test_claude_tool_usage() {
        let provider = match create_claude_provider().await {
            Ok(provider) => provider,
            Err(e) => {
                debug!(?e, "Failed to create Claude Code provider");
                panic!("Failed to create Claude Code provider: {e:?}");
            }
        };

        if let Err(e) = test_tool_usage(provider.clone()).await {
            debug!(?e, "Claude Code tool usage test failed");
            panic!("Claude Code tool usage test failed: {e:?}");
        }
    }

    #[tokio::test]
    #[ignore = "requires local Claude CLI"]
    async fn test_claude_reasoning_with_tools() {
        let provider = match create_claude_provider().await {
            Ok(provider) => provider,
            Err(e) => {
                debug!(?e, "Failed to create Claude Code provider");
                panic!("Failed to create Claude Code provider: {e:?}");
            }
        };

        if let Err(e) = test_reasoning_with_tools(provider).await {
            debug!(?e, "Claude Code reasoning with tools test failed");
            panic!("Claude Code reasoning with tools test failed: {e:?}");
        }
    }
}
