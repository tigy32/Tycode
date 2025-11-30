use anyhow::Result;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::time::Duration;
use tracing::{debug, info};

use crate::ai::model::Model;
use crate::ai::{error::AiError, provider::AiProvider, types::*};

#[derive(Clone)]
pub struct OpenRouterProvider {
    client: Client,
    api_key: String,
    base_url: String,
}

impl OpenRouterProvider {
    pub fn new(api_key: String) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            client,
            api_key,
            base_url: "https://openrouter.ai/api/v1".to_string(),
        }
    }

    fn get_openrouter_model_id(&self, model: &Model) -> Result<String, AiError> {
        let model_id = match model {
            Model::ClaudeSonnet45 => "anthropic/claude-sonnet-4.5",
            Model::ClaudeOpus45 => "anthropic/claude-opus-4.5",
            Model::ClaudeHaiku45 => "anthropic/claude-haiku-4.5",

            Model::Gemini25Pro => "google/gemini-2.5-pro",
            Model::Gemini25Flash => "google/gemini-2.5-flash",

            Model::Grok4Fast => "x-ai/grok-4-fast",
            Model::GrokCodeFast1 => "x-ai/grok-code-fast-1",

            Model::Gpt5Codex => "openai/gpt-5-codex",
            Model::Gpt5 => "openai/gpt-5",
            Model::GptOss120b => "openai/gpt-oss-120b",

            Model::Qwen3Coder => "qwen/qwen3-coder",
            Model::GLM46 => "z-ai/glm-4.6",
            _ => {
                return Err(AiError::Terminal(anyhow::anyhow!(
                    "Model {} is not supported in OpenRouter",
                    model.name()
                )));
            }
        };
        Ok(model_id.to_string())
    }

    fn convert_to_openrouter_messages(
        &self,
        messages: &[Message],
        system_prompt: &str,
        model: Model,
    ) -> Result<Vec<OpenRouterMessage>, AiError> {
        let mut openrouter_messages = Vec::new();

        // Add system message first
        if !system_prompt.trim().is_empty() {
            let cache_control = if model.supports_prompt_caching() {
                Some(CacheControl::ephemeral())
            } else {
                None
            };

            let content = MessageContent::Array(vec![ContentPart {
                r#type: "text".to_string(),
                text: system_prompt.to_string(),
                cache_control,
            }]);

            openrouter_messages.push(OpenRouterMessage {
                role: "system".to_string(),
                content: Some(content),
                name: None,
                tool_call_id: None,
                tool_calls: None,
                reasoning_details: None,
            });
        }

        for msg in messages.iter() {
            openrouter_messages.extend(message_to_openrouter(msg)?);
        }

        if model.supports_prompt_caching() {
            let user_indices: Vec<usize> = openrouter_messages
                .iter()
                .enumerate()
                .filter(|(_, m)| m.role == "user")
                .map(|(i, _)| i)
                .collect();

            for &idx in user_indices.iter().rev().take(2) {
                apply_cache_control_to_message(&mut openrouter_messages, idx);
            }
        }

        Ok(openrouter_messages)
    }
}

#[async_trait::async_trait]
impl AiProvider for OpenRouterProvider {
    fn name(&self) -> &'static str {
        "OpenRouter"
    }

    fn supported_models(&self) -> HashSet<Model> {
        HashSet::from([
            Model::ClaudeSonnet45,
            Model::ClaudeOpus45,
            Model::ClaudeHaiku45,
            Model::Gemini25Pro,
            Model::GptOss120b,
            Model::GrokCodeFast1,
            Model::Qwen3Coder,
            Model::GLM46,
            Model::Gemini25Flash,
            Model::Grok4Fast,
            Model::Gpt5Codex,
            Model::Gpt5,
        ])
    }

    async fn converse(
        &self,
        request: ConversationRequest,
    ) -> Result<ConversationResponse, AiError> {
        let model_id = self.get_openrouter_model_id(&request.model.model)?;
        let messages = self.convert_to_openrouter_messages(
            &request.messages,
            &request.system_prompt,
            request.model.model,
        )?;

        debug!(?model_id, "Using OpenRouter API");

        let openrouter_request = OpenRouterRequest {
            model: model_id,
            messages,
            max_tokens: request.model.max_tokens,
            temperature: request.model.temperature,
            top_p: request.model.top_p,
            stop: if !request.stop_sequences.is_empty() {
                Some(request.stop_sequences.clone())
            } else {
                None
            },
            stream: Some(false),
            tools: if !request.tools.is_empty() {
                Some(convert_tools_to_openrouter(
                    &request.tools,
                    request.model.model,
                ))
            } else {
                None
            },
            tool_choice: if !request.tools.is_empty() {
                Some(ToolChoice::Simple("auto".to_string()))
            } else {
                None
            },
            reasoning: match request.model.reasoning_budget {
                ReasoningBudget::Off => None,
                _ => Some(ReasoningConfig {
                    effort: Some(match request.model.reasoning_budget {
                        ReasoningBudget::Low => ReasoningEffort::Low,
                        ReasoningBudget::High => ReasoningEffort::High,
                        ReasoningBudget::Off => unreachable!(),
                    }),
                }),
            },
            usage: Some(UsageConfig { include: true }),
        };

        let request_json =
            serde_json::to_string(&openrouter_request).expect("OpenRouterRequest should serialize");
        info!(request_json = %request_json, "Full OpenRouter request");

        let response = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .header("HTTP-Referer", "https://tycode.dev")
            .header("X-Title", "TyCode")
            .json(&openrouter_request)
            .send()
            .await
            .map_err(|e| {
                debug!(?e, "OpenRouter API call failed");
                AiError::Retryable(anyhow::anyhow!("Network error: {}", e))
            })?;

        tracing::info!("Response: {response:?}");

        let status = response.status();
        let response_text = response
            .text()
            .await
            .map_err(|e| AiError::Retryable(anyhow::anyhow!("Failed to read response: {}", e)))?;

        if !status.is_success() {
            debug!(?status, ?response_text, "OpenRouter API returned error");

            let error_text_lower = response_text.to_lowercase();
            let is_input_too_long = status.as_u16() == 413
                || ["too long"]
                    .iter()
                    .any(|keyword| error_text_lower.contains(keyword));

            if is_input_too_long {
                return Err(AiError::InputTooLong(anyhow::anyhow!(
                    "OpenRouter API error {}: {}",
                    status,
                    response_text
                )));
            }

            let is_transient = status.as_u16() == 429
                && (error_text_lower.contains("provider returned error")
                    || error_text_lower.contains("rate-limited upstream"));

            if is_transient {
                return Err(AiError::Transient(anyhow::anyhow!(
                    "OpenRouter API error {}: {}",
                    status,
                    response_text
                )));
            }

            return Err(AiError::Terminal(anyhow::anyhow!(
                "OpenRouter API error {}: {}",
                status,
                response_text
            )));
        }

        let openrouter_response: OpenRouterResponse = serde_json::from_str(&response_text)
            .map_err(|e| {
                AiError::Terminal(anyhow::anyhow!(
                    "Failed to parse OpenRouter response: {} - Response: {}",
                    e,
                    response_text
                ))
            })?;

        let choice = openrouter_response
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| AiError::Terminal(anyhow::anyhow!("No choices in response")))?;

        let usage = if let Some(usage) = openrouter_response.usage {
            TokenUsage {
                input_tokens: usage.prompt_tokens,
                output_tokens: usage.completion_tokens,
                total_tokens: usage.total_tokens,
                cached_prompt_tokens: usage.prompt_details.map(|d| d.cached_tokens),
                reasoning_tokens: usage.completion_details.map(|d| d.reasoning_tokens),
                cache_creation_input_tokens: None,
            }
        } else {
            TokenUsage::empty()
        };

        let stop_reason = match choice.finish_reason.as_deref() {
            Some("stop") => StopReason::EndTurn,
            Some("length") => StopReason::MaxTokens,
            Some("tool_calls") => StopReason::ToolUse,
            Some("content_filter") => StopReason::EndTurn,
            Some("error") => StopReason::EndTurn,
            _ => StopReason::EndTurn,
        };

        let content = extract_content_from_response(&choice.message)?;

        Ok(ConversationResponse {
            content,
            usage,
            stop_reason,
        })
    }

    fn get_cost(&self, model: &Model) -> Cost {
        match model {
            Model::ClaudeOpus45 => Cost::new(5.0, 25.0, 6.25, 0.5),
            Model::ClaudeSonnet45 => Cost::new(3.0, 15.0, 3.75, 0.3),
            Model::ClaudeHaiku45 => Cost::new(1.0, 5.0, 1.25, 0.1),
            Model::Gemini25Pro => Cost::new(1.25, 10.0, 0.0, 0.0),
            Model::GptOss120b => Cost::new(0.1, 0.5, 0.0, 0.0),
            Model::GrokCodeFast1 => Cost::new(0.2, 1.5, 0.0, 0.0),
            Model::Qwen3Coder => Cost::new(0.35, 1.5, 0.0, 0.0),
            Model::GLM46 => Cost::new(0.60, 2.20, 0.0, 0.0),
            Model::Gemini25Flash => Cost::new(0.3, 2.5, 0.0, 0.0),
            Model::Grok4Fast => Cost::new(0.2, 0.5, 0.0, 0.0),
            Model::Gpt5Codex => Cost::new(1.25, 10.0, 0.0, 0.0),
            Model::Gpt5 => Cost::new(1.25, 10.0, 0.0, 0.0),
            _ => Cost::new(0.0, 0.0, 0.0, 0.0),
        }
    }
}

// OpenRouter API types

#[derive(Debug, Serialize, Deserialize, Clone)]
struct CacheControl {
    r#type: String,
}

impl CacheControl {
    fn ephemeral() -> Self {
        Self {
            r#type: "ephemeral".to_string(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ContentPart {
    r#type: String,
    text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_control: Option<CacheControl>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
enum MessageContent {
    String(String),
    Array(Vec<ContentPart>),
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct OpenRouterRequest {
    pub model: String,
    pub messages: Vec<OpenRouterMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<OpenRouterTool>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<ToolChoice>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ReasoningConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<UsageConfig>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct OpenRouterMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<MessageContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OpenRouterToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_details: Option<Vec<ReasoningDetail>>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct OpenRouterTool {
    pub r#type: String,
    pub function: FunctionObject,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<CacheControl>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct FunctionObject {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parameters: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub strict: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
enum ToolChoice {
    Simple(String),
    Function(ToolChoiceFunction),
}

#[derive(Debug, Serialize, Deserialize)]
struct ToolChoiceFunction {
    pub r#type: String,
    pub function: FunctionName,
}

#[derive(Debug, Serialize, Deserialize)]
struct FunctionName {
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ReasoningConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<ReasoningEffort>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum ReasoningEffort {
    Low,
    Medium,
    High,
}

#[derive(Debug, Serialize, Deserialize)]
struct UsageConfig {
    pub include: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenRouterResponse {
    pub id: String,
    pub choices: Vec<OpenRouterChoice>,
    pub created: u64,
    pub model: String,
    pub object: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<OpenRouterUsage>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenRouterChoice {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_finish_reason: Option<String>,
    pub message: OpenRouterMessageResponse,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenRouterMessageResponse {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<OpenRouterToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_details: Option<Vec<ReasoningDetail>>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenRouterToolCall {
    pub id: String,
    pub r#type: String,
    pub function: FunctionCall,
}

#[derive(Debug, Serialize, Deserialize)]
struct FunctionCall {
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenRouterUsage {
    #[serde(rename = "prompt_tokens")]
    pub prompt_tokens: u32,
    #[serde(rename = "completion_tokens")]
    pub completion_tokens: u32,
    #[serde(rename = "total_tokens")]
    pub total_tokens: u32,
    #[serde(rename = "prompt_tokens_details")]
    pub prompt_details: Option<OpenRouterPromptDetails>,
    #[serde(rename = "completion_tokens_details")]
    pub completion_details: Option<OpenRouterCompletionDetails>,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenRouterPromptDetails {
    #[serde(rename = "cached_tokens")]
    pub cached_tokens: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct OpenRouterCompletionDetails {
    #[serde(rename = "reasoning_tokens")]
    pub reasoning_tokens: u32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum ReasoningDetail {
    #[serde(rename = "reasoning.summary")]
    Summary {
        summary: String,
        id: Option<String>,
        format: String,
        index: Option<u32>,
    },
    #[serde(rename = "reasoning.text")]
    Text {
        text: String,
        signature: Option<String>,
        id: Option<String>,
        format: String,
        index: Option<u32>,
    },
    #[serde(rename = "reasoning.encrypted")]
    Encrypted {
        data: String,
        id: String,
        format: String,
        index: Option<u32>,
    },
}

fn apply_cache_control_to_message(messages: &mut [OpenRouterMessage], idx: usize) {
    let Some(msg) = messages.get_mut(idx) else {
        return;
    };
    let Some(MessageContent::Array(parts)) = &mut msg.content else {
        return;
    };
    let Some(last_text) = parts.iter_mut().rev().find(|p| p.r#type == "text") else {
        return;
    };
    last_text.cache_control = Some(CacheControl::ephemeral());
}

fn create_message_content(content: String) -> MessageContent {
    MessageContent::Array(vec![ContentPart {
        r#type: "text".to_string(),
        text: content,
        cache_control: None,
    }])
}

fn create_tool_result_message(tool_result: &ToolResultData) -> OpenRouterMessage {
    OpenRouterMessage {
        role: "tool".to_string(),
        content: Some(MessageContent::Array(vec![ContentPart {
            r#type: "text".to_string(),
            text: tool_result.content.trim().to_string(),
            cache_control: None,
        }])),
        name: None,
        tool_call_id: Some(tool_result.tool_use_id.clone()),
        tool_calls: None,
        reasoning_details: None,
    }
}

fn create_user_text_message(message_content: MessageContent) -> OpenRouterMessage {
    OpenRouterMessage {
        role: "user".to_string(),
        content: Some(message_content),
        name: None,
        tool_call_id: None,
        tool_calls: None,
        reasoning_details: None,
    }
}

fn process_user_message(message: &Message) -> Result<Vec<OpenRouterMessage>, AiError> {
    let mut results = vec![];

    for tool_result in message.content.tool_results() {
        results.push(create_tool_result_message(tool_result));
    }

    let content = extract_text_content(&message.content);
    if content.is_empty() {
        return Ok(results);
    }

    let message_content = create_message_content(content);
    results.push(create_user_text_message(message_content));
    Ok(results)
}

fn message_to_openrouter(message: &Message) -> Result<Vec<OpenRouterMessage>, AiError> {
    match message.role {
        MessageRole::User => process_user_message(message),
        MessageRole::Assistant => process_assistant_message(message),
    }
}

fn convert_tool_use_to_openrouter(tool_use: &ToolUseData) -> Result<OpenRouterToolCall, AiError> {
    let arguments = serde_json::to_string(&tool_use.arguments).map_err(|e| {
        AiError::Terminal(anyhow::anyhow!("Failed to serialize tool arguments: {}", e))
    })?;

    Ok(OpenRouterToolCall {
        id: tool_use.id.clone(),
        r#type: "function".to_string(),
        function: FunctionCall {
            name: tool_use.name.clone(),
            arguments,
        },
    })
}

fn process_assistant_message(message: &Message) -> Result<Vec<OpenRouterMessage>, AiError> {
    let mut content_parts = Vec::new();
    let mut reasoning_details: Option<Vec<ReasoningDetail>> = None;
    let mut tool_calls = Vec::new();

    for block in message.content.blocks() {
        match block {
            ContentBlock::Text(text) => {
                if !text.trim().is_empty() {
                    content_parts.push(text.trim().to_string());
                }
            }
            ContentBlock::ReasoningContent(reason) => {
                if let Some(raw_json) = &reason.raw_json {
                    reasoning_details = serde_json::from_value(raw_json.clone()).ok();
                } else {
                    tracing::warn!(?reason, "No raw json found in reasoning. This count happen if switching providers mid conversation");
                }
            }
            ContentBlock::ToolUse(tool_use) => {
                tool_calls.push(convert_tool_use_to_openrouter(tool_use)?);
            }
            ContentBlock::ToolResult(_) => continue,
        }
    }

    let content_text = if content_parts.is_empty() {
        "<no response>".to_string()
    } else {
        content_parts.join("\n")
    };
    let content = create_message_content(content_text);

    let message = OpenRouterMessage {
        role: "assistant".to_string(),
        content: Some(content),
        name: None,
        tool_call_id: None,
        tool_calls: if tool_calls.is_empty() {
            None
        } else {
            Some(tool_calls)
        },
        reasoning_details,
    };

    Ok(vec![message])
}

fn extract_text_content(content: &Content) -> String {
    let mut text_parts = Vec::new();

    for block in content.blocks() {
        match block {
            ContentBlock::Text(text) => {
                if !text.trim().is_empty() {
                    text_parts.push(text.trim().to_string());
                }
            }
            ContentBlock::ReasoningContent(reasoning) => {
                // Include reasoning as text for user messages
                if !reasoning.text.trim().is_empty() {
                    text_parts.push(format!("[Reasoning: {}]", reasoning.text.trim()));
                }
            }
            ContentBlock::ToolUse(_) | ContentBlock::ToolResult(_) => {
                continue;
            }
        }
    }

    text_parts.join("\n")
}

fn convert_tools_to_openrouter(tools: &[ToolDefinition], model: Model) -> Vec<OpenRouterTool> {
    let mut result: Vec<OpenRouterTool> = tools
        .iter()
        .map(|tool| OpenRouterTool {
            r#type: "function".to_string(),
            function: FunctionObject {
                name: tool.name.clone(),
                description: Some(tool.description.clone()),
                parameters: Some(tool.input_schema.clone()),
                strict: Some(true),
            },
            cache_control: None,
        })
        .collect();

    if model.supports_prompt_caching() {
        if let Some(last) = result.last_mut() {
            last.cache_control = Some(CacheControl::ephemeral());
        }
    }

    result
}

fn extract_content_from_response(message: &OpenRouterMessageResponse) -> Result<Content, AiError> {
    let mut content_blocks = Vec::new();

    if let Some(content) = &message.content {
        if !content.trim().is_empty() {
            content_blocks.push(ContentBlock::Text(content.trim().to_string()));
        }
    }

    if let Some(tool_calls) = &message.tool_calls {
        for tool_call in tool_calls {
            if let Ok(arguments) = serde_json::from_str::<Value>(&tool_call.function.arguments) {
                let tool_use_data = ToolUseData {
                    id: tool_call.id.clone(),
                    name: tool_call.function.name.clone(),
                    arguments,
                };
                content_blocks.push(ContentBlock::ToolUse(tool_use_data));
            } else {
                return Err(AiError::Terminal(anyhow::anyhow!(
                    "Failed to parse tool call arguments: {}",
                    tool_call.function.arguments
                )));
            }
        }
    }

    if let Some(reasoning_details) = &message.reasoning_details {
        let raw_json = serde_json::to_value(&reasoning_details)?;

        let mut text_parts = Vec::new();
        let mut signature: Option<String> = None;

        for detail in reasoning_details {
            match detail {
                ReasoningDetail::Text {
                    text,
                    signature: sig,
                    ..
                } => {
                    text_parts.push(text.clone());
                    if signature.is_none() {
                        signature = sig.clone();
                    }
                }
                ReasoningDetail::Summary { summary, .. } => {
                    text_parts.push(summary.clone());
                }
                ReasoningDetail::Encrypted { .. } => {}
            }
        }

        if !text_parts.is_empty() {
            content_blocks.push(ContentBlock::ReasoningContent(ReasoningData {
                text: text_parts.join("\n"),
                signature,
                blob: None,
                raw_json: Some(raw_json),
            }));
        }
    }

    Ok(Content::from(content_blocks))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::tests::{
        test_hello_world, test_reasoning_conversation, test_reasoning_with_tools, test_tool_usage,
    };

    async fn create_openrouter_provider() -> anyhow::Result<OpenRouterProvider> {
        let api_key = "";
        Ok(OpenRouterProvider::new(api_key.to_string()))
    }

    #[tokio::test]
    #[ignore = "requires OpenRouter API key"]
    async fn test_openrouter_hello_world() {
        let provider = match create_openrouter_provider().await {
            Ok(provider) => provider,
            Err(e) => {
                debug!(?e, "Failed to create OpenRouter provider");
                panic!("Failed to create OpenRouter provider: {e:?}");
            }
        };

        if let Err(e) = test_hello_world(provider).await {
            debug!(?e, "OpenRouter hello world test failed");
            panic!("OpenRouter hello world test failed: {e:?}");
        }
    }

    #[tokio::test]
    #[ignore = "requires OpenRouter API key"]
    async fn test_openrouter_reasoning_conversation() {
        let provider = match create_openrouter_provider().await {
            Ok(provider) => provider,
            Err(e) => {
                debug!(?e, "Failed to create OpenRouter provider");
                panic!("Failed to create OpenRouter provider: {e:?}");
            }
        };

        if let Err(e) = test_reasoning_conversation(provider).await {
            debug!(?e, "OpenRouter reasoning conversation test failed");
            panic!("OpenRouter reasoning conversation test failed: {e:?}");
        }
    }

    #[tokio::test]
    #[ignore = "requires OpenRouter API key"]
    async fn test_openrouter_tool_usage() {
        let provider = match create_openrouter_provider().await {
            Ok(provider) => provider,
            Err(e) => {
                debug!(?e, "Failed to create OpenRouter provider");
                panic!("Failed to create OpenRouter provider: {e:?}");
            }
        };

        if let Err(e) = test_tool_usage(provider).await {
            debug!(?e, "OpenRouter tool usage test failed");
            panic!("OpenRouter tool usage test failed: {e:?}");
        }
    }

    #[tokio::test]
    #[ignore = "requires OpenRouter API key"]
    async fn test_openrouter_reasoning_with_tools() {
        let provider = match create_openrouter_provider().await {
            Ok(provider) => provider,
            Err(e) => {
                debug!(?e, "Failed to create OpenRouter provider");
                panic!("Failed to create OpenRouter provider: {e:?}");
            }
        };

        if let Err(e) = test_reasoning_with_tools(provider).await {
            debug!(?e, "OpenRouter reasoning with tools test failed");
            panic!("OpenRouter reasoning with tools test failed: {e:?}");
        }
    }
}
