use tycode_core::chat::events::{ChatEvent, MessageSender};

mod fixture;

#[test]
fn test_show_current_profile() {
    fixture::run(|mut fixture| async move {
        let events = fixture.step("/profile").await;

        let system_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(!system_messages.is_empty());
        assert!(system_messages
            .iter()
            .any(|msg| msg.contains("Current profile:")));
    });
}

#[test]
fn test_show_current_profile_explicit() {
    fixture::run(|mut fixture| async move {
        let events = fixture.step("/profile show").await;

        let system_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(!system_messages.is_empty());
        assert!(system_messages
            .iter()
            .any(|msg| msg.contains("Current profile:")));
    });
}

#[test]
fn test_list_profiles() {
    fixture::run(|mut fixture| async move {
        let events = fixture.step("/profile list").await;

        let system_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(!system_messages.is_empty());
        assert!(system_messages
            .iter()
            .any(|msg| msg.contains("Available profiles:")));
        assert!(system_messages.iter().any(|msg| msg.contains("default")));
    });
}

#[test]
fn test_save_profile() {
    fixture::run(|mut fixture| async move {
        let profile_name = format!("test_save_{}", std::process::id());
        let events = fixture
            .step(format!("/profile save {}", profile_name))
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

        assert!(!system_messages.is_empty());
        assert!(system_messages
            .iter()
            .any(|msg| msg.contains("Saved current settings as profile:")));
        assert!(system_messages
            .iter()
            .any(|msg| msg.contains(&profile_name)));

        let profile_path = fixture
            .workspace_path()
            .join(".tycode")
            .join(format!("settings_{}.toml", profile_name));
        std::fs::remove_file(profile_path).unwrap();
    });
}

#[test]
fn test_switch_profile() {
    fixture::run(|mut fixture| async move {
        let profile_name = format!("test_switch_{}", std::process::id());

        let save_events = fixture
            .step(format!("/profile save {}", profile_name))
            .await;
        assert!(save_events.iter().any(|e| matches!(
            e,
            ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) && msg.content.contains("Saved")
        )));

        let switch_events = fixture
            .step(format!("/profile switch {}", profile_name))
            .await;

        let system_messages: Vec<_> = switch_events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(!system_messages.is_empty());
        assert!(system_messages
            .iter()
            .any(|msg| msg.contains("Switched to profile:")));
        assert!(system_messages
            .iter()
            .any(|msg| msg.contains(&profile_name)));

        let profile_path = fixture
            .workspace_path()
            .join(".tycode")
            .join(format!("settings_{}.toml", profile_name));
        std::fs::remove_file(profile_path).unwrap();
    });
}

#[test]
fn test_profile_persistence_across_switches() {
    fixture::run(|mut fixture| async move {
        let profile1 = format!("test_persist1_{}", std::process::id());
        let profile2 = format!("test_persist2_{}", std::process::id());

        fixture.step(format!("/profile save {}", profile1)).await;
        fixture.step(format!("/profile save {}", profile2)).await;

        fixture.step(format!("/profile switch {}", profile1)).await;
        let show_events = fixture.step("/profile show").await;

        let system_messages: Vec<_> = show_events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(system_messages.iter().any(|msg| msg.contains(&profile1)));

        fixture.step(format!("/profile switch {}", profile2)).await;
        let show_events2 = fixture.step("/profile show").await;

        let system_messages2: Vec<_> = show_events2
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::System) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(system_messages2.iter().any(|msg| msg.contains(&profile2)));

        let tycode_dir = fixture.workspace_path().join(".tycode");
        std::fs::remove_file(tycode_dir.join(format!("settings_{}.toml", profile1))).unwrap();
        std::fs::remove_file(tycode_dir.join(format!("settings_{}.toml", profile2))).unwrap();
    });
}

#[test]
fn test_invalid_profile_command() {
    fixture::run(|mut fixture| async move {
        let events = fixture.step("/profile invalid_subcommand").await;

        let error_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Error) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(!error_messages.is_empty());
        assert!(error_messages
            .iter()
            .any(|msg| msg.contains("Unknown subcommand")));
    });
}

#[test]
fn test_switch_profile_missing_name() {
    fixture::run(|mut fixture| async move {
        let events = fixture.step("/profile switch").await;

        let error_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Error) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(!error_messages.is_empty());
        assert!(error_messages.iter().any(|msg| msg.contains("Usage:")));
    });
}

#[test]
fn test_save_profile_missing_name() {
    fixture::run(|mut fixture| async move {
        let events = fixture.step("/profile save").await;

        let error_messages: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Error) => {
                    Some(msg.content.as_str())
                }
                _ => None,
            })
            .collect();

        assert!(!error_messages.is_empty());
        assert!(error_messages.iter().any(|msg| msg.contains("Usage:")));
    });
}
