use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use tracing::warn;

use crate::chat::events::{ContextInfo, FileInfo};
use crate::cmd::CommandResult;
use crate::file::access::FileAccessManager;
use crate::memory::MemoryLog;
use crate::settings::config::Settings;
use crate::tools::r#trait::ToolExecutor;
use crate::tools::tasks::TaskList;

/// Input data required for context building, decoupled from ActorState.
/// This allows context building to be used by both chat/ai.rs and agents/runner.rs.
pub struct ContextInputs {
    pub workspace_roots: Vec<PathBuf>,
    pub tracked_files: Vec<PathBuf>,
    pub task_list: TaskList,
    pub command_outputs: Vec<CommandResult>,
    pub memory_log: Arc<MemoryLog>,
    pub additional_tools: Vec<Arc<dyn ToolExecutor>>,
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
        let child = self
            .children
            .entry(components[0].to_string())
            .or_insert_with(TrieNode::default);

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

#[derive(Debug, Clone)]
pub struct AllFiles {
    pub files: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct MessageContext {
    pub working_directories: Vec<PathBuf>,
    pub relevant_files: Vec<PathBuf>,
    pub tracked_file_contents: BTreeMap<PathBuf, String>,
    pub task_list: TaskList,
    pub command_outputs: Vec<CommandResult>,
}

impl MessageContext {
    pub fn new(working_directories: Vec<PathBuf>, task_list: TaskList) -> Self {
        Self {
            working_directories,
            relevant_files: Vec::new(),
            tracked_file_contents: BTreeMap::new(),
            task_list,
            command_outputs: Vec::new(),
        }
    }

    pub fn add_tracked_file(&mut self, path: PathBuf, content: String) {
        self.tracked_file_contents.insert(path, content);
    }

    pub fn set_relevant_files(&mut self, files: Vec<PathBuf>) {
        self.relevant_files = files;
    }

    pub fn set_command_outputs(&mut self, outputs: Vec<CommandResult>) {
        self.command_outputs = outputs;
    }

    pub fn get_context_size(&self) -> usize {
        self.tracked_file_contents.values().map(|s| s.len()).sum()
    }

    pub fn to_formatted_string(&self, include_file_list: bool) -> String {
        let mut result = String::new();

        if !self.task_list.tasks.is_empty() {
            result.push_str(&format!("Task List: {}\n", self.task_list.title));
            for task in &self.task_list.tasks {
                result.push_str(&format!(
                    "  - [{:?}] Task {}: {}\n",
                    task.status, task.id, task.description
                ));
            }
            result.push('\n');
        }

        if include_file_list && !self.relevant_files.is_empty() {
            result.push_str("Project Files:\n");
            result.push_str(&self.build_file_tree());
            result.push('\n');
        }

        if !self.tracked_file_contents.is_empty() {
            result.push_str("Tracked Files:\n");
            for (path, content) in &self.tracked_file_contents {
                result.push_str(&format!("\n=== {} ===\n", path.display()));
                result.push_str(content);
                result.push('\n');
            }
        }

        if !self.command_outputs.is_empty() {
            result.push_str("Command Outputs:\n");
            for output in &self.command_outputs {
                result.push_str(&format!("\nCommand: {}\n", output.command));
                result.push_str(&format!("Exit Code: {}\n", output.code));
                if !output.out.is_empty() {
                    result.push_str("Stdout:\n");
                    result.push_str(&output.out);
                    result.push('\n');
                }
                if !output.err.is_empty() {
                    result.push_str("Stderr:\n");
                    result.push_str(&output.err);
                    result.push('\n');
                }
            }
        }

        result
    }

    fn build_file_tree(&self) -> String {
        if self.relevant_files.is_empty() {
            return String::new();
        }

        let mut root = TrieNode::default();

        for file_path in &self.relevant_files {
            let path_str = file_path.to_string_lossy();
            let components: Vec<&str> = path_str.split('/').filter(|s| !s.is_empty()).collect();
            root.insert_path(&components);
        }

        let mut result = String::new();
        root.render(&mut result, 0);
        result
    }
}

pub async fn build_message_context(
    workspace_roots: &[PathBuf],
    tracked_files: &[PathBuf],
    task_list: TaskList,
    command_outputs: Vec<CommandResult>,
    max_bytes: usize,
) -> Result<MessageContext, anyhow::Error> {
    let mut context = MessageContext::new(workspace_roots.to_vec(), task_list);

    let file_manager = FileAccessManager::new(workspace_roots.to_vec())?;
    let all_files = list_all_files(&file_manager, max_bytes).await?;
    context.set_relevant_files(all_files.files);

    let file_manager = FileAccessManager::new(workspace_roots.to_vec())?;

    for file_path in tracked_files {
        let path_str = file_path.to_string_lossy();
        match file_manager.read_file(&path_str).await {
            Ok(content) => {
                context.add_tracked_file(file_path.clone(), content);
            }
            Err(e) => {
                warn!(?e, "Failed to read tracked file: {:?}", file_path);
            }
        }
    }

    context.set_command_outputs(command_outputs);

    Ok(context)
}

async fn list_all_files(
    file_manager: &FileAccessManager,
    max_bytes: usize,
) -> Result<AllFiles, anyhow::Error> {
    let mut all_files = Vec::new();

    for root in &file_manager.roots {
        let files = file_manager
            .list_all_files_recursive(root, Some(max_bytes))
            .await?;
        warn!("Collected {} files from root {}", files.len(), root);
        all_files.extend(files);
    }

    warn!("Total files collected: {}", all_files.len());
    Ok(AllFiles { files: all_files })
}

pub fn create_context_info(message_context: &MessageContext) -> ContextInfo {
    let dir_list_size = message_context
        .relevant_files
        .iter()
        .map(|p| p.to_string_lossy().len() + 1)
        .sum::<usize>();

    let files: Vec<FileInfo> = message_context
        .tracked_file_contents
        .iter()
        .map(|(path, content)| FileInfo {
            path: path.to_string_lossy().to_string(),
            bytes: content.len(),
        })
        .collect();

    ContextInfo {
        directory_list_bytes: dir_list_size,
        files,
    }
}

pub async fn build_context(
    inputs: &ContextInputs,
    settings: &Settings,
) -> Result<(String, ContextInfo), anyhow::Error> {
    let message_context = build_message_context(
        &inputs.workspace_roots,
        &inputs.tracked_files,
        inputs.task_list.clone(),
        inputs.command_outputs.clone(),
        settings.auto_context_bytes,
    )
    .await?;
    let context_info = create_context_info(&message_context);

    let context_string = message_context.to_formatted_string(true);
    let memories_string = format_recent_memories(&inputs.memory_log, settings);
    let context_text = format!("Current Context:\n{memories_string}{context_string}");

    Ok((context_text, context_info))
}

fn format_recent_memories(memory_log: &MemoryLog, settings: &Settings) -> String {
    if !settings.memory.enabled {
        return String::new();
    }

    let memories = match memory_log.read_all() {
        Ok(m) => m,
        Err(e) => {
            warn!("Failed to read memories: {e:?}");
            return String::new();
        }
    };

    if memories.is_empty() {
        return String::new();
    }

    let recent: Vec<_> = memories
        .into_iter()
        .rev()
        .take(settings.memory.recent_memories_count)
        .collect();

    let mut result = String::from("Recent Memories:\n");
    for memory in &recent {
        result.push_str(&format!("- {}\n", memory.content));
    }
    result.push('\n');
    result
}
