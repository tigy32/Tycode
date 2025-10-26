use crate::chat::events::{ContextInfo, FileInfo};
use crate::file::access::FileAccessManager;
use crate::tools::tasks::TaskList;
use std::collections::HashMap;
use std::path::PathBuf;
use tracing::warn;

#[derive(Debug, Clone)]
pub struct AllFiles {
    pub files: Vec<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct MessageContext {
    pub working_directories: Vec<PathBuf>,
    pub relevant_files: Vec<PathBuf>,
    pub tracked_file_contents: HashMap<PathBuf, String>,
    pub task_list: TaskList,
}

impl MessageContext {
    pub fn new(working_directories: Vec<PathBuf>, task_list: TaskList) -> Self {
        Self {
            working_directories,
            relevant_files: Vec::new(),
            tracked_file_contents: HashMap::new(),
            task_list,
        }
    }

    pub fn add_tracked_file(&mut self, path: PathBuf, content: String) {
        self.tracked_file_contents.insert(path, content);
    }

    pub fn set_relevant_files(&mut self, files: Vec<PathBuf>) {
        self.relevant_files = files;
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

        result
    }

    fn build_file_tree(&self) -> String {
        // Changed to flat list for easier AI parsing; sorted for deterministic order.
        let mut sorted_files: Vec<_> = self
            .relevant_files
            .iter()
            .map(|p| p.to_string_lossy())
            .collect();
        sorted_files.sort();

        let mut result = String::new();
        for file in sorted_files {
            result.push_str(&format!("  - {file}\n"));
        }
        result
    }
}

pub async fn build_message_context(
    workspace_roots: &[PathBuf],
    tracked_files: &[PathBuf],
    task_list: TaskList,
) -> MessageContext {
    let mut context = MessageContext::new(workspace_roots.to_vec(), task_list);

    let file_manager = FileAccessManager::new(workspace_roots.to_vec());
    let all_files = list_all_files(&file_manager).await;
    context.set_relevant_files(all_files.files);

    let file_manager = FileAccessManager::new(workspace_roots.to_vec());

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

    context
}

async fn list_all_files(file_manager: &FileAccessManager) -> AllFiles {
    let mut all_files = Vec::new();

    for root in &file_manager.roots {
        match collect_files_recursively(file_manager, root).await {
            Ok(files) => {
                warn!("Collected {} files from root: {}", files.len(), root);
                all_files.extend(files);
            }
            Err(e) => {
                warn!("Failed to collect files from root {}: {:?}", root, e);
            }
        }
    }

    warn!("Total files collected: {}", all_files.len());
    AllFiles { files: all_files }
}

async fn collect_files_recursively(
    file_manager: &FileAccessManager,
    directory_path: &str,
) -> Result<Vec<PathBuf>, anyhow::Error> {
    let mut files = Vec::new();

    let entries = file_manager.list_directory(directory_path).await?;

    for entry in entries {
        let entry_str = entry.to_string_lossy();

        // Check if this entry exists and get metadata
        if file_manager.file_exists(&entry_str).await.unwrap_or(false) {
            // Try to list it as a directory - if this succeeds, it's a directory
            if let Ok(_) = file_manager.list_directory(&entry_str).await {
                // It's a directory, recurse into it
                if let Ok(subfiles) =
                    Box::pin(collect_files_recursively(file_manager, &entry_str)).await
                {
                    files.extend(subfiles);
                }
            } else {
                // It's a file, add it to our list
                files.push(entry);
            }
        }
    }

    Ok(files)
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
