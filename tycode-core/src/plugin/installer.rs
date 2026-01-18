//! Plugin installation from various sources.
//!
//! Supports installing plugins from:
//! - Local filesystem paths
//! - GitHub repositories
//! - Shorthand formats like `name@owner/repo` or `owner/repo`

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{debug, info};

/// Represents a parsed plugin source.
#[derive(Debug, Clone)]
pub enum PluginSource {
    /// Local filesystem path
    LocalPath(PathBuf),
    /// GitHub repository (owner, repo, optional branch/tag)
    GitHub {
        owner: String,
        repo: String,
        reference: Option<String>,
    },
}

impl PluginSource {
    /// Parses a plugin source from a string.
    ///
    /// Supported formats:
    /// - `/path/to/plugin` or `./relative/path` - Local path
    /// - `github:owner/repo` - GitHub explicit
    /// - `github:owner/repo@branch` - GitHub with branch/tag
    /// - `owner/repo` - GitHub shorthand
    /// - `owner/repo@branch` - GitHub shorthand with branch
    /// - `name@owner/repo` - Named GitHub install (name is used for directory)
    /// - `name@owner-repo` - Named GitHub install (dash separator)
    pub fn parse(source: &str) -> Result<(Option<String>, Self)> {
        let source = source.trim();

        // Check for local path (starts with / or . or ~)
        if source.starts_with('/') || source.starts_with('.') || source.starts_with('~') {
            let path = shellexpand::tilde(source);
            return Ok((None, PluginSource::LocalPath(PathBuf::from(path.as_ref()))));
        }

        // Check for explicit github: prefix
        if let Some(rest) = source.strip_prefix("github:") {
            let (owner, repo, reference) = Self::parse_github_ref(rest)?;
            return Ok((
                None,
                PluginSource::GitHub {
                    owner,
                    repo,
                    reference,
                },
            ));
        }

        // Check for name@source format (e.g., obsidian@kepano/obsidian-skills)
        if let Some(at_pos) = source.find('@') {
            let name = &source[..at_pos];
            let github_part = &source[at_pos + 1..];

            // Check if github_part contains a slash (owner/repo format)
            if github_part.contains('/') {
                let (owner, repo, reference) = Self::parse_github_ref(github_part)?;
                return Ok((
                    Some(name.to_string()),
                    PluginSource::GitHub {
                        owner,
                        repo,
                        reference,
                    },
                ));
            } else {
                // Format: name@owner-repo (dash-separated)
                // LIMITATION: This splits on the first dash, so owners with dashes
                // in their username will be parsed incorrectly. For example,
                // "name@some-owner-repo" parses as owner="some" and repo="owner-repo".
                // Prefer the slash format "name@owner/repo" for unambiguous parsing.
                if let Some(dash_pos) = github_part.find('-') {
                    let owner = &github_part[..dash_pos];
                    let repo = &github_part[dash_pos + 1..];
                    return Ok((
                        Some(name.to_string()),
                        PluginSource::GitHub {
                            owner: owner.to_string(),
                            repo: repo.to_string(),
                            reference: None,
                        },
                    ));
                } else {
                    bail!(
                        "Invalid plugin source format: '{}'. Expected 'name@owner/repo' or 'name@owner-repo'",
                        source
                    );
                }
            }
        }

        // Check for simple owner/repo format
        if source.contains('/') {
            let (owner, repo, reference) = Self::parse_github_ref(source)?;
            return Ok((
                None,
                PluginSource::GitHub {
                    owner,
                    repo,
                    reference,
                },
            ));
        }

        bail!(
            "Invalid plugin source: '{}'. Expected a path, 'owner/repo', or 'name@owner/repo'",
            source
        );
    }

    /// Parses a GitHub reference like "owner/repo" or "owner/repo@branch"
    fn parse_github_ref(s: &str) -> Result<(String, String, Option<String>)> {
        let (repo_part, reference) = if let Some(at_pos) = s.find('@') {
            (&s[..at_pos], Some(s[at_pos + 1..].to_string()))
        } else {
            (s, None)
        };

        let parts: Vec<&str> = repo_part.split('/').collect();
        if parts.len() != 2 {
            bail!(
                "Invalid GitHub repository format: '{}'. Expected 'owner/repo'",
                repo_part
            );
        }

        Ok((parts[0].to_string(), parts[1].to_string(), reference))
    }

    /// Returns the GitHub URL for cloning.
    pub fn github_url(&self) -> Option<String> {
        match self {
            PluginSource::GitHub { owner, repo, .. } => {
                Some(format!("https://github.com/{}/{}.git", owner, repo))
            }
            _ => None,
        }
    }
}

/// Result of a plugin installation.
#[derive(Debug)]
pub struct InstallResult {
    /// Name of the installed plugin
    pub name: String,
    /// Path where the plugin was installed
    pub path: PathBuf,
    /// Whether this was an update (plugin already existed)
    pub updated: bool,
}

/// Installs plugins to the user's plugin directory.
pub struct PluginInstaller {
    /// Directory where plugins should be installed
    plugins_dir: PathBuf,
}

impl PluginInstaller {
    /// Creates a new PluginInstaller for the given plugins directory.
    pub fn new(plugins_dir: PathBuf) -> Self {
        Self { plugins_dir }
    }

    /// Creates a PluginInstaller for the user's default plugins directory.
    pub fn user_plugins() -> Result<Self> {
        let home = dirs::home_dir().context("Failed to get home directory")?;
        let plugins_dir = home.join(".tycode").join("plugins");
        Ok(Self::new(plugins_dir))
    }

    /// Installs a plugin from the given source.
    pub fn install(&self, source: &str) -> Result<InstallResult> {
        let (custom_name, parsed_source) = PluginSource::parse(source)?;

        // Ensure plugins directory exists
        std::fs::create_dir_all(&self.plugins_dir)
            .context("Failed to create plugins directory")?;

        match parsed_source {
            PluginSource::LocalPath(path) => self.install_from_local(&path, custom_name),
            PluginSource::GitHub {
                owner,
                repo,
                reference,
            } => self.install_from_github(&owner, &repo, reference.as_deref(), custom_name),
        }
    }

    /// Installs a plugin from a local path by copying it.
    fn install_from_local(
        &self,
        source_path: &Path,
        custom_name: Option<String>,
    ) -> Result<InstallResult> {
        if !source_path.exists() {
            bail!("Source path does not exist: {}", source_path.display());
        }

        if !source_path.is_dir() {
            bail!(
                "Source path is not a directory: {}",
                source_path.display()
            );
        }

        // Determine plugin name from manifest or directory name
        let name = if let Some(n) = custom_name {
            n
        } else {
            self.detect_plugin_name(source_path)?
        };

        let dest_path = self.plugins_dir.join(&name);
        let updated = dest_path.exists();

        if updated {
            // Remove existing installation
            std::fs::remove_dir_all(&dest_path)
                .context("Failed to remove existing plugin")?;
        }

        // Copy the plugin directory
        copy_dir_recursive(source_path, &dest_path)
            .context("Failed to copy plugin")?;

        info!("Installed plugin '{}' from local path", name);

        Ok(InstallResult {
            name,
            path: dest_path,
            updated,
        })
    }

    /// Installs a plugin from a GitHub repository.
    fn install_from_github(
        &self,
        owner: &str,
        repo: &str,
        reference: Option<&str>,
        custom_name: Option<String>,
    ) -> Result<InstallResult> {
        let url = format!("https://github.com/{}/{}.git", owner, repo);
        let name = custom_name.unwrap_or_else(|| repo.to_string());
        let dest_path = self.plugins_dir.join(&name);
        let updated = dest_path.exists();

        if updated {
            // Update existing installation with git pull
            debug!("Updating existing plugin '{}' with git pull", name);

            let mut cmd = Command::new("git");
            cmd.arg("-C")
                .arg(&dest_path)
                .arg("pull")
                .arg("--ff-only");

            let output = cmd.output().context("Failed to run git pull")?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                // If pull fails, try fresh clone
                debug!("Git pull failed, attempting fresh clone: {}", stderr);
                std::fs::remove_dir_all(&dest_path)
                    .context("Failed to remove existing plugin for fresh install")?;
                self.git_clone(&url, &dest_path, reference)?;
            } else if let Some(ref_name) = reference {
                // Checkout specific reference after pull
                self.git_checkout(&dest_path, ref_name)?;
            }
        } else {
            // Fresh clone
            self.git_clone(&url, &dest_path, reference)?;
        }

        info!(
            "Installed plugin '{}' from github.com/{}/{}",
            name, owner, repo
        );

        Ok(InstallResult {
            name,
            path: dest_path,
            updated,
        })
    }

    /// Clones a git repository.
    fn git_clone(&self, url: &str, dest: &Path, reference: Option<&str>) -> Result<()> {
        debug!("Cloning {} to {}", url, dest.display());

        let mut cmd = Command::new("git");
        cmd.arg("clone");

        // Use depth 1 for faster cloning unless we need a specific reference
        if reference.is_none() {
            cmd.arg("--depth").arg("1");
        }

        cmd.arg(url).arg(dest);

        let output = cmd.output().context("Failed to run git clone")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Git clone failed: {}", stderr.trim());
        }

        // Checkout specific reference if provided
        if let Some(ref_name) = reference {
            self.git_checkout(dest, ref_name)?;
        }

        Ok(())
    }

    /// Checks out a specific git reference (branch, tag, or commit).
    fn git_checkout(&self, repo_path: &Path, reference: &str) -> Result<()> {
        debug!("Checking out '{}' in {}", reference, repo_path.display());

        let output = Command::new("git")
            .arg("-C")
            .arg(repo_path)
            .arg("checkout")
            .arg(reference)
            .output()
            .context("Failed to run git checkout")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            bail!("Git checkout failed: {}", stderr.trim());
        }

        Ok(())
    }

    /// Detects the plugin name from its manifest or directory name.
    fn detect_plugin_name(&self, path: &Path) -> Result<String> {
        // Try to read plugin.json manifest
        let manifest_path = path.join(".claude-plugin").join("plugin.json");
        if manifest_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&manifest_path) {
                if let Ok(manifest) = serde_json::from_str::<serde_json::Value>(&content) {
                    if let Some(name) = manifest.get("name").and_then(|n| n.as_str()) {
                        return Ok(name.to_string());
                    }
                }
            }
        }

        // Try to read tycode-plugin.toml manifest
        let native_manifest_path = path.join("tycode-plugin.toml");
        if native_manifest_path.exists() {
            if let Ok(content) = std::fs::read_to_string(&native_manifest_path) {
                if let Ok(manifest) = toml::from_str::<toml::Value>(&content) {
                    if let Some(name) = manifest.get("name").and_then(|n| n.as_str()) {
                        return Ok(name.to_string());
                    }
                }
            }
        }

        // Fall back to directory name
        path.file_name()
            .and_then(|n| n.to_str())
            .map(|s| s.to_string())
            .context("Failed to determine plugin name")
    }

    /// Uninstalls a plugin by name.
    pub fn uninstall(&self, name: &str) -> Result<()> {
        let plugin_path = self.plugins_dir.join(name);

        if !plugin_path.exists() {
            bail!("Plugin '{}' is not installed", name);
        }

        std::fs::remove_dir_all(&plugin_path)
            .context("Failed to remove plugin directory")?;

        info!("Uninstalled plugin '{}'", name);
        Ok(())
    }

    /// Lists installed plugins in this directory.
    pub fn list_installed(&self) -> Result<Vec<String>> {
        if !self.plugins_dir.exists() {
            return Ok(Vec::new());
        }

        let mut plugins = Vec::new();
        for entry in std::fs::read_dir(&self.plugins_dir)? {
            let entry = entry?;
            if entry.path().is_dir() {
                if let Some(name) = entry.file_name().to_str() {
                    plugins.push(name.to_string());
                }
            }
        }

        Ok(plugins)
    }
}

/// Recursively copies a directory.
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    std::fs::create_dir_all(dst)?;

    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            // Skip .git directory
            if entry.file_name() == ".git" {
                continue;
            }
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_local_path() {
        let (name, source) = PluginSource::parse("/path/to/plugin").unwrap();
        assert!(name.is_none());
        assert!(matches!(source, PluginSource::LocalPath(_)));
    }

    #[test]
    fn test_parse_relative_path() {
        let (name, source) = PluginSource::parse("./my-plugin").unwrap();
        assert!(name.is_none());
        assert!(matches!(source, PluginSource::LocalPath(_)));
    }

    #[test]
    fn test_parse_github_explicit() {
        let (name, source) = PluginSource::parse("github:owner/repo").unwrap();
        assert!(name.is_none());
        match source {
            PluginSource::GitHub { owner, repo, reference } => {
                assert_eq!(owner, "owner");
                assert_eq!(repo, "repo");
                assert!(reference.is_none());
            }
            _ => panic!("Expected GitHub source"),
        }
    }

    #[test]
    fn test_parse_github_with_branch() {
        let (name, source) = PluginSource::parse("github:owner/repo@main").unwrap();
        assert!(name.is_none());
        match source {
            PluginSource::GitHub { owner, repo, reference } => {
                assert_eq!(owner, "owner");
                assert_eq!(repo, "repo");
                assert_eq!(reference, Some("main".to_string()));
            }
            _ => panic!("Expected GitHub source"),
        }
    }

    #[test]
    fn test_parse_github_shorthand() {
        let (name, source) = PluginSource::parse("kepano/obsidian-skills").unwrap();
        assert!(name.is_none());
        match source {
            PluginSource::GitHub { owner, repo, .. } => {
                assert_eq!(owner, "kepano");
                assert_eq!(repo, "obsidian-skills");
            }
            _ => panic!("Expected GitHub source"),
        }
    }

    #[test]
    fn test_parse_named_github() {
        let (name, source) = PluginSource::parse("obsidian@kepano/obsidian-skills").unwrap();
        assert_eq!(name, Some("obsidian".to_string()));
        match source {
            PluginSource::GitHub { owner, repo, .. } => {
                assert_eq!(owner, "kepano");
                assert_eq!(repo, "obsidian-skills");
            }
            _ => panic!("Expected GitHub source"),
        }
    }

    #[test]
    fn test_parse_named_github_dash_format() {
        let (name, source) = PluginSource::parse("obsidian@kepano-obsidian-skills").unwrap();
        assert_eq!(name, Some("obsidian".to_string()));
        match source {
            PluginSource::GitHub { owner, repo, .. } => {
                assert_eq!(owner, "kepano");
                assert_eq!(repo, "obsidian-skills");
            }
            _ => panic!("Expected GitHub source"),
        }
    }

    #[test]
    fn test_github_url() {
        let source = PluginSource::GitHub {
            owner: "kepano".to_string(),
            repo: "obsidian-skills".to_string(),
            reference: None,
        };
        assert_eq!(
            source.github_url(),
            Some("https://github.com/kepano/obsidian-skills.git".to_string())
        );
    }
}
