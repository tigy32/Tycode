use tycode_core::chat::events::{ChatEvent, MessageSender};

mod fixture;

#[test]
fn test_debug_ui_command_works() {
    fixture::run(|mut fixture| async move {
        let events = fixture.step("/debug_ui").await;

        assert!(!events.is_empty(), "Should receive events from debug_ui");

        let has_test_events = events.iter().any(|e| {
            matches!(
                e,
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System)
            )
        });

        assert!(has_test_events, "debug_ui should return test events");
    });
}

#[test]
fn test_debug_ui_not_in_help() {
    fixture::run(|mut fixture| async move {
        let events = fixture.step("/help").await;

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

        assert!(
            !help_content.contains("debug_ui"),
            "Help output should not contain debug_ui command. Found: {}",
            help_content
        );
    });
}
