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

use tycode_core::chat::events::{ChatEvent, MessageSender};

mod fixture;

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
            .step("/mcp add test_server /path/to/server --args \"arg1 arg2\" --env API_KEY=secret123")
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
        let events = fixture.step("/mcp add test_server /path/to/server --args").await;

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
        let events = fixture.step("/mcp add test_server /path/to/server --env").await;

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
            error_messages
                .iter()
                .any(|msg| msg.contains("KEY=VALUE")),
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
            error_messages
                .iter()
                .any(|msg| msg.contains("not found")),
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
