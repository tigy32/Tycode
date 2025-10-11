use std::collections::HashSet;

use aws_sdk_bedrockruntime::{
    operation::converse::{builders::ConverseFluentBuilder, ConverseError},
    types::{
        ContentBlock as BedrockContentBlock, Message as BedrockMessage, ReasoningContentBlock,
        ReasoningTextBlock, SystemContentBlock, Tool, ToolConfiguration, ToolInputSchema,
        ToolResultBlock, ToolResultContentBlock, ToolSpecification, ToolUseBlock,
    },
    Client as BedrockClient,
};
use aws_smithy_types::Blob;
use serde_json::json;

use crate::ai::{error::AiError, provider::AiProvider, types::*};
use crate::ai::{
    json::{from_doc, to_doc},
    model::Model,
};

#[derive(Clone)]
pub struct BedrockProvider {
    client: BedrockClient,
}

impl BedrockProvider {
    pub fn new(client: BedrockClient) -> Self {
        Self { client }
    }

    fn get_bedrock_model_id(&self, model: &Model) -> Result<String, AiError> {
        let model_id = match model {
            Model::ClaudeSonnet45 => "us.anthropic.claude-sonnet-4-5-20250929-v1:0",
            Model::ClaudeOpus41 => "us.anthropic.claude-opus-4-1-20250805-v1:0",
            Model::ClaudeOpus4 => "us.anthropic.claude-opus-4-20250514-v1:0",
            Model::ClaudeSonnet4 => "us.anthropic.claude-sonnet-4-20250514-v1:0",
            Model::ClaudeSonnet37 => "us.anthropic.claude-3-7-sonnet-20250219-v1:0",
            Model::GptOss120b => "openai.gpt-oss-120b-1:0",
            _ => {
                return Err(AiError::Terminal(anyhow::anyhow!(
                    "Model {} is not supported in bedrock",
                    model.name()
                )))
            }
        };
        Ok(model_id.to_string())
    }

    fn convert_to_bedrock_messages(
        &self,
        messages: &[Message],
    ) -> Result<Vec<BedrockMessage>, AiError> {
        let mut bedrock_messages = Vec::new();

        for msg in messages.iter() {
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
                            content_blocks.push(BedrockContentBlock::Text(text.clone()));
                        }
                    }
                    ContentBlock::ReasoningContent(reasoning) => {
                        // Reconstruct the ReasoningContent block in the proper format
                        tracing::debug!(
                            "Converting reasoning content block back to Bedrock format"
                        );

                        let reasoning_content = if let Some(blob) = &reasoning.blob {
                            // This is redacted content - reconstruct from blob
                            tracing::debug!("Creating redacted reasoning content from blob");
                            ReasoningContentBlock::RedactedContent(Blob::new(blob.clone()))
                        } else {
                            // This is reasoning text - reconstruct with text and optional signature
                            tracing::debug!(
                                "Creating reasoning text block with {} chars, signature: {}",
                                reasoning.text.len(),
                                reasoning.signature.is_some()
                            );

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
                        let tool_use_block = ToolUseBlock::builder()
                            .tool_use_id(&tool_use.id)
                            .name(&tool_use.name)
                            .input(to_doc(tool_use.arguments.clone()))
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
                }
            }

            if content_blocks.is_empty() {
                content_blocks.push(BedrockContentBlock::Text("...".to_string()));
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

    fn apply_reasoning(
        &self,
        model: &ModelSettings,
        request: ConverseFluentBuilder,
    ) -> ConverseFluentBuilder {
        let Some(reasoning_budget) = model.reasoning_budget.get_max_tokens() else {
            return request;
        };

        match model.model {
            Model::ClaudeSonnet37
            | Model::ClaudeSonnet4
            | Model::ClaudeOpus4
            | Model::ClaudeOpus41
            | Model::ClaudeSonnet45 => {
                tracing::info!(
                    "🧠 Enabling reasoning with budget {} tokens",
                    reasoning_budget
                );

                let reasoning_params = json!({
                    "thinking": {
                        "type": "enabled",
                        "budget_tokens": reasoning_budget
                    }
                });
                tracing::debug!("Added reasoning config: {:?}", reasoning_params);
                request.additional_model_request_fields(to_doc(reasoning_params))
            }
            _ => request,
        }
    }
}

#[async_trait::async_trait]
impl AiProvider for BedrockProvider {
    fn name(&self) -> &'static str {
        "AWS Bedrock"
    }

    fn supported_models(&self) -> HashSet<Model> {
        HashSet::from([
            Model::ClaudeOpus41,
            Model::ClaudeSonnet4,
            Model::ClaudeSonnet45,
            Model::ClaudeOpus4,
            Model::ClaudeSonnet37,
            Model::GptOss120b,
        ])
    }

    async fn converse(
        &self,
        request: ConversationRequest,
    ) -> Result<ConversationResponse, AiError> {
        let model_id = self.get_bedrock_model_id(&request.model.model)?;
        let bedrock_messages = self.convert_to_bedrock_messages(&request.messages)?;

        tracing::debug!(?model_id, "Using Bedrock Converse API");

        let mut converse_request = self
            .client
            .converse()
            .model_id(&model_id)
            .system(SystemContentBlock::Text(request.system_prompt))
            .set_messages(Some(bedrock_messages));

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
        converse_request = self.apply_reasoning(&request.model, converse_request);

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

            let tool_config = ToolConfiguration::builder()
                .set_tools(Some(bedrock_tools))
                .build()
                .expect("Failed to build tool config");
            converse_request = converse_request.tool_config(tool_config);
        }

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
                ConverseError::ValidationException(e) => AiError::Terminal(anyhow::anyhow!(e)),
                _ => AiError::Terminal(anyhow::anyhow!("Unknown error from bedrock: {e:?}")),
            }
        })?;

        tracing::debug!("Full response: {:?}", response);

        let usage = if let Some(usage) = response.usage {
            TokenUsage::new(usage.input_tokens() as u32, usage.output_tokens() as u32)
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

    fn get_cost(&self, model: &Model) -> Cost {
        match model {
            Model::ClaudeSonnet45 => Cost::new(0.003, 0.015),
            Model::ClaudeOpus41 => Cost::new(0.015, 0.075),
            Model::ClaudeOpus4 => Cost::new(0.015, 0.075),
            Model::ClaudeSonnet4 => Cost::new(0.003, 0.015),
            Model::ClaudeSonnet37 => Cost::new(0.003, 0.015),
            Model::GptOss120b => Cost::new(0.00015, 0.0006),
            _ => Cost::new(0.0, 0.0), // Unsupported models have zero cost
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::tests::{
        test_hello_world, test_multiple_tool_calls, test_reasoning_conversation,
        test_reasoning_with_tools, test_tool_usage,
    };

    async fn create_bedrock_provider() -> anyhow::Result<BedrockProvider> {
        let bedrock_config = aws_config::defaults(aws_config::BehaviorVersion::latest())
            .region(aws_config::Region::new("us-west-2"))
            .load()
            .await;
        let bedrock_client = aws_sdk_bedrockruntime::Client::new(&bedrock_config);
        Ok(BedrockProvider::new(bedrock_client))
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
}
