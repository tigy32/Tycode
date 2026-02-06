use tycode_core::ai::types::{ContentBlock, ImageData, MessageRole};
use tycode_core::chat::events::{ChatEvent, MessageSender};

mod fixture;

fn test_image() -> ImageData {
    ImageData {
        media_type: "image/png".to_string(),
        data: "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8/5+hHgAHggJ/PchI7wAAAABJRU5ErkJggg==".to_string(),
    }
}

#[test]
fn test_send_message_with_image() {
    fixture::run(|mut fixture| async move {
        let images = vec![test_image()];
        let events = fixture
            .step_with_images("Describe this image", images)
            .await;

        let user_message = events.iter().find_map(|e| match e {
            ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::User) => Some(msg),
            _ => None,
        });

        assert!(user_message.is_some(), "Should receive user message event");
        let user_msg = user_message.unwrap();
        assert_eq!(
            user_msg.images.len(),
            1,
            "User message should contain 1 image"
        );
        assert_eq!(user_msg.images[0].media_type, "image/png");

        assert!(
            events.iter().any(|e| {
                matches!(
                    e,
                    ChatEvent::StreamEnd { message } if matches!(message.sender, MessageSender::Assistant { .. })
                )
            }),
            "Should receive assistant response after image message"
        );
    });
}

#[test]
fn test_image_in_ai_request() {
    fixture::run(|mut fixture| async move {
        let images = vec![test_image()];
        fixture
            .step_with_images("What's in this image?", images)
            .await;

        let last_request = fixture.get_last_ai_request();
        assert!(last_request.is_some(), "Should have captured AI request");

        let request = last_request.unwrap();
        let user_messages: Vec<_> = request
            .messages
            .iter()
            .filter(|m| m.role == MessageRole::User)
            .collect();

        assert!(
            !user_messages.is_empty(),
            "Should have user messages in request"
        );

        let last_user_msg = user_messages.last().unwrap();
        let has_text = last_user_msg
            .content
            .blocks()
            .iter()
            .any(|b| matches!(b, ContentBlock::Text(t) if t.contains("What's in this image?")));
        let has_image = last_user_msg
            .content
            .blocks()
            .iter()
            .any(|b| matches!(b, ContentBlock::Image(img) if img.media_type == "image/png"));

        assert!(
            has_text,
            "User message in AI request should contain text block"
        );
        assert!(
            has_image,
            "User message in AI request should contain image block"
        );
    });
}

#[test]
fn test_multiple_images() {
    fixture::run(|mut fixture| async move {
        let images = vec![test_image(), test_image()];
        let events = fixture
            .step_with_images("Compare these images", images)
            .await;

        let user_message = events.iter().find_map(|e| match e {
            ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::User) => Some(msg),
            _ => None,
        });

        assert!(user_message.is_some(), "Should receive user message event");
        assert_eq!(
            user_message.unwrap().images.len(),
            2,
            "User message should contain 2 images"
        );

        let request = fixture.get_last_ai_request().unwrap();
        let last_user_msg = request
            .messages
            .iter()
            .filter(|m| m.role == MessageRole::User)
            .last()
            .unwrap();

        let image_count = last_user_msg
            .content
            .blocks()
            .iter()
            .filter(|b| matches!(b, ContentBlock::Image(_)))
            .count();

        assert_eq!(image_count, 2, "AI request should contain 2 image blocks");
    });
}

#[test]
fn test_text_only_message_has_no_images() {
    fixture::run(|mut fixture| async move {
        let events = fixture.step("Hello without images").await;

        let user_message = events.iter().find_map(|e| match e {
            ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::User) => Some(msg),
            _ => None,
        });

        assert!(user_message.is_some(), "Should receive user message event");
        assert!(
            user_message.unwrap().images.is_empty(),
            "Text-only message should have no images"
        );
    });
}
