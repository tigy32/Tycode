use std::collections::HashMap;
use tycode_core::agents::catalog::AgentCatalog;
use tycode_core::ai::model::Model;
use tycode_core::ai::types::ModelSettings;
use tycode_core::security::{SecurityConfig, SecurityMode};
use tycode_core::settings::config::Settings;

pub fn get_test_settings(base_settings: Settings) -> HashMap<String, Settings> {
    let mut map = HashMap::new();
    map.insert(
        "GPT-OSS120B-ONESHOT".to_string(),
        Settings {
            security: SecurityConfig {
                mode: SecurityMode::All,
            },
            agent_models: agent_models(Model::GptOss120b),
            ..base_settings.clone()
        },
    );
    map.insert(
        "QWEN3-CODER-ONESHOT".to_string(),
        Settings {
            security: SecurityConfig {
                mode: SecurityMode::All,
            },
            agent_models: agent_models(Model::Qwen3Coder),
            ..base_settings.clone()
        },
    );
    map.insert(
        "GROK-CODE1-ONESHOT".to_string(),
        Settings {
            security: SecurityConfig {
                mode: SecurityMode::All,
            },
            agent_models: agent_models(Model::GrokCodeFast1),
            ..base_settings.clone()
        },
    );
    map
}

pub fn agent_models(model: Model) -> HashMap<String, ModelSettings> {
    let mut result = HashMap::new();
    for agent in AgentCatalog::list_agents() {
        result.insert(agent.name, model.default_settings());
    }
    result
}
