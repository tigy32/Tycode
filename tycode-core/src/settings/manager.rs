use crate::settings::config::Settings;
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

pub struct SettingsManager {
    settings_path: PathBuf,
}

impl SettingsManager {
    /// Create a new settings manager with default settings location
    pub fn new() -> Result<Self> {
        let settings_path = Self::default_settings_path()?;

        // Ensure directory exists
        if let Some(parent) = settings_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {parent:?}"))?;
        }

        Self::from_path(settings_path)
    }

    /// Create a settings manager from a specific path
    pub fn from_path(path: PathBuf) -> Result<Self> {
        // Ensure default settings file exists if it doesn't
        if !path.exists() {
            let default_settings = Settings::default();
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create directory: {parent:?}"))?;
            }
            let contents = toml::to_string_pretty(&default_settings)
                .context("Failed to serialize default settings")?;
            fs::write(&path, contents)
                .with_context(|| format!("Failed to write default settings to {path:?}"))?;
        }

        Ok(Self {
            settings_path: path,
        })
    }

    /// Get the default settings path (~/.tycode/settings.toml)
    fn default_settings_path() -> Result<PathBuf> {
        let home = dirs::home_dir().context("Failed to get home directory")?;
        Ok(home.join(".tycode").join("settings.toml"))
    }

    /// Load settings from a TOML file with backup on parse failure
    fn load_from_file_with_backup(path: &Path) -> Result<Settings> {
        if !path.exists() {
            return Ok(Settings::default());
        }

        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read settings from {path:?}"))?;

        match toml::from_str(&contents) {
            Ok(settings) => Ok(settings),
            Err(_) => {
                // Move corrupted file to backup
                let backup_path = path.with_extension("toml.backup");
                fs::rename(path, &backup_path).with_context(|| {
                    format!("Failed to backup corrupted settings to {backup_path:?}")
                })?;

                // Create new default settings file
                let default_settings = Settings::default();
                let contents = toml::to_string_pretty(&default_settings)
                    .context("Failed to serialize default settings")?;
                fs::write(path, contents)
                    .with_context(|| format!("Failed to write default settings to {path:?}"))?;

                Ok(default_settings)
            }
        }
    }

    /// Get the current settings (reloads from disk each time)
    pub fn settings(&self) -> Settings {
        Self::load_from_file_with_backup(&self.settings_path)
            .unwrap_or_else(|_| Settings::default())
    }

    /// Update settings with a closure and save
    pub fn update_setting<F>(&mut self, updater: F) -> Result<()>
    where
        F: FnOnce(&mut Settings),
    {
        let mut settings = self.settings();
        updater(&mut settings);
        self.save_settings(settings)?;
        Ok(())
    }

    /// Save provided settings
    pub fn save_settings(&self, settings: Settings) -> Result<()> {
        // Ensure directory exists
        if let Some(parent) = self.settings_path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("Failed to create directory: {parent:?}"))?;
        }

        let contents = toml::to_string_pretty(&settings).context("Failed to serialize settings")?;

        fs::write(&self.settings_path, contents)
            .with_context(|| format!("Failed to write settings to {:?}", self.settings_path))?;

        Ok(())
    }

    /// Get the settings file path
    pub fn path(&self) -> &Path {
        &self.settings_path
    }
}
