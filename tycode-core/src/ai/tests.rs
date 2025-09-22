use crate::ai::{model::Model, provider::AiProvider, types::*};
use anyhow::Result;

pub async fn test_hello_world_model<P: AiProvider>(provider: &P, model: Model) -> Result<()> {
    let request = ConversationRequest {
        messages: vec![Message {
            role: MessageRole::User,
            content: Content::text_only("Say hello in a friendly way.".to_string()),
        }],
        model: ModelSettings {
            model,
            max_tokens: Some(100),
            temperature: Some(0.7),
            top_p: None,
            reasoning_budget: ReasoningBudget::Off,
        },
        system_prompt: "You are a helpful AI assistant.".to_string(),
        stop_sequences: Vec::new(),
        tools: Vec::new(),
    };

    let response = provider.converse(request).await.map_err(|e| {
        tracing::error!(
            ?e,
            "Provider conversation failed for model {}",
            model.name()
        );
        anyhow::anyhow!(
            "Provider conversation failed for model {}: {:?}",
            model.name(),
            e
        )
    })?;

    assert!(
        !response.content.is_empty(),
        "Response content should not be empty for model {}",
        model.name()
    );

    assert!(
        response.usage.input_tokens > 0,
        "Should have input tokens for model {}",
        model.name()
    );

    assert!(
        response.usage.total_tokens > 0,
        "Should have total tokens for model {}",
        model.name()
    );

    assert!(
        response.usage.total_tokens >= response.usage.input_tokens,
        "Total tokens should be at least input tokens for model {}",
        model.name()
    );

    let text_content = response.content.text();

    println!("Model {}: {}", model.name(), text_content);
    println!(
        "Model {} tokens - Input: {}, Output: {}, Total: {}",
        model.name(),
        response.usage.input_tokens,
        response.usage.output_tokens,
        response.usage.total_tokens
    );

    Ok(())
}

pub async fn test_hello_world<P: AiProvider>(provider: P) -> Result<()> {
    let supported_models = provider.supported_models();

    for model in supported_models {
        println!("Testing model: {}", model.name());
        test_hello_world_model(&provider, model)
            .await
            .map_err(|e| {
                tracing::error!(?e, "Hello world test failed for model {}", model.name());
                anyhow::anyhow!(
                    "Hello world test failed for model {}: {:?}",
                    model.name(),
                    e
                )
            })?;
    }

    Ok(())
}

pub async fn test_reasoning_conversation<P: AiProvider>(provider: P) -> Result<()> {
    let supported_models = provider.supported_models();

    for model in supported_models {
        println!(
            "Testing reasoning conversation with model: {}",
            model.name()
        );

        let mut conversation_messages = Vec::new();

        conversation_messages.push(Message {
            role: MessageRole::User,
            content: Content::text_only(
                "Tell me about quantum computing in simple terms.".to_string(),
            ),
        });

        let first_request = ConversationRequest {
            messages: conversation_messages.clone(),
            model: ModelSettings {
                model,
                max_tokens: Some(9000),
                temperature: Some(1.0),
                top_p: None,
                reasoning_budget: ReasoningBudget::High,
            },
            system_prompt:
                "You are a helpful AI assistant. Think step by step and provide clear explanations."
                    .to_string(),
            stop_sequences: Vec::new(),
            tools: Vec::new(),
        };

        let first_response = provider.converse(first_request).await.map_err(|e| {
            tracing::error!(
                ?e,
                "First conversation request failed for model {}",
                model.name()
            );
            anyhow::anyhow!(
                "First conversation request failed for model {}: {:?}",
                model.name(),
                e
            )
        })?;

        assert!(
            !first_response.content.is_empty(),
            "First response should not be empty for model {}",
            model.name()
        );

        let first_text_content = first_response.content.text();
        let first_reasoning_blocks = first_response.content.reasoning();

        let response_preview = if first_text_content.len() > 100 {
            format!("{}...", &first_text_content[..100])
        } else {
            first_text_content.clone()
        };
        println!(
            "Model {} - First response: {}",
            model.name(),
            response_preview
        );

        assert!(
            !first_reasoning_blocks.is_empty(),
            "Reasoning content block should be present for model {} with reasoning budget",
            model.name()
        );

        if let Some(reasoning) = first_reasoning_blocks.first() {
            let reasoning_preview = if reasoning.text.len() > 100 {
                format!("{}...", &reasoning.text[..100])
            } else {
                reasoning.text.clone()
            };
            println!(
                "Model {} - First response reasoning: {}",
                model.name(),
                reasoning_preview
            );

            assert!(
                reasoning.text.len() > 10,
                "Reasoning content block should be substantial for model {}",
                model.name()
            );
        }

        conversation_messages.push(Message {
            role: MessageRole::Assistant,
            content: first_response.content.clone(),
        });

        conversation_messages.push(Message {
            role: MessageRole::User,
            content: Content::text_only(
                "Can you give me a practical example of how this might be used?".to_string(),
            ),
        });

        let second_request = ConversationRequest {
            messages: conversation_messages.clone(),
            model: ModelSettings {
                model,
                max_tokens: Some(9000),
                temperature: Some(1.0),
                top_p: None,
                reasoning_budget: ReasoningBudget::High,
            },
            system_prompt:
                "You are a helpful AI assistant. Think step by step and provide clear explanations."
                    .to_string(),
            stop_sequences: Vec::new(),
            tools: Vec::new(),
        };

        let second_response = provider.converse(second_request).await.map_err(|e| {
            tracing::error!(
                ?e,
                "Second conversation request failed for model {}",
                model.name()
            );
            anyhow::anyhow!(
                "Second conversation request failed for model {}: {:?}",
                model.name(),
                e
            )
        })?;

        assert!(
            !second_response.content.is_empty(),
            "Second response should not be empty for model {}",
            model.name()
        );

        let second_text_content = second_response.content.text();
        let second_reasoning_blocks = second_response.content.reasoning();

        let second_response_preview = if second_text_content.len() > 100 {
            format!("{}...", &second_text_content[..100])
        } else {
            second_text_content.clone()
        };
        println!(
            "Model {} - Second response: {}",
            model.name(),
            second_response_preview
        );

        assert!(
            !second_reasoning_blocks.is_empty(),
            "Reasoning content block should be present for model {} with reasoning budget",
            model.name()
        );

        if let Some(reasoning) = second_reasoning_blocks.first() {
            let reasoning_preview = if reasoning.text.len() > 100 {
                format!("{}...", &reasoning.text[..100])
            } else {
                reasoning.text.clone()
            };
            println!(
                "Model {} - Second response reasoning: {}",
                model.name(),
                reasoning_preview
            );

            assert!(
                reasoning.text.len() > 10,
                "Reasoning content block should be substantial for model {}",
                model.name()
            );
        }

        assert!(
            first_response.usage.total_tokens > 0,
            "First response should have token usage for model {}",
            model.name()
        );

        assert!(
            second_response.usage.total_tokens > 0,
            "Second response should have token usage for model {}",
            model.name()
        );

        println!(
            "Model {} - First response tokens: {}, Second response tokens: {}",
            model.name(),
            first_response.usage.total_tokens,
            second_response.usage.total_tokens
        );
    }

    Ok(())
}

pub async fn test_tool_usage<P: AiProvider>(provider: P) -> Result<()> {
    let supported_models = provider.supported_models();

    for model in supported_models {
        println!("Testing tool usage with model: {}", model.name());

        let calculator_tool = ToolDefinition {
            name: "calculator".to_string(),
            description: "Perform basic arithmetic calculations".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "expression": {
                        "type": "string",
                        "description": "The mathematical expression to evaluate (e.g., '2+2', '10*5')"
                    }
                },
                "required": ["expression"]
            }),
        };

        let request = ConversationRequest {
            messages: vec![Message {
                role: MessageRole::User,
                content: Content::text_only("What is 25 * 4? Please use the calculator tool to solve this.".to_string()),
            }],
            model: ModelSettings {
                model,
                max_tokens: Some(1000),
                temperature: Some(0.1),
                top_p: None,
                reasoning_budget: ReasoningBudget::Off,
            },
            system_prompt: "You are a helpful AI assistant. When asked to perform calculations, use the calculator tool provided.".to_string(),
            stop_sequences: Vec::new(),
            tools: vec![calculator_tool],
        };

        let response = provider.converse(request).await.map_err(|e| {
            tracing::error!(?e, "Tool usage test failed for model {}", model.name());
            anyhow::anyhow!("Tool usage test failed for model {}: {:?}", model.name(), e)
        })?;

        assert!(
            !response.content.is_empty(),
            "Response should not be empty for model {}",
            model.name()
        );

        let tool_uses = response.content.tool_uses();
        println!(
            "Model {} - Found {} tool use(s)",
            model.name(),
            tool_uses.len()
        );

        assert!(
            !tool_uses.is_empty(),
            "Response should contain at least one tool use for model {}",
            model.name()
        );

        let calculator_uses: Vec<_> = tool_uses
            .iter()
            .filter(|tool_use| tool_use.name == "calculator")
            .collect();

        assert!(
            !calculator_uses.is_empty(),
            "Response should contain calculator tool usage for model {}",
            model.name()
        );

        for tool_use in calculator_uses {
            println!(
                "Model {} - Calculator tool use: {} with args: {}",
                model.name(),
                tool_use.id,
                tool_use.arguments
            );

            if let Some(expression) = tool_use.arguments.get("expression") {
                println!(
                    "Model {} - Expression to calculate: {}",
                    model.name(),
                    expression
                );
            }

            assert!(
                !tool_use.id.is_empty(),
                "Tool use should have an ID for model {}",
                model.name()
            );

            assert!(
                tool_use.arguments.is_object(),
                "Tool use arguments should be an object for model {}",
                model.name()
            );
        }

        let text_content = response.content.text();
        println!(
            "Model {} - Response text: {}",
            model.name(),
            if text_content.len() > 200 {
                format!("{}...", &text_content[..200])
            } else {
                text_content
            }
        );

        println!(
            "Model {} - Tool usage test completed successfully",
            model.name()
        );
    }

    Ok(())
}

pub async fn test_reasoning_with_tools<P: AiProvider>(provider: P) -> Result<()> {
    let supported_models = provider.supported_models();

    for model in supported_models {
        println!(
            "Testing reasoning with tool usage for model: {}",
            model.name()
        );

        let calculator_tool = ToolDefinition {
            name: "calculator".to_string(),
            description: "Perform basic arithmetic calculations".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "expression": {
                        "type": "string",
                        "description": "The mathematical expression to evaluate (e.g., '2+2', '10*5')"
                    }
                },
                "required": ["expression"]
            }),
        };

        let request = ConversationRequest {
            messages: vec![Message {
                role: MessageRole::User,
                content: Content::text_only(
                    "I need to calculate the total cost for a restaurant bill. The meal cost $45.50, tax is 8.5%, and I want to leave a 20% tip on the pre-tax amount. Think through this step by step and use the calculator tool for the computations.".to_string()
                ),
            }],
            model: ModelSettings {
                model,
                max_tokens: Some(3000),
                temperature: Some(1.0),
                top_p: None,
                reasoning_budget: ReasoningBudget::Low,
            },
            system_prompt: "You are a helpful AI assistant. Think step by step when solving problems and use the calculator tool when you need to perform arithmetic calculations.".to_string(),
            stop_sequences: Vec::new(),
            tools: vec![calculator_tool],
        };

        let response = provider.converse(request).await.map_err(|e| {
            tracing::error!(
                ?e,
                "Reasoning with tools test failed for model {}",
                model.name()
            );
            anyhow::anyhow!(
                "Reasoning with tools test failed for model {}: {:?}",
                model.name(),
                e
            )
        })?;

        assert!(
            !response.content.is_empty(),
            "Response should not be empty for model {}",
            model.name()
        );

        let reasoning_blocks = response.content.reasoning();
        let tool_uses = response.content.tool_uses();
        let text_content = response.content.text();

        println!(
            "Model {} - Found {} reasoning block(s) and {} tool use(s)",
            model.name(),
            reasoning_blocks.len(),
            tool_uses.len()
        );

        assert!(
            !reasoning_blocks.is_empty(),
            "Response should contain reasoning blocks for model {} with reasoning budget",
            model.name()
        );

        assert!(
            !tool_uses.is_empty(),
            "Response should contain tool use blocks for model {}",
            model.name()
        );

        if let Some(reasoning) = reasoning_blocks.first() {
            let reasoning_preview = if reasoning.text.len() > 200 {
                format!("{}...", &reasoning.text[..200])
            } else {
                reasoning.text.clone()
            };
            println!(
                "Model {} - Reasoning content: {}",
                model.name(),
                reasoning_preview
            );

            assert!(
                reasoning.text.len() > 20,
                "Reasoning content should be substantial for model {}",
                model.name()
            );
        }

        let calculator_uses: Vec<_> = tool_uses
            .iter()
            .filter(|tool_use| tool_use.name == "calculator")
            .collect();

        assert!(
            !calculator_uses.is_empty(),
            "Response should contain calculator tool usage for model {}",
            model.name()
        );

        println!(
            "Model {} - Calculator tool calls: {}",
            model.name(),
            calculator_uses.len()
        );

        for (i, tool_use) in calculator_uses.iter().enumerate() {
            println!(
                "Model {} - Calculator call {}: {} with args: {}",
                model.name(),
                i + 1,
                tool_use.id,
                tool_use.arguments
            );

            if let Some(expression) = tool_use.arguments.get("expression") {
                println!(
                    "Model {} - Expression {}: {}",
                    model.name(),
                    i + 1,
                    expression
                );
            }
        }

        let text_preview = if text_content.len() > 300 {
            format!("{}...", &text_content[..300])
        } else {
            text_content
        };
        println!("Model {} - Response text: {}", model.name(), text_preview);

        assert!(
            response.usage.total_tokens > 0,
            "Response should have token usage for model {}",
            model.name()
        );

        println!(
            "Model {} - Reasoning with tools test completed successfully",
            model.name()
        );
        println!(
            "Model {} - Total tokens used: {}",
            model.name(),
            response.usage.total_tokens
        );
    }

    Ok(())
}

pub async fn test_multiple_tool_calls<P: AiProvider>(provider: P) -> Result<()> {
    use std::collections::HashSet;

    let supported_models = provider.supported_models();

    for model in supported_models {
        println!("Testing multiple tool calls with model: {}", model.name());

        let calculator_tool = ToolDefinition {
            name: "calculator".to_string(),
            description: "Perform basic arithmetic calculations".to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "expression": {
                        "type": "string",
                        "description": "The mathematical expression to evaluate (e.g., '2+2', '10*5')"
                    }
                },
                "required": ["expression"]
            }),
        };

        let request = ConversationRequest {
            messages: vec![Message {
                role: MessageRole::User,
                content: Content::text_only("Calculate 2+3 using the calculator tool, and also calculate 5*7 using the calculator tool.".to_string()),
            }],
            model: ModelSettings {
                model,
                max_tokens: Some(1000),
                temperature: Some(0.1),
                top_p: None,
                reasoning_budget: ReasoningBudget::Off,
            },
            system_prompt: "You are a helpful AI assistant. When asked to perform calculations, use the calculator tool provided.".to_string(),
            stop_sequences: Vec::new(),
            tools: vec![calculator_tool.clone()],
        };

        let response = provider.converse(request).await.map_err(|e| {
            tracing::error!(
                ?e,
                "Multiple tool calls test failed for model {}",
                model.name()
            );
            anyhow::anyhow!(
                "Multiple tool calls test failed for model {}: {:?}",
                model.name(),
                e
            )
        })?;

        assert!(
            !response.content.is_empty(),
            "Response should not be empty for model {}",
            model.name()
        );

        let tool_uses = response.content.tool_uses();
        println!(
            "Model {} - Found {} tool use(s)",
            model.name(),
            tool_uses.len()
        );

        assert!(
            tool_uses.len() >= 2,
            "Response should contain at least two tool uses for model {}: {response:?}",
            model.name()
        );

        let mut ids = HashSet::new();
        let mut responses = vec![];
        for tool_use in &tool_uses {
            assert!(
                tool_use.name == "calculator",
                "Each tool use should be calculator for model {}",
                model.name()
            );
            assert!(
                !tool_use.id.is_empty(),
                "Tool use should have an ID for model {}",
                model.name()
            );
            assert!(
                ids.insert(tool_use.id.clone()),
                "Tool use IDs should be unique for model {}",
                model.name()
            );
            assert!(
                tool_use.arguments.is_object(),
                "Tool use arguments should be an object for model {}",
                model.name()
            );
            println!(
                "Model {} - Tool use ID: {}, expression: {:?}",
                model.name(),
                tool_use.id,
                tool_use.arguments.get("expression")
            );

            responses.push(ContentBlock::ToolResult(ToolResultData {
                tool_use_id: tool_use.id.clone(),
                content: "35".to_string(),
                is_error: false,
            }));
        }

        responses.reverse();
        responses.push(ContentBlock::Text("These are the tool results".to_string()));
        let responses = Content::new(responses);
        let request = ConversationRequest {
            messages: vec![Message {
                role: MessageRole::User,
                content: Content::text_only("Calculate 2+3 using the calculator tool, and also calculate 5*7 using the calculator tool.".to_string()),
            }, Message { role: MessageRole::Assistant, content: response.content }, Message { role: MessageRole::User, content: responses } ],
            model: ModelSettings {
                model,
                max_tokens: Some(1000),
                temperature: Some(0.1),
                top_p: None,
                reasoning_budget: ReasoningBudget::Off,
            },
            system_prompt: "You are a helpful AI assistant. When asked to perform calculations, use the calculator tool provided.".to_string(),
            stop_sequences: Vec::new(),
            tools: vec![calculator_tool],
        };

        println!("Sending follow up request: {request:?}");
        let response = provider.converse(request).await.map_err(|e| {
            tracing::error!(
                ?e,
                "Multiple tool calls test failed for model {}",
                model.name()
            );
            anyhow::anyhow!(
                "Multiple tool calls test failed for model {}: {:?}",
                model.name(),
                e
            )
        })?;

        println!(
            "Model {} - Multiple tool calls test completed successfully: {:?}",
            model.name(),
            response
        );
    }

    Ok(())
}
