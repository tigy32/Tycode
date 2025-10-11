use crate::security::{RiskLevel, SecurityMode, ToolPermission};
use crate::settings::config::Settings;
use anyhow::{Context, Result};
use std::fs;
use std::ops::DerefMut;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Various settings used throughout Tycode. Each process has its own local
/// settings that the user may update without impacting any other session (for
/// example increase the maximum model cost for 1 session). Settings can also be
/// saved and when saved, future processes will use the same settings.
#[derive(Clone)]
pub struct SettingsManager {
    settings_path: PathBuf,
    // Arc<Mutex<..>> is AI slop friendly - everything wants its own settings
    // and this ensures that everyone has the same instance.
    inner: Arc<Mutex<Settings>>,
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

        let loaded = Self::load_from_file_with_backup(&path)?;

        Ok(Self {
            settings_path: path,
            inner: Arc::new(Mutex::new(loaded)),
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

    /// Get the in-memory settings
    pub fn settings(&self) -> Settings {
        self.inner.lock().unwrap().clone()
    }

    /// Update in-memory settings with a closure. Note: settings are not saved to disk
    pub fn update_setting<F>(&self, updater: F)
    where
        F: FnOnce(&mut Settings),
    {
        let mut guard = self.inner.lock().unwrap();
        updater(guard.deref_mut());
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
        *self.inner.lock().unwrap() = settings;

        Ok(())
    }

    /// Explicitly persist in-memory settings to disk
    pub fn save(&self) -> Result<()> {
        self.save_settings(self.settings())
    }

    /// Get the settings file path
    pub fn path(&self) -> &Path {
        &self.settings_path
    }

    pub fn get_mode(&self) -> SecurityMode {
        self.settings().security.mode
    }

    pub fn set_mode(&mut self, mode: SecurityMode) {
        self.update_setting(|settings| settings.security.mode = mode);
    }

    pub fn check_permission(&self, risk: RiskLevel) -> ToolPermission {
        let guard = self.inner.lock().expect("Settings lock poisoned");
        let mode = guard.security.mode;
        match risk {
            RiskLevel::ReadOnly => ToolPermission::Allowed,
            RiskLevel::LowRisk => match mode {
                SecurityMode::ReadOnly => ToolPermission::Denied,
                SecurityMode::Auto | SecurityMode::All => ToolPermission::Allowed,
            },
            RiskLevel::HighRisk => match mode {
                SecurityMode::ReadOnly | SecurityMode::Auto => ToolPermission::Denied,
                SecurityMode::All => ToolPermission::Allowed,
            },
        }
    }
}
