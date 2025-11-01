use crate::file::{
    ignore::Ignored,
    resolver::{ResolvedPath, Resolver},
};
use anyhow::{bail, Context, Result};
use std::path::PathBuf;
use tokio::fs;

#[derive(Clone)]
pub struct FileAccessManager {
    pub roots: Vec<String>,
    resolver: Resolver,
}

impl FileAccessManager {
    pub fn new(workspace_roots: Vec<PathBuf>) -> Self {
        let resolver = Resolver::new(workspace_roots).expect("Unable to resolve workspace roots");
        let roots = resolver.roots();
        Self { resolver, roots }
    }

    pub async fn read_file(&self, file_path: &str) -> Result<String> {
        let path = self.resolve(file_path)?;

        if !path.exists() {
            anyhow::bail!("File not found: {}", file_path);
        }

        if !path.is_file() {
            anyhow::bail!("Path is not a file: {}", file_path);
        }

        fs::read_to_string(&path)
            .await
            .with_context(|| format!("Failed to read file: {file_path}"))
    }

    pub async fn write_file(&self, file_path: &str, content: &str) -> Result<()> {
        let path = self.resolve(file_path)?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .await
                .with_context(|| format!("Failed to create parent directories for: {file_path}"))?;
        }

        fs::write(&path, content)
            .await
            .with_context(|| format!("Failed to write file: {file_path}"))
    }

    pub async fn delete_file(&self, file_path: &str) -> Result<()> {
        let path = self.resolve(file_path)?;

        let metadata = fs::metadata(&path)
            .await
            .with_context(|| format!("Failed to get metadata for: {file_path}"))?;

        if metadata.is_dir() {
            fs::remove_dir(&path)
                .await
                .with_context(|| format!("Failed to delete directory: {file_path}"))?;
        } else {
            fs::remove_file(&path)
                .await
                .with_context(|| format!("Failed to delete file: {file_path}"))?;
        }

        Ok(())
    }

    pub async fn list_directory(&self, directory_path: &str) -> Result<Vec<PathBuf>> {
        let dir_path = self.resolve(directory_path)?;

        if !dir_path.exists() {
            anyhow::bail!("Directory not found: {}", dir_path.display());
        }

        if !dir_path.is_dir() {
            anyhow::bail!("Path is not a directory: {}", dir_path.display());
        }

        let mut entries = fs::read_dir(&dir_path)
            .await
            .with_context(|| format!("Failed to read directory: {}", dir_path.display()))?;

        let mut paths = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let Ok(resolved) = self.resolver.canonicalize(&entry.path()) else {
                // Likely a sym link outside of the working directory (or a bug)
                continue;
            };
            if resolved.virtual_path.file_name() == Some(std::ffi::OsStr::new(".git")) {
                continue;
            }
            if self.ignored(&resolved)? {
                continue;
            }
            paths.push(resolved.virtual_path);
        }

        Ok(paths)
    }

    pub async fn file_exists(&self, file_path: &str) -> Result<bool> {
        let path = self.resolve(file_path)?;
        Ok(path.exists())
    }

    pub fn resolve(&self, virtual_path: &str) -> Result<PathBuf> {
        let path = self.resolver.resolve_path(virtual_path)?;

        // Don't apply ignore rules to workspace roots themselves
        let is_workspace_root = self.roots.contains(&path.workspace)
            && path.virtual_path == PathBuf::from("/").join(&path.workspace);

        if !is_workspace_root && self.ignored(&path)? {
            bail!("File not found: {}", virtual_path);
        }
        Ok(path.real_path)
    }

    fn ignored(&self, path: &ResolvedPath) -> Result<bool> {
        let Some(root) = self.resolver.root(&path.workspace) else {
            bail!("{path:?} is not in a workspace")
        };
        let ignored = Ignored::new(&root)?;
        ignored.is_ignored(&path.real_path)
    }

    pub fn real_root(&self, workspace: &str) -> Option<PathBuf> {
        self.resolver.root(workspace)
    }

    pub async fn list_all_files_recursive(&self, workspace: &str) -> Result<Vec<PathBuf>> {
        let real_root = self
            .real_root(workspace)
            .ok_or_else(|| anyhow::anyhow!("No real path found for workspace: {}", workspace))?;

        if let Ok(files) = self.list_files_with_git(&real_root, workspace).await {
            return Ok(files);
        }

        let root_path = format!("/{}", workspace);
        let mut all_files = Vec::new();
        self.collect_files_recursive(&root_path, &mut all_files)
            .await?;
        Ok(all_files)
    }

    async fn list_files_with_git(
        &self,
        real_root: &PathBuf,
        workspace: &str,
    ) -> Result<Vec<PathBuf>> {
        use tokio::process::Command;

        let tracked_output = Command::new("git")
            .arg("ls-files")
            .current_dir(real_root)
            .output()
            .await?;

        if !tracked_output.status.success() {
            anyhow::bail!("git ls-files failed");
        }

        let mut all_files = Vec::new();

        let tracked_files = String::from_utf8(tracked_output.stdout)?;
        for line in tracked_files.lines() {
            if !line.is_empty() {
                all_files.push(PathBuf::from("/").join(workspace).join(line));
            }
        }

        let untracked_output = Command::new("git")
            .arg("ls-files")
            .arg("-o")
            .arg("--exclude-standard")
            .current_dir(real_root)
            .output()
            .await?;

        let untracked_files = String::from_utf8(untracked_output.stdout)?;
        for line in untracked_files.lines() {
            if !line.is_empty() {
                all_files.push(PathBuf::from("/").join(workspace).join(line));
            }
        }

        Ok(all_files)
    }

    async fn collect_files_recursive(
        &self,
        dir_path: &str,
        files: &mut Vec<PathBuf>,
    ) -> Result<()> {
        let entries = self.list_directory(dir_path).await?;

        for entry in entries {
            let entry_str = entry
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("Invalid path: {entry:?}"))?;
            let real_path = self.resolve(entry_str)?;

            if real_path.is_file() {
                files.push(entry);
            } else if real_path.is_dir() {
                Box::pin(self.collect_files_recursive(entry_str, files)).await?;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs as std_fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_new() {
        let roots = vec![std::env::current_dir().unwrap()];
        let manager = FileAccessManager::new(roots.clone());
        assert_eq!(manager.roots.len(), 1);
    }

    #[tokio::test]
    async fn test_read_file_success() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std_fs::create_dir(&workspace).unwrap();
        let manager = FileAccessManager::new(vec![workspace.clone()]);

        std_fs::write(workspace.join("test.txt"), "content").unwrap();
        let content = manager.read_file("/workspace/test.txt").await.unwrap();
        assert_eq!(content, "content");
    }

    #[tokio::test]
    async fn test_read_file_not_found() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std_fs::create_dir(&workspace).unwrap();
        let manager = FileAccessManager::new(vec![workspace.clone()]);

        let err = manager
            .read_file("/workspace/nonexistent.txt")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("File not found"));
    }

    #[tokio::test]
    async fn test_read_file_not_file() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std_fs::create_dir(&workspace).unwrap();
        let manager = FileAccessManager::new(vec![workspace.clone()]);

        std_fs::create_dir(workspace.join("dir")).unwrap();
        let err = manager.read_file("/workspace/dir").await.unwrap_err();
        assert!(err.to_string().contains("Path is not a file"));
    }

    #[tokio::test]
    async fn test_write_file_success() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std_fs::create_dir(&workspace).unwrap();
        let manager = FileAccessManager::new(vec![workspace.clone()]);

        manager
            .write_file("/workspace/subdir/test.txt", "content")
            .await
            .unwrap();
        let path = workspace.join("subdir/test.txt");
        assert!(path.exists());
        assert_eq!(std_fs::read_to_string(path).unwrap(), "content");
    }

    #[tokio::test]
    async fn test_delete_file_success() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std_fs::create_dir(&workspace).unwrap();
        let manager = FileAccessManager::new(vec![workspace.clone()]);

        let path = workspace.join("test.txt");
        std_fs::write(&path, "content").unwrap();
        manager.delete_file("/workspace/test.txt").await.unwrap();
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn test_delete_directory_success() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std_fs::create_dir(&workspace).unwrap();
        let manager = FileAccessManager::new(vec![workspace.clone()]);

        let dir_path = workspace.join("testdir");
        std_fs::create_dir(&dir_path).unwrap();
        manager.delete_file("/workspace/testdir").await.unwrap();
        assert!(!dir_path.exists());
    }

    #[tokio::test]
    async fn test_delete_file_not_found() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std_fs::create_dir(&workspace).unwrap();
        let manager = FileAccessManager::new(vec![workspace.clone()]);

        let err = manager
            .delete_file("/workspace/nonexistent.txt")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Failed to get metadata"));
    }

    #[tokio::test]
    async fn test_list_directory_success() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std_fs::create_dir(&workspace).unwrap();
        let manager = FileAccessManager::new(vec![workspace.clone()]);

        std_fs::write(workspace.join("a.txt"), "content").unwrap();
        std_fs::write(workspace.join("b.txt"), "content").unwrap();

        let list = manager.list_directory("/workspace").await.unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.contains(&PathBuf::from("/workspace/a.txt")));
        assert!(list.contains(&PathBuf::from("/workspace/b.txt")));
    }

    #[tokio::test]
    async fn test_list_directory_not_found() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std_fs::create_dir(&workspace).unwrap();
        let manager = FileAccessManager::new(vec![workspace.clone()]);

        let err = manager
            .list_directory("/workspace/nonexistent")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Directory not found"));
    }

    #[tokio::test]
    async fn test_list_directory_not_dir() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std_fs::create_dir(&workspace).unwrap();
        let manager = FileAccessManager::new(vec![workspace.clone()]);

        std_fs::write(workspace.join("file.txt"), "content").unwrap();

        let err = manager
            .list_directory("/workspace/file.txt")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Path is not a directory"));
    }

    #[tokio::test]
    async fn test_file_exists_true() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std_fs::create_dir(&workspace).unwrap();
        let manager = FileAccessManager::new(vec![workspace.clone()]);

        std_fs::write(workspace.join("test.txt"), "content").unwrap();
        let exists = manager.file_exists("/workspace/test.txt").await.unwrap();
        assert!(exists);
    }

    #[tokio::test]
    async fn test_file_exists_false() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std_fs::create_dir(&workspace).unwrap();
        let manager = FileAccessManager::new(vec![workspace.clone()]);

        let exists = manager.file_exists("/workspace/test.txt").await.unwrap();
        assert!(!exists);
    }
}
