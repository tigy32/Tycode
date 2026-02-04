use crate::settings::manager::SettingsManager;
use crate::settings::Settings;
use tempfile::TempDir;

#[test]
fn test_infer_profile_from_default_settings() {
    let temp_dir = TempDir::new().unwrap();
    let settings_path = temp_dir.path().join("settings.toml");

    let default_settings = Settings::default();
    let contents = toml::to_string_pretty(&default_settings).unwrap();
    std::fs::write(&settings_path, contents).unwrap();

    let manager = SettingsManager::from_path(settings_path).unwrap();

    assert_eq!(manager.current_profile(), None);
}

#[test]
fn test_infer_profile_from_named_settings() {
    let temp_dir = TempDir::new().unwrap();
    let settings_path = temp_dir.path().join("settings_dev.toml");

    let default_settings = Settings::default();
    let contents = toml::to_string_pretty(&default_settings).unwrap();
    std::fs::write(&settings_path, contents).unwrap();

    let manager = SettingsManager::from_path(settings_path).unwrap();

    assert_eq!(manager.current_profile(), Some("dev"));
}

#[test]
fn test_infer_profile_from_production_settings() {
    let temp_dir = TempDir::new().unwrap();
    let settings_path = temp_dir.path().join("settings_production.toml");

    let default_settings = Settings::default();
    let contents = toml::to_string_pretty(&default_settings).unwrap();
    std::fs::write(&settings_path, contents).unwrap();

    let manager = SettingsManager::from_path(settings_path).unwrap();

    assert_eq!(manager.current_profile(), Some("production"));
}

#[test]
fn test_infer_profile_empty_name() {
    let temp_dir = TempDir::new().unwrap();
    let settings_path = temp_dir.path().join("settings_.toml");

    let default_settings = Settings::default();
    let contents = toml::to_string_pretty(&default_settings).unwrap();
    std::fs::write(&settings_path, contents).unwrap();

    let manager = SettingsManager::from_path(settings_path).unwrap();

    assert_eq!(manager.current_profile(), None);
}

#[test]
fn test_infer_profile_non_toml_file() {
    let temp_dir = TempDir::new().unwrap();
    let settings_path = temp_dir.path().join("settings_dev.json");

    let default_settings = Settings::default();
    let contents = toml::to_string_pretty(&default_settings).unwrap();
    std::fs::write(&settings_path, contents).unwrap();

    let manager = SettingsManager::from_path(settings_path).unwrap();

    assert_eq!(manager.current_profile(), None);
}

#[test]
fn test_infer_profile_wrong_prefix() {
    let temp_dir = TempDir::new().unwrap();
    let settings_path = temp_dir.path().join("config_dev.toml");

    let default_settings = Settings::default();
    let contents = toml::to_string_pretty(&default_settings).unwrap();
    std::fs::write(&settings_path, contents).unwrap();

    let manager = SettingsManager::from_path(settings_path).unwrap();

    assert_eq!(manager.current_profile(), None);
}

#[test]
fn test_switch_profile_creates_new() {
    let temp_dir = TempDir::new().unwrap();
    let settings_path = temp_dir.path().join("settings.toml");

    let default_settings = Settings::default();
    let contents = toml::to_string_pretty(&default_settings).unwrap();
    std::fs::write(&settings_path, contents).unwrap();

    let mut manager = SettingsManager::from_path(settings_path).unwrap();

    assert_eq!(manager.current_profile(), None);

    let profile_name = format!("test_switch_{}", std::process::id());
    manager.switch_profile(&profile_name).unwrap();

    assert_eq!(manager.current_profile(), Some(profile_name.as_str()));

    let expected_path = temp_dir
        .path()
        .join(format!("settings_{}.toml", profile_name));
    assert!(expected_path.exists());

    std::fs::remove_file(expected_path).unwrap();
}

#[test]
fn test_switch_between_profiles() {
    let temp_dir = TempDir::new().unwrap();
    let settings_path = temp_dir.path().join("settings.toml");

    let default_settings = Settings::default();
    let contents = toml::to_string_pretty(&default_settings).unwrap();
    std::fs::write(&settings_path, contents).unwrap();

    let mut manager = SettingsManager::from_path(settings_path).unwrap();

    let profile1 = format!("test_profile1_{}", std::process::id());
    let profile2 = format!("test_profile2_{}", std::process::id());

    manager.switch_profile(&profile1).unwrap();
    assert_eq!(manager.current_profile(), Some(profile1.as_str()));

    manager.switch_profile(&profile2).unwrap();
    assert_eq!(manager.current_profile(), Some(profile2.as_str()));

    manager.switch_profile(&profile1).unwrap();
    assert_eq!(manager.current_profile(), Some(profile1.as_str()));

    let profile1_path = temp_dir.path().join(format!("settings_{}.toml", profile1));
    let profile2_path = temp_dir.path().join(format!("settings_{}.toml", profile2));
    std::fs::remove_file(profile1_path).unwrap();
    std::fs::remove_file(profile2_path).unwrap();
}

#[test]
fn test_save_as_profile() {
    let temp_dir = TempDir::new().unwrap();
    let settings_path = temp_dir.path().join("settings.toml");

    let mut default_settings = Settings::default();
    default_settings.default_agent = "custom_agent".to_string();
    let contents = toml::to_string_pretty(&default_settings).unwrap();
    std::fs::write(&settings_path, contents).unwrap();

    let manager = SettingsManager::from_path(settings_path).unwrap();

    let profile_name = format!("test_backup_{}", std::process::id());
    manager.save_as_profile(&profile_name).unwrap();

    let backup_path = temp_dir
        .path()
        .join(format!("settings_{}.toml", profile_name));
    assert!(backup_path.exists());

    let backup_contents = std::fs::read_to_string(&backup_path).unwrap();
    let backup_settings: Settings = toml::from_str(&backup_contents).unwrap();
    assert_eq!(backup_settings.default_agent, "custom_agent");

    std::fs::remove_file(backup_path).unwrap();
}

#[test]
fn test_settings_isolation_between_profiles() {
    let temp_dir = TempDir::new().unwrap();
    let settings_path = temp_dir.path().join("settings.toml");

    let default_settings = Settings::default();
    let contents = toml::to_string_pretty(&default_settings).unwrap();
    std::fs::write(&settings_path, contents).unwrap();

    let mut manager = SettingsManager::from_path(settings_path.clone()).unwrap();

    manager.update_setting(|s| s.default_agent = "agent_default".to_string());
    manager.save().unwrap();

    let profile1 = format!("test_iso1_{}", std::process::id());
    let profile2 = format!("test_iso2_{}", std::process::id());

    manager.switch_profile(&profile1).unwrap();
    manager.update_setting(|s| s.default_agent = "agent_profile1".to_string());
    manager.save().unwrap();

    manager.switch_profile(&profile2).unwrap();
    manager.update_setting(|s| s.default_agent = "agent_profile2".to_string());
    manager.save().unwrap();

    let default_path = settings_path.clone();
    let default_manager = SettingsManager::from_path(default_path).unwrap();
    assert_eq!(default_manager.settings().default_agent, "agent_default");

    let profile1_path = temp_dir.path().join(format!("settings_{}.toml", profile1));
    let profile1_manager = SettingsManager::from_path(profile1_path.clone()).unwrap();
    assert_eq!(profile1_manager.settings().default_agent, "agent_profile1");

    let profile2_path = temp_dir.path().join(format!("settings_{}.toml", profile2));
    let profile2_manager = SettingsManager::from_path(profile2_path.clone()).unwrap();
    assert_eq!(profile2_manager.settings().default_agent, "agent_profile2");

    std::fs::remove_file(profile1_path).unwrap();
    std::fs::remove_file(profile2_path).unwrap();
}

#[test]
fn test_unknown_settings_ignored() {
    let temp_dir = TempDir::new().unwrap();
    let settings_path = temp_dir.path().join("settings.toml");

    let toml_content = r#"
default_agent = "custom_agent"
unknown_field = "this should be ignored"
another_unknown = 42

[unknown_section]
foo = "bar"
    "#;

    std::fs::write(&settings_path, toml_content).unwrap();

    let manager = SettingsManager::from_path(settings_path).unwrap();
    let settings = manager.settings();

    assert_eq!(settings.default_agent, "custom_agent");
}
