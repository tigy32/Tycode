#[path = "../fixture.rs"]
mod fixture;

use fixture::{run, MockBehavior};
use tycode_core::chat::events::ChatEvent;
use tycode_core::modules::image::config::Image;

/// Check if the generate_image tool is present in the last AI request's tool definitions
fn has_generate_image_tool(fixture: &fixture::Fixture) -> bool {
    fixture
        .get_last_ai_request()
        .map(|req| req.tools.iter().any(|t| t.name == "generate_image"))
        .unwrap_or(false)
}

#[test]
fn test_image_tool_absent_when_disabled() {
    run(|mut fixture| async move {
        // Enable image gen on provider so we isolate the config check
        fixture.set_image_gen_enabled(true);

        fixture.set_mock_behavior(MockBehavior::Success);
        let _events = fixture.step("hello").await;

        assert!(
            !has_generate_image_tool(&fixture),
            "generate_image tool should NOT appear when image module is disabled"
        );
    })
}

#[test]
fn test_image_tool_absent_when_provider_unsupported() {
    run(|mut fixture| async move {
        // Enable the image module in settings
        fixture
            .update_settings(|settings| {
                let mut config: Image = settings.get_module_config("image");
                config.enabled = true;
                settings.set_module_config("image", config);
            })
            .await;

        // Provider does NOT support image gen (default mock behavior)

        fixture.set_mock_behavior(MockBehavior::Success);
        let _events = fixture.step("hello").await;

        assert!(
            !has_generate_image_tool(&fixture),
            "generate_image tool should NOT appear when provider doesn't support image generation"
        );
    })
}

#[test]
fn test_image_tool_present_when_enabled_and_supported() {
    run(|mut fixture| async move {
        // Enable the image module in settings
        fixture
            .update_settings(|settings| {
                let mut config: Image = settings.get_module_config("image");
                config.enabled = true;
                settings.set_module_config("image", config);
            })
            .await;

        // Enable image gen on provider
        fixture.set_image_gen_enabled(true);

        fixture.set_mock_behavior(MockBehavior::Success);
        let _events = fixture.step("hello").await;

        assert!(
            has_generate_image_tool(&fixture),
            "generate_image tool SHOULD appear when image module is enabled AND provider supports it"
        );
    })
}

#[test]
fn test_image_generation_end_to_end() {
    run(|mut fixture| async move {
        let workspace_path = fixture.workspace_path();
        let workspace_name = workspace_path.file_name().unwrap().to_str().unwrap();

        // Enable the image module
        fixture
            .update_settings(|settings| {
                let mut config: Image = settings.get_module_config("image");
                config.enabled = true;
                settings.set_module_config("image", config);
            })
            .await;

        // Enable image gen on provider
        fixture.set_image_gen_enabled(true);

        // Mock AI calls generate_image tool
        fixture.set_mock_behavior(MockBehavior::ToolUseThenSuccess {
            tool_name: "generate_image".to_string(),
            tool_arguments: serde_json::json!({
                "prompt": "A red pixel",
                "output_path": format!("/{}/generated.png", workspace_name)
            })
            .to_string(),
        });

        let events = fixture.step("Generate an image for me").await;

        // Verify assistant response completed
        assert!(
            events.iter().any(|e| {
                matches!(
                    e,
                    ChatEvent::StreamEnd { message } if matches!(message.sender, tycode_core::chat::events::MessageSender::Assistant { .. })
                )
            }),
            "Should receive assistant message after image generation"
        );

        // Verify the image file was written to the workspace
        let image_path = workspace_path.join("generated.png");
        assert!(
            image_path.exists(),
            "Image file should exist at {:?}",
            image_path
        );

        // Verify the file has PNG content (starts with PNG signature)
        let image_bytes = std::fs::read(&image_path).unwrap();
        assert!(
            image_bytes.len() > 8,
            "Image file should have content, got {} bytes",
            image_bytes.len()
        );
        assert_eq!(
            &image_bytes[0..4],
            &[0x89, 0x50, 0x4E, 0x47],
            "File should start with PNG signature"
        );
    })
}
