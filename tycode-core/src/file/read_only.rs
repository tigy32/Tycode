//! Read-only file access module.
//!
//! Provides a context component for file tree display.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use ignore::WalkBuilder;
use tracing::warn;

use crate::module::Module;
use crate::module::PromptComponent;
use crate::module::{ContextComponent, ContextComponentId};
use crate::settings::SettingsManager;
use crate::tools::r#trait::SharedTool;

use super::config::File;
use super::workspace::WorkspacePaths;

pub const FILE_TREE_ID: ContextComponentId = ContextComponentId("file_tree");

/// Module providing read-only file access capabilities.
pub struct ReadOnlyFileModule {
    file_tree: Arc<FileTreeManager>,
}

impl ReadOnlyFileModule {
    pub fn new(workspace_roots: Vec<PathBuf>, settings: SettingsManager) -> Result<Self> {
        let file_tree = Arc::new(FileTreeManager::new(workspace_roots, settings)?);
        Ok(Self { file_tree })
    }
}

#[async_trait::async_trait(?Send)]
impl Module for ReadOnlyFileModule {
    fn prompt_components(&self) -> Vec<Arc<dyn PromptComponent>> {
        vec![]
    }

    fn context_components(&self) -> Vec<Arc<dyn ContextComponent>> {
        vec![self.file_tree.clone() as Arc<dyn ContextComponent>]
    }

    async fn tools(&self) -> Vec<SharedTool> {
        vec![]
    }

    fn settings_namespace(&self) -> Option<&'static str> {
        Some(File::NAMESPACE)
    }

    fn settings_json_schema(&self) -> Option<schemars::schema::RootSchema> {
        Some(schemars::schema_for!(File))
    }
}

/// Manages file tree state and renders project structure to context.
pub struct FileTreeManager {
    workspace_paths: WorkspacePaths,
    settings: SettingsManager,
}

impl FileTreeManager {
    pub fn new(workspace_roots: Vec<PathBuf>, settings: SettingsManager) -> Result<Self> {
        let workspace_paths = WorkspacePaths::new(workspace_roots)?;
        Ok(Self {
            workspace_paths,
            settings,
        })
    }

    pub(crate) fn list_files(&self) -> Vec<PathBuf> {
        let mut all_files = Vec::new();

        for real_root in &self.workspace_paths.roots() {
            let root_for_filter = real_root.clone();
            let root_is_git_repo = real_root.join(".git").exists();

            for result in WalkBuilder::new(real_root)
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
                let entry = match result {
                    Ok(e) => e,
                    Err(e) => {
                        warn!(
                            ?e,
                            "Failed to read directory entry during file tree traversal"
                        );
                        continue;
                    }
                };
                let path = entry.path();

                if !path.is_file() {
                    continue;
                }

                let resolved = match self.workspace_paths.contains_existing_path(path) {
                    Ok(r) => r,
                    Err(e) => {
                        warn!(?e, "Failed to canonicalize path: {:?}", path);
                        continue;
                    }
                };

                all_files.push(resolved);
            }
        }

        let file_config: File = self.settings.get_module_config(File::NAMESPACE);
        let max_bytes = file_config.auto_context_bytes;
        Self::truncate_by_bytes(all_files, max_bytes)
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

#[async_trait::async_trait(?Send)]
impl ContextComponent for FileTreeManager {
    fn id(&self) -> ContextComponentId {
        FILE_TREE_ID
    }

    async fn build_context_section(&self) -> Option<String> {
        let files = self.list_files();
        let roots = self.workspace_paths.roots();
        if roots.is_empty() && files.is_empty() {
            return None;
        }

        let mut output = String::new();
        if !roots.is_empty() {
            output.push_str("Working directories (project roots):\n");
            for root in &roots {
                output.push_str("- ");
                output.push_str(&root.to_string_lossy());
                output.push('\n');
            }
            output.push('\n');
        }

        if !files.is_empty() {
            output.push_str("Project Files:\n");
            output.push_str(&build_file_tree(&files));
            output.push('\n');
        }
        output.push_str(
            "(This listing is a point-in-time snapshot and may be truncated for large \
             projects; other files may exist or may have changed. Verify with bash/read \
             when it matters.)",
        );
        Some(output)
    }
}

#[derive(Default)]
struct TrieNode {
    children: BTreeMap<String, TrieNode>,
    is_file: bool,
}

impl TrieNode {
    fn insert_path(&mut self, components: &[String]) {
        if components.is_empty() {
            return;
        }

        let is_file = components.len() == 1;
        let child = self.children.entry(components[0].clone()).or_default();

        if is_file {
            child.is_file = true;
        } else {
            child.insert_path(&components[1..]);
        }
    }

    fn render(&self, output: &mut String, depth: usize) {
        let indent = "  ".repeat(depth);

        for (name, child) in &self.children {
            output.push_str(&indent);
            output.push_str(name);

            if name != "/" && !child.is_file {
                output.push('/');
            }
            output.push('\n');

            child.render(output, depth + 1);
        }
    }
}

fn build_file_tree(files: &[PathBuf]) -> String {
    if files.is_empty() {
        return String::new();
    }

    let mut root = TrieNode::default();

    for file_path in files {
        let components = display_components(file_path);
        root.insert_path(&components);
    }

    let mut result = String::new();
    root.render(&mut result, 0);
    result
}

fn display_components(path: &Path) -> Vec<String> {
    path.components()
        .filter_map(|component| match component {
            Component::Prefix(prefix) => Some(prefix.as_os_str().to_string_lossy().to_string()),
            Component::RootDir => Some("/".to_string()),
            Component::Normal(part) => Some(part.to_string_lossy().to_string()),
            Component::CurDir | Component::ParentDir => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::agent::Agent;
    use crate::agents::one_shot::OneShotAgent;
    use crate::agents::tycode::TycodeAgent;
    use crate::module::ContextComponentSelection;
    use std::fs as std_fs;
    use tempfile::tempdir;

    fn settings_in(dir: &std::path::Path) -> SettingsManager {
        SettingsManager::from_path(dir.join("settings.toml")).unwrap()
    }

    #[tokio::test]
    async fn file_tree_section_lists_roots_files_and_caveat() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("workspace");
        std_fs::create_dir(&workspace).unwrap();
        std_fs::write(workspace.join("main.rs"), "fn main() {}").unwrap();

        let manager =
            FileTreeManager::new(vec![workspace.clone()], settings_in(temp.path())).unwrap();
        let section = manager.build_context_section().await.expect("has files");

        assert!(
            section.contains("Working directories (project roots):"),
            "missing roots header:\n{section}"
        );
        assert!(
            section.contains(&format!(
                "- {}",
                workspace.canonicalize().unwrap().display()
            )),
            "missing real root:\n{section}"
        );
        assert!(
            section.contains("Project Files:"),
            "missing tree:\n{section}"
        );
        assert!(section.contains("main.rs"), "missing file:\n{section}");
        assert!(
            section.contains("may have changed"),
            "missing freshness caveat:\n{section}"
        );
    }

    /// An empty (or fully ignored) workspace still advertises its roots so the
    /// model knows where it is, even with nothing to list.
    #[tokio::test]
    async fn file_tree_section_shows_roots_when_no_files() {
        let temp = tempdir().unwrap();
        let workspace = temp.path().join("empty-proj");
        std_fs::create_dir(&workspace).unwrap();

        let manager =
            FileTreeManager::new(vec![workspace.clone()], settings_in(temp.path())).unwrap();
        let section = manager
            .build_context_section()
            .await
            .expect("roots present even with no files");

        assert!(
            section.contains(&format!(
                "- {}",
                workspace.canonicalize().unwrap().display()
            )),
            "missing real root:\n{section}"
        );
        assert!(
            !section.contains("Project Files:"),
            "should not render an empty tree:\n{section}"
        );
    }

    /// The conversational roots must include the file tree; sub-agents keep the
    /// lean default that excludes it. This is the regression guard for the file
    /// listing that a "simplify defaults" refactor silently dropped.
    #[test]
    fn conversational_roots_request_the_file_tree() {
        for selection in [
            TycodeAgent.requested_context_components(),
            OneShotAgent::new().requested_context_components(),
        ] {
            let includes_tree = match selection {
                ContextComponentSelection::All => true,
                ContextComponentSelection::Only(ids) => ids.contains(&FILE_TREE_ID),
                ContextComponentSelection::Exclude(ids) => !ids.contains(&FILE_TREE_ID),
                ContextComponentSelection::None => false,
            };
            assert!(includes_tree, "root agent must include the file tree");
        }
    }
}
