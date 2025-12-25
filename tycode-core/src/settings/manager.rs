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
    settings_dir: PathBuf,
    settings_path: PathBuf,
    current_profile: Option<String>,
    // Arc<Mutex<..>> is AI slop friendly - everything wants its own settings
    // and this ensures that everyone has the same instance.
    inner: Arc<Mutex<Settings>>,
}

impl SettingsManager {
    /// Create a settings manager from a specific settings directory and optional profile
    pub fn from_settings_dir(settings_dir: PathBuf, profile_name: Option<&str>) -> Result<Self> {
        // Ensure directory exists
        fs::create_dir_all(&settings_dir)
            .with_context(|| format!("Failed to create settings directory: {:?}", settings_dir))?;

        let settings_path = if let Some(name) = profile_name {
            settings_dir.join(format!("settings_{}.toml", name))
        } else {
            settings_dir.join("settings.toml")
        };

        let current_profile = profile_name.map(|s| s.to_string());

        let loaded = Self::load_from_file_with_backup(&settings_path)?;

        Ok(Self {
            settings_dir,
            settings_path,
            current_profile,
            inner: Arc::new(Mutex::new(loaded)),
        })
    }

    /// Create a settings manager from a specific path
    pub fn from_path(path: PathBuf) -> Result<Self> {
        let settings_dir = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("Settings path has no parent directory"))?
            .to_path_buf();

        let current_profile = Self::infer_profile_from_path(&path);

        let loaded = Self::load_from_file_with_backup(&path)?;

        Ok(Self {
            settings_dir,
            settings_path: path,
            current_profile,
            inner: Arc::new(Mutex::new(loaded)),
        })
    }

    fn infer_profile_from_path(path: &Path) -> Option<String> {
        let file_name = match path.file_name().and_then(|s| s.to_str()) {
            Some(name) => name,
            None => return None,
        };

        if file_name == "settings.toml" {
            return None;
        }

        if !file_name.ends_with(".toml") {
            return None;
        }

        let len = file_name.len();
        let without_ext = &file_name[..len - 5];

        if !without_ext.starts_with("settings_") {
            return None;
        }

        let potential_name = &without_ext[9..];

        if potential_name.is_empty() {
            None
        } else {
            Some(potential_name.to_string())
        }
    }

    /// Load settings from a TOML file with backup on parse failure
    fn load_from_file_with_backup(path: &Path) -> Result<Settings> {
        if !path.exists() {
            let default_settings = Settings::default();
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create directory: {parent:?}"))?;
            }
            let contents = toml::to_string_pretty(&default_settings)
                .context("Failed to serialize default settings")?;
            fs::write(path, contents)
                .with_context(|| format!("Failed to write default settings to {path:?}"))?;
            return Ok(default_settings);
        }

        let contents = fs::read_to_string(path)
            .with_context(|| format!("Failed to read settings from {path:?}"))?;

        match toml::from_str::<Settings>(&contents) {
            Ok(settings) => Ok(settings),
            Err(_) => {
                // Move corrupted file to backup
                let backup_path = path.with_extension("toml.backup");
                fs::rename(path, &backup_path).with_context(|| {
                    format!("Failed to backup corrupted settings to {backup_path:?}")
                })?;

                // Create new default settings file
                let default_settings = Settings::default();
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)
                        .with_context(|| format!("Failed to create directory: {parent:?}"))?;
                }
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

    /// Get the current profile name if set
    pub fn current_profile(&self) -> Option<&str> {
        self.current_profile.as_deref()
    }

    /// Switch to a different profile, loading its settings (creates if not exists)
    pub fn switch_profile(&mut self, name: &str) -> Result<()> {
        let new_path = if name == "default" {
            self.settings_dir.join("settings.toml")
        } else {
            self.settings_dir.join(format!("settings_{}.toml", name))
        };
        fs::create_dir_all(&self.settings_dir)
            .with_context(|| format!("Failed to create directory: {:?}", self.settings_dir))?;
        let new_settings = Self::load_from_file_with_backup(&new_path)?;
        self.settings_path = new_path;
        self.current_profile = if name == "default" {
            None
        } else {
            Some(name.to_string())
        };
        *self.inner.lock().unwrap() = new_settings;
        Ok(())
    }

    /// Save current settings as a new profile file
    pub fn save_as_profile(&self, name: &str) -> Result<()> {
        fs::create_dir_all(&self.settings_dir)
            .with_context(|| format!("Failed to create directory: {:?}", self.settings_dir))?;
        let target_path = self.settings_dir.join(format!("settings_{}.toml", name));
        let contents =
            toml::to_string_pretty(&self.settings()).context("Failed to serialize settings")?;
        fs::write(&target_path, contents)
            .with_context(|| format!("Failed to write settings to {target_path:?}"))?;
        Ok(())
    }

    /// List all available profile names
    pub fn list_profiles(&self) -> Result<Vec<String>> {
        let mut profiles = Vec::new();

        let default_path = self.settings_dir.join("settings.toml");
        if default_path.exists() {
            profiles.push("default".to_string());
        }

        let entries = fs::read_dir(&self.settings_dir).with_context(|| {
            format!("Failed to read settings directory: {:?}", self.settings_dir)
        })?;

        for entry in entries {
            let entry = entry?;
            let path = entry.path();

            if !path.is_file() {
                continue;
            }

            if let Some(profile_name) = Self::infer_profile_from_path(&path) {
                profiles.push(profile_name);
            }
        }

        profiles.sort();
        Ok(profiles)
    }

    /// Get the settings file path
    pub fn path(&self) -> &Path {
        &self.settings_path
    }
}
