use anyhow::Result;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::time::Duration;
use tokio_stream::Stream;
use tracing::{debug, info};

use crate::ai::model::Model;
use crate::ai::{error::AiError, provider::AiProvider, types::*};

#[derive(Clone)]
pub struct OpenRouterProvider {
    client: Client,
    api_key: String,
    base_url: String,
    models: HashMap<Model, OpenRouterModel>,
}

#[derive(Clone, Debug)]
struct OpenRouterModel {
    id: String,
    created: u64,
    context_window: u32,
    cost: Cost,
}

#[derive(Debug, Deserialize)]
struct OpenRouterCatalog {
    data: Vec<OpenRouterCatalogModel>,
}

#[derive(Clone, Debug, Deserialize)]
struct OpenRouterCatalogModel {
    id: String,
    #[serde(default)]
    created: u64,
    context_length: u32,
    pricing: OpenRouterPricing,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct OpenRouterPricing {
    prompt: String,
    completion: String,
    #[serde(default)]
    input_cache_write: Option<String>,
    #[serde(default)]
    input_cache_read: Option<String>,
}

impl OpenRouterCatalogModel {
    fn into_resolved(self, model: Model) -> OpenRouterModel {
        fn per_million(value: Option<&str>) -> Option<f64> {
            value
                .and_then(|value| value.parse::<f64>().ok())
                .filter(|value| *value >= 0.0)
                .map(|value| value * 1_000_000.0)
        }

        let fallback = match model {
            // OpenRouter reports -1 because Auto's price depends on the model
            // it routes to; retain the existing estimate for cost selection.
            Model::OpenRouterAuto => Cost::new(3.0, 15.0, 3.75, 0.3),
            _ => Cost::new(0.0, 0.0, 0.0, 0.0),
        };

        OpenRouterModel {
            id: self.id,
            created: self.created,
            context_window: self.context_length,
            cost: Cost::new(
                per_million(Some(&self.pricing.prompt))
                    .unwrap_or(fallback.input_cost_per_million_tokens),
                per_million(Some(&self.pricing.completion))
                    .unwrap_or(fallback.output_cost_per_million_tokens),
                per_million(self.pricing.input_cache_write.as_deref())
                    .unwrap_or(fallback.cache_write_cost_per_million_tokens),
                per_million(self.pricing.input_cache_read.as_deref())
                    .unwrap_or(fallback.cache_read_cost_per_million_tokens),
            ),
        }
    }
}

fn numeric_version(value: &str) -> bool {
    !value.is_empty()
        && value.chars().any(|character| character.is_ascii_digit())
        && value
            .chars()
            .all(|character| character.is_ascii_digit() || character == '.')
}

fn display_version(model_id: &str) -> String {
    model_id
        .rsplit('/')
        .next()
        .unwrap_or(model_id)
        .replace(":free", "-free")
}

fn classify_openrouter_model(id: &str) -> Option<Model> {
    let (author, name) = id.split_once('/')?;

    match author {
        "anthropic" => {
            if name.starts_with("claude-fable-") {
                Some(Model::ClaudeFable)
            } else if name.starts_with("claude-opus-") && name.ends_with("-fast") {
                Some(Model::ClaudeOpusFast)
            } else if name.starts_with("claude-opus-") {
                Some(Model::ClaudeOpus)
            } else if name.starts_with("claude-sonnet-") {
                Some(Model::ClaudeSonnet)
            } else if name.starts_with("claude-haiku-") {
                Some(Model::ClaudeHaiku)
            } else {
                None
            }
        }
        "google" if name.starts_with("gemini-") && !name.contains("image") => {
            if name.ends_with("-pro") || name.ends_with("-pro-preview") {
                Some(Model::GeminiPro)
            } else if name.ends_with("-flash-lite") || name.ends_with("-flash-lite-preview") {
                Some(Model::GeminiFlashLite)
            } else if name.ends_with("-flash") || name.ends_with("-flash-preview") {
                Some(Model::GeminiFlash)
            } else {
                None
            }
        }
        "openai" => classify_openai_model(name),
        "deepseek" => {
            let free = name.ends_with(":free");
            let name = name.strip_suffix(":free").unwrap_or(name);
            if name.starts_with("deepseek-v") && name.ends_with("-pro") {
                Some(Model::DeepSeekPro)
            } else if name.starts_with("deepseek-v") && name.ends_with("-flash") {
                Some(if free {
                    Model::DeepSeekFlashFree
                } else {
                    Model::DeepSeekFlash
                })
            } else {
                None
            }
        }
        "moonshotai" => name
            .strip_prefix("kimi-k")
            .filter(|version| numeric_version(version))
            .map(|_| Model::Kimi),
        "minimax" => name
            .strip_prefix("minimax-m")
            .filter(|version| numeric_version(version))
            .map(|_| Model::Minimax),
        "x-ai" => {
            if let Some(version) = name.strip_prefix("grok-build-") {
                numeric_version(version).then_some(Model::GrokBuild)
            } else {
                name.strip_prefix("grok-")
                    .filter(|version| numeric_version(version))
                    .map(|_| Model::Grok)
            }
        }
        "qwen" => {
            if name.contains("coder") && !name.contains("instruct") {
                Some(Model::QwenCoder)
            } else if name.contains("-max") && !name.contains("thinking") {
                Some(Model::QwenMax)
            } else if name.contains("-plus") && !name.contains("thinking") {
                Some(Model::QwenPlus)
            } else if name.contains("-flash") && !name.contains("thinking") {
                Some(Model::QwenFlash)
            } else {
                None
            }
        }
        "z-ai" => name
            .strip_prefix("glm-")
            .filter(|version| numeric_version(version))
            .map(|_| Model::GLM),
        "inclusionai" if name.starts_with("ring-") => Some(Model::Ring),
        "stepfun" if name.starts_with("step-") && name.ends_with("-flash") => {
            Some(Model::StepFlash)
        }
        "openrouter" if name == "auto" => Some(Model::OpenRouterAuto),
        _ => None,
    }
}

fn classify_openai_model(name: &str) -> Option<Model> {
    if name == "gpt-oss-120b:free" {
        return Some(Model::GptOss120bFree);
    }
    if name == "gpt-oss-120b" {
        return Some(Model::GptOss120b);
    }

    let version = name.strip_prefix("gpt-")?;
    for (suffix, model) in [
        ("-codex-max", Model::GptCodexMax),
        ("-codex", Model::GptCodex),
        ("-sol", Model::GptSol),
        ("-terra", Model::GptTerra),
        ("-luna", Model::GptLuna),
        ("-pro", Model::GptPro),
        ("-mini", Model::GptMini),
    ] {
        if let Some(version) = version.strip_suffix(suffix) {
            return numeric_version(version).then_some(model);
        }
    }
    numeric_version(version).then_some(Model::Gpt)
}

impl OpenRouterProvider {
    pub async fn new(api_key: String) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(300))
            .build()
            .expect("Failed to create HTTP client");

        let mut provider = Self {
            client,
            api_key,
            base_url: "https://openrouter.ai/api/v1".to_string(),
            models: HashMap::new(),
        };
        provider.models = provider.discover_models().await?;
        Ok(provider)
    }

    async fn discover_models(&self) -> Result<HashMap<Model, OpenRouterModel>> {
        let response = self
            .client
            .get(format!("{}/models", self.base_url))
            .bearer_auth(&self.api_key)
            .send()
            .await?
            .error_for_status()?;
        let catalog: OpenRouterCatalog = response.json().await?;
        Ok(Self::resolve_catalog(catalog.data))
    }

    fn resolve_catalog(
        catalog: impl IntoIterator<Item = OpenRouterCatalogModel>,
    ) -> HashMap<Model, OpenRouterModel> {
        let mut resolved = HashMap::new();
        for entry in catalog {
            let Some(model) = classify_openrouter_model(&entry.id) else {
                continue;
            };
            let candidate = entry.into_resolved(model);
            let replace = resolved
                .get(&model)
                .map(|current: &OpenRouterModel| {
                    (candidate.created, &candidate.id) > (current.created, &current.id)
                })
                .unwrap_or(true);
            if replace {
                resolved.insert(model, candidate);
            }
        }
        resolved
    }

    #[cfg(test)]
    fn from_catalog(catalog: Vec<OpenRouterCatalogModel>) -> Self {
        Self {
            client: Client::new(),
            api_key: String::new(),
            base_url: "https://openrouter.ai/api/v1".to_string(),
            models: Self::resolve_catalog(catalog),
        }
    }

    fn get_openrouter_model_id(&self, model: &Model) -> Result<String, AiError> {
        self.models
            .get(model)
            .map(|resolved| resolved.id.clone())
            .ok_or_else(|| {
                AiError::Terminal(anyhow::anyhow!(
                    "Model {} is not available in the OpenRouter catalog",
                    model.name()
                ))
            })
    }

    fn convert_to_openrouter_messages(
        &self,
        messages: &[Message],
        system_prompt: &str,
        model: Model,
    ) -> Result<Vec<OpenRouterMessage>, AiError> {
        let mut openrouter_messages = Vec::new();

        if !system_prompt.trim().is_empty() {
            let cache_control = if model.supports_prompt_caching() {
                Some(CacheControl::ephemeral())
            } else {
                None
            };

            let content = MessageContent::Array(vec![ContentPart::Text {
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
        self.models.keys().copied().collect()
    }

    fn supports_image_generation(&self) -> bool {
        true
    }

    async fn generate_image(
        &self,
        request: ImageGenerationRequest,
    ) -> Result<ImageGenerationResponse, AiError> {
        use base64::Engine;

        let url = format!("{}/chat/completions", self.base_url);

        let mut body = serde_json::json!({
            "model": request.model_id,
            "messages": [{
                "role": "user",
                "content": request.prompt
            }],
            "modalities": ["image", "text"],
            "stream": false
        });

        let mut image_config = serde_json::Map::new();
        if let Some(ratio) = &request.aspect_ratio {
            image_config.insert("aspect_ratio".to_string(), serde_json::json!(ratio));
        }
        if let Some(size) = &request.image_size {
            image_config.insert("image_size".to_string(), serde_json::json!(size));
        }
        if !image_config.is_empty() {
            body["image_config"] = serde_json::Value::Object(image_config);
        }

        let response = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                AiError::Transient(anyhow::anyhow!("Image generation request failed: {e:?}"))
            })?;

        let status = response.status();
        let response_text = response.text().await.map_err(|e| {
            AiError::Transient(anyhow::anyhow!("Failed to read image response: {e:?}"))
        })?;

        if !status.is_success() {
            return Err(AiError::Terminal(anyhow::anyhow!(
                "Image generation failed with status {status}: {response_text}"
            )));
        }

        let response_json: serde_json::Value =
            serde_json::from_str(&response_text).map_err(|e| {
                AiError::Terminal(anyhow::anyhow!("Failed to parse image response: {e:?}"))
            })?;

        let data_url = response_json
            .pointer("/choices/0/message/images/0/image_url/url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                AiError::Terminal(anyhow::anyhow!(
                    "No image found in response: {response_text}"
                ))
            })?;

        let (media_type, base64_data) = parse_data_url(data_url).ok_or_else(|| {
            AiError::Terminal(anyhow::anyhow!("Invalid data URL format in image response"))
        })?;

        let image_data = base64::engine::general_purpose::STANDARD
            .decode(base64_data)
            .map_err(|e| {
                AiError::Terminal(anyhow::anyhow!("Failed to decode base64 image: {e:?}"))
            })?;

        Ok(ImageGenerationResponse {
            image_data,
            media_type,
        })
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
                        ReasoningBudget::Medium => ReasoningEffort::Medium,
                        ReasoningBudget::High => ReasoningEffort::High,
                        ReasoningBudget::Max => ReasoningEffort::XHigh,
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

    async fn converse_stream(
        &self,
        request: ConversationRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamEvent, AiError>> + Send>>, AiError> {
        let model_id = self.get_openrouter_model_id(&request.model.model)?;
        let messages = self.convert_to_openrouter_messages(
            &request.messages,
            &request.system_prompt,
            request.model.model,
        )?;

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
            stream: Some(true),
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
                        ReasoningBudget::Medium => ReasoningEffort::Medium,
                        ReasoningBudget::High => ReasoningEffort::High,
                        ReasoningBudget::Max => ReasoningEffort::XHigh,
                        ReasoningBudget::Off => unreachable!(),
                    }),
                }),
            },
            usage: Some(UsageConfig { include: true }),
        };

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
            .map_err(|e| AiError::Retryable(anyhow::anyhow!("Network error: {}", e)))?;

        let status = response.status();
        if !status.is_success() {
            let response_text = response.text().await.map_err(|e| {
                AiError::Retryable(anyhow::anyhow!("Failed to read response: {}", e))
            })?;
            return Err(AiError::Terminal(anyhow::anyhow!(
                "OpenRouter API error {}: {}",
                status,
                response_text
            )));
        }

        let byte_stream = response.bytes_stream();

        let stream = async_stream::stream! {
            let mut state = OpenRouterStreamAccumulator::default();
            let mut line_buffer = String::new();

            futures_util::pin_mut!(byte_stream);

            while let Some(chunk_result) = byte_stream.next().await {
                let Ok(chunk) = chunk_result else {
                    yield Err(AiError::Retryable(anyhow::anyhow!("Stream read error")));
                    return;
                };
                line_buffer.push_str(&String::from_utf8_lossy(&chunk));
                for event in state.process_line_buffer(&mut line_buffer) {
                    yield Ok(event);
                }
            }

            yield Ok(StreamEvent::MessageComplete { response: state.into_response() });
        };

        Ok(Box::pin(stream))
    }

    fn get_cost(&self, model: &Model) -> Cost {
        self.models
            .get(model)
            .map(|resolved| resolved.cost.clone())
            .unwrap_or_else(|| Cost::new(0.0, 0.0, 0.0, 0.0))
    }

    fn model_version(&self, model: &Model) -> String {
        self.models
            .get(model)
            .map(|resolved| display_version(&resolved.id))
            .unwrap_or_else(|| model.versioned_name().to_string())
    }

    fn context_window(&self, model: &Model) -> u32 {
        self.models
            .get(model)
            .map(|resolved| resolved.context_window)
            .unwrap_or_else(|| model.context_window())
    }
}

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
#[serde(tag = "type")]
enum ContentPart {
    #[serde(rename = "text")]
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<CacheControl>,
    },
    #[serde(rename = "image_url")]
    ImageUrl { image_url: ImageUrlData },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ImageUrlData {
    url: String,
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
    XHigh,
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

#[derive(Debug, Deserialize)]
struct StreamChunk {
    #[serde(default)]
    choices: Vec<StreamChoice>,
    #[serde(default)]
    usage: Option<OpenRouterUsage>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: Option<StreamDelta>,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    reasoning: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<StreamToolCallDelta>>,
}

#[derive(Debug, Deserialize)]
struct StreamToolCallDelta {
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<StreamFunctionDelta>,
}

#[derive(Debug, Deserialize)]
struct StreamFunctionDelta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

#[derive(Default)]
struct OpenRouterStreamAccumulator {
    accumulated_text: String,
    accumulated_reasoning: String,
    tool_calls: Vec<(String, String, String)>,
    finish_reason: Option<String>,
    usage: Option<OpenRouterUsage>,
}

impl OpenRouterStreamAccumulator {
    fn process_line_buffer(&mut self, line_buffer: &mut String) -> Vec<StreamEvent> {
        let mut events = Vec::new();
        while let Some(newline_pos) = line_buffer.find('\n') {
            let line = line_buffer[..newline_pos].trim().to_string();
            line_buffer.drain(..=newline_pos);
            events.extend(self.process_sse_line(&line));
        }
        events
    }

    fn process_sse_line(&mut self, line: &str) -> Vec<StreamEvent> {
        if line.is_empty() || line.starts_with(':') {
            return vec![];
        }

        let data = match line.strip_prefix("data: ") {
            Some(d) => d.trim(),
            None => return vec![],
        };

        if data == "[DONE]" {
            return vec![];
        }

        let chunk: StreamChunk = match serde_json::from_str(data) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Failed to parse SSE chunk: {e:?}");
                return vec![];
            }
        };

        if let Some(u) = chunk.usage {
            self.usage = Some(u);
        }

        let Some(choice) = chunk.choices.into_iter().next() else {
            return vec![];
        };

        if let Some(reason) = choice.finish_reason {
            self.finish_reason = Some(reason);
        }

        let Some(delta) = choice.delta else {
            return vec![];
        };

        self.process_delta(delta)
    }

    fn process_delta(&mut self, delta: StreamDelta) -> Vec<StreamEvent> {
        let mut events = Vec::new();

        if let Some(content) = delta.content {
            if !content.is_empty() {
                self.accumulated_text.push_str(&content);
                events.push(StreamEvent::TextDelta { text: content });
            }
        }

        if let Some(reasoning) = delta.reasoning {
            if !reasoning.is_empty() {
                self.accumulated_reasoning.push_str(&reasoning);
                events.push(StreamEvent::ReasoningDelta { text: reasoning });
            }
        }

        if let Some(tc_deltas) = delta.tool_calls {
            self.accumulate_tool_calls(tc_deltas);
        }

        events
    }

    fn accumulate_tool_calls(&mut self, tc_deltas: Vec<StreamToolCallDelta>) {
        for tc in tc_deltas {
            let idx = tc.index;
            while self.tool_calls.len() <= idx {
                self.tool_calls
                    .push((String::new(), String::new(), String::new()));
            }
            if let Some(id) = tc.id {
                self.tool_calls[idx].0 = id;
            }
            let Some(func) = tc.function else { continue };
            if let Some(name) = func.name {
                self.tool_calls[idx].1 = name;
            }
            if let Some(args) = func.arguments {
                self.tool_calls[idx].2.push_str(&args);
            }
        }
    }

    fn into_response(self) -> ConversationResponse {
        let mut content_blocks = Vec::new();

        if !self.accumulated_text.trim().is_empty() {
            content_blocks.push(ContentBlock::Text(self.accumulated_text.trim().to_string()));
        }

        if !self.accumulated_reasoning.trim().is_empty() {
            content_blocks.push(ContentBlock::ReasoningContent(ReasoningData {
                text: self.accumulated_reasoning.trim().to_string(),
                signature: None,
                blob: None,
                raw_json: None,
            }));
        }

        for (id, name, args_str) in &self.tool_calls {
            if !name.is_empty() {
                let arguments = serde_json::from_str(args_str).unwrap_or(Value::Null);
                content_blocks.push(ContentBlock::ToolUse(ToolUseData {
                    id: id.clone(),
                    name: name.clone(),
                    arguments,
                }));
            }
        }

        let token_usage = match self.usage {
            Some(u) => TokenUsage {
                input_tokens: u.prompt_tokens,
                output_tokens: u.completion_tokens,
                total_tokens: u.total_tokens,
                cached_prompt_tokens: u.prompt_details.map(|d| d.cached_tokens),
                reasoning_tokens: u.completion_details.map(|d| d.reasoning_tokens),
                cache_creation_input_tokens: None,
            },
            None => TokenUsage::empty(),
        };

        let stop_reason = match self.finish_reason.as_deref() {
            Some("stop") => StopReason::EndTurn,
            Some("length") => StopReason::MaxTokens,
            Some("tool_calls") => StopReason::ToolUse,
            _ => StopReason::EndTurn,
        };

        ConversationResponse {
            content: Content::from(content_blocks),
            usage: token_usage,
            stop_reason,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum ReasoningDetail {
    #[serde(rename = "reasoning.summary")]
    Summary {
        summary: String,
        id: Option<String>,
        format: Option<String>,
        index: Option<u32>,
    },
    #[serde(rename = "reasoning.text")]
    Text {
        text: String,
        signature: Option<String>,
        id: Option<String>,
        format: Option<String>,
        index: Option<u32>,
    },
    #[serde(rename = "reasoning.encrypted")]
    Encrypted {
        data: String,
        id: Option<String>,
        format: Option<String>,
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
    let Some(last_text) = parts
        .iter_mut()
        .rev()
        .find(|p| matches!(p, ContentPart::Text { .. }))
    else {
        return;
    };
    if let ContentPart::Text { cache_control, .. } = last_text {
        *cache_control = Some(CacheControl::ephemeral());
    }
}

fn create_message_content(content: String) -> MessageContent {
    MessageContent::Array(vec![ContentPart::Text {
        text: content,
        cache_control: None,
    }])
}

fn create_tool_result_message(tool_result: &ToolResultData) -> OpenRouterMessage {
    OpenRouterMessage {
        role: "tool".to_string(),
        content: Some(MessageContent::Array(vec![ContentPart::Text {
            text: tool_result.content.trim().to_string(),
            cache_control: None,
        }])),
        name: None,
        tool_call_id: Some(tool_result.tool_use_id.clone()),
        tool_calls: None,
        reasoning_details: None,
    }
}

fn process_user_message(message: &Message) -> Result<Vec<OpenRouterMessage>, AiError> {
    let mut results = vec![];

    for tool_result in message.content.tool_results() {
        results.push(create_tool_result_message(tool_result));
    }

    let text = extract_text_content(&message.content);
    let images: Vec<&ImageData> = message.content.images();

    if text.is_empty() && images.is_empty() {
        return Ok(results);
    }

    let mut parts = Vec::new();
    if !text.is_empty() {
        parts.push(ContentPart::Text {
            text,
            cache_control: None,
        });
    }
    for img in images {
        parts.push(ContentPart::ImageUrl {
            image_url: ImageUrlData {
                url: format!("data:{};base64,{}", img.media_type, img.data),
            },
        });
    }

    results.push(OpenRouterMessage {
        role: "user".to_string(),
        content: Some(MessageContent::Array(parts)),
        name: None,
        tool_call_id: None,
        tool_calls: None,
        reasoning_details: None,
    });
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
            ContentBlock::ToolResult(_) | ContentBlock::Image(_) => continue,
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
                if !reasoning.text.trim().is_empty() {
                    text_parts.push(format!("[Reasoning: {}]", reasoning.text.trim()));
                }
            }
            ContentBlock::ToolUse(_) | ContentBlock::ToolResult(_) | ContentBlock::Image(_) => {
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
                // Strict function calling requires every property to appear in
                // `required` and `additionalProperties: false`; our tool
                // schemas legitimately have optional fields (e.g. append_memory's
                // `source`), so models that enforce strict (gpt-5.5) reject the
                // request with a 400. Omit strict and rely on tool-call parsing,
                // matching the Bedrock/Anthropic paths.
                strict: None,
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

fn parse_data_url(data_url: &str) -> Option<(String, &str)> {
    let rest = data_url.strip_prefix("data:")?;
    let semicolon = rest.find(';')?;
    let media_type = &rest[..semicolon];
    let after_semi = &rest[semicolon + 1..];
    let base64_data = after_semi.strip_prefix("base64,")?;
    Some((media_type.to_string(), base64_data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::tests::{
        test_hello_world, test_hello_world_model, test_reasoning_conversation,
        test_reasoning_with_tools, test_tool_usage,
    };

    async fn create_openrouter_provider() -> anyhow::Result<OpenRouterProvider> {
        let api_key = std::env::var("OPENROUTER_API_KEY")?;
        OpenRouterProvider::new(api_key).await
    }

    fn catalog_model(id: &str, created: u64, context_length: u32) -> OpenRouterCatalogModel {
        OpenRouterCatalogModel {
            id: id.to_string(),
            created,
            context_length,
            pricing: OpenRouterPricing {
                prompt: "0.000002".to_string(),
                completion: "0.000006".to_string(),
                input_cache_write: None,
                input_cache_read: Some("0.0000003".to_string()),
            },
        }
    }

    #[test]
    fn catalog_resolves_only_enum_families_to_latest_ids() {
        let provider = OpenRouterProvider::from_catalog(vec![
            catalog_model("x-ai/grok-4.3", 100, 1_000_000),
            catalog_model("x-ai/grok-4.5", 200, 500_000),
            catalog_model("x-ai/grok-4.5-multi-agent", 300, 500_000),
            catalog_model("openai/gpt-5.6-sol", 200, 1_050_000),
            catalog_model("openai/gpt-5.6-sol-pro", 300, 1_050_000),
            catalog_model("moonshotai/kimi-k2.7-code", 300, 262_144),
            catalog_model("moonshotai/kimi-k3", 200, 1_048_576),
            catalog_model("minimax/minimax-m2.7", 100, 204_800),
            catalog_model("minimax/minimax-m3", 200, 1_048_576),
            catalog_model("unknown/new-model-9", 999, 9_000_000),
        ]);
        for (model, expected) in [
            (Model::GptSol, "openai/gpt-5.6-sol"),
            (Model::Kimi, "moonshotai/kimi-k3"),
            (Model::Minimax, "minimax/minimax-m3"),
            (Model::Grok, "x-ai/grok-4.5"),
        ] {
            assert_eq!(provider.get_openrouter_model_id(&model).unwrap(), expected);
        }

        assert_eq!(provider.model_version(&Model::Grok), "grok-4.5");
        assert_eq!(provider.context_window(&Model::Grok), 500_000);
        assert_eq!(
            provider
                .get_cost(&Model::Grok)
                .input_cost_per_million_tokens,
            2.0
        );
        assert_eq!(provider.supported_models().len(), 4);
    }

    #[tokio::test]
    #[ignore = "requires OpenRouter API key"]
    async fn test_openrouter_catalog_discovery_live() {
        let provider = create_openrouter_provider()
            .await
            .expect("discover OpenRouter models");
        let mut models: Vec<_> = provider.supported_models().into_iter().collect();
        models.sort_by_key(|model| model.name());
        for model in models {
            println!(
                "{} -> {}",
                model.name(),
                provider.get_openrouter_model_id(&model).unwrap()
            );
        }
        assert!(provider.supported_models().contains(&Model::Grok));
        assert!(provider.supported_models().contains(&Model::Kimi));
        assert!(provider.supported_models().contains(&Model::Minimax));
    }

    #[tokio::test]
    #[ignore = "requires OpenRouter API key and credits"]
    async fn test_openrouter_discovered_grok_live() {
        let provider = create_openrouter_provider()
            .await
            .expect("discover OpenRouter models");
        test_hello_world_model(&provider, Model::Grok)
            .await
            .expect("invoke discovered Grok model");
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
    #[ignore = "requires OpenRouter API key and credits"]
    async fn test_openrouter_image_generation() {
        use crate::settings::manager::SettingsManager;
        use std::path::PathBuf;

        let home = std::env::var("HOME").expect("HOME env var not set");
        let settings_dir = PathBuf::from(home).join(".tycode");
        let manager = SettingsManager::from_settings_dir(settings_dir, None)
            .expect("Failed to load settings");
        let settings = manager.settings();

        let api_key = settings
            .providers
            .values()
            .find_map(|p| p.openrouter_api_key())
            .expect("No OpenRouter provider configured in settings");

        let provider = OpenRouterProvider::new(api_key.to_string())
            .await
            .expect("Failed to discover OpenRouter models");

        let request = ImageGenerationRequest {
            prompt: "A simple red circle on a white background".to_string(),
            model_id: "google/gemini-3.1-flash-image-preview".to_string(),
            aspect_ratio: None,
            image_size: None,
        };

        let response = provider
            .generate_image(request)
            .await
            .expect("Image generation failed");

        assert!(
            !response.image_data.is_empty(),
            "Image data should not be empty"
        );
        assert!(
            !response.media_type.is_empty(),
            "Media type should not be empty"
        );
        println!(
            "Image generated: {} bytes, type: {}",
            response.image_data.len(),
            response.media_type
        );
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
