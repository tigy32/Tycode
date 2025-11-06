# Testing Guide for Tycode

## Testing Philosophy

All tests in this project are **end-to-end tests** that interact with the system 
only through the public API: the `ChatActor` and its event stream. This design 
provides several critical benefits:

### Why End-to-End Tests?

1. **Covariant with Implementation**: Tests are written against behavior, not 
implementation. You can completely refactor internal code without breaking 
tests, as long as the external behavior remains the same.

2. **No Tight Coupling**: Traditional unit tests that reach into internal 
functions, mock private dependencies, or test implementation details become 
obsolete when you refactor. Our tests remain valid regardless of how you 
restructure the internals.

## Test Architecture

### The Fixture Pattern

All tests use the `Fixture` struct from `tests/fixture.rs`, which provides:

```rust
pub struct Fixture {
    pub actor: ChatActor,              // The actor handle for sending messages
    pub event_rx: UnboundedReceiver<ChatEvent>,  // Event stream from the actor
    pub workspace_dir: TempDir,        // Isolated workspace for each test
    pub sessions_dir: PathBuf,         // Sessions directory
    mock_provider: MockProvider,       // Controllable mock AI provider
}
```

### Key Testing Utilities

#### `Fixture::step(message)` - Drive Conversation Forward

The `step()` method is the primary way to interact with the actor in tests:

```rust
let events = fixture.step("Hello").await;
```

What `step()` does:
1. Sends the message to the actor
2. Waits for typing to start (TypingStatusChanged(true))
3. Collects all events until typing stops (TypingStatusChanged(false))
4. Returns only non-typing events for easier assertion

This simulates a complete user interaction cycle and returns all meaningful events.

#### Mock Behaviors

The `MockProvider` allows deterministic AI responses:

```rust
// Simple success response
fixture.set_mock_behavior(MockBehavior::Success);

// Tool use followed by success
fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
    tool_name: "set_tracked_files".to_string(),
    tool_arguments: r#"{"file_paths": ["src/main.rs"]}"#.to_string(),
});

// Multiple tool uses
fixture.set_mock_behavior(MockBehavior::ToolUseThenToolUse {
    first_tool_name: "nonexistent_tool".to_string(),
    first_tool_arguments: r#"{"foo": "bar"}"#.to_string(),
    second_tool_name: "set_tracked_files".to_string(),
    second_tool_arguments: r#"{"file_paths": []}"#.to_string(),
});

// Error conditions
fixture.set_mock_behavior(MockBehavior::InputTooLongThenSuccess {
    remaining_errors: 1,
});
```

## How to Write a Test

### Basic Test Structure

```rust
use tycode_core::chat::events::{ChatEvent, MessageSender};

mod fixture;

#[test]
fn test_feature_name() {
    fixture::run(|mut fixture| async move {
        // Setup: Configure mock behavior if needed
        fixture.set_mock_behavior(MockBehavior::Success);

        // Action: Send a message and drive the conversation
        let events = fixture.step("User message").await;

        // Assert: Verify the expected events were received
        assert!(
            events.iter().any(|e| {
                matches!(
                    e,
                    ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Assistant { .. })
                )
            }),
            "Should receive assistant message"
        );
    });
}
```

### Testing with Specific Agents

```rust
#[test]
fn test_coder_agent_behavior() {
    fixture::run_with_agent("coder", |mut fixture| async move {
        // Test runs with the "coder" agent instead of default "one_shot"
        let events = fixture.step("Write a function").await;
        // Assertions...
    });
}
```

### Testing Commands

```rust
#[test]
fn test_help_command() {
    fixture::run(|mut fixture| async move {
        let events = fixture.step("/help").await;

        // Extract system messages
        let help_content = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.clone())
                }
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(help_content.contains("Available commands"));
    });
}
```

### Testing Error Recovery

```rust
#[test]
fn test_invalid_tool_calls_continue_conversation() {
    fixture::run(|mut fixture| async move {
        // Configure mock to return invalid tool call, then valid one
        fixture.set_mock_behavior(MockBehavior::ToolUseThenToolUse {
            first_tool_name: "nonexistent_tool".to_string(),
            first_tool_arguments: r#"{"foo": "bar"}"#.to_string(),
            second_tool_name: "set_tracked_files".to_string(),
            second_tool_arguments: r#"{"file_paths": []}"#.to_string(),
        });

        let events = fixture.step("Use a tool").await;

        // Verify conversation continued after error
        let assistant_message_count = events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Assistant { .. })
                )
            })
            .count();

        assert!(
            assistant_message_count >= 2,
            "Expected at least 2 assistant messages (initial + continuation after error)"
        );
    });
}
```

### Testing with File System State

```rust
#[test]
fn test_workspace_files() {
    fixture::run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();

        // Create test files
        std::fs::write(workspace_path.join("test.rs"), "fn main() {}").unwrap();

        let events = fixture.step("/context").await;

        let response_text: String = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) => Some(msg.content.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(response_text.contains("test.rs"));
    });
}
```

### Testing Session Persistence

```rust
#[test]
fn test_session_auto_save() {
    fixture::run(|mut fixture| async move {
        let events = fixture.step("Hello test").await;

        // Verify we got response
        assert!(events.iter().any(|e| matches!(
            e,
            ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Assistant { .. })
        )));

        // Give time for auto-save
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // Verify session was saved
        let sessions_dir = fixture.sessions_dir();
        let sessions = storage::list_sessions(Some(&sessions_dir)).unwrap();
        assert_eq!(sessions.len(), 1, "Expected exactly one session to be saved");
    });
}
```

## Common Patterns

### Event Filtering

```rust
// Check for any assistant message
let has_assistant_response = events.iter().any(|e| {
    matches!(
        e,
        ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Assistant { .. })
    )
});

// Extract all system messages
let system_messages: Vec<String> = events
    .iter()
    .filter_map(|e| match e {
        ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
            Some(msg.content.clone())
        }
        _ => None,
    })
    .collect();

// Check for tool execution
let has_tool_request = events.iter().any(|e| matches!(e, ChatEvent::ToolRequest(_)));
```

### Multi-Step Conversations

```rust
// First interaction
let events1 = fixture.step("First message").await;
assert!(!events1.is_empty());

// Change mock behavior for second interaction
fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
    tool_name: "some_tool".to_string(),
    tool_arguments: r#"{"arg": "value"}"#.to_string(),
});

// Second interaction builds on conversation history
let events2 = fixture.step("Second message").await;
```

## What NOT to Test

❌ **Don't test internal functions directly**
```rust
// BAD: Testing internal implementation
#[test]
fn test_internal_compaction_logic() {
    let result = internal::compact_messages(messages);  // Don't do this!
}
```

❌ **Don't mock internal dependencies**
```rust
// BAD: Mocking internal components
#[test]
fn test_with_mocked_internals() {
    let mock_context = MockContext::new();  // Don't do this!
    let result = some_internal_function(mock_context);
}
```

❌ **Don't test implementation details**
```rust
// BAD: Testing how something is implemented
#[test]
fn test_uses_specific_data_structure() {
    assert!(actor.internal_state.uses_hashmap());  // Don't do this!
}
```

✅ **Do test observable behavior**
```rust
// GOOD: Testing user-facing behavior
#[test]
fn test_handles_large_context() {
    fixture::run(|mut fixture| async move {
        // Setup large context scenario
        fixture.set_mock_behavior(MockBehavior::InputTooLongThenSuccess {
            remaining_errors: 1,
        });

        // Verify it handles the situation gracefully
        let events = fixture.step("Message").await;
        assert!(events.iter().any(|e| matches!(
            e,
            ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Assistant { .. })
        )));
    });
}
```

## Benefits of This Approach

1. **Fearless Refactoring**: Change internal implementation without updating tests
2. **Meaningful Failures**: When tests fail, it means user-facing behavior broke
3. **Documentation**: Tests show how the system actually behaves
4. **Regression Prevention**: Behavior remains consistent across refactorings
5. **Faster Development**: Spend time building features, not maintaining brittle tests

## Running Tests

```bash
# Run all tests
cargo test

# Run specific test file
cargo test --test basic

# Run specific test
cargo test test_fixture

# Run with output
cargo test -- --nocapture
```

## Summary

Write tests that verify **what the system does**, not **how it does it**. Use the `Fixture` to interact with the `ChatActor` through its public API, drive conversations with `step()`, and assert on the events you receive. This approach gives you confidence that your system works correctly while maintaining the freedom to refactor and improve the implementation.
