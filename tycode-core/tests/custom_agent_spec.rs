use tycode_core::{
    agents::custom::CustomAgentSpec,
    ai::mock::MockBehavior,
    chat::events::{ChatEvent, MessageSender},
};

mod fixture;

fn make_spec(name: &str, system_prompt: &str) -> CustomAgentSpec {
    CustomAgentSpec {
        name: name.to_string(),
        description: format!("Test agent: {name}"),
        system_prompt: system_prompt.to_string(),
        tools: None,
        disallowed_tools: None,
        model: None,
        max_turns: None,
    }
}

#[test]
fn test_custom_agent_spec_is_selected_and_responds() {
    let spec = make_spec("test-inline-agent", "You are a helpful test agent.");

    fixture::run_with_custom_agent_spec(spec, |mut fixture| async move {
        // Custom agents require tool use, so set appropriate mock behavior
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "complete_task".to_string(),
            tool_arguments: r#"{"success": true, "result": "done"}"#.to_string(),
        });

        let events = fixture.step("Hello").await;

        assert!(
            events.iter().any(|e| matches!(
                e,
                ChatEvent::StreamEnd { message } if matches!(message.sender, MessageSender::Assistant { .. })
            )),
            "Custom agent spec should produce an assistant response"
        );
    });
}

#[test]
fn test_custom_agent_spec_system_prompt_is_sent_to_provider() {
    let spec = make_spec("prompt-check-agent", "UNIQUE_SYSTEM_PROMPT_MARKER_12345");

    fixture::run_with_custom_agent_spec(spec, |mut fixture| async move {
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "complete_task".to_string(),
            tool_arguments: r#"{"success": true, "result": "done"}"#.to_string(),
        });

        let events = fixture.step("Hello").await;

        assert!(
            events.iter().any(|e| matches!(
                e,
                ChatEvent::StreamEnd { message } if matches!(message.sender, MessageSender::Assistant { .. })
            )),
            "Should get assistant response"
        );

        let last_request = fixture
            .get_last_ai_request()
            .expect("Should have captured an AI request");
        assert!(
            last_request
                .system_prompt
                .contains("UNIQUE_SYSTEM_PROMPT_MARKER_12345"),
            "System prompt should contain the custom agent's prompt, got: {}",
            last_request.system_prompt
        );
    });
}

#[test]
fn test_custom_agent_spec_with_disallowed_tools() {
    let spec = CustomAgentSpec {
        name: "restricted-agent".to_string(),
        description: "Agent with restricted tools".to_string(),
        system_prompt: "You are a restricted agent.".to_string(),
        tools: None,
        disallowed_tools: Some(vec!["Write".to_string(), "Edit".to_string()]),
        model: None,
        max_turns: None,
    };

    fixture::run_with_custom_agent_spec(spec, |mut fixture| async move {
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "complete_task".to_string(),
            tool_arguments: r#"{"success": true, "result": "done"}"#.to_string(),
        });

        let events = fixture.step("Do something").await;

        // Verify the agent responds
        assert!(
            events.iter().any(|e| matches!(
                e,
                ChatEvent::StreamEnd { message } if matches!(message.sender, MessageSender::Assistant { .. })
            )),
            "Should get assistant response"
        );

        // Verify disallowed tools are not in the tool definitions sent to the provider
        let request = fixture
            .get_last_ai_request()
            .expect("Should have captured an AI request");
        let tool_names: Vec<&str> = request.tools.iter().map(|t| t.name.as_str()).collect();
        assert!(
            !tool_names.contains(&"Write"),
            "Write should be excluded from tools, got: {:?}",
            tool_names
        );
        assert!(
            !tool_names.contains(&"Edit"),
            "Edit should be excluded from tools, got: {:?}",
            tool_names
        );
    });
}

#[test]
fn test_custom_agent_spec_json_round_trip() {
    let spec = CustomAgentSpec {
        name: "json-agent".to_string(),
        description: "Test JSON serialization".to_string(),
        system_prompt: "You are a JSON test agent.".to_string(),
        tools: Some(vec!["Read".to_string(), "Grep".to_string()]),
        disallowed_tools: None,
        model: Some("test-model".to_string()),
        max_turns: Some(5),
    };

    let json = serde_json::to_string(&spec).expect("Should serialize");
    let deserialized: CustomAgentSpec = serde_json::from_str(&json).expect("Should deserialize");

    assert_eq!(deserialized.name, "json-agent");
    assert_eq!(deserialized.description, "Test JSON serialization");
    assert_eq!(deserialized.system_prompt, "You are a JSON test agent.");
    assert_eq!(
        deserialized.tools,
        Some(vec!["Read".to_string(), "Grep".to_string()])
    );
    assert_eq!(deserialized.model, Some("test-model".to_string()));
    assert_eq!(deserialized.max_turns, Some(5));
}

#[test]
fn test_custom_agent_spec_camel_case_json() {
    let json = r#"{
        "name": "camel-agent",
        "description": "Test camelCase",
        "systemPrompt": "You are a camelCase test agent.",
        "disallowedTools": ["Write"]
    }"#;

    let spec: CustomAgentSpec =
        serde_json::from_str(json).expect("Should deserialize camelCase JSON");
    assert_eq!(spec.name, "camel-agent");
    assert_eq!(spec.system_prompt, "You are a camelCase test agent.");
    assert_eq!(spec.disallowed_tools, Some(vec!["Write".to_string()]));
    assert!(spec.tools.is_none());
    assert!(spec.model.is_none());
    assert!(spec.max_turns.is_none());
}
