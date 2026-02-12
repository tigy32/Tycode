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

use crate::modules::execution::{compact_output, config::ExecutionConfig};

use crate::chat::actor::ActorState;
use crate::chat::events::{
    ChatMessage, ToolExecutionResult, ToolRequest as ToolRequestEvent, ToolRequestType,
};
use crate::module::Module;
use crate::module::PromptComponent;
use crate::module::{ContextComponent, ContextComponentId, SlashCommand};
use crate::settings::SettingsManager;
use crate::tools::r#trait::{
    ContinuationPreference, ToolCallHandle, ToolCategory, ToolExecutor, ToolOutput, ToolRequest,
};
use crate::tools::ToolName;

use super::access::FileAccessManager;
use super::config::File;
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
        let tracked_files = Arc::new(TrackedFilesManager::new(
            workspace_roots.clone(),
            settings.clone(),
        )?);
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

    fn slash_commands(&self) -> Vec<Arc<dyn SlashCommand>> {
        vec![Arc::new(FileInjectSlashCommand {
            tracked_files: self.tracked_files.clone(),
            file_tree: self.file_tree.clone(),
        })]
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

struct TrackedFilesInner {
    ai_tracked: BTreeSet<PathBuf>,
    user_pinned: BTreeSet<PathBuf>,
}

/// Manages tracked files state and provides both context rendering and tool execution.
pub struct TrackedFilesManager {
    inner: Arc<RwLock<TrackedFilesInner>>,
    pub(crate) file_manager: FileAccessManager,
    settings: SettingsManager,
}

impl TrackedFilesManager {
    pub fn tool_name() -> ToolName {
        ToolName::new("set_tracked_files")
    }

    pub fn new(workspace_roots: Vec<PathBuf>, settings: SettingsManager) -> Result<Self> {
        let file_manager = FileAccessManager::new(workspace_roots)?;
        Ok(Self {
            inner: Arc::new(RwLock::new(TrackedFilesInner {
                ai_tracked: BTreeSet::new(),
                user_pinned: BTreeSet::new(),
            })),
            file_manager,
            settings,
        })
    }

    pub fn get_tracked_files(&self) -> Vec<PathBuf> {
        let inner = self.inner.read().expect("lock poisoned");
        inner
            .ai_tracked
            .union(&inner.user_pinned)
            .cloned()
            .collect()
    }

    pub fn clear(&self) {
        self.inner
            .write()
            .expect("lock poisoned")
            .ai_tracked
            .clear();
    }

    pub fn set_files(&self, files: Vec<PathBuf>) {
        let mut inner = self.inner.write().expect("lock poisoned");
        inner.ai_tracked.clear();
        for file in files {
            if !inner.user_pinned.contains(&file) {
                inner.ai_tracked.insert(file);
            }
        }
    }

    pub fn pin_files(&self, files: Vec<PathBuf>) {
        let mut inner = self.inner.write().expect("lock poisoned");
        for file in files {
            inner.ai_tracked.remove(&file);
            inner.user_pinned.insert(file);
        }
    }

    pub fn unpin_all(&self) {
        self.inner
            .write()
            .expect("lock poisoned")
            .user_pinned
            .clear();
    }

    pub fn get_pinned_files(&self) -> Vec<PathBuf> {
        self.inner
            .read()
            .expect("lock poisoned")
            .user_pinned
            .iter()
            .cloned()
            .collect()
    }

    async fn read_file_contents(&self) -> Vec<(PathBuf, String)> {
        let all_files: BTreeSet<PathBuf> = {
            let inner = self.inner.read().expect("lock poisoned");
            inner
                .ai_tracked
                .union(&inner.user_pinned)
                .cloned()
                .collect()
        };
        let mut results = Vec::new();

        for path in all_files {
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

        let execution_config: ExecutionConfig = self.settings.get_module_config("execution");
        let max_bytes = execution_config.max_output_bytes.unwrap_or(200_000);

        let mut output = String::from("Tracked Files:\n");
        for (path, content) in contents {
            let content = compact_output(&content, max_bytes);
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
        "Set the complete list of files to track for inclusion in all future messages. Each call REPLACES ALL previously tracked files — include every file you need in a single call. Do NOT make multiple calls per turn; only the last call takes effect, wasting earlier calls. Pass an empty array to clear all tracked files. Minimize tracked files to conserve context.".to_string()
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
            inner: self.inner.clone(),
        }))
    }
}

struct SetTrackedFilesHandle {
    file_paths: Vec<PathBuf>,
    tool_use_id: String,
    inner: Arc<RwLock<TrackedFilesInner>>,
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
        let mut inner = self.inner.write().expect("lock poisoned");
        inner.ai_tracked.clear();
        for path in &self.file_paths {
            if !inner.user_pinned.contains(path) {
                inner.ai_tracked.insert(path.clone());
            }
        }
        drop(inner);

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

/// Slash command for injecting files into context that the AI cannot remove.
struct FileInjectSlashCommand {
    tracked_files: Arc<TrackedFilesManager>,
    file_tree: Arc<FileTreeManager>,
}

impl FileInjectSlashCommand {
    async fn pin_single_file(&self, path_str: &str) -> Vec<ChatMessage> {
        let exists = self.tracked_files.file_manager.file_exists(path_str).await;
        match exists {
            Ok(false) => return vec![ChatMessage::error(format!("File not found: {path_str}"))],
            Err(e) => return vec![ChatMessage::error(format!("Error checking file: {e:?}"))],
            Ok(true) => {}
        }
        self.tracked_files.pin_files(vec![PathBuf::from(path_str)]);
        vec![ChatMessage::system(format!("Pinned: {path_str}"))]
    }
}

#[async_trait::async_trait(?Send)]
impl SlashCommand for FileInjectSlashCommand {
    fn name(&self) -> &'static str {
        "@"
    }

    fn description(&self) -> &'static str {
        "Pin files into context (AI cannot remove). /@ <path>, /@ all, /@ clear, /@ list"
    }

    fn usage(&self) -> &'static str {
        "/@ <file_path> | /@ all | /@ clear | /@ list"
    }

    async fn execute(&self, _state: &mut ActorState, args: &[&str]) -> Vec<ChatMessage> {
        let Some(subcommand) = args.first() else {
            return vec![ChatMessage::system(
                "Usage: /@ <file_path> | /@ all | /@ clear | /@ list".to_string(),
            )];
        };

        match *subcommand {
            "all" => {
                let files = self.file_tree.list_files();
                let count = files.len();
                self.tracked_files.pin_files(files);
                vec![ChatMessage::system(format!(
                    "Pinned {count} files from file tree."
                ))]
            }
            "clear" => {
                self.tracked_files.unpin_all();
                vec![ChatMessage::system("All pinned files cleared.".to_string())]
            }
            "list" => {
                let pinned = self.tracked_files.get_pinned_files();
                if pinned.is_empty() {
                    return vec![ChatMessage::system("No pinned files.".to_string())];
                }
                let mut msg = format!("Pinned files ({}):\n", pinned.len());
                for path in &pinned {
                    msg.push_str(&format!("  {}\n", path.display()));
                }
                vec![ChatMessage::system(msg)]
            }
            path => self.pin_single_file(path).await,
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
