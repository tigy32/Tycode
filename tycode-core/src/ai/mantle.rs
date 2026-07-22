//! OpenAI Responses API client for the Amazon Bedrock Mantle endpoint.
//!
//! Mantle is Bedrock's inference engine for OpenAI and xAI models (GPT-5.5,
//! GPT-5.6, Grok 4.3). These models are not served by the Converse API or the
//! bedrock-runtime endpoint at all; they are only reachable through
//! `https://bedrock-mantle.{region}.api.aws/openai/v1` using an OpenAI-style
//! Responses API, authenticated with a Bedrock API key as a Bearer token.
//!
//! No stored API key is required: a short-term bearer token is minted per
//! request from the provider's AWS credentials, using the same presigned-URL
//! scheme as AWS's official aws-bedrock-token-generator libraries.
//!
//! Requests are sent with `store: false` so Bedrock retains nothing and the
//! full conversation is replayed each turn, matching how the rest of tycode
//! treats providers as stateless. Reasoning round-trips via
//! `include: ["reasoning.encrypted_content"]`: the complete reasoning output
//! item is preserved in `ReasoningData::raw_json` and replayed verbatim on
//! subsequent turns.

use std::pin::Pin;
use std::time::{Duration, SystemTime};

use anyhow::anyhow;
use aws_credential_types::provider::{ProvideCredentials, SharedCredentialsProvider};
use aws_credential_types::Credentials;
use aws_sigv4::http_request::{
    sign, SignableBody, SignableRequest, SignatureLocation, SigningSettings,
};
use aws_sigv4::sign::v4;
use aws_smithy_runtime_api::client::identity::Identity;
use base64::Engine;
use futures_util::StreamExt;
use percent_encoding::{utf8_percent_encode, AsciiSet, NON_ALPHANUMERIC};
use reqwest::Client;
use serde_json::{json, Value};
use tokio_stream::Stream;
use tracing::{debug, warn};

use crate::ai::{error::AiError, types::*};

#[derive(Clone)]
pub struct MantleClient {
    client: Client,
    region: String,
    credentials: SharedCredentialsProvider,
    base_url: String,
}

impl MantleClient {
    pub fn new(region: &str, credentials: SharedCredentialsProvider) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            region: region.to_string(),
            credentials,
            base_url: format!("https://bedrock-mantle.{region}.api.aws/openai/v1"),
        }
    }

    async fn bearer_token(&self) -> Result<String, AiError> {
        let credentials = self.credentials.provide_credentials().await.map_err(|e| {
            AiError::Terminal(anyhow!(
                "Failed to resolve AWS credentials for bedrock-mantle: {e}"
            ))
        })?;
        generate_bearer_token(credentials, &self.region, SystemTime::now())
    }

    pub async fn converse(
        &self,
        model_id: &str,
        request: &ConversationRequest,
    ) -> Result<ConversationResponse, AiError> {
        let body = build_request_body(model_id, request, false)?;
        let token = self.bearer_token().await?;
        debug!(?model_id, "Using Bedrock Mantle Responses API");

        let response = self
            .client
            .post(format!("{}/responses", self.base_url))
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AiError::Retryable(anyhow!("Network error: {e}")))?;

        let status = response.status();
        let response_text = response
            .text()
            .await
            .map_err(|e| AiError::Retryable(anyhow!("Failed to read response: {e}")))?;

        if !status.is_success() {
            return Err(map_http_error(status.as_u16(), &response_text));
        }

        let response_json: Value = serde_json::from_str(&response_text).map_err(|e| {
            AiError::Terminal(anyhow!(
                "Failed to parse Mantle response: {e} - Response: {response_text}"
            ))
        })?;

        parse_response(&response_json)
    }

    pub async fn converse_stream(
        &self,
        model_id: &str,
        request: &ConversationRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, AiError>> + Send>>, AiError> {
        let body = build_request_body(model_id, request, true)?;
        let token = self.bearer_token().await?;
        debug!(?model_id, "Using Bedrock Mantle Responses API (streaming)");

        let response = self
            .client
            .post(format!("{}/responses", self.base_url))
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| AiError::Retryable(anyhow!("Network error: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            let response_text = response
                .text()
                .await
                .map_err(|e| AiError::Retryable(anyhow!("Failed to read response: {e}")))?;
            return Err(map_http_error(status.as_u16(), &response_text));
        }

        let byte_stream = response.bytes_stream();

        let stream = async_stream::stream! {
            let mut line_buffer = String::new();
            let mut completed = false;

            futures_util::pin_mut!(byte_stream);

            while let Some(chunk_result) = byte_stream.next().await {
                let Ok(chunk) = chunk_result else {
                    yield Err(AiError::Retryable(anyhow!("Stream read error")));
                    return;
                };
                line_buffer.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(newline_pos) = line_buffer.find('\n') {
                    let line = line_buffer[..newline_pos].trim().to_string();
                    line_buffer.drain(..=newline_pos);
                    match process_sse_line(&line) {
                        Ok(events) => {
                            for event in events {
                                if matches!(event, StreamEvent::MessageComplete { .. }) {
                                    completed = true;
                                }
                                yield Ok(event);
                            }
                        }
                        Err(e) => {
                            yield Err(e);
                            return;
                        }
                    }
                }
            }

            if !completed {
                yield Err(AiError::Retryable(anyhow!(
                    "Mantle stream ended without a response.completed event"
                )));
            }
        };

        Ok(Box::pin(stream))
    }
}

const TOKEN_HOST: &str = "bedrock.amazonaws.com";
const TOKEN_ACTION: &str = "Action=CallWithBearerToken";
/// Tokens only need to be valid when the request is authenticated; a short
/// lifetime keeps the leak window small while absorbing clock skew.
const TOKEN_DURATION: Duration = Duration::from_secs(900);

/// SigV4 query values percent-encode everything except the unreserved
/// characters A-Z a-z 0-9 - . _ ~
const SIGV4_QUERY_SET: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Mint a short-term Bedrock API key from AWS credentials.
///
/// Mirrors AWS's aws-bedrock-token-generator: SigV4-presign
/// `POST https://bedrock.amazonaws.com/?Action=CallWithBearerToken` with the
/// signature in the query string, strip the scheme, append `&Version=1`, and
/// base64-encode behind the `bedrock-api-key-` prefix.
fn generate_bearer_token(
    credentials: Credentials,
    region: &str,
    now: SystemTime,
) -> Result<String, AiError> {
    let mut settings = SigningSettings::default();
    settings.signature_location = SignatureLocation::QueryParams;
    settings.expires_in = Some(TOKEN_DURATION);

    let identity = Identity::from(credentials);
    let params = v4::SigningParams::builder()
        .identity(&identity)
        .region(region)
        .name("bedrock")
        .time(now)
        .settings(settings)
        .build()
        .map_err(|e| AiError::Terminal(anyhow!("Failed to build signing params: {e}")))?;

    let request = SignableRequest::new(
        "POST",
        format!("https://{TOKEN_HOST}/?{TOKEN_ACTION}"),
        std::iter::once(("host", TOKEN_HOST)),
        SignableBody::Bytes(&[]),
    )
    .map_err(|e| AiError::Terminal(anyhow!("Failed to build signable request: {e}")))?;

    let (instructions, _signature) = sign(request, &params.into())
        .map_err(|e| AiError::Terminal(anyhow!("Failed to sign bearer token request: {e}")))?
        .into_parts();

    let mut presigned = format!("{TOKEN_HOST}/?{TOKEN_ACTION}");
    for (name, value) in instructions.params() {
        presigned.push('&');
        presigned.push_str(&utf8_percent_encode(name, SIGV4_QUERY_SET).to_string());
        presigned.push('=');
        presigned.push_str(&utf8_percent_encode(value, SIGV4_QUERY_SET).to_string());
    }
    presigned.push_str("&Version=1");

    Ok(format!(
        "bedrock-api-key-{}",
        base64::engine::general_purpose::STANDARD.encode(presigned)
    ))
}

fn build_request_body(
    model_id: &str,
    request: &ConversationRequest,
    stream: bool,
) -> Result<Value, AiError> {
    let input = build_input_items(&request.messages)?;

    let mut body = json!({
        "model": model_id,
        "input": input,
        "store": false,
        "include": ["reasoning.encrypted_content"],
        "stream": stream,
    });

    if !request.system_prompt.trim().is_empty() {
        body["instructions"] = json!(request.system_prompt);
    }

    if let Some(max_tokens) = request.model.max_tokens {
        body["max_output_tokens"] = json!(max_tokens);
    }

    if !request.tools.is_empty() {
        body["tools"] = Value::Array(
            request
                .tools
                .iter()
                .map(|tool| {
                    json!({
                        "type": "function",
                        "name": tool.name,
                        "description": tool.description,
                        "parameters": tool.input_schema,
                        "strict": false,
                    })
                })
                .collect(),
        );
        body["tool_choice"] = json!("auto");
    }

    // Max maps to "high": Grok 4.3 tops out at high, and unlike OpenRouter
    // there is no normalization layer to downgrade an unsupported "xhigh".
    let effort = match request.model.reasoning_budget {
        ReasoningBudget::Off => None,
        ReasoningBudget::Low => Some("low"),
        ReasoningBudget::Medium => Some("medium"),
        ReasoningBudget::High | ReasoningBudget::Max => Some("high"),
    };
    if let Some(effort) = effort {
        body["reasoning"] = json!({ "effort": effort });
    }

    // GPT reasoning models reject sampling parameters; Grok accepts them.
    if model_id.starts_with("xai.") {
        if let Some(temperature) = request.model.temperature {
            body["temperature"] = json!(temperature);
        }
        if let Some(top_p) = request.model.top_p {
            body["top_p"] = json!(top_p);
        }
    }

    Ok(body)
}

fn build_input_items(messages: &[Message]) -> Result<Vec<Value>, AiError> {
    let mut items = Vec::new();

    for message in messages {
        match message.role {
            MessageRole::User => {
                for tool_result in message.content.tool_results() {
                    items.push(json!({
                        "type": "function_call_output",
                        "call_id": tool_result.tool_use_id,
                        "output": tool_result.content,
                    }));
                }

                let mut parts = Vec::new();
                let text: Vec<String> = message
                    .content
                    .blocks()
                    .iter()
                    .filter_map(|block| match block {
                        ContentBlock::Text(text) if !text.trim().is_empty() => {
                            Some(text.trim().to_string())
                        }
                        _ => None,
                    })
                    .collect();
                if !text.is_empty() {
                    parts.push(json!({ "type": "input_text", "text": text.join("\n") }));
                }
                for image in message.content.images() {
                    parts.push(json!({
                        "type": "input_image",
                        "image_url": format!("data:{};base64,{}", image.media_type, image.data),
                    }));
                }
                if !parts.is_empty() {
                    items.push(json!({ "type": "message", "role": "user", "content": parts }));
                }
            }
            MessageRole::Assistant => {
                // Blocks replay in original order so reasoning items keep
                // preceding the function calls they were emitted with.
                for block in message.content.blocks() {
                    match block {
                        ContentBlock::Text(text) => {
                            if !text.trim().is_empty() {
                                items.push(json!({
                                    "type": "message",
                                    "role": "assistant",
                                    "content": [{ "type": "output_text", "text": text.trim() }],
                                }));
                            }
                        }
                        ContentBlock::ReasoningContent(reasoning) => match &reasoning.raw_json {
                            Some(raw)
                                if raw.get("type").and_then(|t| t.as_str())
                                    == Some("reasoning") =>
                            {
                                items.push(raw.clone());
                            }
                            _ => {
                                warn!("Reasoning block without a Mantle reasoning item; dropping. This can happen when switching providers mid conversation");
                            }
                        },
                        ContentBlock::ToolUse(tool_use) => {
                            let arguments =
                                serde_json::to_string(&tool_use.arguments).map_err(|e| {
                                    AiError::Terminal(anyhow!(
                                        "Failed to serialize tool arguments: {e}"
                                    ))
                                })?;
                            items.push(json!({
                                "type": "function_call",
                                "call_id": tool_use.id,
                                "name": tool_use.name,
                                "arguments": arguments,
                            }));
                        }
                        ContentBlock::ToolResult(_) | ContentBlock::Image(_) => {}
                    }
                }
            }
        }
    }

    Ok(items)
}

fn parse_response(response: &Value) -> Result<ConversationResponse, AiError> {
    let output = response
        .get("output")
        .and_then(|v| v.as_array())
        .ok_or_else(|| AiError::Terminal(anyhow!("Mantle response has no output array")))?;

    let mut blocks = Vec::new();
    let mut has_tool_use = false;

    for item in output {
        match item.get("type").and_then(|v| v.as_str()).unwrap_or("") {
            "message" => {
                for part in item
                    .get("content")
                    .and_then(|v| v.as_array())
                    .into_iter()
                    .flatten()
                {
                    if part.get("type").and_then(|v| v.as_str()) == Some("output_text") {
                        if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                            if !text.trim().is_empty() {
                                blocks.push(ContentBlock::Text(text.trim().to_string()));
                            }
                        }
                    }
                }
            }
            "function_call" => {
                has_tool_use = true;
                let call_id = item
                    .get("call_id")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        AiError::Terminal(anyhow!("Mantle function_call missing call_id: {item}"))
                    })?
                    .to_string();
                let name = item
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| {
                        AiError::Terminal(anyhow!("Mantle function_call missing name: {item}"))
                    })?
                    .to_string();
                let arguments_str = item
                    .get("arguments")
                    .and_then(|v| v.as_str())
                    .unwrap_or("{}");
                let arguments = serde_json::from_str(arguments_str).unwrap_or(Value::Null);
                blocks.push(ContentBlock::ToolUse(ToolUseData {
                    id: call_id,
                    name,
                    arguments,
                }));
            }
            "reasoning" => {
                let mut text_parts = Vec::new();
                for part in item
                    .get("summary")
                    .and_then(|v| v.as_array())
                    .into_iter()
                    .flatten()
                {
                    if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                        if !text.trim().is_empty() {
                            text_parts.push(text.trim().to_string());
                        }
                    }
                }
                for part in item
                    .get("content")
                    .and_then(|v| v.as_array())
                    .into_iter()
                    .flatten()
                {
                    if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                        if !text.trim().is_empty() {
                            text_parts.push(text.trim().to_string());
                        }
                    }
                }
                blocks.push(ContentBlock::ReasoningContent(ReasoningData {
                    text: text_parts.join("\n"),
                    signature: None,
                    blob: None,
                    raw_json: Some(item.clone()),
                }));
            }
            _ => {}
        }
    }

    let usage = parse_usage(response.get("usage"));

    let stop_reason = if has_tool_use {
        StopReason::ToolUse
    } else if response
        .pointer("/incomplete_details/reason")
        .and_then(|v| v.as_str())
        == Some("max_output_tokens")
    {
        StopReason::MaxTokens
    } else {
        StopReason::EndTurn
    };

    Ok(ConversationResponse {
        content: Content::from(blocks),
        usage,
        stop_reason,
    })
}

fn parse_usage(usage: Option<&Value>) -> TokenUsage {
    let Some(usage) = usage else {
        return TokenUsage::empty();
    };

    let get = |pointer: &str| {
        usage
            .pointer(pointer)
            .and_then(|v| v.as_u64())
            .map(|v| v as u32)
    };

    let input_tokens = get("/input_tokens").unwrap_or(0);
    let output_tokens = get("/output_tokens").unwrap_or(0);
    TokenUsage {
        input_tokens,
        output_tokens,
        total_tokens: get("/total_tokens").unwrap_or(input_tokens + output_tokens),
        cached_prompt_tokens: get("/input_tokens_details/cached_tokens"),
        cache_creation_input_tokens: None,
        reasoning_tokens: get("/output_tokens_details/reasoning_tokens"),
    }
}

fn process_sse_line(line: &str) -> Result<Vec<StreamEvent>, AiError> {
    // `event:` lines, comments, and blanks carry no payload; the data JSON
    // repeats the event type in its own "type" field.
    let Some(data) = line.strip_prefix("data: ") else {
        return Ok(vec![]);
    };
    let data = data.trim();
    if data == "[DONE]" {
        return Ok(vec![]);
    }

    let value: Value = match serde_json::from_str(data) {
        Ok(v) => v,
        Err(e) => {
            warn!("Failed to parse Mantle SSE chunk: {e:?}");
            return Ok(vec![]);
        }
    };

    match value.get("type").and_then(|v| v.as_str()).unwrap_or("") {
        "response.output_text.delta" => {
            let Some(delta) = value.get("delta").and_then(|v| v.as_str()) else {
                return Ok(vec![]);
            };
            if delta.is_empty() {
                return Ok(vec![]);
            }
            Ok(vec![StreamEvent::TextDelta {
                text: delta.to_string(),
            }])
        }
        "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
            let Some(delta) = value.get("delta").and_then(|v| v.as_str()) else {
                return Ok(vec![]);
            };
            if delta.is_empty() {
                return Ok(vec![]);
            }
            Ok(vec![StreamEvent::ReasoningDelta {
                text: delta.to_string(),
            }])
        }
        "response.completed" | "response.incomplete" => {
            let response = value.get("response").ok_or_else(|| {
                AiError::Terminal(anyhow!("Mantle completion event missing response object"))
            })?;
            Ok(vec![StreamEvent::MessageComplete {
                response: parse_response(response)?,
            }])
        }
        "response.failed" => {
            let message = value
                .pointer("/response/error/message")
                .and_then(|v| v.as_str())
                .unwrap_or("Mantle response failed");
            Err(AiError::Retryable(anyhow!("{message}")))
        }
        "error" => {
            let message = value
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("Mantle stream error");
            Err(AiError::Terminal(anyhow!("{message}")))
        }
        _ => Ok(vec![]),
    }
}

fn map_http_error(status: u16, body: &str) -> AiError {
    let body_lower = body.to_lowercase();

    let is_input_too_long = status == 413
        || ["context window", "too long", "exceeds the maximum"]
            .iter()
            .any(|keyword| body_lower.contains(keyword));
    if is_input_too_long {
        return AiError::InputTooLong(anyhow!("Mantle API error {status}: {body}"));
    }

    match status {
        401 | 403 => AiError::Terminal(anyhow!(
            "Mantle API error {status}: {body}. The bedrock-mantle endpoint requires a valid Bedrock API key."
        )),
        429 => AiError::Retryable(anyhow!("Mantle API error {status}: {body}")),
        500..=599 => AiError::Retryable(anyhow!("Mantle API error {status}: {body}")),
        _ => AiError::Terminal(anyhow!("Mantle API error {status}: {body}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::model::Model;

    fn request_with(messages: Vec<Message>, tools: Vec<ToolDefinition>) -> ConversationRequest {
        ConversationRequest {
            messages,
            model: ModelSettings {
                model: Model::Gpt,
                max_tokens: Some(32000),
                temperature: Some(1.0),
                top_p: None,
                reasoning_budget: ReasoningBudget::High,
            },
            system_prompt: "You are a test agent.".to_string(),
            stop_sequences: vec![],
            tools,
        }
    }

    #[test]
    fn bearer_token_encodes_presigned_call_with_bearer_token_url() {
        let credentials = Credentials::new(
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            None,
            None,
            "test",
        );
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_750_000_000);

        let token = generate_bearer_token(credentials.clone(), "us-west-2", now).unwrap();

        let encoded = token.strip_prefix("bedrock-api-key-").unwrap();
        let decoded = String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .unwrap(),
        )
        .unwrap();

        assert!(
            decoded.starts_with("bedrock.amazonaws.com/?Action=CallWithBearerToken&"),
            "unexpected presigned url: {decoded}"
        );
        assert!(decoded.contains("X-Amz-Algorithm=AWS4-HMAC-SHA256"));
        // Credential scope slashes must be percent-encoded.
        assert!(decoded.contains("X-Amz-Credential=AKIDEXAMPLE%2F"));
        assert!(decoded.contains("%2Fus-west-2%2Fbedrock%2Faws4_request"));
        assert!(decoded.contains("X-Amz-Expires=900"));
        assert!(decoded.contains("X-Amz-SignedHeaders=host"));
        assert!(decoded.contains("X-Amz-Signature="));
        assert!(decoded.ends_with("&Version=1"));

        // Same credentials and time produce the same token.
        let again = generate_bearer_token(credentials, "us-west-2", now).unwrap();
        assert_eq!(token, again);
    }

    #[test]
    fn bearer_token_includes_session_token_for_temporary_credentials() {
        let credentials = Credentials::new(
            "AKIDEXAMPLE",
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            Some("session/token+with=reserved".to_string()),
            None,
            "test",
        );
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_750_000_000);

        let token = generate_bearer_token(credentials, "us-west-2", now).unwrap();
        let decoded = String::from_utf8(
            base64::engine::general_purpose::STANDARD
                .decode(token.strip_prefix("bedrock-api-key-").unwrap())
                .unwrap(),
        )
        .unwrap();

        assert!(decoded.contains("X-Amz-Security-Token=session%2Ftoken%2Bwith%3Dreserved"));
    }

    #[test]
    fn request_body_replays_conversation_as_input_items() {
        let reasoning_item = json!({
            "type": "reasoning",
            "id": "rs_1",
            "summary": [],
            "encrypted_content": "opaque",
        });
        let messages = vec![
            Message::user(Content::text_only("Run the tests".to_string())),
            Message::assistant(Content::new(vec![
                ContentBlock::ReasoningContent(ReasoningData {
                    text: String::new(),
                    signature: None,
                    blob: None,
                    raw_json: Some(reasoning_item.clone()),
                }),
                ContentBlock::ToolUse(ToolUseData {
                    id: "call_1".to_string(),
                    name: "bash".to_string(),
                    arguments: json!({"command": "cargo test"}),
                }),
            ])),
            Message::user(Content::new(vec![ContentBlock::ToolResult(
                ToolResultData {
                    tool_use_id: "call_1".to_string(),
                    content: "ok".to_string(),
                    is_error: false,
                },
            )])),
        ];

        let body =
            build_request_body("openai.gpt-5.5", &request_with(messages, vec![]), false).unwrap();

        assert_eq!(body["model"], "openai.gpt-5.5");
        assert_eq!(body["store"], false);
        assert_eq!(body["instructions"], "You are a test agent.");
        assert_eq!(body["include"][0], "reasoning.encrypted_content");
        assert_eq!(body["reasoning"]["effort"], "high");
        // GPT models reject sampling parameters.
        assert!(body.get("temperature").is_none());

        let input = body["input"].as_array().unwrap();
        assert_eq!(input[0]["type"], "message");
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[1], reasoning_item);
        assert_eq!(input[2]["type"], "function_call");
        assert_eq!(input[2]["call_id"], "call_1");
        assert_eq!(input[2]["arguments"], "{\"command\":\"cargo test\"}");
        assert_eq!(input[3]["type"], "function_call_output");
        assert_eq!(input[3]["call_id"], "call_1");
        assert_eq!(input[3]["output"], "ok");
    }

    #[test]
    fn request_body_includes_sampling_for_xai_and_flattened_tools() {
        let messages = vec![Message::user(Content::text_only("hi".to_string()))];
        let tools = vec![ToolDefinition {
            name: "bash".to_string(),
            description: "Run a command".to_string(),
            input_schema: json!({"type": "object"}),
        }];

        let body =
            build_request_body("xai.grok-4.3", &request_with(messages, tools), true).unwrap();

        assert_eq!(body["stream"], true);
        assert_eq!(body["temperature"], 1.0);
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tools"][0]["name"], "bash");
        assert_eq!(body["tool_choice"], "auto");
    }

    #[test]
    fn parses_response_output_items() {
        let response = json!({
            "output": [
                {
                    "type": "reasoning",
                    "id": "rs_1",
                    "summary": [{"type": "summary_text", "text": "thinking"}],
                    "encrypted_content": "opaque",
                },
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "Running tests"}],
                },
                {
                    "type": "function_call",
                    "call_id": "call_9",
                    "name": "bash",
                    "arguments": "{\"command\":\"ls\"}",
                },
            ],
            "usage": {
                "input_tokens": 100,
                "input_tokens_details": {"cached_tokens": 40},
                "output_tokens": 20,
                "output_tokens_details": {"reasoning_tokens": 5},
                "total_tokens": 120,
            },
            "status": "completed",
        });

        let parsed = parse_response(&response).unwrap();

        assert!(matches!(parsed.stop_reason, StopReason::ToolUse));
        assert_eq!(parsed.usage.input_tokens, 100);
        assert_eq!(parsed.usage.cached_prompt_tokens, Some(40));
        assert_eq!(parsed.usage.reasoning_tokens, Some(5));

        let reasoning = parsed.content.reasoning();
        assert_eq!(reasoning.len(), 1);
        assert_eq!(reasoning[0].text, "thinking");
        assert_eq!(
            reasoning[0].raw_json.as_ref().unwrap()["encrypted_content"],
            "opaque"
        );

        assert_eq!(parsed.content.text(), "Running tests");

        let tool_uses = parsed.content.tool_uses();
        assert_eq!(tool_uses.len(), 1);
        assert_eq!(tool_uses[0].id, "call_9");
        assert_eq!(tool_uses[0].arguments["command"], "ls");
    }

    #[test]
    fn sse_lines_map_to_stream_events() {
        let text_delta =
            process_sse_line(r#"data: {"type":"response.output_text.delta","delta":"Hello"}"#)
                .unwrap();
        assert!(matches!(&text_delta[..], [StreamEvent::TextDelta { text }] if text == "Hello"));

        let reasoning_delta = process_sse_line(
            r#"data: {"type":"response.reasoning_summary_text.delta","delta":"hmm"}"#,
        )
        .unwrap();
        assert!(matches!(
            &reasoning_delta[..],
            [StreamEvent::ReasoningDelta { text }] if text == "hmm"
        ));

        assert!(process_sse_line("event: response.output_text.delta")
            .unwrap()
            .is_empty());
        assert!(process_sse_line("").unwrap().is_empty());

        let completed = process_sse_line(
            r#"data: {"type":"response.completed","response":{"output":[{"type":"message","content":[{"type":"output_text","text":"done"}]}],"usage":{"input_tokens":1,"output_tokens":2,"total_tokens":3}}}"#,
        )
        .unwrap();
        let [StreamEvent::MessageComplete { response }] = &completed[..] else {
            panic!("expected MessageComplete");
        };
        assert_eq!(response.content.text(), "done");
        assert_eq!(response.usage.total_tokens, 3);
    }
}
