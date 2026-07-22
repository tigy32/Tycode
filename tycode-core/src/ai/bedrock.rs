use std::collections::{HashMap, HashSet};
use std::pin::Pin;

use tokio_stream::Stream;

use base64::Engine;

use aws_sdk_bedrockruntime::{
    operation::converse::{builders::ConverseFluentBuilder, ConverseError},
    operation::converse_stream::{builders::ConverseStreamFluentBuilder, ConverseStreamError},
    types::ConverseStreamOutput as BedrockStreamEvent,
    types::{
        CachePointBlock, ContentBlock as BedrockContentBlock, ImageBlock, ImageFormat, ImageSource,
        Message as BedrockMessage, ReasoningContentBlock, ReasoningTextBlock, SystemContentBlock,
        TokenUsage as BedrockTokenUsage, Tool, ToolConfiguration, ToolInputSchema, ToolResultBlock,
        ToolResultContentBlock, ToolSpecification, ToolUseBlock,
    },
    Client as BedrockClient,
};
use aws_smithy_types::Blob;
use serde_json::json;

use crate::ai::{error::AiError, provider::AiProvider, types::*};
use crate::ai::{
    json::{from_doc, to_doc},
    mantle::{MantleClient, MantleModel},
    model::Model,
};

#[derive(Clone)]
pub struct BedrockProvider {
    client: BedrockClient,
    mantle: Option<MantleClient>,
    native_models: HashMap<Model, String>,
    mantle_models: HashMap<Model, String>,
}

fn version_numbers(value: &str) -> Vec<u64> {
    value
        .split(|character: char| !character.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .filter_map(|part| part.parse().ok())
        .collect()
}

fn newer_model_id(candidate: &str, current: &str) -> bool {
    (version_numbers(candidate), candidate) > (version_numbers(current), current)
}

fn classify_bedrock_native_model(id: &str) -> Option<Model> {
    let id = id
        .strip_prefix("global.")
        .or_else(|| id.strip_prefix("us."))
        .unwrap_or(id);
    if id.starts_with("anthropic.claude-fable-") {
        Some(Model::ClaudeFable)
    } else if id.starts_with("anthropic.claude-opus-") {
        Some(Model::ClaudeOpus)
    } else if id.starts_with("anthropic.claude-sonnet-") {
        Some(Model::ClaudeSonnet)
    } else if id.starts_with("anthropic.claude-haiku-") {
        Some(Model::ClaudeHaiku)
    } else if id.starts_with("openai.gpt-oss-120b") {
        Some(Model::GptOss120b)
    } else {
        None
    }
}

fn native_invocation_id(model: Model, catalog_id: &str) -> String {
    let catalog_id = catalog_id
        .strip_prefix("global.")
        .or_else(|| catalog_id.strip_prefix("us."))
        .unwrap_or(catalog_id);
    match model {
        Model::ClaudeFable | Model::ClaudeSonnet => format!("global.{catalog_id}"),
        Model::ClaudeOpus | Model::ClaudeHaiku => format!("us.{catalog_id}"),
        _ => catalog_id.to_string(),
    }
}

async fn discover_native_models(
    client: &aws_sdk_bedrock::Client,
) -> Result<HashMap<Model, String>, AiError> {
    let response = client
        .list_foundation_models()
        .send()
        .await
        .map_err(|error| {
            AiError::Retryable(anyhow::anyhow!(
                "Failed to discover Bedrock native models: {error}"
            ))
        })?;
    let mut resolved: HashMap<Model, String> = HashMap::new();
    for summary in response.model_summaries() {
        let id = summary.model_id();
        let Some(model) = classify_bedrock_native_model(id) else {
            continue;
        };
        let candidate = native_invocation_id(model, id);
        let replace = resolved
            .get(&model)
            .map(|current| newer_model_id(&candidate, current))
            .unwrap_or(true);
        if replace {
            resolved.insert(model, candidate);
        }
    }
    Ok(resolved)
}

fn classify_mantle_model(id: &str) -> Option<Model> {
    if let Some(name) = id.strip_prefix("openai.gpt-") {
        for (suffix, model) in [
            ("-sol", Model::GptSol),
            ("-terra", Model::GptTerra),
            ("-luna", Model::GptLuna),
        ] {
            if let Some(version) = name.strip_suffix(suffix) {
                return version
                    .chars()
                    .all(|character| character.is_ascii_digit() || character == '.')
                    .then_some(model);
            }
        }
        return name
            .chars()
            .all(|character| character.is_ascii_digit() || character == '.')
            .then_some(Model::Gpt);
    }
    id.strip_prefix("xai.grok-")
        .filter(|version| {
            version
                .chars()
                .all(|character| character.is_ascii_digit() || character == '.')
        })
        .map(|_| Model::Grok)
}

fn resolve_mantle_models(catalog: Vec<MantleModel>) -> HashMap<Model, String> {
    let mut resolved: HashMap<Model, MantleModel> = HashMap::new();
    for candidate in catalog {
        let Some(model) = classify_mantle_model(&candidate.id) else {
            continue;
        };
        let replace = resolved
            .get(&model)
            .map(|current| {
                (
                    candidate.created,
                    version_numbers(&candidate.id),
                    &candidate.id,
                ) > (current.created, version_numbers(&current.id), &current.id)
            })
            .unwrap_or(true);
        if replace {
            resolved.insert(model, candidate);
        }
    }
    resolved
        .into_iter()
        .map(|(model, resolved)| (model, resolved.id))
        .collect()
}

fn bedrock_display_version(model_id: &str) -> String {
    model_id
        .strip_prefix("global.")
        .or_else(|| model_id.strip_prefix("us."))
        .unwrap_or(model_id)
        .split_once('.')
        .map(|(_, version)| version)
        .unwrap_or(model_id)
        .to_string()
}

impl BedrockProvider {
    pub fn new(client: BedrockClient) -> Self {
        Self {
            client,
            mantle: None,
            native_models: Self::default_native_models(),
            mantle_models: HashMap::new(),
        }
    }

    pub fn with_mantle(client: BedrockClient, mantle: MantleClient) -> Self {
        Self {
            client,
            mantle: Some(mantle),
            native_models: Self::default_native_models(),
            mantle_models: Self::default_mantle_models(),
        }
    }

    fn default_native_models() -> HashMap<Model, String> {
        HashMap::from([
            (
                Model::ClaudeFable,
                "global.anthropic.claude-fable-5".to_string(),
            ),
            (
                Model::ClaudeSonnet,
                "global.anthropic.claude-sonnet-4-6".to_string(),
            ),
            (
                Model::ClaudeHaiku,
                "us.anthropic.claude-haiku-4-5-20251001-v1:0".to_string(),
            ),
            (
                Model::ClaudeOpus,
                "us.anthropic.claude-opus-4-8".to_string(),
            ),
            (Model::GptOss120b, "openai.gpt-oss-120b-1:0".to_string()),
        ])
    }

    fn default_mantle_models() -> HashMap<Model, String> {
        HashMap::from([
            (Model::Gpt, "openai.gpt-5.5".to_string()),
            (Model::GptSol, "openai.gpt-5.6-sol".to_string()),
            (Model::GptTerra, "openai.gpt-5.6-terra".to_string()),
            (Model::GptLuna, "openai.gpt-5.6-luna".to_string()),
            (Model::Grok, "xai.grok-4.3".to_string()),
        ])
    }

    pub async fn discover(
        client: BedrockClient,
        catalog_client: &aws_sdk_bedrock::Client,
        mantle: Option<MantleClient>,
    ) -> Result<Self, AiError> {
        let native_models = discover_native_models(catalog_client).await?;
        let mantle_models = match &mantle {
            Some(mantle) => resolve_mantle_models(mantle.list_models().await?),
            None => HashMap::new(),
        };
        Ok(Self {
            client,
            mantle,
            native_models,
            mantle_models,
        })
    }

    fn native_model_id(&self, model: &Model) -> Result<&str, AiError> {
        self.native_models
            .get(model)
            .map(String::as_str)
            .ok_or_else(|| {
                AiError::Terminal(anyhow::anyhow!(
                    "Model {} is not available in the Bedrock catalog",
                    model.name()
                ))
            })
    }

    fn mantle_for(&self, model: &Model) -> Result<Option<(&MantleClient, &str)>, AiError> {
        let Some(model_id) = self.mantle_models.get(model) else {
            return Ok(None);
        };
        let Some(mantle) = &self.mantle else {
            return Err(AiError::Terminal(anyhow::anyhow!(
                "Model {} ({model_id}) is only served by the bedrock-mantle endpoint, which requires AWS credentials to mint a bearer token, but this provider has no credentials configured.",
                model.name()
            )));
        };
        Ok(Some((mantle, model_id.as_str())))
    }

    fn convert_to_bedrock_messages(
        &self,
        messages: &[Message],
        model: Model,
    ) -> Result<Vec<BedrockMessage>, AiError> {
        let mut bedrock_messages = Vec::new();

        for (msg_index, msg) in messages.iter().enumerate() {
            let role = match msg.role {
                MessageRole::User => aws_sdk_bedrockruntime::types::ConversationRole::User,
                MessageRole::Assistant => {
                    aws_sdk_bedrockruntime::types::ConversationRole::Assistant
                }
            };

            let mut content_blocks = Vec::new();
            for block in msg.content.blocks() {
                match block {
                    ContentBlock::Text(text) => {
                        if !text.trim().is_empty() {
                            content_blocks.push(BedrockContentBlock::Text(text.trim().to_string()));
                        }
                    }
                    ContentBlock::ReasoningContent(reasoning) => {
                        let reasoning_content = if let Some(blob) = &reasoning.blob {
                            ReasoningContentBlock::RedactedContent(Blob::new(blob.clone()))
                        } else {
                            let mut text_block_builder =
                                ReasoningTextBlock::builder().text(&reasoning.text);

                            if let Some(signature) = &reasoning.signature {
                                text_block_builder = text_block_builder.signature(signature);
                            }

                            let text_block = text_block_builder.build().map_err(|e| {
                                AiError::Terminal(anyhow::anyhow!(
                                    "Failed to build reasoning text block: {:?}",
                                    e
                                ))
                            })?;

                            ReasoningContentBlock::ReasoningText(text_block)
                        };

                        content_blocks
                            .push(BedrockContentBlock::ReasoningContent(reasoning_content));
                    }
                    ContentBlock::ToolUse(tool_use) => {
                        let args = if tool_use.arguments.is_null() {
                            tracing::warn!(
                                tool_name = %tool_use.name,
                                "Null tool arguments in conversation history, substituting empty object"
                            );
                            serde_json::Value::Object(Default::default())
                        } else {
                            tool_use.arguments.clone()
                        };
                        let tool_use_block = ToolUseBlock::builder()
                            .tool_use_id(&tool_use.id)
                            .name(&tool_use.name)
                            .input(to_doc(args))
                            .build()
                            .map_err(|e| {
                                AiError::Terminal(anyhow::anyhow!(
                                    "Failed to build tool use block: {:?}",
                                    e
                                ))
                            })?;
                        content_blocks.push(BedrockContentBlock::ToolUse(tool_use_block));
                    }
                    ContentBlock::ToolResult(tool_result) => {
                        let tool_result_block = ToolResultBlock::builder()
                            .tool_use_id(&tool_result.tool_use_id)
                            .content(ToolResultContentBlock::Text(tool_result.content.clone()))
                            .build()
                            .map_err(|e| {
                                AiError::Terminal(anyhow::anyhow!(
                                    "Failed to build tool result block: {:?}",
                                    e
                                ))
                            })?;
                        content_blocks.push(BedrockContentBlock::ToolResult(tool_result_block));
                    }
                    ContentBlock::Image(image) => {
                        content_blocks.push(BedrockContentBlock::Image(build_bedrock_image_block(
                            image,
                        )?));
                    }
                }
            }

            if content_blocks.is_empty() {
                content_blocks.push(BedrockContentBlock::Text("...".to_string()));
            }

            // Reorder: reasoning blocks first for deterministic ordering
            // and cache point compatibility (cache point cannot follow reasoning)
            let (reasoning, non_reasoning): (Vec<_>, Vec<_>) = content_blocks
                .into_iter()
                .partition(|b| matches!(b, BedrockContentBlock::ReasoningContent(_)));
            content_blocks = reasoning;
            content_blocks.extend(non_reasoning);

            let last_is_reasoning = content_blocks
                .last()
                .is_some_and(|b| matches!(b, BedrockContentBlock::ReasoningContent(_)));
            if model.supports_prompt_caching()
                && messages.len() >= 2
                && msg_index == messages.len() - 2
                && !last_is_reasoning
            {
                content_blocks.push(BedrockContentBlock::CachePoint(Self::build_cache_point()?));
            }

            bedrock_messages.push(
                BedrockMessage::builder()
                    .role(role)
                    .set_content(Some(content_blocks))
                    .build()
                    .map_err(|e| {
                        AiError::Terminal(anyhow::anyhow!("Failed to build message: {:?}", e))
                    })?,
            );
        }

        Ok(bedrock_messages)
    }
}

fn map_image_format(media_type: &str) -> Result<ImageFormat, AiError> {
    match media_type {
        "image/png" => Ok(ImageFormat::Png),
        "image/jpeg" => Ok(ImageFormat::Jpeg),
        "image/gif" => Ok(ImageFormat::Gif),
        "image/webp" => Ok(ImageFormat::Webp),
        other => Err(AiError::Terminal(anyhow::anyhow!(
            "Unsupported image format: {other}"
        ))),
    }
}

fn build_bedrock_image_block(image: &ImageData) -> Result<ImageBlock, AiError> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&image.data)
        .map_err(|e| AiError::Terminal(anyhow::anyhow!("Failed to decode image base64: {e:?}")))?;

    let format = map_image_format(&image.media_type)?;

    ImageBlock::builder()
        .format(format)
        .source(ImageSource::Bytes(Blob::new(bytes)))
        .build()
        .map_err(|e| AiError::Terminal(anyhow::anyhow!("Failed to build image block: {e:?}")))
}

fn token_usage_from_bedrock(usage: &BedrockTokenUsage) -> TokenUsage {
    TokenUsage {
        input_tokens: usage.input_tokens() as u32,
        output_tokens: usage.output_tokens() as u32,
        total_tokens: usage.total_tokens() as u32,
        cached_prompt_tokens: usage.cache_read_input_tokens().map(|v| v as u32),
        cache_creation_input_tokens: usage.cache_write_input_tokens().map(|v| v as u32),
        reasoning_tokens: None,
    }
}

impl BedrockProvider {
    fn extract_content_blocks(&self, message: BedrockMessage) -> Content {
        let mut content_blocks = Vec::new();

        tracing::debug!("Processing {} content blocks", message.content().len());

        for (i, content) in message.content().iter().enumerate() {
            tracing::debug!("Content block {}: {:?}", i, content);

            match content {
                BedrockContentBlock::Text(text) => {
                    tracing::debug!("Text block: {}", text);
                    content_blocks.push(ContentBlock::Text(text.clone()));
                }
                BedrockContentBlock::ReasoningContent(block) => {
                    let reasoning_data = if block.is_reasoning_text() {
                        let block = block.as_reasoning_text().unwrap();
                        ReasoningData {
                            text: block.text.clone(),
                            signature: block.signature.clone(),
                            blob: None,
                            raw_json: None,
                        }
                    } else {
                        let block = block.as_redacted_content().unwrap();
                        ReasoningData {
                            text: "** Redacted reasoning content **".to_string(),
                            signature: None,
                            blob: Some(block.clone().into_inner()),
                            raw_json: None,
                        }
                    };
                    content_blocks.push(ContentBlock::ReasoningContent(reasoning_data));
                }
                BedrockContentBlock::ToolUse(tool_use) => {
                    let tool_use_data = ToolUseData {
                        id: tool_use.tool_use_id().to_string(),
                        name: tool_use.name().to_string(),
                        arguments: from_doc(tool_use.input().clone()),
                    };
                    content_blocks.push(ContentBlock::ToolUse(tool_use_data));
                }
                _ => (),
            }
        }

        Content::from(content_blocks)
    }

    fn build_cache_point() -> Result<CachePointBlock, AiError> {
        CachePointBlock::builder()
            .r#type(aws_sdk_bedrockruntime::types::CachePointType::Default)
            .build()
            .map_err(|e| {
                AiError::Terminal(anyhow::anyhow!(
                    "Failed to build cache point block: {:?}",
                    e
                ))
            })
    }

    fn effective_reasoning_budget_tokens(model: &ModelSettings) -> Option<u32> {
        let requested_budget = model.reasoning_budget.get_max_tokens()?;

        let Some(max_tokens) = model.max_tokens else {
            return Some(requested_budget);
        };

        // Bedrock requires max_tokens > thinking.budget_tokens.
        if max_tokens <= 1 {
            tracing::warn!(
                max_tokens,
                requested_budget,
                "Skipping reasoning budget because max_tokens is too low"
            );
            return None;
        }

        let capped_budget = max_tokens.saturating_sub(1);
        if requested_budget > capped_budget {
            tracing::warn!(
                requested_budget,
                max_tokens,
                capped_budget,
                "Capping reasoning budget so it remains below max_tokens"
            );
            Some(capped_budget)
        } else {
            Some(requested_budget)
        }
    }

    fn adaptive_reasoning_effort(model: &ModelSettings) -> Option<&'static str> {
        match (&model.model, &model.reasoning_budget) {
            (_, ReasoningBudget::Off) => None,
            (Model::ClaudeFable | Model::ClaudeOpus, ReasoningBudget::Max) => Some("xhigh"),
            _ => model.reasoning_budget.get_effort_level(),
        }
    }

    fn build_adaptive_thinking(model: &ModelSettings) -> Option<serde_json::Value> {
        Self::adaptive_reasoning_effort(model)?;
        let mut thinking = serde_json::Map::new();
        thinking.insert("type".to_string(), json!("adaptive"));

        if matches!(model.model, Model::ClaudeFable | Model::ClaudeOpus) {
            thinking.insert("display".to_string(), json!("summarized"));
        }

        Some(serde_json::Value::Object(thinking))
    }

    fn build_adaptive_output_config(model: &ModelSettings) -> Option<serde_json::Value> {
        if !matches!(
            model.model,
            Model::ClaudeFable | Model::ClaudeOpus | Model::ClaudeSonnet
        ) {
            return None;
        }

        let effort = Self::adaptive_reasoning_effort(model)?;
        Some(json!({ "effort": effort }))
    }

    fn apply_additional_model_fields(
        &self,
        model: &ModelSettings,
        request: ConverseFluentBuilder,
    ) -> ConverseFluentBuilder {
        let mut additional_fields = serde_json::Map::new();

        match model.model {
            Model::ClaudeFable | Model::ClaudeOpus | Model::ClaudeSonnet => {
                if let Some(thinking) = Self::build_adaptive_thinking(model) {
                    let effort = Self::adaptive_reasoning_effort(model).unwrap_or("unknown");
                    tracing::info!("Enabling adaptive reasoning with effort '{effort}'");
                    additional_fields.insert("thinking".to_string(), thinking);
                    if let Some(output_config) = Self::build_adaptive_output_config(model) {
                        additional_fields.insert("output_config".to_string(), output_config);
                    }
                }
            }
            Model::ClaudeHaiku => {
                if let Some(reasoning_budget) = Self::effective_reasoning_budget_tokens(model) {
                    tracing::info!("Enabling reasoning with budget {} tokens", reasoning_budget);
                    additional_fields.insert(
                        "thinking".to_string(),
                        json!({
                            "type": "enabled",
                            "budget_tokens": reasoning_budget
                        }),
                    );
                }
            }
            _ => {}
        }

        if additional_fields.is_empty() {
            return request;
        }

        let additional_params = serde_json::Value::Object(additional_fields);
        tracing::debug!("Additional model request fields: {:?}", additional_params);
        request.additional_model_request_fields(to_doc(additional_params))
    }

    fn apply_additional_model_fields_stream(
        &self,
        model: &ModelSettings,
        request: ConverseStreamFluentBuilder,
    ) -> ConverseStreamFluentBuilder {
        let mut additional_fields = serde_json::Map::new();

        match model.model {
            Model::ClaudeFable | Model::ClaudeOpus | Model::ClaudeSonnet => {
                if let Some(thinking) = Self::build_adaptive_thinking(model) {
                    let effort = Self::adaptive_reasoning_effort(model).unwrap_or("unknown");
                    tracing::info!("Enabling adaptive reasoning with effort '{effort}'");
                    additional_fields.insert("thinking".to_string(), thinking);
                    if let Some(output_config) = Self::build_adaptive_output_config(model) {
                        additional_fields.insert("output_config".to_string(), output_config);
                    }
                }
            }
            Model::ClaudeHaiku => {
                if let Some(reasoning_budget) = Self::effective_reasoning_budget_tokens(model) {
                    tracing::info!("Enabling reasoning with budget {} tokens", reasoning_budget);
                    additional_fields.insert(
                        "thinking".to_string(),
                        json!({
                            "type": "enabled",
                            "budget_tokens": reasoning_budget
                        }),
                    );
                }
            }
            _ => {}
        }

        if additional_fields.is_empty() {
            return request;
        }

        let additional_params = serde_json::Value::Object(additional_fields);
        tracing::debug!("Additional model request fields: {:?}", additional_params);
        request.additional_model_request_fields(to_doc(additional_params))
    }
}

struct BedrockStreamAccumulator {
    content_blocks: Vec<ContentBlock>,
    pending_text: String,
    pending_reasoning: String,
    pending_tool_id: String,
    pending_tool_name: String,
    pending_tool_input: String,
    in_text_block: bool,
    in_reasoning_block: bool,
    in_tool_block: bool,
    pending_reasoning_signature: Option<String>,
    pending_reasoning_blob: Option<Vec<u8>>,
    usage: TokenUsage,
    stop_reason: StopReason,
}

impl BedrockStreamAccumulator {
    fn new() -> Self {
        Self {
            content_blocks: Vec::new(),
            pending_text: String::new(),
            pending_reasoning: String::new(),
            pending_tool_id: String::new(),
            pending_tool_name: String::new(),
            pending_tool_input: String::new(),
            in_text_block: false,
            in_reasoning_block: false,
            in_tool_block: false,
            pending_reasoning_signature: None,
            pending_reasoning_blob: None,
            usage: TokenUsage::empty(),
            stop_reason: StopReason::EndTurn,
        }
    }

    fn process_event(&mut self, event: BedrockStreamEvent) -> Vec<StreamEvent> {
        match event {
            BedrockStreamEvent::ContentBlockStart(start) => self.handle_block_start(start),
            BedrockStreamEvent::ContentBlockDelta(delta) => self.handle_block_delta(delta),
            BedrockStreamEvent::ContentBlockStop(_) => self.handle_block_stop(),
            BedrockStreamEvent::MessageStop(stop) => {
                self.handle_message_stop(stop);
                vec![]
            }
            BedrockStreamEvent::Metadata(metadata) => {
                self.handle_metadata(metadata);
                vec![]
            }
            unknown if unknown.is_unknown() => {
                tracing::warn!(
                    "Unknown Bedrock stream event; consider updating aws-sdk-bedrockruntime"
                );
                vec![]
            }
            _ => vec![],
        }
    }

    fn handle_block_start(
        &mut self,
        start: aws_sdk_bedrockruntime::types::ContentBlockStartEvent,
    ) -> Vec<StreamEvent> {
        let content_start = match start.start() {
            Some(s) => s,
            None => return vec![StreamEvent::ContentBlockStart],
        };

        if content_start.is_tool_use() {
            let tool_use = content_start.as_tool_use().unwrap();
            self.in_tool_block = true;
            self.pending_tool_id = tool_use.tool_use_id().to_string();
            self.pending_tool_name = tool_use.name().to_string();
            self.pending_tool_input.clear();
        } else if content_start.is_unknown() {
            tracing::warn!("Unknown Bedrock content block start");
        }

        vec![StreamEvent::ContentBlockStart]
    }

    fn handle_block_delta(
        &mut self,
        delta_event: aws_sdk_bedrockruntime::types::ContentBlockDeltaEvent,
    ) -> Vec<StreamEvent> {
        let delta = match delta_event.delta() {
            Some(d) => d,
            None => return vec![],
        };

        if let Ok(text) = delta.as_text() {
            self.in_text_block = true;
            self.pending_text.push_str(text);
            return vec![StreamEvent::TextDelta {
                text: text.to_string(),
            }];
        }

        if let Ok(reasoning) = delta.as_reasoning_content() {
            self.in_reasoning_block = true;
            if let Ok(text) = reasoning.as_text() {
                self.pending_reasoning.push_str(text);
                if !text.is_empty() {
                    return vec![StreamEvent::ReasoningDelta {
                        text: text.to_string(),
                    }];
                }
                return vec![];
            }
            if let Ok(sig) = reasoning.as_signature() {
                self.pending_reasoning_signature = Some(sig.to_string());
                return vec![];
            }
            if let Ok(blob) = reasoning.as_redacted_content() {
                self.pending_reasoning_blob = Some(blob.clone().into_inner());
                return vec![];
            }
            if reasoning.is_unknown() {
                tracing::warn!(
                    "Unknown Bedrock reasoning content delta; consider updating aws-sdk-bedrockruntime"
                );
            }
        }

        if let Ok(tool_delta) = delta.as_tool_use() {
            self.pending_tool_input.push_str(tool_delta.input());
            return vec![];
        }

        if delta.is_unknown() {
            tracing::warn!(
                "Unknown Bedrock content block delta; consider updating aws-sdk-bedrockruntime"
            );
        }

        vec![]
    }

    fn handle_block_stop(&mut self) -> Vec<StreamEvent> {
        if self.in_tool_block {
            self.finalize_tool_block();
        } else if self.in_reasoning_block {
            self.finalize_reasoning_block();
        } else if self.in_text_block {
            self.finalize_text_block();
        }
        vec![StreamEvent::ContentBlockStop]
    }

    fn finalize_tool_block(&mut self) {
        let arguments = if self.pending_tool_input.trim().is_empty() {
            tracing::warn!(
                tool_name = %self.pending_tool_name,
                tool_id = %self.pending_tool_id,
                "Streamed tool use block had no input deltas, defaulting to empty object"
            );
            serde_json::Value::Object(Default::default())
        } else {
            serde_json::from_str(&self.pending_tool_input).unwrap_or_else(|e| {
                tracing::warn!(
                    tool_name = %self.pending_tool_name,
                    input = %self.pending_tool_input,
                    error = ?e,
                    "Failed to parse streamed tool input as JSON"
                );
                serde_json::Value::Object(Default::default())
            })
        };
        self.content_blocks.push(ContentBlock::ToolUse(ToolUseData {
            id: std::mem::take(&mut self.pending_tool_id),
            name: std::mem::take(&mut self.pending_tool_name),
            arguments,
        }));
        self.pending_tool_input.clear();
        self.in_tool_block = false;
    }

    fn finalize_reasoning_block(&mut self) {
        let has_text = !self.pending_reasoning.trim().is_empty();
        let has_signature = self.pending_reasoning_signature.is_some();
        let has_blob = self.pending_reasoning_blob.is_some();

        if has_text || has_signature || has_blob {
            self.content_blocks
                .push(ContentBlock::ReasoningContent(ReasoningData {
                    text: if has_text {
                        std::mem::take(&mut self.pending_reasoning)
                    } else if has_blob {
                        "** Redacted reasoning content **".to_string()
                    } else {
                        String::new()
                    },
                    signature: self.pending_reasoning_signature.take(),
                    blob: self.pending_reasoning_blob.take(),
                    raw_json: None,
                }));
        }
        self.pending_reasoning.clear();
        self.pending_reasoning_signature = None;
        self.pending_reasoning_blob = None;
        self.in_reasoning_block = false;
    }

    fn finalize_text_block(&mut self) {
        if !self.pending_text.trim().is_empty() {
            self.content_blocks.push(ContentBlock::Text(
                std::mem::take(&mut self.pending_text).trim().to_string(),
            ));
        }
        self.in_text_block = false;
    }

    fn handle_message_stop(&mut self, stop: aws_sdk_bedrockruntime::types::MessageStopEvent) {
        self.stop_reason = match stop.stop_reason() {
            aws_sdk_bedrockruntime::types::StopReason::EndTurn => StopReason::EndTurn,
            aws_sdk_bedrockruntime::types::StopReason::MaxTokens => StopReason::MaxTokens,
            aws_sdk_bedrockruntime::types::StopReason::StopSequence => {
                StopReason::StopSequence("unknown".to_string())
            }
            aws_sdk_bedrockruntime::types::StopReason::ToolUse => StopReason::ToolUse,
            _ => StopReason::EndTurn,
        };
    }

    fn handle_metadata(
        &mut self,
        metadata: aws_sdk_bedrockruntime::types::ConverseStreamMetadataEvent,
    ) {
        let Some(u) = metadata.usage() else { return };
        let usage = token_usage_from_bedrock(u);
        if usage.total_tokens == 0 && self.usage.total_tokens > 0 {
            tracing::warn!("Ignoring zero Bedrock usage metadata after non-zero usage");
            return;
        }
        self.usage = usage;
    }

    fn into_response(self) -> ConversationResponse {
        ConversationResponse {
            content: Content::from(self.content_blocks),
            usage: self.usage,
            stop_reason: self.stop_reason,
        }
    }
}

#[async_trait::async_trait]
impl AiProvider for BedrockProvider {
    fn name(&self) -> &'static str {
        "AWS Bedrock"
    }

    fn supported_models(&self) -> HashSet<Model> {
        let mut models: HashSet<Model> = self.native_models.keys().copied().collect();
        models.extend(self.mantle_models.keys().copied());
        models
    }

    async fn converse(
        &self,
        request: ConversationRequest,
    ) -> Result<ConversationResponse, AiError> {
        if let Some((mantle, mantle_id)) = self.mantle_for(&request.model.model)? {
            return mantle.converse(mantle_id, &request).await;
        }

        let model_id = self.native_model_id(&request.model.model)?;
        let bedrock_messages =
            self.convert_to_bedrock_messages(&request.messages, request.model.model)?;

        tracing::debug!(?model_id, "Using Bedrock Converse API");

        let mut converse_request = self
            .client
            .converse()
            .model_id(model_id)
            .system(SystemContentBlock::Text(request.system_prompt));

        if request.model.model.supports_prompt_caching() {
            converse_request =
                converse_request.system(SystemContentBlock::CachePoint(Self::build_cache_point()?));
        }

        converse_request = converse_request.set_messages(Some(bedrock_messages));

        let mut inference_config_builder =
            aws_sdk_bedrockruntime::types::InferenceConfiguration::builder();

        if let Some(max_tokens) = request.model.max_tokens {
            inference_config_builder = inference_config_builder.max_tokens(max_tokens as i32);
        }

        if let Some(temperature) = request.model.temperature {
            inference_config_builder = inference_config_builder.temperature(temperature);
        }

        if let Some(top_p) = request.model.top_p {
            inference_config_builder = inference_config_builder.top_p(top_p);
        }

        if !request.stop_sequences.is_empty() {
            inference_config_builder =
                inference_config_builder.set_stop_sequences(Some(request.stop_sequences));
        }

        converse_request = converse_request.inference_config(inference_config_builder.build());
        converse_request = self.apply_additional_model_fields(&request.model, converse_request);

        if !request.tools.is_empty() {
            let bedrock_tools: Vec<Tool> = request
                .tools
                .iter()
                .map(|tool| {
                    Tool::ToolSpec(
                        ToolSpecification::builder()
                            .name(&tool.name)
                            .description(&tool.description)
                            .input_schema(ToolInputSchema::Json(to_doc(tool.input_schema.clone())))
                            .build()
                            .expect("Failed to build tool spec"),
                    )
                })
                .collect();

            let mut tool_config_builder =
                ToolConfiguration::builder().set_tools(Some(bedrock_tools));

            if request.model.model.supports_prompt_caching() {
                tool_config_builder =
                    tool_config_builder.tools(Tool::CachePoint(Self::build_cache_point()?));
            }

            let tool_config = tool_config_builder
                .build()
                .expect("Failed to build tool config");
            converse_request = converse_request.tool_config(tool_config);
        }

        tracing::debug!(?converse_request, "Sending bedrock request");
        let response = converse_request.send().await.map_err(|e| {
            tracing::warn!(?e, "Bedrock converse failed");

            let e = e.into_service_error();
            match e {
                ConverseError::ThrottlingException(e) => AiError::Retryable(anyhow::anyhow!(e)),
                ConverseError::ServiceUnavailableException(e) => {
                    AiError::Retryable(anyhow::anyhow!(e))
                }
                ConverseError::InternalServerException(e) => AiError::Retryable(anyhow::anyhow!(e)),
                ConverseError::ModelTimeoutException(e) => AiError::Retryable(anyhow::anyhow!(e)),

                ConverseError::ResourceNotFoundException(e) => {
                    AiError::Terminal(anyhow::anyhow!(e))
                }
                ConverseError::AccessDeniedException(e) => AiError::Terminal(anyhow::anyhow!(e)),
                ConverseError::ModelErrorException(e) => AiError::Terminal(anyhow::anyhow!(e)),
                ConverseError::ModelNotReadyException(e) => AiError::Terminal(anyhow::anyhow!(e)),
                ConverseError::ValidationException(e) => {
                    let error_message = format!("{}", e).to_lowercase();
                    let is_input_too_long = ["too long"]
                        .iter()
                        .any(|keyword| error_message.contains(keyword));

                    if is_input_too_long {
                        AiError::InputTooLong(anyhow::anyhow!(e))
                    } else {
                        AiError::Terminal(anyhow::anyhow!(e))
                    }
                }
                _ => AiError::Terminal(anyhow::anyhow!("Unknown error from bedrock: {e:?}")),
            }
        })?;

        tracing::debug!("Full response: {:?}", response);

        let usage = if let Some(usage) = response.usage.as_ref() {
            token_usage_from_bedrock(usage)
        } else {
            TokenUsage::empty()
        };

        let stop_reason = match response.stop_reason {
            aws_sdk_bedrockruntime::types::StopReason::EndTurn => StopReason::EndTurn,
            aws_sdk_bedrockruntime::types::StopReason::MaxTokens => StopReason::MaxTokens,
            aws_sdk_bedrockruntime::types::StopReason::StopSequence => {
                StopReason::StopSequence("unknown".to_string())
            }
            aws_sdk_bedrockruntime::types::StopReason::ToolUse => StopReason::ToolUse,
            _ => StopReason::EndTurn,
        };

        let message = response
            .output
            .ok_or_else(|| AiError::Terminal(anyhow::anyhow!("No output in response")))?
            .as_message()
            .map_err(|_| AiError::Terminal(anyhow::anyhow!("Output is not a message")))?
            .clone();

        tracing::debug!("Message content blocks: {:?}", message.content());

        let content = self.extract_content_blocks(message.clone());

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
        if let Some((mantle, mantle_id)) = self.mantle_for(&request.model.model)? {
            return mantle.converse_stream(mantle_id, &request).await;
        }

        let model_id = self.native_model_id(&request.model.model)?;
        let bedrock_messages =
            self.convert_to_bedrock_messages(&request.messages, request.model.model)?;

        tracing::debug!(?model_id, "Using Bedrock Converse Stream API");

        let mut stream_request = self
            .client
            .converse_stream()
            .model_id(model_id)
            .system(SystemContentBlock::Text(request.system_prompt));

        if request.model.model.supports_prompt_caching() {
            stream_request =
                stream_request.system(SystemContentBlock::CachePoint(Self::build_cache_point()?));
        }

        stream_request = stream_request.set_messages(Some(bedrock_messages));

        let mut inference_config_builder =
            aws_sdk_bedrockruntime::types::InferenceConfiguration::builder();

        if let Some(max_tokens) = request.model.max_tokens {
            inference_config_builder = inference_config_builder.max_tokens(max_tokens as i32);
        }

        if let Some(temperature) = request.model.temperature {
            inference_config_builder = inference_config_builder.temperature(temperature);
        }

        if let Some(top_p) = request.model.top_p {
            inference_config_builder = inference_config_builder.top_p(top_p);
        }

        if !request.stop_sequences.is_empty() {
            inference_config_builder =
                inference_config_builder.set_stop_sequences(Some(request.stop_sequences));
        }

        stream_request = stream_request.inference_config(inference_config_builder.build());
        stream_request = self.apply_additional_model_fields_stream(&request.model, stream_request);

        if !request.tools.is_empty() {
            let bedrock_tools: Vec<Tool> = request
                .tools
                .iter()
                .map(|tool| {
                    Tool::ToolSpec(
                        ToolSpecification::builder()
                            .name(&tool.name)
                            .description(&tool.description)
                            .input_schema(ToolInputSchema::Json(to_doc(tool.input_schema.clone())))
                            .build()
                            .expect("Failed to build tool spec"),
                    )
                })
                .collect();

            let mut tool_config_builder =
                ToolConfiguration::builder().set_tools(Some(bedrock_tools));

            if request.model.model.supports_prompt_caching() {
                tool_config_builder =
                    tool_config_builder.tools(Tool::CachePoint(Self::build_cache_point()?));
            }

            let tool_config = tool_config_builder
                .build()
                .expect("Failed to build tool config");
            stream_request = stream_request.tool_config(tool_config);
        }

        let response = stream_request.send().await.map_err(|e| {
            tracing::warn!(?e, "Bedrock converse_stream failed");
            let e = e.into_service_error();
            match e {
                ConverseStreamError::ThrottlingException(e) => {
                    AiError::Retryable(anyhow::anyhow!(e))
                }
                ConverseStreamError::ServiceUnavailableException(e) => {
                    AiError::Retryable(anyhow::anyhow!(e))
                }
                ConverseStreamError::InternalServerException(e) => {
                    AiError::Retryable(anyhow::anyhow!(e))
                }
                ConverseStreamError::ModelTimeoutException(e) => {
                    AiError::Retryable(anyhow::anyhow!(e))
                }
                ConverseStreamError::ResourceNotFoundException(e) => {
                    AiError::Terminal(anyhow::anyhow!(e))
                }
                ConverseStreamError::AccessDeniedException(e) => {
                    AiError::Terminal(anyhow::anyhow!(e))
                }
                ConverseStreamError::ModelErrorException(e) => {
                    AiError::Terminal(anyhow::anyhow!(e))
                }
                ConverseStreamError::ModelNotReadyException(e) => {
                    AiError::Terminal(anyhow::anyhow!(e))
                }
                ConverseStreamError::ValidationException(e) => {
                    let error_message = format!("{}", e).to_lowercase();
                    if error_message.contains("too long") {
                        AiError::InputTooLong(anyhow::anyhow!(e))
                    } else {
                        AiError::Terminal(anyhow::anyhow!(e))
                    }
                }
                _ => AiError::Terminal(anyhow::anyhow!("Unknown error from bedrock stream: {e:?}")),
            }
        })?;

        let mut event_stream = response.stream;

        let stream = async_stream::stream! {
            let mut state = BedrockStreamAccumulator::new();

            loop {
                let recv_result = event_stream.recv().await;
                let Ok(maybe_event) = recv_result else {
                    tracing::warn!("Error in bedrock stream");
                    yield Err(AiError::Retryable(anyhow::anyhow!("Bedrock stream error")));
                    return;
                };
                let Some(event) = maybe_event else { break };
                for stream_event in state.process_event(event) {
                    yield Ok(stream_event);
                }
            }

            yield Ok(StreamEvent::MessageComplete { response: state.into_response() });
        };

        Ok(Box::pin(stream))
    }

    fn get_cost(&self, model: &Model) -> Cost {
        match model {
            Model::ClaudeFable => Cost::new(10.0, 50.0, 12.5, 1.0),
            Model::ClaudeSonnet => Cost::new(3.0, 15.0, 3.75, 0.3),
            Model::ClaudeHaiku => Cost::new(1.0, 5.0, 1.25, 0.1),
            Model::ClaudeOpus => Cost::new(5.0, 25.0, 6.25, 0.5),
            Model::GptOss120b => Cost::new(0.15, 0.6, 0.0, 0.0),
            Model::Gpt => Cost::new(5.5, 33.0, 0.0, 0.55),
            Model::GptSol => Cost::new(5.5, 33.0, 6.88, 0.55),
            Model::GptTerra => Cost::new(2.75, 16.5, 3.44, 0.28),
            Model::GptLuna => Cost::new(1.1, 6.6, 1.38, 0.11),
            Model::Grok => Cost::new(1.25, 2.5, 0.0, 0.2),
            _ => Cost::new(0.0, 0.0, 0.0, 0.0),
        }
    }

    fn model_version(&self, model: &Model) -> String {
        self.mantle_models
            .get(model)
            .or_else(|| self.native_models.get(model))
            .map(|model_id| bedrock_display_version(model_id))
            .unwrap_or_else(|| model.versioned_name().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::tests::{
        test_hello_world, test_multiple_tool_calls, test_reasoning_conversation,
        test_reasoning_with_tools, test_tool_usage,
    };
    use aws_sdk_bedrockruntime::types::{
        ContentBlockDelta, ContentBlockDeltaEvent, ContentBlockStopEvent,
        ReasoningContentBlockDelta,
    };
    use tokio_stream::StreamExt;

    #[test]
    fn mantle_catalog_resolves_only_supported_families_to_latest_ids() {
        let resolved = resolve_mantle_models(vec![
            MantleModel {
                id: "openai.gpt-5.5".to_string(),
                created: 100,
            },
            MantleModel {
                id: "openai.gpt-5.6-sol".to_string(),
                created: 200,
            },
            MantleModel {
                id: "openai.gpt-5.6-sol-pro".to_string(),
                created: 300,
            },
            MantleModel {
                id: "xai.grok-4.3".to_string(),
                created: 200,
            },
            MantleModel {
                id: "xai.grok-4.4".to_string(),
                created: 300,
            },
            MantleModel {
                id: "new-provider/unregistered-9".to_string(),
                created: 999,
            },
        ]);

        assert_eq!(resolved.get(&Model::Gpt).unwrap(), "openai.gpt-5.5");
        assert_eq!(resolved.get(&Model::GptSol).unwrap(), "openai.gpt-5.6-sol");
        assert_eq!(resolved.get(&Model::Grok).unwrap(), "xai.grok-4.4");
        assert_eq!(resolved.len(), 3);
    }

    #[test]
    fn gpt_56_models_never_classify_as_native_bedrock_models() {
        for model_id in [
            "openai.gpt-5.6-sol",
            "openai.gpt-5.6-terra",
            "openai.gpt-5.6-luna",
        ] {
            assert_eq!(classify_bedrock_native_model(model_id), None);
            assert!(classify_mantle_model(model_id).is_some());
        }
    }

    #[test]
    fn test_adaptive_thinking_uses_converse_shape() {
        let thinking = BedrockProvider::build_adaptive_thinking(&ModelSettings {
            model: Model::ClaudeOpus,
            max_tokens: Some(32_000),
            temperature: None,
            top_p: None,
            reasoning_budget: ReasoningBudget::Max,
        })
        .unwrap();

        assert_eq!(thinking["type"], "adaptive");
        assert!(thinking.get("effort").is_none());
        assert_eq!(thinking["display"], "summarized");

        let output_config = BedrockProvider::build_adaptive_output_config(&ModelSettings {
            model: Model::ClaudeOpus,
            max_tokens: Some(32_000),
            temperature: None,
            top_p: None,
            reasoning_budget: ReasoningBudget::Max,
        })
        .unwrap();
        assert_eq!(output_config["effort"], "xhigh");

        for (model, budget, expected_effort) in [
            (Model::ClaudeSonnet, ReasoningBudget::Max, "max"),
            (Model::ClaudeSonnet, ReasoningBudget::High, "high"),
        ] {
            let settings = ModelSettings {
                model,
                max_tokens: Some(32_000),
                temperature: None,
                top_p: None,
                reasoning_budget: budget,
            };

            let thinking = BedrockProvider::build_adaptive_thinking(&settings).unwrap();
            assert_eq!(thinking["type"], "adaptive");
            assert!(thinking.get("effort").is_none());
            assert!(thinking.get("display").is_none());

            let output_config = BedrockProvider::build_adaptive_output_config(&settings).unwrap();
            assert_eq!(output_config["effort"], expected_effort);
        }
    }

    #[test]
    fn test_stream_accumulator_preserves_signature_only_reasoning() {
        let mut state = BedrockStreamAccumulator::new();
        let delta = ContentBlockDeltaEvent::builder()
            .content_block_index(0)
            .delta(ContentBlockDelta::ReasoningContent(
                ReasoningContentBlockDelta::Signature("sig".to_string()),
            ))
            .build()
            .unwrap();
        state.process_event(BedrockStreamEvent::ContentBlockDelta(delta));

        let stop = ContentBlockStopEvent::builder()
            .content_block_index(0)
            .build()
            .unwrap();
        state.process_event(BedrockStreamEvent::ContentBlockStop(stop));

        let response = state.into_response();
        let blocks = response.content.blocks();

        assert_eq!(blocks.len(), 1);
        match &blocks[0] {
            ContentBlock::ReasoningContent(reasoning) => {
                assert_eq!(reasoning.text, "");
                assert_eq!(reasoning.signature.as_deref(), Some("sig"));
            }
            other => panic!("expected reasoning block, got {other:?}"),
        }
    }

    async fn create_bedrock_provider() -> anyhow::Result<BedrockProvider> {
        let bedrock_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new("us-west-2"))
            .load()
            .await;
        let bedrock_client = aws_sdk_bedrockruntime::Client::new(&bedrock_config);
        Ok(BedrockProvider::new(bedrock_client))
    }

    async fn create_mantle_bedrock_provider(
        region: &'static str,
    ) -> anyhow::Result<BedrockProvider> {
        let bedrock_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .profile_name("default")
            .region(aws_config::Region::new(region))
            .load()
            .await;
        let credentials = bedrock_config
            .credentials_provider()
            .ok_or_else(|| anyhow::anyhow!("AWS profile has no credentials provider"))?;
        let bedrock_client = aws_sdk_bedrockruntime::Client::new(&bedrock_config);
        let catalog_client = aws_sdk_bedrock::Client::new(&bedrock_config);
        Ok(BedrockProvider::discover(
            bedrock_client,
            &catalog_client,
            Some(MantleClient::new(region, credentials)),
        )
        .await?)
    }

    #[tokio::test]
    #[ignore = "requires AWS credentials and Mantle model access"]
    async fn test_bedrock_catalog_discovery_live() {
        let provider = create_mantle_bedrock_provider("us-east-2")
            .await
            .expect("discover Bedrock models");
        let mut models: Vec<_> = provider.supported_models().into_iter().collect();
        models.sort_by_key(|model| model.name());
        for model in models {
            let id = provider
                .mantle_models
                .get(&model)
                .or_else(|| provider.native_models.get(&model))
                .unwrap();
            println!("{} -> {id}", model.name());
        }
        assert!(provider.supported_models().contains(&Model::Grok));
    }

    #[tokio::test]
    #[ignore = "requires AWS credentials, Mantle model access, and incurs Bedrock charges"]
    async fn test_bedrock_mantle_current_models_live() {
        let provider = create_mantle_bedrock_provider("us-east-2")
            .await
            .expect("create Mantle-backed Bedrock provider");

        let mut failures = Vec::new();

        for model in [Model::GptSol, Model::GptTerra, Model::GptLuna, Model::Grok] {
            let response = match provider
                .converse(ConversationRequest {
                    messages: vec![Message::user(Content::text_only(
                        "Reply with exactly: OK".to_string(),
                    ))],
                    model: ModelSettings {
                        model,
                        max_tokens: Some(1_024),
                        temperature: None,
                        top_p: None,
                        reasoning_budget: ReasoningBudget::Off,
                    },
                    system_prompt: String::new(),
                    stop_sequences: Vec::new(),
                    tools: Vec::new(),
                })
                .await
            {
                Ok(response) => response,
                Err(error) => {
                    failures.push(format!("{}: {error}", model.name()));
                    continue;
                }
            };

            if response.content.text().trim().is_empty() {
                failures.push(format!("{} returned no text", model.name()));
            }
            if response.usage.total_tokens == 0 {
                failures.push(format!("{} returned no usage", model.name()));
            }
        }

        assert!(
            failures.is_empty(),
            "live request failures:\n{}",
            failures.join("\n")
        );
    }

    #[tokio::test]
    #[ignore = "requires AWS credentials"]
    async fn test_bedrock_hello_world() {
        let provider = match create_bedrock_provider().await {
            Ok(provider) => provider,
            Err(e) => {
                tracing::error!(?e, "Failed to create Bedrock provider");
                panic!("Failed to create Bedrock provider: {e:?}");
            }
        };

        if let Err(e) = test_hello_world(provider).await {
            tracing::error!(?e, "Bedrock hello world test failed");
            panic!("Bedrock hello world test failed: {e:?}");
        }
    }

    #[tokio::test]
    #[ignore = "requires AWS credentials"]
    async fn test_bedrock_reasoning_conversation() {
        let provider = match create_bedrock_provider().await {
            Ok(provider) => provider,
            Err(e) => {
                tracing::error!(?e, "Failed to create Bedrock provider");
                panic!("Failed to create Bedrock provider: {e:?}");
            }
        };

        if let Err(e) = test_reasoning_conversation(provider).await {
            tracing::error!(?e, "Bedrock reasoning conversation test failed");
            panic!("Bedrock reasoning conversation test failed: {e:?}");
        }
    }

    #[tokio::test]
    #[ignore = "requires AWS credentials"]
    async fn test_bedrock_tool_usage() {
        let provider = match create_bedrock_provider().await {
            Ok(provider) => provider,
            Err(e) => {
                tracing::error!(?e, "Failed to create Bedrock provider");
                panic!("Failed to create Bedrock provider: {e:?}");
            }
        };

        if let Err(e) = test_tool_usage(provider).await {
            tracing::error!(?e, "Bedrock tool usage test failed");
            panic!("Bedrock tool usage test failed: {e:?}");
        }
    }

    #[tokio::test]
    #[ignore = "requires AWS credentials"]
    async fn test_bedrock_reasoning_with_tools() {
        let provider = match create_bedrock_provider().await {
            Ok(provider) => provider,
            Err(e) => {
                tracing::error!(?e, "Failed to create Bedrock provider");
                panic!("Failed to create Bedrock provider: {e:?}");
            }
        };

        if let Err(e) = test_reasoning_with_tools(provider).await {
            tracing::error!(?e, "Bedrock reasoning with tools test failed");
            panic!("Bedrock reasoning with tools test failed: {e:?}");
        }
    }

    #[tokio::test]
    #[ignore = "requires AWS credentials"]
    async fn test_bedrock_multiple_tool_calls() {
        let provider = match create_bedrock_provider().await {
            Ok(provider) => provider,
            Err(e) => {
                tracing::error!(?e, "Failed to create Bedrock provider");
                panic!("Failed to create Bedrock provider: {e:?}");
            }
        };

        if let Err(e) = test_multiple_tool_calls(provider).await {
            tracing::error!(?e, "Bedrock reasoning with tools test failed");
            panic!("Bedrock reasoning with tools test failed: {e:?}");
        }
    }

    #[tokio::test]
    #[ignore = "requires AWS credentials and incurs Bedrock charges"]
    async fn test_bedrock_opus47_streaming_usage_and_tool_use() {
        let provider = match create_bedrock_provider().await {
            Ok(provider) => provider,
            Err(e) => {
                tracing::error!(?e, "Failed to create Bedrock provider");
                panic!("Failed to create Bedrock provider: {e:?}");
            }
        };

        let calculator_tool = ToolDefinition {
            name: "calculator".to_string(),
            description: "Perform basic arithmetic calculations".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "expression": {
                        "type": "string",
                        "description": "The mathematical expression to evaluate"
                    }
                },
                "required": ["expression"]
            }),
        };

        let request = ConversationRequest {
            messages: vec![Message {
                role: MessageRole::User,
                content: Content::text_only(
                    "Use the calculator tool to compute 2 + 2. Do not answer directly.".to_string(),
                ),
            }],
            model: ModelSettings {
                model: Model::ClaudeOpus,
                max_tokens: Some(1024),
                temperature: None,
                top_p: None,
                reasoning_budget: ReasoningBudget::High,
            },
            system_prompt: "You are a helpful AI assistant. Use tools when requested.".to_string(),
            stop_sequences: Vec::new(),
            tools: vec![calculator_tool],
        };

        let mut stream = provider
            .converse_stream(request)
            .await
            .expect("Opus streaming request should start");
        let mut response = None;

        while let Some(event) = stream.next().await {
            match event.expect("Opus stream event should succeed") {
                StreamEvent::MessageComplete { response: complete } => {
                    response = Some(complete);
                    break;
                }
                StreamEvent::TextDelta { text } => {
                    println!("text delta: {text}");
                }
                StreamEvent::ReasoningDelta { text } => {
                    println!("reasoning delta: {text}");
                }
                StreamEvent::ContentBlockStart | StreamEvent::ContentBlockStop => {}
            }
        }

        let response = response.expect("Opus stream should produce MessageComplete");
        println!("Opus streaming response: {response:?}");

        assert!(
            matches!(response.stop_reason, StopReason::ToolUse),
            "expected tool_use stop reason, got {:?}",
            response.stop_reason
        );
        assert!(
            !response.content.tool_uses().is_empty(),
            "streaming response should contain a tool use"
        );
        assert!(
            response.usage.input_tokens > 0,
            "streaming response should report input tokens"
        );
        assert!(
            response.usage.output_tokens > 0,
            "streaming response should report output tokens"
        );
        assert!(
            response.usage.total_tokens > 0,
            "streaming response should report total tokens"
        );

        let tool_use_id = response.content.tool_uses()[0].id.clone();
        let followup_request = ConversationRequest {
            messages: vec![
                Message {
                    role: MessageRole::User,
                    content: Content::text_only(
                        "Use the calculator tool to compute 2 + 2. Do not answer directly."
                            .to_string(),
                    ),
                },
                Message {
                    role: MessageRole::Assistant,
                    content: response.content,
                },
                Message {
                    role: MessageRole::User,
                    content: Content::new(vec![ContentBlock::ToolResult(ToolResultData {
                        tool_use_id,
                        content: "4".to_string(),
                        is_error: false,
                    })]),
                },
            ],
            model: ModelSettings {
                model: Model::ClaudeOpus,
                max_tokens: Some(1024),
                temperature: None,
                top_p: None,
                reasoning_budget: ReasoningBudget::High,
            },
            system_prompt: "You are a helpful AI assistant. Use tools when requested.".to_string(),
            stop_sequences: Vec::new(),
            tools: vec![ToolDefinition {
                name: "calculator".to_string(),
                description: "Perform basic arithmetic calculations".to_string(),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "expression": {
                            "type": "string",
                            "description": "The mathematical expression to evaluate"
                        }
                    },
                    "required": ["expression"]
                }),
            }],
        };

        let mut followup_stream = provider
            .converse_stream(followup_request)
            .await
            .expect("Opus 4.7 follow-up streaming request should start");
        let mut followup_response = None;

        while let Some(event) = followup_stream.next().await {
            if let StreamEvent::MessageComplete { response } =
                event.expect("Opus 4.7 follow-up stream event should succeed")
            {
                followup_response = Some(response);
                break;
            }
        }

        let followup_response =
            followup_response.expect("Opus 4.7 follow-up stream should produce MessageComplete");
        println!("Opus 4.7 follow-up streaming response: {followup_response:?}");

        assert!(
            matches!(followup_response.stop_reason, StopReason::EndTurn),
            "expected end_turn stop reason, got {:?}",
            followup_response.stop_reason
        );
        assert!(
            followup_response.content.text().contains('4'),
            "follow-up response should include the calculator result"
        );
        assert!(
            followup_response.usage.input_tokens > 0,
            "follow-up response should report input tokens"
        );
        assert!(
            followup_response.usage.output_tokens > 0,
            "follow-up response should report output tokens"
        );
        assert!(
            followup_response.usage.total_tokens > 0,
            "follow-up response should report total tokens"
        );
    }

    #[tokio::test]
    #[ignore = "requires AWS credentials and incurs Bedrock charges"]
    async fn test_bedrock_streaming_reasoning_round_trip() {
        let provider = match create_bedrock_provider().await {
            Ok(provider) => provider,
            Err(e) => {
                tracing::error!(?e, "Failed to create Bedrock provider");
                panic!("Failed to create Bedrock provider: {e:?}");
            }
        };

        for model in [Model::ClaudeHaiku, Model::ClaudeSonnet, Model::ClaudeOpus] {
            assert_streaming_reasoning_round_trip(&provider, model).await;
        }
    }

    async fn assert_streaming_reasoning_round_trip(provider: &BedrockProvider, model: Model) {
        let reasoning_budget = if matches!(model, Model::ClaudeOpus) {
            ReasoningBudget::Max
        } else {
            ReasoningBudget::Low
        };
        let first_prompt = format!(
            "This is a difficult constraint-satisfaction puzzle. Solve it carefully and give the final answer.\n\n\
             Five engineers (Avery, Blair, Casey, Devon, and Ellis) each own one laptop color \
             (red, blue, green, silver, black) and one pet (cat, dog, fish, bird, turtle). \
             No two engineers share a color or pet.\n\
             Clues:\n\
             1. Avery does not own the red or black laptop.\n\
             2. Blair owns the dog.\n\
             3. The green laptop owner owns the fish.\n\
             4. Devon owns the silver laptop.\n\
             5. Ellis owns neither the bird nor the turtle.\n\
             6. The blue laptop owner is either Avery or Casey.\n\
             7. Casey owns the turtle.\n\
             8. Avery owns the cat.\n\n\
             Who owns the green laptop? Model under test: {}.",
            model.name()
        );

        let first_request = ConversationRequest {
            messages: vec![Message {
                role: MessageRole::User,
                content: Content::text_only(first_prompt.clone()),
            }],
            model: ModelSettings {
                model,
                max_tokens: Some(6000),
                temperature: None,
                top_p: None,
                reasoning_budget: reasoning_budget.clone(),
            },
            system_prompt:
                "You are a helpful AI assistant. Use reasoning when solving logic puzzles."
                    .to_string(),
            stop_sequences: Vec::new(),
            tools: Vec::new(),
        };

        let mut first_stream = provider
            .converse_stream(first_request)
            .await
            .expect("Bedrock reasoning first streaming request should start");
        let mut first_response = None;

        while let Some(event) = first_stream.next().await {
            if let StreamEvent::MessageComplete { response } =
                event.expect("Bedrock reasoning first stream event should succeed")
            {
                first_response = Some(response);
                break;
            }
        }

        let first_response =
            first_response.expect("Bedrock reasoning first stream should produce MessageComplete");
        println!(
            "Bedrock {} first reasoning response: {first_response:?}",
            model.name()
        );

        let reasoning_blocks = first_response.content.reasoning();
        assert!(
            !reasoning_blocks.is_empty(),
            "first response should include reasoning content for {}",
            model.name()
        );
        assert!(
            reasoning_blocks
                .iter()
                .any(|reasoning| !reasoning.text.is_empty()
                    || reasoning.signature.is_some()
                    || reasoning.blob.is_some()),
            "reasoning content should carry text, a signature, or redacted data for {}",
            model.name()
        );
        assert!(
            first_response.usage.input_tokens > 0,
            "first response should report input tokens for {}",
            model.name()
        );
        assert!(
            first_response.usage.output_tokens > 0,
            "first response should report output tokens for {}",
            model.name()
        );
        assert!(
            first_response.usage.total_tokens > 0,
            "first response should report total tokens for {}",
            model.name()
        );

        let second_request = ConversationRequest {
            messages: vec![
                Message {
                    role: MessageRole::User,
                    content: Content::text_only(first_prompt),
                },
                Message {
                    role: MessageRole::Assistant,
                    content: first_response.content.clone(),
                },
                Message {
                    role: MessageRole::User,
                    content: Content::text_only(
                        "Now answer in one short sentence and mention only the green laptop owner."
                            .to_string(),
                    ),
                },
            ],
            model: ModelSettings {
                model,
                max_tokens: Some(6000),
                temperature: None,
                top_p: None,
                reasoning_budget,
            },
            system_prompt:
                "You are a helpful AI assistant. Use reasoning when solving logic puzzles."
                    .to_string(),
            stop_sequences: Vec::new(),
            tools: Vec::new(),
        };

        let mut second_stream = provider
            .converse_stream(second_request)
            .await
            .expect("Bedrock reasoning second streaming request should start");
        let mut second_response = None;

        while let Some(event) = second_stream.next().await {
            if let StreamEvent::MessageComplete { response } =
                event.expect("Bedrock reasoning second stream event should succeed")
            {
                second_response = Some(response);
                break;
            }
        }

        let second_response = second_response
            .expect("Bedrock reasoning second stream should produce MessageComplete");
        println!(
            "Bedrock {} second reasoning response: {second_response:?}",
            model.name()
        );

        assert!(
            matches!(second_response.stop_reason, StopReason::EndTurn),
            "expected end_turn stop reason for {}, got {:?}",
            model.name(),
            second_response.stop_reason
        );
        assert!(
            second_response
                .content
                .text()
                .to_lowercase()
                .contains("ellis"),
            "second response should use the previous reasoning conversation for {}",
            model.name()
        );
        assert!(
            second_response.usage.input_tokens > 0,
            "second response should report input tokens for {}",
            model.name()
        );
        assert!(
            second_response.usage.output_tokens > 0,
            "second response should report output tokens for {}",
            model.name()
        );
        assert!(
            second_response.usage.total_tokens > 0,
            "second response should report total tokens for {}",
            model.name()
        );
    }
}
