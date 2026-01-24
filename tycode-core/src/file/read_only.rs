//! Read-only file access module.
//!
//! Provides context components for file tree display and tracked file contents,
//! plus the set_tracked_files tool for managing which files appear in context.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use anyhow::{bail, Result};
use ignore::WalkBuilder;
use serde_json::{json, Value};
use tracing::warn;

use crate::chat::events::{ToolExecutionResult, ToolRequest as ToolRequestEvent, ToolRequestType};
use crate::module::Module;
use crate::module::PromptComponent;
use crate::module::{ContextComponent, ContextComponentId};
use crate::settings::SettingsManager;
use crate::tools::r#trait::{
    ContinuationPreference, ToolCallHandle, ToolCategory, ToolExecutor, ToolOutput, ToolRequest,
};
use crate::tools::ToolName;

use super::access::FileAccessManager;
use super::resolver::Resolver;

pub const FILE_TREE_ID: ContextComponentId = ContextComponentId("file_tree");
pub const TRACKED_FILES_ID: ContextComponentId = ContextComponentId("tracked_files");

/// Module providing read-only file access capabilities.
///
/// Bundles:
/// - FileTreeManager: Shows project file structure in context
/// - TrackedFilesManager: Displays tracked file contents in context and exposes set_tracked_files tool
pub struct ReadOnlyFileModule {
    tracked_files: Arc<TrackedFilesManager>,
    file_tree: Arc<FileTreeManager>,
}

impl ReadOnlyFileModule {
    pub fn new(workspace_roots: Vec<PathBuf>, settings: SettingsManager) -> Result<Self> {
        let tracked_files = Arc::new(TrackedFilesManager::new(workspace_roots.clone())?);
        let file_tree = Arc::new(FileTreeManager::new(workspace_roots, settings)?);
        Ok(Self {
            tracked_files,
            file_tree,
        })
    }

    pub fn tracked_files(&self) -> &Arc<TrackedFilesManager> {
        &self.tracked_files
    }
}

impl Module for ReadOnlyFileModule {
    fn prompt_components(&self) -> Vec<Arc<dyn PromptComponent>> {
        vec![]
    }

    fn context_components(&self) -> Vec<Arc<dyn ContextComponent>> {
        vec![
            self.file_tree.clone() as Arc<dyn ContextComponent>,
            self.tracked_files.clone() as Arc<dyn ContextComponent>,
        ]
    }

    fn tools(&self) -> Vec<Arc<dyn ToolExecutor>> {
        vec![self.tracked_files.clone() as Arc<dyn ToolExecutor>]
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

/// Manages tracked files state and provides both context rendering and tool execution.
pub struct TrackedFilesManager {
    tracked_files: Arc<RwLock<BTreeSet<PathBuf>>>,
    file_manager: FileAccessManager,
}

impl TrackedFilesManager {
    pub fn tool_name() -> ToolName {
        ToolName::new("set_tracked_files")
    }

    pub fn new(workspace_roots: Vec<PathBuf>) -> Result<Self> {
        let file_manager = FileAccessManager::new(workspace_roots)?;
        Ok(Self {
            tracked_files: Arc::new(RwLock::new(BTreeSet::new())),
            file_manager,
        })
    }

    pub fn get_tracked_files(&self) -> Vec<PathBuf> {
        self.tracked_files
            .read()
            .expect("lock poisoned")
            .iter()
            .cloned()
            .collect()
    }

    pub fn clear(&self) {
        self.tracked_files.write().expect("lock poisoned").clear();
    }

    pub fn set_files(&self, files: Vec<PathBuf>) {
        let mut tracked = self.tracked_files.write().expect("lock poisoned");
        tracked.clear();
        tracked.extend(files);
    }

    async fn read_file_contents(&self) -> Vec<(PathBuf, String)> {
        let tracked = self.tracked_files.read().expect("lock poisoned").clone();
        let mut results = Vec::new();

        for path in tracked {
            let path_str = path.to_string_lossy();
            match self.file_manager.read_file(&path_str).await {
                Ok(content) => results.push((path, content)),
                Err(e) => warn!(?e, "Failed to read tracked file: {:?}", path),
            }
        }

        results
    }
}

#[async_trait::async_trait(?Send)]
impl ContextComponent for TrackedFilesManager {
    fn id(&self) -> ContextComponentId {
        TRACKED_FILES_ID
    }

    async fn build_context_section(&self) -> Option<String> {
        let contents = self.read_file_contents().await;
        if contents.is_empty() {
            return None;
        }

        let mut output = String::from("Tracked Files:\n");
        for (path, content) in contents {
            output.push_str(&format!("\n=== {} ===\n{}", path.display(), content));
        }
        Some(output)
    }
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for TrackedFilesManager {
    fn name(&self) -> String {
        "set_tracked_files".to_string()
    }

    fn description(&self) -> String {
        "Set the complete list of files to track for inclusion in all future messages. This replaces any previously tracked files. Minimize tracked files to conserve context. Pass an empty array to clear all tracked files.".to_string()
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_paths": {
                    "type": "array",
                    "items": {
                        "type": "string"
                    },
                    "description": "Array of file paths to track. Empty array clears all tracked files."
                }
            },
            "required": ["file_paths"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Execution
    }

    async fn process(&self, request: &ToolRequest) -> Result<Box<dyn ToolCallHandle>> {
        let mut file_paths_value = request
            .arguments
            .get("file_paths")
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: file_paths"))?
            .clone();

        let file_paths_arr: Vec<String> = loop {
            match file_paths_value {
                Value::Array(arr) => {
                    break arr
                        .into_iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                }
                Value::String(s) => {
                    file_paths_value = serde_json::from_str::<Value>(&s)
                        .map_err(|_| anyhow::anyhow!("file_paths must be an array of strings"))?;
                }
                _ => bail!("file_paths must be an array of strings"),
            }
        };

        let mut valid_paths = Vec::new();
        let mut invalid_files = Vec::new();

        for path_str in file_paths_arr {
            if self.file_manager.file_exists(&path_str).await? {
                valid_paths.push(PathBuf::from(&path_str));
            } else {
                invalid_files.push(path_str);
            }
        }

        if !invalid_files.is_empty() {
            return Err(anyhow::anyhow!(
                "The following files do not exist: {:?}",
                invalid_files
            ));
        }

        Ok(Box::new(SetTrackedFilesHandle {
            file_paths: valid_paths,
            tool_use_id: request.tool_use_id.clone(),
            tracked_files: self.tracked_files.clone(),
        }))
    }
}

struct SetTrackedFilesHandle {
    file_paths: Vec<PathBuf>,
    tool_use_id: String,
    tracked_files: Arc<RwLock<BTreeSet<PathBuf>>>,
}

#[async_trait::async_trait(?Send)]
impl ToolCallHandle for SetTrackedFilesHandle {
    fn tool_request(&self) -> ToolRequestEvent {
        let file_path_strings: Vec<String> = self
            .file_paths
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        ToolRequestEvent {
            tool_call_id: self.tool_use_id.clone(),
            tool_name: "set_tracked_files".to_string(),
            tool_type: ToolRequestType::ReadFiles {
                file_paths: file_path_strings,
            },
        }
    }

    async fn execute(self: Box<Self>) -> ToolOutput {
        {
            let mut tracked = self.tracked_files.write().expect("lock poisoned");
            tracked.clear();
            tracked.extend(self.file_paths.clone());
        }

        let file_path_strings: Vec<String> = self
            .file_paths
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        ToolOutput::Result {
            content: json!({
                "action": "set_tracked_files",
                "tracked_files": file_path_strings
            })
            .to_string(),
            is_error: false,
            continuation: ContinuationPreference::Continue,
            ui_result: ToolExecutionResult::Other {
                result: json!({
                    "action": "set_tracked_files",
                    "tracked_files": file_path_strings
                }),
            },
        }
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
