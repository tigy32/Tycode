use crate::ai::{error::AiError, model::Model, provider::AiProvider, types::*};
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

fn validate_tool_use_results(messages: &[Message]) -> Result<(), AiError> {
    for (i, message) in messages.iter().enumerate() {
        if message.role != MessageRole::Assistant {
            continue;
        }

        let tool_uses = message.content.tool_uses();
        if tool_uses.is_empty() {
            continue;
        }

        let tool_use_ids: HashSet<&str> = tool_uses.iter().map(|tu| tu.id.as_str()).collect();

        let Some(next_message) = messages.get(i + 1) else {
            continue;
        };

        if next_message.role != MessageRole::User {
            let ids: Vec<&str> = tool_use_ids.into_iter().collect();
            return Err(AiError::Terminal(anyhow::anyhow!(
                "ValidationException: messages.{}: tool_use ids were found without tool_result blocks immediately after: {}. Each tool_use block must have a corresponding tool_result block in the next message",
                i,
                ids.join(", ")
            )));
        }

        let tool_results = next_message.content.tool_results();
        let result_ids: HashSet<&str> = tool_results
            .iter()
            .map(|tr| tr.tool_use_id.as_str())
            .collect();

        let missing_ids: Vec<&str> = tool_use_ids
            .iter()
            .filter(|id| !result_ids.contains(*id))
            .copied()
            .collect();

        if !missing_ids.is_empty() {
            return Err(AiError::Terminal(anyhow::anyhow!(
                "ValidationException: messages.{}: tool_use ids were found without tool_result blocks immediately after: {}. Each tool_use block must have a corresponding tool_result block in the next message",
                i,
                missing_ids.join(", ")
            )));
        }
    }

    Ok(())
}

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
    /// Always return an InputTooLong error
    AlwaysInputTooLong,
    /// Return InputTooLong error N times, then succeed
    InputTooLongThenSuccess { remaining_errors: usize },
    /// Return text-only responses N times, then a tool use response
    TextOnlyThenToolUse {
        remaining_text_responses: usize,
        tool_name: String,
        tool_arguments: String,
    },
    /// Return two tool uses in sequence, then success
    ToolUseThenToolUse {
        first_tool_name: String,
        first_tool_arguments: String,
        second_tool_name: String,
        second_tool_arguments: String,
    },
    /// Return multiple tool uses in a single response, then success
    MultipleToolUses { tool_uses: Vec<(String, String)> },
    /// Enables sequential multi-turn conversation testing by orchestrating predetermined agent responses
    BehaviorQueue { behaviors: Vec<MockBehavior> },
}

/// Mock AI provider for testing
#[derive(Clone)]
pub struct MockProvider {
    behavior: Arc<Mutex<MockBehavior>>,
    call_count: Arc<Mutex<usize>>,
    captured_requests: Arc<Mutex<Vec<ConversationRequest>>>,
}

impl MockProvider {
    pub fn new(behavior: MockBehavior) -> Self {
        Self {
            behavior: Arc::new(Mutex::new(behavior)),
            call_count: Arc::new(Mutex::new(0)),
            captured_requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn pop_behavior_from_queue(behavior: &mut MockBehavior) -> MockBehavior {
        if let MockBehavior::BehaviorQueue { behaviors } = behavior {
            if behaviors.is_empty() {
                return MockBehavior::Success;
            }
            return behaviors.remove(0);
        }
        behavior.clone()
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

    pub fn get_captured_requests(&self) -> Vec<ConversationRequest> {
        self.captured_requests.lock().unwrap().clone()
    }

    pub fn get_last_captured_request(&self) -> Option<ConversationRequest> {
        self.captured_requests.lock().unwrap().last().cloned()
    }

    pub fn clear_captured_requests(&self) {
        self.captured_requests.lock().unwrap().clear();
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
        request: ConversationRequest,
    ) -> Result<ConversationResponse, AiError> {
        validate_tool_use_results(&request.messages)?;

        // Capture the request
        {
            let mut requests = self.captured_requests.lock().unwrap();
            requests.push(request.clone());
        }

        // Increment call count
        {
            let mut count = self.call_count.lock().unwrap();
            *count += 1;
        }

        let effective = {
            let mut behavior = self.behavior.lock().unwrap();
            Self::pop_behavior_from_queue(&mut behavior)
        };

        match effective {
            MockBehavior::Success => Ok(ConversationResponse {
                content: Content::text_only("Mock response".to_string()),
                usage: TokenUsage::new(10, 10),
                stop_reason: StopReason::EndTurn,
            }),
            MockBehavior::RetryableErrorThenSuccess {
                mut remaining_errors,
            } => {
                if remaining_errors > 0 {
                    remaining_errors -= 1;
                    self.set_behavior(MockBehavior::RetryableErrorThenSuccess { remaining_errors });
                    Err(AiError::Retryable(anyhow::anyhow!(
                        "Mock retryable error (remaining: {})",
                        remaining_errors
                    )))
                } else {
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
                let tool_use = ToolUseData {
                    id: format!("tool_{tool_name}"),
                    name: tool_name.clone(),
                    arguments: serde_json::from_str(&tool_arguments)
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
                let tool_use = ToolUseData {
                    id: format!("tool_{tool_name}"),
                    name: tool_name.clone(),
                    arguments: serde_json::from_str(&tool_arguments)
                        .unwrap_or_else(|_| serde_json::json!({})),
                };

                let response = ConversationResponse {
                    content: Content::new(vec![
                        ContentBlock::Text(format!(
                            "I'll use the {tool_name} tool to help with this task."
                        )),
                        ContentBlock::ToolUse(tool_use),
                    ]),
                    usage: TokenUsage::new(10, 10),
                    stop_reason: StopReason::ToolUse,
                };

                self.set_behavior(MockBehavior::Success);
                Ok(response)
            }
            MockBehavior::AlwaysInputTooLong => Err(AiError::InputTooLong(anyhow::anyhow!(
                "Mock input too long error (always fails)"
            ))),
            MockBehavior::InputTooLongThenSuccess {
                mut remaining_errors,
            } => {
                if remaining_errors > 0 {
                    remaining_errors -= 1;
                    self.set_behavior(MockBehavior::InputTooLongThenSuccess { remaining_errors });
                    Err(AiError::InputTooLong(anyhow::anyhow!(
                        "Mock input too long error (remaining: {})",
                        remaining_errors
                    )))
                } else {
                    Ok(ConversationResponse {
                        content: Content::text_only(
                            "Success after input too long errors".to_string(),
                        ),
                        usage: TokenUsage::new(10, 10),
                        stop_reason: StopReason::EndTurn,
                    })
                }
            }
            MockBehavior::TextOnlyThenToolUse {
                mut remaining_text_responses,
                tool_name,
                tool_arguments,
            } => {
                remaining_text_responses = remaining_text_responses.saturating_sub(1);

                if remaining_text_responses == 0 {
                    self.set_behavior(MockBehavior::ToolUseThenSuccess {
                        tool_name,
                        tool_arguments,
                    });
                } else {
                    self.set_behavior(MockBehavior::TextOnlyThenToolUse {
                        remaining_text_responses,
                        tool_name,
                        tool_arguments,
                    });
                }

                Ok(ConversationResponse {
                    content: Content::text_only("Mock text response without tools".to_string()),
                    usage: TokenUsage::new(10, 10),
                    stop_reason: StopReason::EndTurn,
                })
            }
            MockBehavior::ToolUseThenToolUse {
                first_tool_name,
                first_tool_arguments,
                second_tool_name,
                second_tool_arguments,
            } => {
                let tool_use = ToolUseData {
                    id: format!("tool_{first_tool_name}"),
                    name: first_tool_name.clone(),
                    arguments: serde_json::from_str(&first_tool_arguments)
                        .unwrap_or_else(|_| serde_json::json!({})),
                };

                let response = ConversationResponse {
                    content: Content::new(vec![
                        ContentBlock::Text(format!(
                            "I'll use the {first_tool_name} tool to help with this task."
                        )),
                        ContentBlock::ToolUse(tool_use),
                    ]),
                    usage: TokenUsage::new(10, 10),
                    stop_reason: StopReason::ToolUse,
                };

                self.set_behavior(MockBehavior::ToolUseThenSuccess {
                    tool_name: second_tool_name,
                    tool_arguments: second_tool_arguments,
                });

                Ok(response)
            }
            MockBehavior::MultipleToolUses { tool_uses } => {
                let mut content_blocks = vec![ContentBlock::Text(
                    "I'll use multiple tools to help with this task.".to_string(),
                )];

                for (index, (tool_name, tool_arguments)) in tool_uses.iter().enumerate() {
                    let tool_use = ToolUseData {
                        id: format!("tool_{}_{}", tool_name, index),
                        name: tool_name.clone(),
                        arguments: serde_json::from_str(tool_arguments)
                            .unwrap_or_else(|_| serde_json::json!({})),
                    };
                    content_blocks.push(ContentBlock::ToolUse(tool_use));
                }

                self.set_behavior(MockBehavior::Success);

                Ok(ConversationResponse {
                    content: Content::new(content_blocks),
                    usage: TokenUsage::new(10, 10),
                    stop_reason: StopReason::ToolUse,
                })
            }
            MockBehavior::BehaviorQueue { .. } => {
                panic!("Bug: nested BehaviorQueue detected. Test setup error - BehaviorQueues cannot contain other BehaviorQueues")
            }
        }
    }

    fn get_cost(&self, _model: &Model) -> Cost {
        // Mock provider uses test costs
        Cost::new(0.001, 0.002, 0.0, 0.0)
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
