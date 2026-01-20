//! File modification module.
//!
//! Provides tools for creating, updating, and deleting files.
//! The modify_file tool implementation is selected based on FileModificationApi setting.

pub mod apply_codex_patch;
pub mod delete_file;
pub mod replace_in_file;
pub mod write_file;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;

use crate::context::ContextComponent;
use crate::module::Module;
use crate::prompt::PromptComponent;
use crate::settings::config::FileModificationApi;
use crate::settings::SettingsManager;
use crate::tools::r#trait::ToolExecutor;

use apply_codex_patch::ApplyCodexPatchTool;
use delete_file::DeleteFileTool;
use replace_in_file::ReplaceInFileTool;
use write_file::WriteFileTool;

/// Module providing file modification capabilities.
///
/// Bundles:
/// - WriteFileTool: Create or overwrite files
/// - DeleteFileTool: Delete files or empty directories
/// - modify_file tool: Selected based on FileModificationApi setting (late bound)
pub struct FileModifyModule {
    write_file: Arc<WriteFileTool>,
    delete_file: Arc<DeleteFileTool>,
    apply_codex_patch: Arc<ApplyCodexPatchTool>,
    replace_in_file: Arc<ReplaceInFileTool>,
    settings: SettingsManager,
}

impl FileModifyModule {
    pub fn new(workspace_roots: Vec<PathBuf>, settings: SettingsManager) -> Result<Self> {
        Ok(Self {
            write_file: Arc::new(WriteFileTool::new(workspace_roots.clone())?),
            delete_file: Arc::new(DeleteFileTool::new(workspace_roots.clone())?),
            apply_codex_patch: Arc::new(ApplyCodexPatchTool::new(workspace_roots.clone())?),
            replace_in_file: Arc::new(ReplaceInFileTool::new(workspace_roots)?),
            settings,
        })
    }
}

impl Module for FileModifyModule {
    fn prompt_components(&self) -> Vec<Arc<dyn PromptComponent>> {
        vec![]
    }

    fn context_components(&self) -> Vec<Arc<dyn ContextComponent>> {
        vec![]
    }

    fn tools(&self) -> Vec<Arc<dyn ToolExecutor>> {
        let modify_file: Arc<dyn ToolExecutor> =
            match self.settings.settings().file_modification_api {
                FileModificationApi::Patch => self.apply_codex_patch.clone(),
                FileModificationApi::Default | FileModificationApi::FindReplace => {
                    self.replace_in_file.clone()
                }
            };

        vec![
            self.write_file.clone(),
            self.delete_file.clone(),
            modify_file,
        ]
    }
}
