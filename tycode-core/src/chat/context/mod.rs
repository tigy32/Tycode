use crate::chat::actor::ActorState;
use crate::chat::events::{ContextInfo, FileInfo};
use crate::file::access::FileAccessManager;
use crate::tools::tasks::TaskList;
use std::collections::BTreeMap;
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
    pub tracked_file_contents: BTreeMap<PathBuf, String>,
    pub task_list: TaskList,
}

impl MessageContext {
    pub fn new(working_directories: Vec<PathBuf>, task_list: TaskList) -> Self {
        Self {
            working_directories,
            relevant_files: Vec::new(),
            tracked_file_contents: BTreeMap::new(),
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
) -> Result<MessageContext, anyhow::Error> {
    let mut context = MessageContext::new(workspace_roots.to_vec(), task_list);

    let file_manager = FileAccessManager::new(workspace_roots.to_vec());
    let all_files = list_all_files(&file_manager).await?;
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

    Ok(context)
}

async fn list_all_files(file_manager: &FileAccessManager) -> Result<AllFiles, anyhow::Error> {
    let mut all_files = Vec::new();

    for root in &file_manager.roots {
        let files = file_manager.list_all_files_recursive(root).await?;
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
    state: &ActorState,
    auto_context_bytes: usize,
) -> Result<(String, ContextInfo), anyhow::Error> {
    let tracked_files: Vec<PathBuf> = state.tracked_files.iter().cloned().collect();
    let message_context = build_message_context(
        &state.workspace_roots,
        &tracked_files,
        state.task_list.clone(),
    )
    .await?;
    let context_info = create_context_info(&message_context);

    let include_file_list = context_info.directory_list_bytes <= auto_context_bytes;
    let context_string = message_context.to_formatted_string(include_file_list);
    let context_text = format!("Current Context:\n{context_string}");

    Ok((context_text, context_info))
}
