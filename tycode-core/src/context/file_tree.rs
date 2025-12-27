use std::collections::BTreeMap;
use std::path::PathBuf;

use ignore::WalkBuilder;

use crate::context::{ContextComponent, ContextComponentId};
use crate::file::resolver::Resolver;
use crate::settings::SettingsManager;

pub const ID: ContextComponentId = ContextComponentId("file_tree");

/// Manages file tree state and renders project structure to context.
pub struct FileTreeManager {
    resolver: Resolver,
    settings: SettingsManager,
}

impl FileTreeManager {
    pub fn new(
        workspace_roots: Vec<PathBuf>,
        settings: SettingsManager,
    ) -> Result<Self, anyhow::Error> {
        let resolver = Resolver::new(workspace_roots)?;
        Ok(Self { resolver, settings })
    }

    /// List all files synchronously using WalkBuilder.
    fn list_files(&self) -> Vec<PathBuf> {
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
                let Ok(entry) = result else { continue };
                let path = entry.path();

                if !path.is_file() {
                    continue;
                }

                let Ok(resolved) = self.resolver.canonicalize(path) else {
                    continue;
                };

                all_files.push(resolved.virtual_path);
            }
        }

        let max_bytes = self.settings.settings().auto_context_bytes;
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
        ID
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
