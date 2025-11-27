use crate::file::access::FileAccessManager;
use crate::tools::r#trait::{
    FileModification, FileOperation, ToolCategory, ToolExecutor, ToolRequest, ValidatedToolCall,
};
use anyhow::Result;
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Clone)]
pub struct WriteFileTool {
    file_manager: FileAccessManager,
}

impl WriteFileTool {
    pub fn new(workspace_roots: Vec<PathBuf>) -> anyhow::Result<Self> {
        let file_manager = FileAccessManager::new(workspace_roots)?;
        Ok(Self { file_manager })
    }
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for WriteFileTool {
    fn name(&self) -> &'static str {
        "write_file"
    }

    fn description(&self) -> &'static str {
        "Create a new file or completely overwrite an existing file"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path where the file should be created"
                },
                "content": {
                    "type": "string",
                    "description": "Complete content to write to the file"
                }
            },
            "required": ["file_path", "content"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Execution
    }

    async fn validate(&self, request: &ToolRequest) -> Result<ValidatedToolCall> {
        let file_path = request
            .arguments
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: file_path"))?;

        let content = request.arguments
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: content. Sometimes this can happen if you hit a token limit; try writing a smaller file"))?;

        // Try to read original content if file exists
        let original_content = self.file_manager.read_file(file_path).await.ok();
        let operation = if original_content.is_some() {
            FileOperation::Update
        } else {
            FileOperation::Create
        };

        let modification = FileModification {
            path: PathBuf::from(file_path),
            operation,
            original_content,
            new_content: Some(content.to_string()),
            warning: None,
        };

        Ok(ValidatedToolCall::FileModification(modification))
    }
}
