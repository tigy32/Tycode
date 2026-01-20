//! File search and discovery module.
//!
//! Provides tools for searching, listing, and reading files.

pub mod list_files;
pub mod read_file;
pub mod search_files;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;

use crate::context::ContextComponent;
use crate::file::access::FileAccessManager;
use crate::module::Module;
use crate::module::PromptComponent;
use crate::tools::r#trait::ToolExecutor;

use list_files::ListFilesTool;
use read_file::ReadFileTool;
use search_files::SearchFilesTool;

/// Module providing file search and discovery capabilities.
///
/// Bundles:
/// - SearchFilesTool: Search for regex patterns in files
/// - ListFilesTool: List directory contents
/// - ReadFileTool: Read file contents (deprecated)
pub struct FileSearchModule {
    search_files: Arc<SearchFilesTool>,
    list_files: Arc<ListFilesTool>,
    read_file: Arc<ReadFileTool>,
}

impl FileSearchModule {
    pub fn new(workspace_roots: Vec<PathBuf>) -> Result<Self> {
        let file_manager = FileAccessManager::new(workspace_roots.clone())?;
        Ok(Self {
            search_files: Arc::new(SearchFilesTool::new(file_manager)),
            list_files: Arc::new(ListFilesTool::new(workspace_roots.clone())?),
            read_file: Arc::new(ReadFileTool::new(workspace_roots)?),
        })
    }
}

impl Module for FileSearchModule {
    fn prompt_components(&self) -> Vec<Arc<dyn PromptComponent>> {
        vec![]
    }

    fn context_components(&self) -> Vec<Arc<dyn ContextComponent>> {
        vec![]
    }

    fn tools(&self) -> Vec<Arc<dyn ToolExecutor>> {
        vec![
            self.search_files.clone(),
            self.list_files.clone(),
            self.read_file.clone(),
        ]
    }
}
