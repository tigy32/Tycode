use crate::chat::events::{ToolExecutionResult, ToolRequest as ToolRequestEvent, ToolRequestType};
use crate::file::access::FileAccessManager;
use crate::file::manager::FileModificationManager;
use crate::tools::r#trait::{
    ContinuationPreference, FileModification, FileOperation, ToolCallHandle, ToolCategory,
    ToolExecutor, ToolOutput, ToolRequest,
};
use anyhow::Result;
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Clone)]
pub struct DeleteFileTool {
    file_manager: FileAccessManager,
}

impl DeleteFileTool {
    pub fn new(workspace_roots: Vec<PathBuf>) -> anyhow::Result<Self> {
        let file_manager = FileAccessManager::new(workspace_roots)?;
        Ok(Self { file_manager })
    }
}

struct DeleteFileHandle {
    file_path: String,
    original_content: Option<String>,
    tool_use_id: String,
    file_manager: FileAccessManager,
}

#[async_trait::async_trait(?Send)]
impl ToolCallHandle for DeleteFileHandle {
    fn tool_request(&self) -> ToolRequestEvent {
        ToolRequestEvent {
            tool_call_id: self.tool_use_id.clone(),
            tool_name: "delete_file".to_string(),
            tool_type: ToolRequestType::ModifyFile {
                file_path: self.file_path.clone(),
                before: self.original_content.clone().unwrap_or_default(),
                after: String::new(),
            },
        }
    }

    async fn execute(self: Box<Self>) -> ToolOutput {
        let modification = FileModification {
            path: PathBuf::from(&self.file_path),
            operation: FileOperation::Delete,
            original_content: self.original_content,
            new_content: None,
            warning: None,
        };

        let manager = FileModificationManager::new(self.file_manager);
        match manager.apply_modification(modification).await {
            Ok(stats) => ToolOutput::Result {
                content: json!({
                    "success": true,
                    "path": self.file_path,
                    "lines_removed": stats.lines_removed
                })
                .to_string(),
                is_error: false,
                continuation: ContinuationPreference::Continue,
                ui_result: ToolExecutionResult::Other {
                    result: json!({
                        "deleted": true,
                        "path": self.file_path,
                        "lines_removed": stats.lines_removed
                    }),
                },
            },
            Err(e) => ToolOutput::Result {
                content: format!("Failed to delete file: {e:?}"),
                is_error: true,
                continuation: ContinuationPreference::Continue,
                ui_result: ToolExecutionResult::Error {
                    short_message: "Delete failed".to_string(),
                    detailed_message: format!("{e:?}"),
                },
            },
        }
    }
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for DeleteFileTool {
    fn name(&self) -> &'static str {
        "delete_file"
    }

    fn description(&self) -> &'static str {
        "Delete a file or empty directory"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to the file or directory to delete"
                }
            },
            "required": ["file_path"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Execution
    }

    async fn process(&self, request: &ToolRequest) -> Result<Box<dyn ToolCallHandle>> {
        let file_path = request
            .arguments
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: file_path"))?;

        let original_content = self.file_manager.read_file(file_path).await.ok();

        Ok(Box::new(DeleteFileHandle {
            file_path: file_path.to_string(),
            original_content,
            tool_use_id: request.tool_use_id.clone(),
            file_manager: self.file_manager.clone(),
        }))
    }
}
