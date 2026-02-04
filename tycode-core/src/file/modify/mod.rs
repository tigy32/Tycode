//! File modification module.
//!
//! Provides tools for creating, updating, and deleting files.
//! The modify_file tool implementation is selected based on FileModificationApi setting.

pub mod apply_codex_patch;
pub mod cline_replace_in_file;
pub mod command;
pub mod delete_file;
pub mod replace_in_file;
pub mod write_file;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;

use crate::file::config::File;
use crate::module::ContextComponent;
use crate::module::Module;
use crate::module::PromptComponent;
use crate::module::SlashCommand;
use crate::settings::config::FileModificationApi;
use crate::settings::SettingsManager;
use crate::tools::r#trait::ToolExecutor;

use command::FileApiSlashCommand;

use apply_codex_patch::ApplyCodexPatchTool;
use cline_replace_in_file::ClineReplaceInFileTool;
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
    cline_replace_in_file: Arc<ClineReplaceInFileTool>,
    settings: SettingsManager,
}

impl FileModifyModule {
    pub fn new(workspace_roots: Vec<PathBuf>, settings: SettingsManager) -> Result<Self> {
        Ok(Self {
            write_file: Arc::new(WriteFileTool::new(workspace_roots.clone())?),
            delete_file: Arc::new(DeleteFileTool::new(workspace_roots.clone())?),
            apply_codex_patch: Arc::new(ApplyCodexPatchTool::new(workspace_roots.clone())?),
            replace_in_file: Arc::new(ReplaceInFileTool::new(workspace_roots.clone())?),
            cline_replace_in_file: Arc::new(ClineReplaceInFileTool::new(workspace_roots)?),
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

    fn slash_commands(&self) -> Vec<Arc<dyn SlashCommand>> {
        vec![Arc::new(FileApiSlashCommand)]
    }

    fn tools(&self) -> Vec<Arc<dyn ToolExecutor>> {
        let modify_file: Arc<dyn ToolExecutor> = match self
            .settings
            .get_module_config::<File>(File::NAMESPACE)
            .file_modification_api
        {
            FileModificationApi::Patch => self.apply_codex_patch.clone(),
            FileModificationApi::Default | FileModificationApi::FindReplace => {
                self.replace_in_file.clone()
            }
            FileModificationApi::ClineSearchReplace => self.cline_replace_in_file.clone(),
        };

        vec![
            self.write_file.clone(),
            self.delete_file.clone(),
            modify_file,
        ]
    }
}
