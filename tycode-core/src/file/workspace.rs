use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context};

/// Configured workspace roots with real-path containment checks.
#[derive(Debug, Clone)]
pub struct WorkspacePaths {
    roots: Vec<PathBuf>,
}

impl WorkspacePaths {
    /// Create a workspace path set from configured root directories.
    /// Non-existent directories are skipped with a warning (handles VSCode multi-workspace deletion).
    pub fn new(workspace_roots: Vec<PathBuf>) -> anyhow::Result<Self> {
        let mut roots = Vec::new();
        for workspace_root in workspace_roots {
            // VSCode multi-workspace scenarios can have folder references persist after deletion on disk.
            if !workspace_root.exists() {
                tracing::warn!(
                    "Workspace root does not exist, skipping: {}",
                    workspace_root.display()
                );
                continue;
            }

            roots.push(workspace_root.canonicalize()?);
        }

        roots.sort();
        roots.dedup();
        Ok(Self { roots })
    }

    pub fn roots(&self) -> Vec<PathBuf> {
        self.roots.clone()
    }

    pub fn resolve(&self, path_str: &str) -> anyhow::Result<PathBuf> {
        let path = PathBuf::from(path_str);
        if !path.is_absolute() {
            bail!("Path must be absolute and inside a workspace root: {path_str}");
        }

        let path = normalize_absolute(&path)?;
        self.resolve_absolute_path(&path)
    }

    pub fn resolve_root(&self, path_str: &str) -> anyhow::Result<PathBuf> {
        let path = self.resolve(path_str)?;
        if self.roots.iter().any(|root| root == &path) {
            return Ok(path);
        }

        bail!(
            "workspace_root must be one of the configured workspace roots: {:?}",
            self.roots
        );
    }

    pub fn contains_existing_path(&self, path: &Path) -> anyhow::Result<PathBuf> {
        let canonical = path
            .canonicalize()
            .with_context(|| format!("Failed to canonicalize path: {}", path.display()))?;
        self.containing_root(&canonical).ok_or_else(|| {
            anyhow::anyhow!("Path is outside configured workspace roots: {path:?}")
        })?;
        Ok(canonical)
    }

    fn resolve_absolute_path(&self, path: &Path) -> anyhow::Result<PathBuf> {
        if self.roots.is_empty() {
            bail!("No workspace roots configured");
        }

        if path.exists() {
            return self.contains_existing_path(path);
        }

        let mut ancestor = path;
        let mut missing_components = Vec::new();
        while !ancestor.exists() {
            let Some(name) = ancestor.file_name() else {
                bail!("Path is outside configured workspace roots: {path:?}");
            };
            missing_components.push(name.to_os_string());
            ancestor = ancestor.parent().ok_or_else(|| {
                anyhow::anyhow!("Path is outside configured workspace roots: {path:?}")
            })?;
        }

        let mut resolved = self.contains_existing_path(ancestor)?;
        for component in missing_components.iter().rev() {
            resolved.push(component);
        }

        Ok(resolved)
    }

    fn containing_root(&self, path: &Path) -> Option<&PathBuf> {
        self.roots.iter().find(|root| path.starts_with(root))
    }
}

fn normalize_absolute(path: &Path) -> anyhow::Result<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::Normal(part) => normalized.push(part),
            Component::CurDir => {}
            Component::ParentDir => bail!("Parent directory components are not allowed in paths"),
        }
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn roots_are_canonical_real_paths() -> anyhow::Result<()> {
        let workspace_root =
            std::env::var("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR env variable");
        let workspace_root = PathBuf::from(workspace_root);
        let paths = WorkspacePaths::new(vec![workspace_root.clone()])?;

        assert_eq!(vec![workspace_root.canonicalize()?], paths.roots());
        Ok(())
    }

    #[test]
    fn resolves_real_absolute_path_inside_workspace() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let ws = temp.path().join("myworkspace");
        let src = ws.join("src");
        fs::create_dir_all(&src)?;
        let file = src.join("lib.rs");
        fs::write(&file, "pub fn example() {}\n")?;

        let paths = WorkspacePaths::new(vec![ws])?;
        let resolved = paths.resolve(&file.to_string_lossy())?;

        assert_eq!(file.canonicalize()?, resolved);
        Ok(())
    }

    #[test]
    fn resolves_new_file_under_workspace() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let ws = temp.path().join("myworkspace");
        fs::create_dir(&ws)?;

        let paths = WorkspacePaths::new(vec![ws.clone()])?;
        let file = ws.join("src").join("lib.rs");
        let resolved = paths.resolve(&file.to_string_lossy())?;

        assert_eq!(ws.canonicalize()?.join("src").join("lib.rs"), resolved);
        Ok(())
    }

    #[test]
    fn rejects_relative_paths() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let ws = temp.path().join("myworkspace");
        fs::create_dir(&ws)?;

        let paths = WorkspacePaths::new(vec![ws])?;
        assert!(paths.resolve("src/lib.rs").is_err());
        assert!(paths.resolve("../outside").is_err());
        Ok(())
    }

    #[test]
    fn rejects_parent_directory_escape() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let ws = temp.path().join("myworkspace");
        fs::create_dir(&ws)?;

        let paths = WorkspacePaths::new(vec![ws.clone()])?;
        let escape = ws.join("..").join("outside");
        assert!(paths.resolve(&escape.to_string_lossy()).is_err());
        Ok(())
    }

    #[test]
    fn rejects_absolute_path_outside_workspace() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let ws = temp.path().join("myworkspace");
        let outside = temp.path().join("outside.txt");
        fs::create_dir(&ws)?;
        fs::write(&outside, "outside")?;

        let paths = WorkspacePaths::new(vec![ws])?;
        assert!(paths.resolve(&outside.to_string_lossy()).is_err());
        Ok(())
    }

    #[test]
    fn resolve_root_requires_configured_root() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let ws = temp.path().join("myworkspace");
        let subdir = ws.join("subdir");
        fs::create_dir_all(&subdir)?;

        let paths = WorkspacePaths::new(vec![ws.clone()])?;
        assert_eq!(
            ws.canonicalize()?,
            paths.resolve_root(&ws.to_string_lossy())?
        );
        assert!(paths.resolve_root(&subdir.to_string_lossy()).is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlink_parent_escape_for_new_file() -> anyhow::Result<()> {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir()?;
        let ws = temp.path().join("myworkspace");
        let outside = temp.path().join("outside");
        fs::create_dir(&ws)?;
        fs::create_dir(&outside)?;
        symlink(&outside, ws.join("link_out"))?;

        let paths = WorkspacePaths::new(vec![ws.clone()])?;
        let escaped = ws.join("link_out").join("new_file.rs");

        assert!(paths.resolve(&escaped.to_string_lossy()).is_err());
        Ok(())
    }
}
