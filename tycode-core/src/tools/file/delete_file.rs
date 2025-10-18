use crate::file::access::FileAccessManager;
use crate::tools::r#trait::{
    FileModification, FileOperation, ToolCategory, ToolExecutor, ToolRequest, ValidatedToolCall,
};
use anyhow::Result;
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Clone)]
pub struct DeleteFileTool {
    file_manager: FileAccessManager,
}

impl DeleteFileTool {
    pub fn new(workspace_roots: Vec<PathBuf>) -> Self {
        let file_manager = FileAccessManager::new(workspace_roots);
        Self { file_manager }
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
        ToolCategory::Modification
    }

    async fn validate(&self, request: &ToolRequest) -> Result<ValidatedToolCall> {
        let file_path = request
            .arguments
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: file_path"))?;

        // Try to read original content before deletion
        let original_content = self.file_manager.read_file(file_path).await.ok();

        let modification = FileModification {
            path: PathBuf::from(file_path),
            operation: FileOperation::Delete,
            original_content,
            new_content: None,
            warning: None,
        };

        Ok(ValidatedToolCall::FileModification(modification))
    }
}
