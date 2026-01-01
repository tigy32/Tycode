use crate::chat::events::{
    FileInfo, ToolExecutionResult, ToolRequest as ToolRequestEvent, ToolRequestType,
};
use crate::file::access::FileAccessManager;
use crate::tools::r#trait::{
    ContinuationPreference, ToolCallHandle, ToolCategory, ToolExecutor, ToolOutput, ToolRequest,
};
use crate::tools::ToolName;
use anyhow::Result;
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Clone)]
pub struct ReadFileTool {
    workspace_roots: Vec<PathBuf>,
    file_manager: FileAccessManager,
}

impl ReadFileTool {
    pub fn tool_name() -> ToolName {
        ToolName::new("read_file")
    }

    pub fn new(workspace_roots: Vec<PathBuf>) -> anyhow::Result<Self> {
        let file_manager = FileAccessManager::new(workspace_roots.clone())?;
        Ok(Self {
            workspace_roots,
            file_manager,
        })
    }
}

struct ReadFileHandle {
    file_path: String,
    summary: bool,
    tool_use_id: String,
    workspace_roots: Vec<PathBuf>,
    file_manager: FileAccessManager,
}

impl ReadFileHandle {
    fn find_index_path(&self, file_path: &str) -> Option<PathBuf> {
        let path = std::path::Path::new(file_path);

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
impl ToolCallHandle for ReadFileHandle {
    fn tool_request(&self) -> ToolRequestEvent {
        ToolRequestEvent {
            tool_call_id: self.tool_use_id.clone(),
            tool_name: "read_file".to_string(),
            tool_type: ToolRequestType::ReadFiles {
                file_paths: vec![self.file_path.clone()],
            },
        }
    }

    async fn execute(self: Box<Self>) -> ToolOutput {
        if self.summary {
            if let Some(index_path) = self.find_index_path(&self.file_path) {
                let index_path_str = index_path.to_string_lossy().to_string();
                if let Ok(summary_content) = self.file_manager.read_file(&index_path_str).await {
                    let result = json!({
                        "content": summary_content,
                        "size": summary_content.len(),
                        "path": self.file_path,
                        "is_summary": true
                    });
                    return ToolOutput::Result {
                        content: result.to_string(),
                        is_error: false,
                        continuation: ContinuationPreference::Continue,
                        ui_result: ToolExecutionResult::ReadFiles {
                            files: vec![FileInfo {
                                path: self.file_path.clone(),
                                bytes: summary_content.len(),
                            }],
                        },
                    };
                }
            }
            return ToolOutput::Result {
                content: format!("No summary index found for: {}", self.file_path),
                is_error: true,
                continuation: ContinuationPreference::Continue,
                ui_result: ToolExecutionResult::Error {
                    short_message: "No summary found".to_string(),
                    detailed_message: format!("No summary index found for: {}", self.file_path),
                },
            };
        }

        if self
            .file_manager
            .list_directory(&self.file_path)
            .await
            .is_ok()
        {
            return ToolOutput::Result {
                content: format!("Path is a directory, not a file: {}", self.file_path),
                is_error: true,
                continuation: ContinuationPreference::Continue,
                ui_result: ToolExecutionResult::Error {
                    short_message: "Path is a directory".to_string(),
                    detailed_message: format!(
                        "Path is a directory, not a file: {}",
                        self.file_path
                    ),
                },
            };
        }

        match self.file_manager.read_file(&self.file_path).await {
            Ok(content) => {
                let result = json!({
                    "content": content,
                    "size": content.len(),
                    "path": self.file_path,
                    "is_summary": false
                });
                ToolOutput::Result {
                    content: result.to_string(),
                    is_error: false,
                    continuation: ContinuationPreference::Continue,
                    ui_result: ToolExecutionResult::ReadFiles {
                        files: vec![FileInfo {
                            path: self.file_path.clone(),
                            bytes: content.len(),
                        }],
                    },
                }
            }
            Err(e) => ToolOutput::Result {
                content: format!("Failed to read file: {e:?}"),
                is_error: true,
                continuation: ContinuationPreference::Continue,
                ui_result: ToolExecutionResult::Error {
                    short_message: "Read failed".to_string(),
                    detailed_message: format!("{e:?}"),
                },
            },
        }
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

    fn category(&self) -> ToolCategory {
        ToolCategory::Execution
    }

    async fn process(&self, request: &ToolRequest) -> Result<Box<dyn ToolCallHandle>> {
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

        Ok(Box::new(ReadFileHandle {
            file_path: file_path.to_string(),
            summary,
            tool_use_id: request.tool_use_id.clone(),
            workspace_roots: self.workspace_roots.clone(),
            file_manager: self.file_manager.clone(),
        }))
    }
}
