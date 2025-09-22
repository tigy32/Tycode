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
    /// Create a new PathResolver with the given workspace roots
    pub fn new(workspace_roots: Vec<PathBuf>) -> anyhow::Result<Self> {
        let mut workspaces = HashMap::new();
        for workspace_root in workspace_roots {
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

        // Normalize the virtual_path so we always have /<workspace>/<...>
        let virtual_path = PathBuf::from("/").join(&root).join(&relative);

        let Some(workspace) = self.workspaces.get(&root) else {
            bail!(
                "No root directory: {root} (known: {:?}). Be sure to use absolute paths!",
                self.workspaces.keys()
            );
        };

        let real_path = workspace.join(relative);

        Ok(ResolvedPath {
            workspace: root,
            virtual_path,
            real_path,
        })
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

    // Skip root `/` if present, otherwise the first real component
    if let Some(std::path::Component::RootDir) = comps.next() {
        // Skip the first "real" component if present
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
}
