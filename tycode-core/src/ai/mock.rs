use crate::ai::{error::AiError, model::Model, provider::AiProvider, types::*};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

/// Mock behavior for the mock provider
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MockBehavior {
    /// Return successful responses
    #[default]
    Success,
    /// Return a retryable error N times, then succeed
    RetryableErrorThenSuccess { remaining_errors: usize },
    /// Always return a retryable error
    AlwaysRetryableError,
    /// Always return a non-retryable error
    AlwaysNonRetryableError,
    /// Return a tool use response
    ToolUse {
        tool_name: String,
        tool_arguments: String,
    },
    /// Return a tool use response once, then success
    ToolUseThenSuccess {
        tool_name: String,
        tool_arguments: String,
    },
}

/// Mock AI provider for testing
#[derive(Clone)]
pub struct MockProvider {
    behavior: Arc<Mutex<MockBehavior>>,
    call_count: Arc<Mutex<usize>>,
}

impl MockProvider {
    pub fn new(behavior: MockBehavior) -> Self {
        Self {
            behavior: Arc::new(Mutex::new(behavior)),
            call_count: Arc::new(Mutex::new(0)),
        }
    }

    pub fn set_behavior(&self, behavior: MockBehavior) {
        *self.behavior.lock().unwrap() = behavior;
    }

    pub fn get_call_count(&self) -> usize {
        *self.call_count.lock().unwrap()
    }

    pub fn reset_call_count(&self) {
        *self.call_count.lock().unwrap() = 0;
    }
}

#[async_trait::async_trait]
impl AiProvider for MockProvider {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn supported_models(&self) -> HashSet<Model> {
        HashSet::from([Model::None])
    }

    async fn converse(
        &self,
        _request: ConversationRequest,
    ) -> Result<ConversationResponse, AiError> {
        // Increment call count
        {
            let mut count = self.call_count.lock().unwrap();
            *count += 1;
        }

        // Get current behavior
        let mut behavior = self.behavior.lock().unwrap();

        match &mut *behavior {
            MockBehavior::Success => Ok(ConversationResponse {
                content: Content::text_only("Mock response".to_string()),
                usage: TokenUsage::new(10, 10),
                stop_reason: StopReason::EndTurn,
            }),
            MockBehavior::RetryableErrorThenSuccess { remaining_errors } => {
                if *remaining_errors > 0 {
                    *remaining_errors -= 1;
                    Err(AiError::Retryable(anyhow::anyhow!(
                        "Mock retryable error (remaining: {})",
                        remaining_errors
                    )))
                } else {
                    // Success after retries
                    Ok(ConversationResponse {
                        content: Content::text_only("Success after retries".to_string()),
                        usage: TokenUsage::new(10, 10),
                        stop_reason: StopReason::EndTurn,
                    })
                }
            }
            MockBehavior::AlwaysRetryableError => Err(AiError::Retryable(anyhow::anyhow!(
                "Mock retryable error (always fails)"
            ))),
            MockBehavior::AlwaysNonRetryableError => Err(AiError::Terminal(anyhow::anyhow!(
                "Mock non-retryable error"
            ))),
            MockBehavior::ToolUse {
                tool_name,
                tool_arguments,
            } => {
                // Return a tool use response with text (like real models do)
                let tool_use = ToolUseData {
                    id: format!("tool_{tool_name}"),
                    name: tool_name.clone(),
                    arguments: serde_json::from_str(tool_arguments)
                        .unwrap_or_else(|_| serde_json::json!({})),
                };

                Ok(ConversationResponse {
                    content: Content::new(vec![
                        ContentBlock::Text(format!(
                            "I'll use the {tool_name} tool to help with this task."
                        )),
                        ContentBlock::ToolUse(tool_use),
                    ]),
                    usage: TokenUsage::new(10, 10),
                    stop_reason: StopReason::ToolUse,
                })
            }
            MockBehavior::ToolUseThenSuccess {
                tool_name,
                tool_arguments,
            } => {
                // Clone values before dropping the lock
                let tool_name_clone = tool_name.clone();
                let tool_arguments_clone = tool_arguments.clone();

                // Prepare tool use response
                let tool_use = ToolUseData {
                    id: format!("tool_{tool_name_clone}"),
                    name: tool_name_clone.clone(),
                    arguments: serde_json::from_str(&tool_arguments_clone)
                        .unwrap_or_else(|_| serde_json::json!({})),
                };

                let response = ConversationResponse {
                    content: Content::new(vec![
                        ContentBlock::Text(format!(
                            "I'll use the {tool_name_clone} tool to help with this task."
                        )),
                        ContentBlock::ToolUse(tool_use),
                    ]),
                    usage: TokenUsage::new(10, 10),
                    stop_reason: StopReason::ToolUse,
                };

                // Swap behavior to Success for next call
                drop(behavior);
                self.set_behavior(MockBehavior::Success);

                Ok(response)
            }
        }
    }

    fn get_cost(&self, _model: &Model) -> Cost {
        // Mock provider uses test costs
        Cost::new(0.001, 0.002)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_mock_provider_success() {
        let provider = MockProvider::new(MockBehavior::Success);

        let request = ConversationRequest {
            messages: vec![Message::user("Test")],
            model: Model::None.default_settings(),
            system_prompt: String::new(),
            stop_sequences: vec![],
            tools: vec![],
        };

        let response = provider.converse(request).await.unwrap();
        assert_eq!(response.content.text(), "Mock response");
        assert_eq!(provider.get_call_count(), 1);
    }

    #[tokio::test]
    async fn test_mock_provider_retry_then_success() {
        let provider = MockProvider::new(MockBehavior::RetryableErrorThenSuccess {
            remaining_errors: 2,
        });

        let request = ConversationRequest {
            messages: vec![Message::user("Test")],
            model: Model::None.default_settings(),
            system_prompt: String::new(),
            stop_sequences: vec![],
            tools: vec![],
        };

        // First call should error
        let result1 = provider.converse(request.clone()).await;
        assert!(matches!(result1, Err(AiError::Retryable(_))));

        // Second call should error
        let result2 = provider.converse(request.clone()).await;
        assert!(matches!(result2, Err(AiError::Retryable(_))));

        // Third call should succeed
        let result3 = provider.converse(request).await;
        assert!(result3.is_ok());
        assert_eq!(result3.unwrap().content.text(), "Success after retries");
        assert_eq!(provider.get_call_count(), 3);
    }
}
