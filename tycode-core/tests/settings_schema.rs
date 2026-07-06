use std::collections::HashSet;

use tycode_core::chat::events::{ChatEvent, SettingsSchemaInfo};
use tycode_core::settings::{ProviderConfig, Settings};

mod fixture;

async fn recv_settings_schema(fixture: &mut fixture::Fixture) -> SettingsSchemaInfo {
    fixture.actor.get_settings_schema().unwrap();

    let mut schema = None;
    while let Some(event) = fixture.event_rx.recv().await {
        match event {
            ChatEvent::SettingsSchema { schema: current } => {
                schema = Some(current);
            }
            ChatEvent::TypingStatusChanged(false) => break,
            _ => {}
        }
    }

    schema.expect("GetSettingsSchema must emit SettingsSchema")
}

async fn drain_until_idle(fixture: &mut fixture::Fixture) {
    while let Some(event) = fixture.event_rx.recv().await {
        if matches!(event, ChatEvent::TypingStatusChanged(false)) {
            break;
        }
    }
}

#[test]
fn settings_schema_exposes_all_settings_groups_and_values() {
    fixture::run(|mut fixture| async move {
        let schema = recv_settings_schema(&mut fixture).await;

        assert_eq!(schema.settings["active_provider"], "mock");
        assert_eq!(schema.settings["default_agent"], "one_shot");
        assert_eq!(schema.settings["profile"], "default");

        let group_ids: HashSet<&str> = schema
            .groups
            .iter()
            .map(|group| group.id.as_str())
            .collect();
        for expected in [
            "general",
            "providers",
            "mcp",
            "agents",
            "advanced",
            "module:file",
            "module:memory",
            "module:execution",
            "module:context_management",
            "module:image",
        ] {
            assert!(
                group_ids.contains(expected),
                "missing settings group {expected}"
            );
        }

        let represented_core_fields: HashSet<String> = schema
            .groups
            .iter()
            .filter(|group| group.settings_path.is_empty())
            .flat_map(|group| {
                group
                    .schema
                    .get("properties")
                    .and_then(|value| value.as_object())
                    .into_iter()
                    .flat_map(|properties| properties.keys().cloned())
            })
            .collect();

        let settings_object = schema
            .settings
            .as_object()
            .expect("settings payload must be an object");
        for field in settings_object.keys() {
            if field == "modules" || field == "profile" {
                continue;
            }
            assert!(
                represented_core_fields.contains(field),
                "settings field {field} is not represented in any core settings group"
            );
        }

        let memory_group = schema
            .groups
            .iter()
            .find(|group| group.id == "module:memory")
            .expect("memory module schema group must exist");
        assert_eq!(
            memory_group.settings_path,
            vec!["modules".to_string(), "memory".to_string()]
        );
        assert!(
            memory_group
                .schema
                .get("properties")
                .and_then(|value| value.get("enabled"))
                .is_some(),
            "memory module schema should expose its enabled setting"
        );
    });
}

#[test]
fn settings_schema_round_trips_saved_values_without_schema_secrets() {
    fixture::run(|mut fixture| async move {
        let schema = recv_settings_schema(&mut fixture).await;
        let mut settings: Settings = serde_json::from_value(schema.settings).unwrap();
        let secret = "sk-schema-test-secret";

        settings.providers.insert(
            "openrouter".to_string(),
            ProviderConfig::OpenRouter {
                api_key: secret.to_string(),
            },
        );
        settings.active_provider = Some("openrouter".to_string());

        fixture
            .actor
            .save_settings(serde_json::to_value(settings).unwrap(), false)
            .unwrap();
        drain_until_idle(&mut fixture).await;

        let schema = recv_settings_schema(&mut fixture).await;
        assert_eq!(
            schema.settings["providers"]["openrouter"]["api_key"],
            secret
        );

        let group_schema_json = serde_json::to_string(&schema.groups).unwrap();
        assert!(
            !group_schema_json.contains(secret),
            "settings schema metadata must not contain secret values"
        );
    });
}
