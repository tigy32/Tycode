use tycode_core::agents::defaults;
use tycode_core::chat::events::{ChatEvent, MessageSender};

mod fixture;

async fn reload_agent(fixture: &mut fixture::Fixture) {
    let _events = fixture.step("/agent one_shot").await;
}

#[test]
fn test_default_steering_documents_included() {
    fixture::run(|mut fixture| async move {
        let _events = fixture.step("Hello").await;

        let request = fixture
            .get_last_ai_request()
            .expect("Should have captured AI request");
        let system_prompt = &request.system_prompt;

        assert!(
            system_prompt.contains("YAGNI"),
            "System prompt should contain style mandates content"
        );
        assert!(
            system_prompt.contains("Use a short/terse communication style"),
            "System prompt should contain communication guidelines content"
        );
    });
}

#[test]
fn test_workspace_override_takes_precedence() {
    fixture::run(|mut fixture| async move {
        let workspace = fixture.workspace_path();
        let tycode_dir = workspace.join(".tycode");
        std::fs::create_dir_all(&tycode_dir).unwrap();
        std::fs::write(
            tycode_dir.join("style_mandates.md"),
            "CUSTOM_WORKSPACE_STYLE_MANDATES",
        )
        .unwrap();

        reload_agent(&mut fixture).await;

        let _events = fixture.step("Hello").await;

        let request = fixture
            .get_last_ai_request()
            .expect("Should have captured AI request");
        let system_prompt = &request.system_prompt;

        assert!(
            system_prompt.contains("CUSTOM_WORKSPACE_STYLE_MANDATES"),
            "System prompt should contain custom workspace style mandates"
        );
        assert!(
            !system_prompt.contains(defaults::STYLE_MANDATES),
            "System prompt should not contain default style mandates when overridden"
        );
    });
}

#[test]
fn test_custom_documents_appended() {
    fixture::run(|mut fixture| async move {
        let workspace = fixture.workspace_path();
        let tycode_dir = workspace.join(".tycode");
        std::fs::create_dir_all(&tycode_dir).unwrap();
        std::fs::write(
            tycode_dir.join("my_custom_rules.md"),
            "MY_CUSTOM_RULES_CONTENT",
        )
        .unwrap();

        reload_agent(&mut fixture).await;

        let _events = fixture.step("Hello").await;

        let request = fixture
            .get_last_ai_request()
            .expect("Should have captured AI request");
        let system_prompt = &request.system_prompt;

        assert!(
            system_prompt.contains("MY_CUSTOM_RULES_CONTENT"),
            "System prompt should contain custom rules content"
        );
    });
}

#[test]
fn test_cursor_rules_loaded() {
    fixture::run(|mut fixture| async move {
        let workspace = fixture.workspace_path();
        std::fs::write(workspace.join(".cursorrules"), "CURSOR_RULES_CONTENT").unwrap();

        reload_agent(&mut fixture).await;

        let _events = fixture.step("Hello").await;

        let request = fixture
            .get_last_ai_request()
            .expect("Should have captured AI request");
        let system_prompt = &request.system_prompt;

        assert!(
            system_prompt.contains("CURSOR_RULES_CONTENT"),
            "System prompt should contain Cursor rules content"
        );
    });
}

#[test]
fn test_cursor_rules_directory_loaded() {
    fixture::run(|mut fixture| async move {
        let workspace = fixture.workspace_path();
        let cursor_rules_dir = workspace.join(".cursor").join("rules");
        std::fs::create_dir_all(&cursor_rules_dir).unwrap();
        std::fs::write(cursor_rules_dir.join("rule1.md"), "CURSOR_RULE_ONE").unwrap();

        reload_agent(&mut fixture).await;

        let _events = fixture.step("Hello").await;

        let request = fixture
            .get_last_ai_request()
            .expect("Should have captured AI request");
        let system_prompt = &request.system_prompt;

        assert!(
            system_prompt.contains("CURSOR_RULE_ONE"),
            "System prompt should contain Cursor rules directory content"
        );
    });
}

#[test]
fn test_cline_rules_loaded() {
    fixture::run(|mut fixture| async move {
        let workspace = fixture.workspace_path();
        std::fs::write(workspace.join(".clinerules"), "CLINE_RULES_CONTENT").unwrap();

        reload_agent(&mut fixture).await;

        let _events = fixture.step("Hello").await;

        let request = fixture
            .get_last_ai_request()
            .expect("Should have captured AI request");
        let system_prompt = &request.system_prompt;

        assert!(
            system_prompt.contains("CLINE_RULES_CONTENT"),
            "System prompt should contain Cline rules content"
        );
    });
}

#[test]
fn test_roo_rules_loaded() {
    fixture::run(|mut fixture| async move {
        let workspace = fixture.workspace_path();
        std::fs::write(workspace.join(".roorules"), "ROO_RULES_CONTENT").unwrap();

        reload_agent(&mut fixture).await;

        let _events = fixture.step("Hello").await;

        let request = fixture
            .get_last_ai_request()
            .expect("Should have captured AI request");
        let system_prompt = &request.system_prompt;

        assert!(
            system_prompt.contains("ROO_RULES_CONTENT"),
            "System prompt should contain Roo rules content"
        );
    });
}

#[test]
fn test_kiro_steering_docs_loaded() {
    fixture::run(|mut fixture| async move {
        let workspace = fixture.workspace_path();
        let kiro_dir = workspace.join(".kiro").join("steering-docs");
        std::fs::create_dir_all(&kiro_dir).unwrap();
        std::fs::write(kiro_dir.join("doc.md"), "KIRO_STEERING_CONTENT").unwrap();

        reload_agent(&mut fixture).await;

        let _events = fixture.step("Hello").await;

        let request = fixture
            .get_last_ai_request()
            .expect("Should have captured AI request");
        let system_prompt = &request.system_prompt;

        assert!(
            system_prompt.contains("KIRO_STEERING_CONTENT"),
            "System prompt should contain Kiro steering docs content"
        );
    });
}

#[test]
fn test_multiple_external_agents_combined() {
    fixture::run(|mut fixture| async move {
        let workspace = fixture.workspace_path();
        std::fs::write(workspace.join(".cursorrules"), "CURSOR_COMBINED_TEST").unwrap();
        std::fs::write(workspace.join(".clinerules"), "CLINE_COMBINED_TEST").unwrap();
        std::fs::write(workspace.join(".roorules"), "ROO_COMBINED_TEST").unwrap();

        reload_agent(&mut fixture).await;

        let _events = fixture.step("Hello").await;

        let request = fixture
            .get_last_ai_request()
            .expect("Should have captured AI request");
        let system_prompt = &request.system_prompt;

        assert!(
            system_prompt.contains("CURSOR_COMBINED_TEST"),
            "System prompt should contain Cursor rules"
        );
        assert!(
            system_prompt.contains("CLINE_COMBINED_TEST"),
            "System prompt should contain Cline rules"
        );
        assert!(
            system_prompt.contains("ROO_COMBINED_TEST"),
            "System prompt should contain Roo rules"
        );
    });
}

#[test]
fn test_builtin_names_not_duplicated_in_custom() {
    fixture::run(|mut fixture| async move {
        let workspace = fixture.workspace_path();
        let tycode_dir = workspace.join(".tycode");
        std::fs::create_dir_all(&tycode_dir).unwrap();
        std::fs::write(tycode_dir.join("style_mandates.md"), "OVERRIDE_CONTENT").unwrap();
        std::fs::write(tycode_dir.join("other.md"), "OTHER_CONTENT").unwrap();

        reload_agent(&mut fixture).await;

        let _events = fixture.step("Hello").await;

        let request = fixture
            .get_last_ai_request()
            .expect("Should have captured AI request");
        let system_prompt = &request.system_prompt;

        assert!(
            system_prompt.contains("OVERRIDE_CONTENT"),
            "System prompt should contain override content"
        );
        assert!(
            system_prompt.contains("OTHER_CONTENT"),
            "System prompt should contain custom other content"
        );

        let override_count = system_prompt.matches("OVERRIDE_CONTENT").count();
        assert_eq!(
            override_count, 1,
            "Override content should appear exactly once, found {}",
            override_count
        );
    });
}

#[test]
fn test_cline_directory_loaded() {
    fixture::run(|mut fixture| async move {
        let workspace = fixture.workspace_path();
        let cline_dir = workspace.join(".cline");
        std::fs::create_dir_all(&cline_dir).unwrap();
        std::fs::write(cline_dir.join("rules.md"), "CLINE_DIR_CONTENT").unwrap();

        reload_agent(&mut fixture).await;

        let _events = fixture.step("Hello").await;

        let request = fixture
            .get_last_ai_request()
            .expect("Should have captured AI request");
        let system_prompt = &request.system_prompt;

        assert!(
            system_prompt.contains("CLINE_DIR_CONTENT"),
            "System prompt should contain Cline directory content"
        );
    });
}

#[test]
fn test_roo_rules_directory_loaded() {
    fixture::run(|mut fixture| async move {
        let workspace = fixture.workspace_path();
        let roo_dir = workspace.join(".roo").join("rules");
        std::fs::create_dir_all(&roo_dir).unwrap();
        std::fs::write(roo_dir.join("rule.md"), "ROO_DIR_CONTENT").unwrap();

        reload_agent(&mut fixture).await;

        let _events = fixture.step("Hello").await;

        let request = fixture
            .get_last_ai_request()
            .expect("Should have captured AI request");
        let system_prompt = &request.system_prompt;

        assert!(
            system_prompt.contains("ROO_DIR_CONTENT"),
            "System prompt should contain Roo rules directory content"
        );
    });
}

#[test]
fn test_multiple_custom_documents() {
    fixture::run(|mut fixture| async move {
        let workspace = fixture.workspace_path();
        let tycode_dir = workspace.join(".tycode");
        std::fs::create_dir_all(&tycode_dir).unwrap();
        std::fs::write(tycode_dir.join("custom1.md"), "CUSTOM_DOC_ONE").unwrap();
        std::fs::write(tycode_dir.join("custom2.md"), "CUSTOM_DOC_TWO").unwrap();
        std::fs::write(tycode_dir.join("custom3.md"), "CUSTOM_DOC_THREE").unwrap();

        reload_agent(&mut fixture).await;

        let _events = fixture.step("Hello").await;

        let request = fixture
            .get_last_ai_request()
            .expect("Should have captured AI request");
        let system_prompt = &request.system_prompt;

        assert!(
            system_prompt.contains("CUSTOM_DOC_ONE"),
            "System prompt should contain first custom document"
        );
        assert!(
            system_prompt.contains("CUSTOM_DOC_TWO"),
            "System prompt should contain second custom document"
        );
        assert!(
            system_prompt.contains("CUSTOM_DOC_THREE"),
            "System prompt should contain third custom document"
        );
    });
}

#[test]
fn test_non_md_files_ignored() {
    fixture::run(|mut fixture| async move {
        let workspace = fixture.workspace_path();
        let tycode_dir = workspace.join(".tycode");
        std::fs::create_dir_all(&tycode_dir).unwrap();
        std::fs::write(tycode_dir.join("valid.md"), "VALID_MD_CONTENT").unwrap();
        std::fs::write(tycode_dir.join("invalid.txt"), "INVALID_TXT_CONTENT").unwrap();
        std::fs::write(tycode_dir.join("invalid.json"), "INVALID_JSON_CONTENT").unwrap();

        reload_agent(&mut fixture).await;

        let _events = fixture.step("Hello").await;

        let request = fixture
            .get_last_ai_request()
            .expect("Should have captured AI request");
        let system_prompt = &request.system_prompt;

        assert!(
            system_prompt.contains("VALID_MD_CONTENT"),
            "System prompt should contain valid md content"
        );
        assert!(
            !system_prompt.contains("INVALID_TXT_CONTENT"),
            "System prompt should not contain txt file content"
        );
        assert!(
            !system_prompt.contains("INVALID_JSON_CONTENT"),
            "System prompt should not contain json file content"
        );
    });
}

#[test]
fn test_assistant_message_received_with_steering() {
    fixture::run(|mut fixture| async move {
        let workspace = fixture.workspace_path();
        std::fs::write(workspace.join(".cursorrules"), "CURSOR_TEST_RULES").unwrap();

        reload_agent(&mut fixture).await;

        let events = fixture.step("Hello").await;

        assert!(
            events.iter().any(|e| {
                matches!(
                    e,
                    ChatEvent::MessageAdded(msg) if matches!(msg.sender, MessageSender::Assistant { .. })
                )
            }),
            "Should receive assistant message"
        );

        let request = fixture
            .get_last_ai_request()
            .expect("Should have captured AI request");
        assert!(
            request.system_prompt.contains("CURSOR_TEST_RULES"),
            "System prompt should contain steering documents"
        );
    });
}
