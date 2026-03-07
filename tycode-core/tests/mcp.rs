// MCP (Model Context Protocol) Command Tests
//
// STATUS: ✅ ALL BUGS FIXED
// See MCP_FIXES_SUMMARY.md for implementation details
//
// These tests verify:
// - List MCP servers (empty and populated)
// - Add servers (basic, with args, with env vars, combinations)
// - Update existing servers
// - Remove servers (valid and invalid)
// - Error handling (missing args, invalid formats, validation)
// - Edge cases (special chars, multiple env vars, quoted strings)
// - Persistence across operations
//
// Fixed bugs:
// ✅ Bug #1: --args and --env now properly handle quoted strings with spaces
// ✅ Bug #2: Multiple --env flags work correctly
// ✅ Bug #3: Server names are validated (cannot be empty)
// ✅ Bug #4: Command paths are validated (cannot be empty)

use tycode_core::ai::mock::MockBehavior;
use tycode_core::chat::events::{ChatEvent, MessageSender};
use tycode_core::settings::config::McpServerConfig;

mod fixture;

// Minimal MCP stdio server that returns a PNG image when `generate_image` is called.
// Uses newline-delimited JSON transport (one JSON object per line), which is the
// standard MCP stdio transport used by rmcp.
const MCP_IMAGE_SERVER_SCRIPT: &str = r#"
const TINY_PNG = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI6QAAAABJRU5ErkJggg==';

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
            serverInfo: { name: 'img_server', version: '1.0' }
        }});
    } else if (msg.method === 'notifications/initialized') {
        // no response needed for notifications
    } else if (msg.method === 'tools/list') {
        send({ jsonrpc: '2.0', id: msg.id, result: {
            tools: [{ name: 'generate_image', description: 'Generate an image', inputSchema: { type: 'object', properties: {} } }]
        }});
    } else if (msg.method === 'tools/call') {
        send({ jsonrpc: '2.0', id: msg.id, result: {
            content: [{ type: 'image', data: TINY_PNG, mimeType: 'image/png' }],
            isError: false
        }});
    } else if (msg.id !== undefined) {
        send({ jsonrpc: '2.0', id: msg.id, error: { code: -32601, message: 'Method not found' } });
    }
}
"#;

#[test]
fn test_mcp_list_when_empty() {
    fixture::run(|mut fixture| async move {
        let events = fixture.step("/mcp").await;

        let system_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(!system_messages.is_empty(), "Should receive response");
        assert!(
            system_messages
                .iter()
                .any(|msg| msg.contains("No MCP servers configured")),
            "Should indicate no servers are configured"
        );
        assert!(
            system_messages.iter().any(|msg| msg.contains("/mcp add")),
            "Should show how to add a server"
        );
    });
}

#[test]
fn test_mcp_add_basic() {
    fixture::run(|mut fixture| async move {
        let events = fixture.step("/mcp add test_server /path/to/server").await;

        let system_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(!system_messages.is_empty(), "Should receive response");
        assert!(
            system_messages
                .iter()
                .any(|msg| msg.contains("Added MCP server 'test_server'")),
            "Should confirm server was added. Got: {:?}",
            system_messages
        );
        assert!(
            system_messages
                .iter()
                .any(|msg| msg.contains("persistent across sessions")),
            "Should indicate settings were saved"
        );
    });
}

#[test]
fn test_mcp_add_with_args() {
    fixture::run(|mut fixture| async move {
        let events = fixture
            .step("/mcp add test_server /path/to/server --args \"arg1 arg2\"")
            .await;

        let system_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(
            system_messages
                .iter()
                .any(|msg| msg.contains("Added MCP server 'test_server'")),
            "Should confirm server was added with args. Got: {:?}",
            system_messages
        );
    });
}

#[test]
fn test_mcp_add_with_env() {
    fixture::run(|mut fixture| async move {
        let events = fixture
            .step("/mcp add test_server /path/to/server --env API_KEY=secret123")
            .await;

        let system_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(
            system_messages
                .iter()
                .any(|msg| msg.contains("Added MCP server 'test_server'")),
            "Should confirm server was added with env. Got: {:?}",
            system_messages
        );
    });
}

#[test]
fn test_mcp_add_with_args_and_env() {
    fixture::run(|mut fixture| async move {
        let events = fixture
            .step(
                "/mcp add test_server /path/to/server --args \"arg1 arg2\" --env API_KEY=secret123",
            )
            .await;

        let system_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(
            system_messages
                .iter()
                .any(|msg| msg.contains("Added MCP server 'test_server'")),
            "Should confirm server was added with args and env. Got: {:?}",
            system_messages
        );
    });
}

#[test]
fn test_mcp_add_multiple_env_vars() {
    fixture::run(|mut fixture| async move {
        let events = fixture
            .step("/mcp add test_server /path/to/server --env API_KEY=secret123 --env DEBUG=true")
            .await;

        let system_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(
            system_messages
                .iter()
                .any(|msg| msg.contains("Added MCP server 'test_server'")),
            "Should confirm server was added with multiple env vars. Got: {:?}",
            system_messages
        );
    });
}

#[test]
fn test_mcp_add_replaces_existing() {
    fixture::run(|mut fixture| async move {
        // Add server first time
        let events1 = fixture.step("/mcp add test_server /path/to/server1").await;
        assert!(
            events1.iter().any(|e| matches!(
                e,
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System)
                    && msg.content.contains("Added MCP server 'test_server'")
            )),
            "First add should succeed"
        );

        // Add same server again with different command
        let events2 = fixture.step("/mcp add test_server /path/to/server2").await;

        let system_messages: Vec<_> = events2
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(
            system_messages
                .iter()
                .any(|msg| msg.contains("Updated MCP server 'test_server'")),
            "Should confirm server was updated, not added. Got: {:?}",
            system_messages
        );
    });
}

#[test]
fn test_mcp_add_missing_arguments() {
    fixture::run(|mut fixture| async move {
        let events = fixture.step("/mcp add test_server").await;

        let error_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Error) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(!error_messages.is_empty(), "Should receive error message");
        assert!(
            error_messages.iter().any(|msg| msg.contains("Usage:")),
            "Should show usage message. Got: {:?}",
            error_messages
        );
    });
}

#[test]
fn test_mcp_add_args_without_value() {
    fixture::run(|mut fixture| async move {
        let events = fixture
            .step("/mcp add test_server /path/to/server --args")
            .await;

        let error_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Error) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(!error_messages.is_empty(), "Should receive error message");
        assert!(
            error_messages
                .iter()
                .any(|msg| msg.contains("--args requires a value")),
            "Should indicate --args needs a value. Got: {:?}",
            error_messages
        );
    });
}

#[test]
fn test_mcp_add_env_without_value() {
    fixture::run(|mut fixture| async move {
        let events = fixture
            .step("/mcp add test_server /path/to/server --env")
            .await;

        let error_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Error) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(!error_messages.is_empty(), "Should receive error message");
        assert!(
            error_messages
                .iter()
                .any(|msg| msg.contains("--env requires a value")),
            "Should indicate --env needs a value. Got: {:?}",
            error_messages
        );
    });
}

#[test]
fn test_mcp_add_env_invalid_format() {
    fixture::run(|mut fixture| async move {
        let events = fixture
            .step("/mcp add test_server /path/to/server --env INVALID_FORMAT")
            .await;

        let error_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Error) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(!error_messages.is_empty(), "Should receive error message");
        assert!(
            error_messages.iter().any(|msg| msg.contains("KEY=VALUE")),
            "Should indicate env var must be in KEY=VALUE format. Got: {:?}",
            error_messages
        );
    });
}

#[test]
fn test_mcp_add_unknown_argument() {
    fixture::run(|mut fixture| async move {
        let events = fixture
            .step("/mcp add test_server /path/to/server --unknown arg")
            .await;

        let error_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Error) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(!error_messages.is_empty(), "Should receive error message");
        assert!(
            error_messages
                .iter()
                .any(|msg| msg.contains("Unknown argument")),
            "Should indicate unknown argument. Got: {:?}",
            error_messages
        );
    });
}

#[test]
fn test_mcp_list_after_add() {
    fixture::run(|mut fixture| async move {
        // Add a server
        fixture.step("/mcp add test_server /path/to/server").await;

        // List servers
        let events = fixture.step("/mcp").await;

        let system_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        let response = system_messages.join("\n");

        assert!(
            response.contains("Configured MCP servers"),
            "Should show configured servers header"
        );
        assert!(
            response.contains("test_server"),
            "Should list the added server"
        );
        assert!(
            response.contains("/path/to/server"),
            "Should show the server command"
        );
    });
}

#[test]
fn test_mcp_list_shows_args_and_env() {
    fixture::run(|mut fixture| async move {
        // Add server with args and env
        fixture
            .step("/mcp add test_server /path/to/server --args \"arg1 arg2\" --env API_KEY=secret")
            .await;

        // List servers
        let events = fixture.step("/mcp").await;

        let system_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        let response = system_messages.join("\n");

        assert!(response.contains("test_server"), "Should list the server");
        assert!(response.contains("Args:"), "Should show args section");
        assert!(response.contains("Env:"), "Should show env section");
    });
}

#[test]
fn test_mcp_remove_existing_server() {
    fixture::run(|mut fixture| async move {
        // Add a server first
        fixture.step("/mcp add test_server /path/to/server").await;

        // Remove it
        let events = fixture.step("/mcp remove test_server").await;

        let system_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(
            system_messages
                .iter()
                .any(|msg| msg.contains("Removed MCP server 'test_server'")),
            "Should confirm server was removed. Got: {:?}",
            system_messages
        );
        assert!(
            system_messages
                .iter()
                .any(|msg| msg.contains("persistent across sessions")),
            "Should indicate settings were saved"
        );
    });
}

#[test]
fn test_mcp_remove_nonexistent_server() {
    fixture::run(|mut fixture| async move {
        let events = fixture.step("/mcp remove nonexistent").await;

        let error_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Error) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(!error_messages.is_empty(), "Should receive error message");
        assert!(
            error_messages.iter().any(|msg| msg.contains("not found")),
            "Should indicate server was not found. Got: {:?}",
            error_messages
        );
    });
}

#[test]
fn test_mcp_remove_missing_name() {
    fixture::run(|mut fixture| async move {
        let events = fixture.step("/mcp remove").await;

        let error_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Error) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(!error_messages.is_empty(), "Should receive error message");
        assert!(
            error_messages.iter().any(|msg| msg.contains("Usage:")),
            "Should show usage message. Got: {:?}",
            error_messages
        );
    });
}

#[test]
fn test_mcp_list_after_remove() {
    fixture::run(|mut fixture| async move {
        // Add two servers
        fixture.step("/mcp add server1 /path/to/server1").await;
        fixture.step("/mcp add server2 /path/to/server2").await;

        // Remove one
        fixture.step("/mcp remove server1").await;

        // List servers
        let events = fixture.step("/mcp").await;

        let system_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        let response = system_messages.join("\n");

        assert!(
            !response.contains("server1"),
            "Should not list removed server"
        );
        assert!(response.contains("server2"), "Should still list server2");
    });
}

#[test]
fn test_mcp_invalid_subcommand() {
    fixture::run(|mut fixture| async move {
        let events = fixture.step("/mcp invalid").await;

        let error_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Error) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(!error_messages.is_empty(), "Should receive error message");
        assert!(
            error_messages.iter().any(|msg| msg.contains("Usage:")),
            "Should show usage message. Got: {:?}",
            error_messages
        );
    });
}

#[test]
fn test_mcp_add_with_env_containing_equals() {
    fixture::run(|mut fixture| async move {
        // Test that env values can contain = signs (like base64 strings)
        let events = fixture
            .step("/mcp add test_server /path/to/server --env TOKEN=abc=def=ghi")
            .await;

        let system_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(
            system_messages
                .iter()
                .any(|msg| msg.contains("Added MCP server 'test_server'")),
            "Should handle env values with = signs. Got: {:?}",
            system_messages
        );
    });
}

#[test]
fn test_mcp_server_name_with_special_characters() {
    fixture::run(|mut fixture| async move {
        // Test server names with dashes, underscores, etc.
        let events = fixture
            .step("/mcp add my-test_server.v1 /path/to/server")
            .await;

        let system_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(
            system_messages
                .iter()
                .any(|msg| msg.contains("Added MCP server 'my-test_server.v1'")),
            "Should handle server names with special characters. Got: {:?}",
            system_messages
        );
    });
}

#[test]
fn test_mcp_persistence_across_operations() {
    fixture::run(|mut fixture| async move {
        // Add multiple servers
        fixture.step("/mcp add server1 /path/to/server1").await;
        fixture.step("/mcp add server2 /path/to/server2").await;
        fixture.step("/mcp add server3 /path/to/server3").await;

        // Remove one
        fixture.step("/mcp remove server2").await;

        // Update one
        fixture.step("/mcp add server1 /new/path/to/server1").await;

        // List and verify final state
        let events = fixture.step("/mcp").await;

        let system_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        let response = system_messages.join("\n");

        assert!(response.contains("server1"), "Should have server1");
        assert!(
            !response.contains("server2"),
            "Should not have removed server2"
        );
        assert!(response.contains("server3"), "Should have server3");
        assert!(
            response.contains("/new/path/to/server1"),
            "Should show updated path for server1"
        );
    });
}

#[test]
fn mcp_image_tool_result_reaches_model() {
    fixture::run(|mut fixture| async move {
        let node_check = std::process::Command::new("node").arg("--version").output();
        if node_check.is_err() || !node_check.unwrap().status.success() {
            eprintln!("Skipping mcp_image_tool_result_reaches_model: node not available");
            return;
        }

        let script_path = std::env::temp_dir().join("tycode_mcp_image_server.js");
        std::fs::write(&script_path, MCP_IMAGE_SERVER_SCRIPT).unwrap();

        let events = fixture
            .step(&format!(
                "/mcp add img_server node --args \"{}\"",
                script_path.display()
            ))
            .await;

        assert!(
            events.iter().any(|e| matches!(
                e,
                ChatEvent::MessageAdded(msg)
                    if matches!(msg.sender, MessageSender::System)
                    && msg.content.contains("Added MCP server 'img_server'")
            )),
            "MCP server should be added. Events: {:?}",
            events
        );

        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "mcp_generate_image".to_string(),
            tool_arguments: "{}".to_string(),
        });

        fixture.step("Generate an image").await;

        let request = fixture
            .get_last_ai_request()
            .expect("Should have AI request after tool execution");

        let has_image = request
            .messages
            .iter()
            .any(|msg| !msg.content.images().is_empty());

        assert!(
            has_image,
            "Follow-up AI request should contain ContentBlock::Image from MCP tool result, \
             but only found text. This confirms the bug: mcp/tool.rs discards image bytes."
        );
    });
}

#[test]
fn end_to_end_mcp_call() {
    fixture::run(|mut fixture| async move {
        // npx may be unavailable in CI/CD environments without Node.js
        let npx_check = std::process::Command::new("npx").arg("--version").output();
        if npx_check.is_err() || !npx_check.unwrap().status.success() {
            eprintln!("Skipping end_to_end_mcp_call: npx not available");
            return;
        }

        // Add the official MCP filesystem server
        let events = fixture
            .step("/mcp add fs_server npx --args \"@modelcontextprotocol/server-filesystem /tmp\"")
            .await;

        // Verify server was added
        let system_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(
            system_messages
                .iter()
                .any(|msg| msg.contains("Added MCP server 'fs_server'")),
            "MCP server should be added. Got: {:?}",
            system_messages
        );

        // Set mock to call an MCP tool
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "mcp_list_directory".to_string(),
            tool_arguments: r#"{"path": "/tmp"}"#.to_string(),
        });

        // Send a message - the mock will respond with mcp_list_directory tool call
        let events = fixture.step("List the files in /tmp").await;

        // Verify the MCP tool was executed by checking for ToolExecutionCompleted
        let tool_completed = events.iter().any(|e| {
            matches!(
                e,
                ChatEvent::ToolExecutionCompleted {
                    tool_name,
                    success: true,
                    ..
                } if tool_name == "mcp_list_directory"
            )
        });

        assert!(
            tool_completed,
            "mcp_list_directory should have executed successfully. Events: {:?}",
            events
                .iter()
                .filter(|e| matches!(e, ChatEvent::ToolExecutionCompleted { .. }))
                .collect::<Vec<_>>()
        );
    });
}

#[test]
fn mcp_tools_visible_in_ai_prompt() {
    fixture::run(|mut fixture| async move {
        // npx may be unavailable in CI/CD environments without Node.js
        let npx_check = std::process::Command::new("npx").arg("--version").output();
        if npx_check.is_err() || !npx_check.unwrap().status.success() {
            eprintln!("Skipping mcp_tools_visible_in_ai_prompt: npx not available");
            return;
        }

        // Add the official MCP filesystem server
        fixture
            .step("/mcp add fs_server npx --args \"@modelcontextprotocol/server-filesystem /tmp\"")
            .await;

        // Send a message to trigger an AI request
        fixture.step("Hello").await;

        // Get the last AI request
        let request = fixture
            .get_last_ai_request()
            .expect("Should have AI request");

        // Check that MCP tools are in the tools list
        let mcp_tools: Vec<_> = request
            .tools
            .iter()
            .filter(|t| t.name.starts_with("mcp_"))
            .collect();

        assert!(
            !mcp_tools.is_empty(),
            "MCP tools should appear in AI prompt's available tools. Found tools: {:?}",
            request.tools.iter().map(|t| &t.name).collect::<Vec<_>>()
        );
    });
}

// ============================================================
// HTTP MCP server command tests
// ============================================================

#[test]
fn test_mcp_add_http_basic() {
    fixture::run(|mut fixture| async move {
        let events = fixture
            .step("/mcp add remote --url http://localhost:8000/mcp")
            .await;

        let system_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(
            system_messages
                .iter()
                .any(|msg| msg.contains("Added MCP server 'remote'")),
            "Should confirm HTTP server was added. Got: {:?}",
            system_messages
        );
    });
}

#[test]
fn test_mcp_add_http_with_header() {
    fixture::run(|mut fixture| async move {
        let events = fixture
            .step("/mcp add remote --url http://localhost:8000/mcp --header \"Authorization: Bearer tok123\"")
            .await;

        let system_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(
            system_messages
                .iter()
                .any(|msg| msg.contains("Added MCP server 'remote'")),
            "Should confirm HTTP server with header was added. Got: {:?}",
            system_messages
        );
    });
}

#[test]
fn test_mcp_add_http_with_multiple_headers() {
    fixture::run(|mut fixture| async move {
        let events = fixture
            .step("/mcp add remote --url http://localhost:8000/mcp --header \"Authorization: Bearer tok\" --header \"X-Custom: value\"")
            .await;

        let system_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(
            system_messages
                .iter()
                .any(|msg| msg.contains("Added MCP server 'remote'")),
            "Should confirm HTTP server with multiple headers was added. Got: {:?}",
            system_messages
        );
    });
}

#[test]
fn test_mcp_add_http_missing_url_value() {
    fixture::run(|mut fixture| async move {
        let events = fixture.step("/mcp add remote --url").await;

        let error_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Error) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(!error_messages.is_empty(), "Should receive error message");
        assert!(
            error_messages
                .iter()
                .any(|msg| msg.contains("--url requires a URL value")),
            "Should indicate --url needs a value. Got: {:?}",
            error_messages
        );
    });
}

#[test]
fn test_mcp_add_http_header_without_value() {
    fixture::run(|mut fixture| async move {
        let events = fixture
            .step("/mcp add remote --url http://localhost:8000/mcp --header")
            .await;

        let error_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Error) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(!error_messages.is_empty(), "Should receive error message");
        assert!(
            error_messages
                .iter()
                .any(|msg| msg.contains("--header requires a value")),
            "Should indicate --header needs a value. Got: {:?}",
            error_messages
        );
    });
}

#[test]
fn test_mcp_add_http_header_invalid_format() {
    fixture::run(|mut fixture| async move {
        let events = fixture
            .step("/mcp add remote --url http://localhost:8000/mcp --header \"NoColonHere\"")
            .await;

        let error_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Error) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(!error_messages.is_empty(), "Should receive error message");
        assert!(
            error_messages.iter().any(|msg| msg.contains("Name: Value")),
            "Should indicate header must be in Name: Value format. Got: {:?}",
            error_messages
        );
    });
}

#[test]
fn test_mcp_add_http_unknown_argument() {
    fixture::run(|mut fixture| async move {
        let events = fixture
            .step("/mcp add remote --url http://localhost:8000/mcp --unknown arg")
            .await;

        let error_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Error) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(!error_messages.is_empty(), "Should receive error message");
        assert!(
            error_messages
                .iter()
                .any(|msg| msg.contains("Unknown argument")),
            "Should indicate unknown argument. Got: {:?}",
            error_messages
        );
    });
}

#[test]
fn test_mcp_list_shows_http_server() {
    fixture::run(|mut fixture| async move {
        // Add an HTTP server
        fixture
            .step("/mcp add remote --url http://localhost:8000/mcp")
            .await;

        // List servers
        let events = fixture.step("/mcp").await;

        let system_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        let response = system_messages.join("\n");

        assert!(response.contains("remote"), "Should list the HTTP server");
        assert!(
            response.contains("Type: http"),
            "Should show type as http. Got: {}",
            response
        );
        assert!(
            response.contains("http://localhost:8000/mcp"),
            "Should show the URL"
        );
    });
}

#[test]
fn test_mcp_list_shows_mixed_servers() {
    fixture::run(|mut fixture| async move {
        // Add a stdio server
        fixture.step("/mcp add local_server /path/to/server").await;

        // Add an HTTP server
        fixture
            .step("/mcp add remote_server --url http://localhost:8000/mcp")
            .await;

        // List servers
        let events = fixture.step("/mcp").await;

        let system_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        let response = system_messages.join("\n");

        assert!(
            response.contains("local_server"),
            "Should list stdio server"
        );
        assert!(
            response.contains("remote_server"),
            "Should list HTTP server"
        );
        assert!(response.contains("Type: stdio"), "Should show stdio type");
        assert!(response.contains("Type: http"), "Should show http type");
    });
}

#[test]
fn test_mcp_remove_http_server() {
    fixture::run(|mut fixture| async move {
        // Add an HTTP server
        fixture
            .step("/mcp add remote --url http://localhost:8000/mcp")
            .await;

        // Remove it
        let events = fixture.step("/mcp remove remote").await;

        let system_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(
            system_messages
                .iter()
                .any(|msg| msg.contains("Removed MCP server 'remote'")),
            "Should confirm HTTP server was removed. Got: {:?}",
            system_messages
        );

        // Verify it's gone
        let events = fixture.step("/mcp").await;
        let list_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        let response = list_messages.join("\n");
        assert!(
            !response.contains("remote"),
            "Removed HTTP server should not appear in list"
        );
    });
}

#[test]
fn test_mcp_replace_stdio_with_http() {
    fixture::run(|mut fixture| async move {
        // Add a stdio server
        fixture.step("/mcp add myserver /path/to/server").await;

        // Replace with HTTP
        let events = fixture
            .step("/mcp add myserver --url http://localhost:8000/mcp")
            .await;

        let system_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(
            system_messages
                .iter()
                .any(|msg| msg.contains("Updated MCP server 'myserver'")),
            "Should confirm server was updated. Got: {:?}",
            system_messages
        );

        // Verify it's now HTTP
        let events = fixture.step("/mcp").await;
        let list_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        let response = list_messages.join("\n");
        assert!(response.contains("Type: http"), "Should now be HTTP type");
        assert!(
            response.contains("http://localhost:8000/mcp"),
            "Should show the URL"
        );
    });
}

#[test]
fn test_mcp_list_empty_shows_both_syntaxes() {
    fixture::run(|mut fixture| async move {
        let events = fixture.step("/mcp").await;

        let system_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        let response = system_messages.join("\n");
        assert!(
            response.contains("--url"),
            "Empty list help should mention --url syntax. Got: {}",
            response
        );
    });
}

// ============================================================
// McpServerConfig serialization/deserialization tests
// ============================================================

#[test]
fn test_config_stdio_backward_compat() {
    let toml_content = r#"
[mcp_servers.fetch]
command = "uvx"
args = ["mcp-server-fetch"]
    "#;

    #[derive(serde::Deserialize)]
    struct Partial {
        mcp_servers: std::collections::HashMap<String, McpServerConfig>,
    }

    let parsed: Partial = toml::from_str(toml_content).expect("Should parse stdio config");
    match &parsed.mcp_servers["fetch"] {
        McpServerConfig::Stdio { command, args, .. } => {
            assert_eq!(command, "uvx");
            assert_eq!(args, &["mcp-server-fetch"]);
        }
        other => panic!("Expected Stdio variant, got: {:?}", other),
    }
}

#[test]
fn test_config_http_deserialization() {
    let toml_content = r#"
[mcp_servers.remote]
url = "http://localhost:8000/mcp"

[mcp_servers.remote.headers]
Authorization = "Bearer token123"
    "#;

    #[derive(serde::Deserialize)]
    struct Partial {
        mcp_servers: std::collections::HashMap<String, McpServerConfig>,
    }

    let parsed: Partial = toml::from_str(toml_content).expect("Should parse HTTP config");
    match &parsed.mcp_servers["remote"] {
        McpServerConfig::Http { url, headers } => {
            assert_eq!(url, "http://localhost:8000/mcp");
            assert_eq!(headers.get("Authorization").unwrap(), "Bearer token123");
        }
        other => panic!("Expected Http variant, got: {:?}", other),
    }
}

#[test]
fn test_config_mixed_deserialization() {
    let toml_content = r#"
[mcp_servers.stdio_server]
command = "npx"
args = ["-y", "@modelcontextprotocol/server-filesystem"]

[mcp_servers.http_server]
url = "https://api.example.com/mcp"
    "#;

    #[derive(serde::Deserialize)]
    struct Partial {
        mcp_servers: std::collections::HashMap<String, McpServerConfig>,
    }

    let parsed: Partial = toml::from_str(toml_content).expect("Should parse mixed config");
    assert!(
        matches!(
            parsed.mcp_servers["stdio_server"],
            McpServerConfig::Stdio { .. }
        ),
        "stdio_server should be Stdio variant"
    );
    assert!(
        matches!(
            parsed.mcp_servers["http_server"],
            McpServerConfig::Http { .. }
        ),
        "http_server should be Http variant"
    );
}

#[test]
fn test_config_http_roundtrip() {
    let config = McpServerConfig::Http {
        url: "http://localhost:8000/mcp".to_string(),
        headers: {
            let mut h = std::collections::HashMap::new();
            h.insert("Authorization".to_string(), "Bearer secret".to_string());
            h
        },
    };

    let serialized = toml::to_string(&config).expect("Should serialize HTTP config");
    let deserialized: McpServerConfig =
        toml::from_str(&serialized).expect("Should deserialize HTTP config");

    match deserialized {
        McpServerConfig::Http { url, headers } => {
            assert_eq!(url, "http://localhost:8000/mcp");
            assert_eq!(headers.get("Authorization").unwrap(), "Bearer secret");
        }
        other => panic!("Expected Http variant after roundtrip, got: {:?}", other),
    }
}

#[test]
fn test_config_stdio_roundtrip() {
    let config = McpServerConfig::Stdio {
        command: "uvx".to_string(),
        args: vec!["mcp-server-fetch".to_string()],
        env: std::collections::HashMap::new(),
    };

    let serialized = toml::to_string(&config).expect("Should serialize Stdio config");
    let deserialized: McpServerConfig =
        toml::from_str(&serialized).expect("Should deserialize Stdio config");

    match deserialized {
        McpServerConfig::Stdio { command, args, .. } => {
            assert_eq!(command, "uvx");
            assert_eq!(args, vec!["mcp-server-fetch"]);
        }
        other => panic!("Expected Stdio variant after roundtrip, got: {:?}", other),
    }
}

// ============================================================
// HTTP MCP end-to-end integration tests
// ============================================================

/// Helper: spawn the everything MCP server in streamableHttp mode and wait for it to be ready.
/// Returns the child process handle. Caller is responsible for killing it.
fn spawn_everything_http_server() -> Option<std::process::Child> {
    // Check npx is available
    let npx_check = std::process::Command::new("npx").arg("--version").output();
    if npx_check.is_err() || !npx_check.unwrap().status.success() {
        return None;
    }

    // Ensure port 3001 is free (kill any leftover from a previous run)
    let _ = std::process::Command::new("lsof")
        .args(["-ti", ":3001"])
        .output()
        .ok()
        .and_then(|o| {
            if o.status.success() {
                let pids = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if !pids.is_empty() {
                    let _ = std::process::Command::new("kill")
                        .args(pids.split_whitespace())
                        .output();
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                Some(())
            } else {
                None
            }
        });

    let child = std::process::Command::new("npx")
        .args(["@modelcontextprotocol/server-everything", "streamableHttp"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;

    // Wait for the server to be ready (poll the port)
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(30);
    while start.elapsed() < timeout {
        if std::net::TcpStream::connect("127.0.0.1:3001").is_ok() {
            // Give it a moment to fully initialize
            std::thread::sleep(std::time::Duration::from_millis(500));
            return Some(child);
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }

    eprintln!("Timed out waiting for everything server on port 3001");
    None
}

#[test]
fn test_http_mcp_end_to_end() {
    let mut server = match spawn_everything_http_server() {
        Some(s) => s,
        None => {
            eprintln!("Skipping test_http_mcp_end_to_end: npx or server not available");
            return;
        }
    };

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        fixture::run(|mut fixture| async move {
            // Add the HTTP MCP server
            let events = fixture
                .step("/mcp add everything --url http://127.0.0.1:3001/mcp")
                .await;

            let system_messages: Vec<_> = events
                .iter()
                .filter_map(|e| match e {
                    ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                        Some(msg.content.as_str())
                    }
                    _ => None,
                })
                .collect();

            assert!(
                system_messages
                    .iter()
                    .any(|msg| msg.contains("Added MCP server 'everything'")),
                "HTTP MCP server should be added. Got: {:?}",
                system_messages
            );

            // Verify tools are visible to the AI by sending a message
            fixture.step("Hello").await;

            let request = fixture
                .get_last_ai_request()
                .expect("Should have AI request");

            let mcp_tools: Vec<_> = request
                .tools
                .iter()
                .filter(|t| t.name.starts_with("mcp_"))
                .map(|t| &t.name)
                .collect();

            assert!(
                mcp_tools.iter().any(|name| name.as_str() == "mcp_echo"),
                "mcp_echo tool should be available via HTTP transport. Found tools: {:?}",
                mcp_tools
            );

            // Actually call the echo tool end-to-end
            fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
                tool_name: "mcp_echo".to_string(),
                tool_arguments: r#"{"message": "hello from http test"}"#.to_string(),
            });

            let events = fixture.step("Echo something for me").await;

            let tool_completed = events.iter().any(|e| {
                matches!(
                    e,
                    ChatEvent::ToolExecutionCompleted {
                        tool_name,
                        success: true,
                        ..
                    } if tool_name == "mcp_echo"
                )
            });

            assert!(
                tool_completed,
                "mcp_echo should have executed successfully via HTTP transport. Events: {:?}",
                events
                    .iter()
                    .filter(|e| matches!(e, ChatEvent::ToolExecutionCompleted { .. }))
                    .collect::<Vec<_>>()
            );
        });
    }));

    let _ = server.kill();
    let _ = server.wait();

    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}
