use crate::file::access::FileAccessManager;
use crate::tools::r#trait::{ToolExecutor, ToolRequest, ValidatedToolCall};
use anyhow::Result;
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Clone)]
pub struct ReadFileTool {
    workspace_roots: Vec<PathBuf>,
    file_manager: FileAccessManager,
}

impl ReadFileTool {
    pub fn new(workspace_roots: Vec<PathBuf>) -> Self {
        let file_manager = FileAccessManager::new(workspace_roots.clone());
        Self {
            workspace_roots,
            file_manager,
        }
    }

    /// Looks for index file in the workspace root that contains the file
    fn find_index_path(&self, file_path: &str) -> Option<PathBuf> {
        let path = std::path::Path::new(file_path);

        // Try to find which workspace root contains this file
        for workspace_root in &self.workspace_roots {
            let full_path = if path.is_absolute() {
                path.to_path_buf()
            } else {
                workspace_root.join(path)
            };

            if full_path.starts_with(workspace_root) || full_path.exists() {
                let index_base = workspace_root.join(".tycode").join("index");
                let index_file = format!("{file_path}.md");
                let index_path = index_base.join(index_file);

                if index_path.exists() {
                    return Some(index_path);
                }
            }
        }

        None
    }
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for ReadFileTool {
    fn name(&self) -> &'static str {
        "read_file"
    }

    fn description(&self) -> &'static str {
        "[DEPRECATED - Use 'track_file' instead] Read the contents of a file. Note: For better context management, use 'track_file' to include file contents in all future messages, and 'untrack_file' to remove them. This provides continuous awareness of file changes."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to the file to read"
                },
                "summary": {
                    "type": "boolean",
                    "description": "If true, return a summary of the file rather than the full file content. Use summaries to understand project structure and interfaces without needing to read full source files."
                }
            },
            "required": ["file_path", "summary"]
        })
    }

    async fn validate(&self, request: &ToolRequest) -> Result<ValidatedToolCall> {
        let file_path = request
            .arguments
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: file_path"))?;

        let summary = request
            .arguments
            .get("summary")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: summary"))?;

        if summary {
            if let Some(index_path) = self.find_index_path(file_path) {
                // Try to read the index file
                let index_path_str = index_path.to_string_lossy().to_string();
                if let Ok(summary_content) = self.file_manager.read_file(&index_path_str).await {
                    return Ok(ValidatedToolCall::context_only(json!({
                        "content": summary_content,
                        "size": summary_content.len(),
                        "path": file_path,
                        "is_summary": true
                    })));
                }
            }

            // No index found
            return Err(anyhow::anyhow!("No summary index found for: {}", file_path));
        }

        // Check if the path is a directory
        if self.file_manager.list_directory(file_path).await.is_ok() {
            return Err(anyhow::anyhow!(
                "Path is a directory, not a file: {}",
                file_path
            ));
        }

        // Read the full file
        let content = self.file_manager.read_file(file_path).await?;

        Ok(ValidatedToolCall::context_only(json!({
            "content": content,
            "size": content.len(),
            "path": file_path,
            "is_summary": false
        })))
    }
}
