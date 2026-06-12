//! Read-only file access module.
//!
//! Provides a context component for file tree display.

use std::collections::BTreeMap;
use std::path::PathBuf;
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
use super::resolver::Resolver;

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
    resolver: Resolver,
    settings: SettingsManager,
}

impl FileTreeManager {
    pub fn new(workspace_roots: Vec<PathBuf>, settings: SettingsManager) -> Result<Self> {
        let resolver = Resolver::new(workspace_roots)?;
        Ok(Self { resolver, settings })
    }

    pub(crate) fn list_files(&self) -> Vec<PathBuf> {
        let mut all_files = Vec::new();

        for workspace in &self.resolver.roots() {
            let Some(real_root) = self.resolver.root(workspace) else {
                continue;
            };

            let root_for_filter = real_root.clone();
            let root_is_git_repo = real_root.join(".git").exists();

            for result in WalkBuilder::new(&real_root)
                .hidden(false)
                .filter_entry(move |entry| {
                    if entry.file_name().to_string_lossy() == ".git" {
                        return false;
                    }
                    if root_is_git_repo && entry.file_type().map_or(false, |ft| ft.is_dir()) {
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

                let resolved = match self.resolver.canonicalize(path) {
                    Ok(r) => r,
                    Err(e) => {
                        warn!(?e, "Failed to canonicalize path: {:?}", path);
                        continue;
                    }
                };

                all_files.push(resolved.virtual_path);
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
        if files.is_empty() {
            return None;
        }

        let mut output = String::from("Project Files:\n");
        output.push_str(&build_file_tree(&files));
        Some(output)
    }
}

#[derive(Default)]
struct TrieNode {
    children: BTreeMap<String, TrieNode>,
    is_file: bool,
}

impl TrieNode {
    fn insert_path(&mut self, components: &[&str]) {
        if components.is_empty() {
            return;
        }

        let is_file = components.len() == 1;
        let child = self.children.entry(components[0].to_string()).or_default();

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

            if !child.is_file {
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
        let path_str = file_path.to_string_lossy();
        let components: Vec<&str> = path_str.split('/').filter(|s| !s.is_empty()).collect();
        root.insert_path(&components);
    }

    let mut result = String::new();
    root.render(&mut result, 0);
    result
}
