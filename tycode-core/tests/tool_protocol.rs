mod fixture;

use std::collections::{BTreeSet, HashMap};

use fixture::MockBehavior;
use serde_json::{json, Value};
use tycode_core::chat::events::ChatEvent;
use tycode_core::modules::image::config::Image;

const MCP_PROTOCOL_SERVER_SCRIPT: &str = r#"
process.stdin.setEncoding('utf8');
let buf = '';

process.stdin.on('data', chunk => {
    buf += chunk;
    const lines = buf.split('\n');
    buf = lines.pop();
    for (const line of lines) {
        if (line.trim()) handle(JSON.parse(line));
    }
});

function send(obj) {
    process.stdout.write(JSON.stringify(obj) + '\n');
}

function handle(msg) {
    if (msg.method === 'initialize') {
        send({ jsonrpc: '2.0', id: msg.id, result: {
            protocolVersion: '2024-11-05',
            capabilities: { tools: {} },
            serverInfo: { name: 'protocol_server', version: '1.0' }
        }});
    } else if (msg.method === 'notifications/initialized') {
        // no response
    } else if (msg.method === 'tools/list') {
        send({ jsonrpc: '2.0', id: msg.id, result: {
            tools: [{
                name: 'echo',
                description: 'Echo a message',
                inputSchema: {
                    type: 'object',
                    properties: { message: { type: 'string' } },
                    required: ['message']
                }
            }]
        }});
    } else if (msg.method === 'tools/call') {
        const message = msg.params?.arguments?.message ?? '';
        send({ jsonrpc: '2.0', id: msg.id, result: {
            content: [{ type: 'text', text: 'echo: ' + message }],
            isError: false
        }});
    } else if (msg.id !== undefined) {
        send({ jsonrpc: '2.0', id: msg.id, error: { code: -32601, message: 'Method not found' } });
    }
}
"#;

fn assert_tool_request_response_protocol(events: &[ChatEvent]) {
    let mut pending: HashMap<String, String> = HashMap::new();

    for event in events {
        match event {
            ChatEvent::ToolRequest(request) => {
                assert!(
                    pending
                        .insert(request.tool_call_id.clone(), request.tool_name.clone())
                        .is_none(),
                    "duplicate ToolRequest before ToolExecutionCompleted for id {} in events: {events:#?}",
                    request.tool_call_id
                );
            }
            ChatEvent::ToolExecutionCompleted {
                tool_call_id,
                tool_name,
                ..
            } => {
                let expected = pending.remove(tool_call_id).unwrap_or_else(|| {
                    panic!(
                        "ToolExecutionCompleted without preceding ToolRequest for id {tool_call_id} in events: {events:#?}"
                    )
                });
                assert_eq!(
                    expected, *tool_name,
                    "ToolExecutionCompleted tool_name mismatch for id {tool_call_id}"
                );
            }
            _ => {}
        }
    }

    assert!(
        pending.is_empty(),
        "ToolRequest without ToolExecutionCompleted: {pending:#?}; events: {events:#?}"
    );
}

fn assert_tool_was_covered(events: &[ChatEvent], expected_tool: &str) {
    let saw_request = events.iter().any(|event| {
        matches!(
            event,
            ChatEvent::ToolRequest(request) if request.tool_name == expected_tool
        )
    });
    let saw_completion = events.iter().any(|event| {
        matches!(
            event,
            ChatEvent::ToolExecutionCompleted { tool_name, .. } if tool_name == expected_tool
        )
    });

    assert!(
        saw_request && saw_completion,
        "expected paired ToolRequest/ToolExecutionCompleted for {expected_tool}; events: {events:#?}"
    );
}

async fn exercise_tool(
    fixture: &mut fixture::Fixture,
    tool_name: &str,
    arguments: Value,
) -> Vec<ChatEvent> {
    fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
        tool_name: tool_name.to_string(),
        tool_arguments: arguments.to_string(),
    });

    let events = fixture.step(format!("exercise {tool_name}")).await;
    assert_tool_request_response_protocol(&events);
    assert_tool_was_covered(&events, tool_name);
    events
}

#[test]
fn every_advertised_builtin_tool_emits_paired_request_and_completion() {
    fixture::run_with_agent("tycode", |mut fixture| async move {
        let workspace_path = fixture.workspace_path();
        let workspace_name = workspace_path.file_name().unwrap().to_str().unwrap();
        std::fs::write(workspace_path.join("modify_me.txt"), "before\n").unwrap();
        std::fs::write(workspace_path.join("delete_me.txt"), "delete me\n").unwrap();
        std::fs::write(workspace_path.join("image.png"), [0x89, b'P', b'N', b'G']).unwrap();

        fixture
            .update_settings(|settings| {
                let mut config: Image = settings.get_module_config("image");
                config.enabled = true;
                settings.set_module_config("image", config);
            })
            .await;
        fixture.set_image_gen_enabled(true);

        fixture.set_mock_behavior(MockBehavior::Success);
        let capture_events = fixture.step("capture advertised tools").await;
        assert_tool_request_response_protocol(&capture_events);

        let advertised_tools: BTreeSet<String> = fixture
            .get_last_ai_request()
            .expect("mock provider should capture an AI request")
            .tools
            .into_iter()
            .map(|tool| tool.name)
            .collect();

        let cases = [
            (
                "write_file",
                json!({ "file_path": "created_by_protocol_test.txt", "content": "hello\n" }),
            ),
            (
                "modify_file",
                json!({
                    "file_path": "modify_me.txt",
                    "diff": [{ "search": "before", "replace": "after" }]
                }),
            ),
            ("delete_file", json!({ "file_path": "delete_me.txt" })),
            (
                "bash",
                json!({
                    "command": "echo protocol-ok",
                    "timeout_seconds": 5,
                    "working_directory": format!("/{workspace_name}")
                }),
            ),
            (
                "manage_task_list",
                json!({
                    "title": "Protocol coverage",
                    "tasks": [{ "description": "cover tools", "status": "in_progress" }]
                }),
            ),
            (
                "ask_user_question",
                json!({ "question": "Protocol coverage question?" }),
            ),
            (
                "complete_task",
                json!({ "success": true, "result": "root task complete" }),
            ),
            (
                "append_memory",
                json!({ "content": "Protocol test memory", "source": "tool_protocol" }),
            ),
            (
                "invoke_skill",
                json!({ "skill_name": "missing-protocol-test-skill" }),
            ),
            (
                "search_types",
                json!({
                    "language": "rust",
                    "workspace_root": "missing-workspace",
                    "type_name": "Protocol"
                }),
            ),
            (
                "get_type_docs",
                json!({
                    "language": "rust",
                    "workspace_root": "missing-workspace",
                    "type_path": "Protocol"
                }),
            ),
            (
                "generate_image",
                json!({
                    "prompt": "A red protocol pixel",
                    "output_path": format!("/{workspace_name}/generated-protocol.png")
                }),
            ),
            (
                "read_image",
                json!({ "file_path": format!("/{workspace_name}/image.png") }),
            ),
        ];

        let mut covered_tools = BTreeSet::new();
        for (tool_name, arguments) in cases {
            exercise_tool(&mut fixture, tool_name, arguments).await;
            covered_tools.insert(tool_name.to_string());
        }

        fixture.set_mock_behavior(MockBehavior::BehaviorQueue {
            behaviors: vec![
                MockBehavior::ToolUse {
                    tool_name: "spawn_agent".to_string(),
                    tool_arguments: json!({
                        "agent_type": "coder",
                        "task": "Immediately call complete_task with success."
                    })
                    .to_string(),
                },
                MockBehavior::ToolUse {
                    tool_name: "complete_task".to_string(),
                    tool_arguments: json!({
                        "success": true,
                        "result": "coder complete"
                    })
                    .to_string(),
                },
                MockBehavior::Success,
            ],
        });
        let spawn_events = fixture.step("exercise spawn_agent").await;
        assert_tool_request_response_protocol(&spawn_events);
        assert_tool_was_covered(&spawn_events, "spawn_agent");
        covered_tools.insert("spawn_agent".to_string());

        let missing: BTreeSet<_> = advertised_tools
            .difference(&covered_tools)
            .cloned()
            .collect();
        assert!(
            missing.is_empty(),
            "advertised tools missing protocol coverage: {missing:#?}; advertised={advertised_tools:#?}; covered={covered_tools:#?}"
        );
    });
}

#[test]
fn invalid_tool_call_emits_paired_error_request_and_completion() {
    fixture::run(|mut fixture| async move {
        let events = exercise_tool(
            &mut fixture,
            "definitely_not_a_real_tool",
            json!({ "arg": "value" }),
        )
        .await;

        assert!(events.iter().any(|event| {
            matches!(
                event,
                ChatEvent::ToolExecutionCompleted {
                    tool_name,
                    success: false,
                    error: Some(_),
                    ..
                } if tool_name == "definitely_not_a_real_tool"
            )
        }));
    });
}

#[test]
fn mcp_tool_emits_paired_request_and_completion() {
    fixture::run(|mut fixture| async move {
        let node_check = std::process::Command::new("node").arg("--version").output();
        if node_check.is_err() || !node_check.unwrap().status.success() {
            eprintln!("Skipping mcp_tool_emits_paired_request_and_completion: node not available");
            return;
        }

        let script_path = std::env::temp_dir().join(format!(
            "tycode_mcp_protocol_server_{}.js",
            std::process::id()
        ));
        std::fs::write(&script_path, MCP_PROTOCOL_SERVER_SCRIPT).unwrap();

        let add_events = fixture
            .step(&format!(
                "/mcp add protocol_server node --args \"{}\"",
                script_path.display()
            ))
            .await;
        assert_tool_request_response_protocol(&add_events);

        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "mcp_echo".to_string(),
            tool_arguments: json!({ "message": "protocol ok" }).to_string(),
        });

        let events = fixture.step("call the MCP echo tool").await;
        assert_tool_request_response_protocol(&events);
        assert_tool_was_covered(&events, "mcp_echo");

        let _ = std::fs::remove_file(script_path);
    });
}
