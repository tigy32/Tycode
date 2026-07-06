use crate::file::workspace::WorkspacePaths;
use anyhow::{Context, Result};
use ignore::WalkBuilder;
use std::path::PathBuf;
use tokio::fs;

#[derive(Clone)]
pub struct FileAccessManager {
    pub roots: Vec<PathBuf>,
    workspace_paths: WorkspacePaths,
}

impl FileAccessManager {
    pub fn new(workspace_roots: Vec<PathBuf>) -> anyhow::Result<Self> {
        let workspace_paths = WorkspacePaths::new(workspace_roots)?;
        let roots = workspace_paths.roots();

        Ok(Self {
            roots,
            workspace_paths,
        })
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

    pub async fn read_bytes(&self, file_path: &str) -> Result<Vec<u8>> {
        let path = self.resolve(file_path)?;

        if !path.exists() {
            anyhow::bail!("File not found: {}", file_path);
        }

        if !path.is_file() {
            anyhow::bail!("Path is not a file: {}", file_path);
        }

        fs::read(&path)
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

    pub async fn write_bytes(&self, file_path: &str, data: &[u8]) -> Result<()> {
        let path = self.resolve(file_path)?;

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .await
                .with_context(|| format!("Failed to create parent directories for: {file_path}"))?;
        }

        fs::write(&path, data)
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

        let mut paths = Vec::new();

        for result in WalkBuilder::new(&dir_path)
            .hidden(false)
            .filter_entry(|entry| entry.file_name().to_string_lossy() != ".git")
            .max_depth(Some(1))
            .build()
            .skip(1)
        {
            let entry = result?;
            let path = entry.path();

            let Ok(resolved) = self.workspace_paths.contains_existing_path(path) else {
                // Likely a sym link outside of the working directory (or a bug)
                continue;
            };

            paths.push(resolved);
        }

        Ok(paths)
    }

    pub async fn file_exists(&self, file_path: &str) -> Result<bool> {
        let path = self.resolve(file_path)?;
        Ok(path.exists())
    }

    pub fn resolve(&self, path: &str) -> Result<PathBuf> {
        self.workspace_paths.resolve(path)
    }

    pub fn resolve_root(&self, workspace_root: &str) -> Result<PathBuf> {
        self.workspace_paths.resolve_root(workspace_root)
    }

    pub async fn list_all_files_recursive(
        &self,
        workspace_root: &str,
        max_bytes: Option<usize>,
    ) -> Result<Vec<PathBuf>> {
        let real_root = self.resolve_root(workspace_root)?;

        let mut files = Vec::new();
        let root_for_filter = real_root.clone();
        let root_is_git_repo = real_root.join(".git").exists();

        for result in WalkBuilder::new(&real_root)
            .hidden(false)
            .filter_entry(move |entry| {
                if entry.file_name().to_string_lossy() == ".git" {
                    return false;
                }

                if root_is_git_repo && entry.file_type().is_some_and(|ft| ft.is_dir()) {
                    let is_root = entry.path() == root_for_filter;
                    if !is_root && entry.path().join(".git").exists() {
                        return false;
                    }
                }
                true
            })
            .build()
        {
            let entry = result?;
            let path = entry.path();

            if !path.is_file() {
                continue;
            }

            let Ok(resolved) = self.workspace_paths.contains_existing_path(path) else {
                // Likely a sym link outside of the working directory (or a bug)
                continue;
            };

            files.push(resolved);
        }

        if let Some(limit) = max_bytes {
            Ok(Self::truncate_by_bytes(files, limit))
        } else {
            Ok(files)
        }
    }

    fn truncate_by_bytes(files: Vec<PathBuf>, max_bytes: usize) -> Vec<PathBuf> {
        let mut result = Vec::new();
        let mut current_bytes = 0;

        for file in files {
            let file_bytes = file.to_string_lossy().len() + 1;
            if current_bytes + file_bytes > max_bytes {
                break;
            }
            current_bytes += file_bytes;
            result.push(file);
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs as std_fs;
    use std::path::Path;
    use tempfile::tempdir;

    fn path_str(path: &Path) -> String {
        path.to_string_lossy().to_string()
    }

    #[tokio::test]
    async fn test_new() {
        let roots = vec![std::env::current_dir().unwrap()];
        let manager = FileAccessManager::new(roots.clone()).unwrap();
        assert_eq!(manager.roots.len(), 1);
    }

    #[tokio::test]
    async fn test_read_file_success() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std_fs::create_dir(&workspace).unwrap();
        let manager = FileAccessManager::new(vec![workspace.clone()]).unwrap();

        std_fs::write(workspace.join("test.txt"), "content").unwrap();
        let content = manager
            .read_file(&path_str(&workspace.join("test.txt")))
            .await
            .unwrap();
        assert_eq!(content, "content");
    }

    #[tokio::test]
    async fn test_read_file_not_found() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std_fs::create_dir(&workspace).unwrap();
        let manager = FileAccessManager::new(vec![workspace.clone()]).unwrap();

        let err = manager
            .read_file(&path_str(&workspace.join("nonexistent.txt")))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("File not found"));
    }

    #[tokio::test]
    async fn test_read_file_not_file() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std_fs::create_dir(&workspace).unwrap();
        let manager = FileAccessManager::new(vec![workspace.clone()]).unwrap();

        std_fs::create_dir(workspace.join("dir")).unwrap();
        let err = manager
            .read_file(&path_str(&workspace.join("dir")))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Path is not a file"));
    }

    #[tokio::test]
    async fn test_write_file_success() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std_fs::create_dir(&workspace).unwrap();
        let manager = FileAccessManager::new(vec![workspace.clone()]).unwrap();

        manager
            .write_file(&path_str(&workspace.join("subdir/test.txt")), "content")
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
        let manager = FileAccessManager::new(vec![workspace.clone()]).unwrap();

        let path = workspace.join("test.txt");
        std_fs::write(&path, "content").unwrap();
        manager.delete_file(&path_str(&path)).await.unwrap();
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn test_delete_directory_success() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std_fs::create_dir(&workspace).unwrap();
        let manager = FileAccessManager::new(vec![workspace.clone()]).unwrap();

        let dir_path = workspace.join("testdir");
        std_fs::create_dir(&dir_path).unwrap();
        manager.delete_file(&path_str(&dir_path)).await.unwrap();
        assert!(!dir_path.exists());
    }

    #[tokio::test]
    async fn test_delete_file_not_found() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std_fs::create_dir(&workspace).unwrap();
        let manager = FileAccessManager::new(vec![workspace.clone()]).unwrap();

        let err = manager
            .delete_file(&path_str(&workspace.join("nonexistent.txt")))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Failed to get metadata"));
    }

    #[tokio::test]
    async fn test_list_directory_success() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std_fs::create_dir(&workspace).unwrap();
        let manager = FileAccessManager::new(vec![workspace.clone()]).unwrap();

        std_fs::write(workspace.join("a.txt"), "content").unwrap();
        std_fs::write(workspace.join("b.txt"), "content").unwrap();

        let list = manager.list_directory(&path_str(&workspace)).await.unwrap();
        assert_eq!(list.len(), 2);
        assert!(list.contains(&workspace.join("a.txt").canonicalize().unwrap()));
        assert!(list.contains(&workspace.join("b.txt").canonicalize().unwrap()));
    }

    #[tokio::test]
    async fn test_list_directory_not_found() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std_fs::create_dir(&workspace).unwrap();
        let manager = FileAccessManager::new(vec![workspace.clone()]).unwrap();

        let err = manager
            .list_directory(&path_str(&workspace.join("nonexistent")))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Directory not found"));
    }

    #[tokio::test]
    async fn test_list_directory_not_dir() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std_fs::create_dir(&workspace).unwrap();
        let manager = FileAccessManager::new(vec![workspace.clone()]).unwrap();

        std_fs::write(workspace.join("file.txt"), "content").unwrap();

        let err = manager
            .list_directory(&path_str(&workspace.join("file.txt")))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("Path is not a directory"));
    }

    #[tokio::test]
    async fn test_file_exists_true() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std_fs::create_dir(&workspace).unwrap();
        let manager = FileAccessManager::new(vec![workspace.clone()]).unwrap();

        std_fs::write(workspace.join("test.txt"), "content").unwrap();
        let exists = manager
            .file_exists(&path_str(&workspace.join("test.txt")))
            .await
            .unwrap();
        assert!(exists);
    }

    #[tokio::test]
    async fn test_file_exists_false() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std_fs::create_dir(&workspace).unwrap();
        let manager = FileAccessManager::new(vec![workspace.clone()]).unwrap();

        let exists = manager
            .file_exists(&path_str(&workspace.join("test.txt")))
            .await
            .unwrap();
        assert!(!exists);
    }
}
