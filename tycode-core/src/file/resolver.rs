use std::{
    collections::HashMap,
    ffi::OsString,
    path::{Component, Path, PathBuf},
};

use anyhow::bail;

#[derive(Debug, Clone)]
pub struct ResolvedPath {
    pub workspace: String,
    pub virtual_path: PathBuf,
    pub real_path: PathBuf,
}

/// Responsible for mapping to and from the virtual file system we present to
/// AI agents where each workspace is in a root file system.
#[derive(Debug, Clone)]
pub struct Resolver {
    workspaces: HashMap<String, PathBuf>,
}

impl Resolver {
    /// Create a new PathResolver with the given workspace roots.
    /// Non-existent directories are skipped with a warning (handles VSCode multi-workspace deletion).
    pub fn new(workspace_roots: Vec<PathBuf>) -> anyhow::Result<Self> {
        let mut workspaces = HashMap::new();
        for workspace_root in workspace_roots {
            // VSCode multi-workspace scenarios can have folder references persist after deletion on disk
            if !workspace_root.exists() {
                tracing::warn!(
                    "Workspace root does not exist, skipping: {}",
                    workspace_root.display()
                );
                continue;
            }

            let workspace_root = workspace_root.canonicalize()?;
            let Some(name) = workspace_root.file_name() else {
                bail!("Cannot get workspace name for {workspace_root:?}");
            };

            let name = os_to_string(name.to_os_string())?;
            workspaces.insert(name, workspace_root);
        }
        Ok(Self { workspaces })
    }

    /// Resolves a path in the virtual file system to the real path on disk
    pub fn resolve_path(&self, path_str: &str) -> anyhow::Result<ResolvedPath> {
        let virtual_path = PathBuf::from(path_str);
        let root = root(&virtual_path)?;
        let relative = remaining(&virtual_path);

        if let Some(workspace) = self.workspaces.get(&root) {
            let virtual_path = PathBuf::from("/").join(&root).join(&relative);
            let real_path = workspace.join(relative);
            return Ok(ResolvedPath {
                workspace: root,
                virtual_path,
                real_path,
            });
        }

        // Check if the path is already a real filesystem path within a workspace.
        // This handles cases where the AI passes the full real path instead of the virtual path.
        let input_path = PathBuf::from(path_str);
        for (ws_name, ws_path) in &self.workspaces {
            if let Ok(rel) = input_path.strip_prefix(ws_path) {
                let virtual_path = PathBuf::from("/").join(ws_name).join(rel);
                return Ok(ResolvedPath {
                    workspace: ws_name.clone(),
                    virtual_path,
                    real_path: input_path,
                });
            }
        }

        if self.workspaces.len() == 1 {
            let (ws_name, ws_path) = self.workspaces.iter().next().unwrap();
            let trimmed = path_str.trim_start_matches('/').trim_start_matches("./");
            let full_relative = PathBuf::from(trimmed);
            let virtual_path = PathBuf::from("/").join(ws_name).join(&full_relative);
            let real_path = ws_path.join(&full_relative);
            return Ok(ResolvedPath {
                workspace: ws_name.clone(),
                virtual_path,
                real_path,
            });
        }

        bail!(
            "No root directory: {root} (known: {:?}). Be sure to use absolute paths!",
            self.workspaces.keys()
        );
    }

    /// Converts a real on disk path to the virtual file system path
    pub fn canonicalize(&self, path: &Path) -> anyhow::Result<ResolvedPath> {
        let real_path = path.canonicalize()?;
        for (name, root) in &self.workspaces {
            let Ok(path) = real_path.strip_prefix(root) else {
                continue;
            };
            return Ok(ResolvedPath {
                workspace: name.clone(),
                virtual_path: PathBuf::from("/").join(name).join(path),
                real_path,
            });
        }
        bail!("No workspace found containing {path:?}")
    }

    pub fn root(&self, workspace: &str) -> Option<PathBuf> {
        self.workspaces.get(workspace).cloned()
    }

    pub fn roots(&self) -> Vec<String> {
        self.workspaces.keys().cloned().collect()
    }
}

fn root(path: &Path) -> anyhow::Result<String> {
    let root = path.components().find_map(|c| match c {
        Component::Normal(name) => Some(name),
        _ => None, // skip Prefix, RootDir, CurDir, ParentDir
    });

    let Some(root) = root else {
        bail!("No root directory in {path:?}");
    };

    os_to_string(root.to_os_string())
}

/// Return the path with the first component stripped off
fn remaining(path: &Path) -> PathBuf {
    let mut comps = path.components();
    let first = comps.next();

    if matches!(first, Some(Component::RootDir) | Some(Component::CurDir)) {
        comps.next();
    }

    let mut out = PathBuf::new();
    for c in comps {
        out.push(c);
    }

    out
}

fn os_to_string(str: OsString) -> anyhow::Result<String> {
    let Some(str) = str.to_str() else {
        bail!("Workspace name is not utf8: {str:?}");
    };
    Ok(str.to_string())
}

#[cfg(test)]
mod tests {
    use std::{
        env, fs,
        path::{Path, PathBuf},
    };

    use crate::file::resolver::{remaining, root, Resolver};

    #[test]
    fn test_root() -> anyhow::Result<()> {
        assert!(root(&PathBuf::from(".")).is_err());
        assert!(root(&PathBuf::from("/")).is_err());
        assert_eq!("foo", root(&PathBuf::from("foo"))?);
        assert_eq!("foo", root(&PathBuf::from("foo/"))?);
        assert_eq!("foo", root(&PathBuf::from("foo/bar"))?);
        assert_eq!("foo", root(&PathBuf::from("foo/bar/"))?);
        assert_eq!("foo", root(&PathBuf::from("/foo"))?);
        assert_eq!("foo", root(&PathBuf::from("/foo/"))?);
        assert_eq!("foo", root(&PathBuf::from("/foo/bar"))?);
        assert_eq!("foo", root(&PathBuf::from("/foo/bar/"))?);
        Ok(())
    }

    #[test]
    fn test_remaining() {
        assert_eq!(remaining(Path::new("/")), Path::new(""));
        assert_eq!(remaining(Path::new("/foo")), Path::new(""));
        assert_eq!(remaining(Path::new("/foo/")), Path::new(""));
        assert_eq!(remaining(Path::new("/foo/bar")), Path::new("bar"));
        assert_eq!(remaining(Path::new("/foo/bar/")), Path::new("bar"));
        assert_eq!(remaining(Path::new("/foo/bar/dog")), Path::new("bar/dog"));

        assert_eq!(remaining(Path::new("")), Path::new(""));
        assert_eq!(remaining(Path::new("foo")), Path::new(""));
        assert_eq!(remaining(Path::new("foo/")), Path::new(""));
        assert_eq!(remaining(Path::new("foo/bar")), Path::new("bar"));
        assert_eq!(remaining(Path::new("foo/bar/")), Path::new("bar"));
        assert_eq!(remaining(Path::new("foo/bar/dog")), Path::new("bar/dog"));

        assert_eq!(remaining(Path::new("./")), Path::new(""));
        assert_eq!(remaining(Path::new("./foo")), Path::new(""));
        assert_eq!(remaining(Path::new("./foo/")), Path::new(""));
        assert_eq!(remaining(Path::new("./foo/bar")), Path::new("bar"));
        assert_eq!(remaining(Path::new("./foo/bar/")), Path::new("bar"));
        assert_eq!(remaining(Path::new("./foo/bar/dog")), Path::new("bar/dog"));
    }

    #[test]
    fn test_single_workspace() -> anyhow::Result<()> {
        let workspace_root =
            env::var("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR env variable");
        let workspace_root = PathBuf::from(workspace_root);
        let resolver = Resolver::new(vec![workspace_root])?;

        let roots = resolver.roots();
        assert_eq!(1, roots.len());
        assert_eq!("tycode-core", roots[0]);

        for root in ["tycode-core", "/tycode-core", "/tycode-core/"] {
            let resolved = resolver.resolve_path(root)?;
            assert_eq!("tycode-core", resolved.workspace);
            assert_eq!(PathBuf::from("/tycode-core/"), resolved.virtual_path);
            assert_ne!(PathBuf::from("/tycode-core/"), resolved.real_path);
        }

        for root in [
            "tycode-core/foo",
            "tycode-core/foo/",
            "/tycode-core/foo",
            "/tycode-core/foo/",
        ] {
            let resolved = resolver.resolve_path(root)?;
            assert_eq!("tycode-core", resolved.workspace);
            assert_eq!(PathBuf::from("/tycode-core/foo"), resolved.virtual_path);
        }

        Ok(())
    }

    #[test]
    fn test_canonicalize() -> anyhow::Result<()> {
        let workspace_root =
            env::var("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR env variable");
        let workspace_root = PathBuf::from(workspace_root);
        let resolver = Resolver::new(vec![workspace_root.clone()])?;

        let resolved = resolver.canonicalize(&workspace_root)?;
        for directory in fs::read_dir(resolved.real_path)? {
            let path = directory?.path();
            let resolved = resolver.canonicalize(&path)?;
            assert_eq!("tycode-core", resolved.workspace);
            assert_eq!(path, resolved.real_path);
            assert_ne!(path, resolved.virtual_path);
        }

        Ok(())
    }

    #[test]
    fn test_single_workspace_auto_resolve() -> anyhow::Result<()> {
        let workspace_root =
            env::var("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR env variable");
        let workspace_root = PathBuf::from(workspace_root);
        let resolver = Resolver::new(vec![workspace_root])?;

        for path in ["src/lib.rs", "/src/lib.rs", "src/file/resolver.rs"] {
            let resolved = resolver.resolve_path(path)?;
            assert_eq!("tycode-core", resolved.workspace);
            assert!(resolved.virtual_path.starts_with("/tycode-core/"));
        }

        Ok(())
    }

    #[test]
    fn test_multi_workspace_no_auto_resolve() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let ws1 = temp.path().join("workspace1");
        let ws2 = temp.path().join("workspace2");
        fs::create_dir(&ws1)?;
        fs::create_dir(&ws2)?;

        let resolver = Resolver::new(vec![ws1, ws2])?;

        let result = resolver.resolve_path("src/lib.rs");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("No root directory"));

        Ok(())
    }

    #[test]
    fn test_curdir_workspace_path() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let ws = temp.path().join("myworkspace");
        fs::create_dir(&ws)?;

        let resolver = Resolver::new(vec![ws])?;

        let resolved = resolver.resolve_path("./myworkspace/src")?;
        assert_eq!("myworkspace", resolved.workspace);
        assert_eq!(PathBuf::from("/myworkspace/src"), resolved.virtual_path);

        let resolved = resolver.resolve_path("./src/lib.rs")?;
        assert_eq!("myworkspace", resolved.workspace);
        assert_eq!(
            PathBuf::from("/myworkspace/src/lib.rs"),
            resolved.virtual_path
        );

        Ok(())
    }

    #[test]
    fn test_real_path_not_doubled() -> anyhow::Result<()> {
        let temp = tempfile::tempdir()?;
        let ws = temp.path().join("myworkspace");
        fs::create_dir(&ws)?;

        let resolver = Resolver::new(vec![ws.clone()])?;
        let ws_canonical = ws.canonicalize()?;

        // Passing the real workspace path should NOT double it
        let real_path_str = ws_canonical.to_string_lossy();
        let resolved = resolver.resolve_path(&real_path_str)?;
        assert_eq!("myworkspace", resolved.workspace);
        assert_eq!(PathBuf::from("/myworkspace"), resolved.virtual_path);
        assert_eq!(ws_canonical, resolved.real_path);

        // Passing a subpath within the real workspace should also work
        let subdir = ws.join("src");
        fs::create_dir(&subdir)?;
        let subdir_canonical = subdir.canonicalize()?;
        let subdir_path_str = subdir_canonical.to_string_lossy();
        let resolved = resolver.resolve_path(&subdir_path_str)?;
        assert_eq!("myworkspace", resolved.workspace);
        assert_eq!(PathBuf::from("/myworkspace/src"), resolved.virtual_path);
        assert_eq!(subdir_canonical, resolved.real_path);

        Ok(())
    }
}
