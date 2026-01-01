use crate::chat::events::{ToolExecutionResult, ToolRequest as ToolRequestEvent, ToolRequestType};
use crate::file::access::FileAccessManager;
use crate::file::manager::FileModificationManager;
use crate::tools::r#trait::{
    ContinuationPreference, FileModification, FileOperation, ToolCallHandle, ToolCategory,
    ToolExecutor, ToolOutput, ToolRequest,
};
use crate::tools::ToolName;
use anyhow::Result;
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Clone)]
pub struct WriteFileTool {
    file_manager: FileAccessManager,
}

impl WriteFileTool {
    pub fn tool_name() -> ToolName {
        ToolName::new("write_file")
    }

    pub fn new(workspace_roots: Vec<PathBuf>) -> anyhow::Result<Self> {
        let file_manager = FileAccessManager::new(workspace_roots)?;
        Ok(Self { file_manager })
    }
}

struct WriteFileHandle {
    modification: FileModification,
    tool_use_id: String,
    file_manager: FileAccessManager,
}

#[async_trait::async_trait(?Send)]
impl ToolCallHandle for WriteFileHandle {
    fn tool_request(&self) -> ToolRequestEvent {
        ToolRequestEvent {
            tool_call_id: self.tool_use_id.clone(),
            tool_name: "write_file".to_string(),
            tool_type: ToolRequestType::ModifyFile {
                file_path: self.modification.path.to_string_lossy().to_string(),
                before: self
                    .modification
                    .original_content
                    .clone()
                    .unwrap_or_default(),
                after: self.modification.new_content.clone().unwrap_or_default(),
            },
        }
    }

    async fn execute(self: Box<Self>) -> ToolOutput {
        let manager = FileModificationManager::new(self.file_manager.clone());
        match manager.apply_modification(self.modification).await {
            Ok(stats) => ToolOutput::Result {
                content: json!({
                    "success": true,
                    "lines_added": stats.lines_added,
                    "lines_removed": stats.lines_removed
                })
                .to_string(),
                is_error: false,
                continuation: ContinuationPreference::Continue,
                ui_result: ToolExecutionResult::ModifyFile {
                    lines_added: stats.lines_added,
                    lines_removed: stats.lines_removed,
                },
            },
            Err(e) => {
                let msg = format!("{e:?}");
                ToolOutput::Result {
                    content: msg.clone(),
                    is_error: true,
                    continuation: ContinuationPreference::Continue,
                    ui_result: ToolExecutionResult::Error {
                        short_message: if msg.len() > 100 {
                            format!("{}...", &msg[..97])
                        } else {
                            msg.clone()
                        },
                        detailed_message: msg,
                    },
                }
            }
        }
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

    async fn process(&self, request: &ToolRequest) -> Result<Box<dyn ToolCallHandle>> {
        let file_path = request
            .arguments
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: file_path"))?;

        let content = request.arguments
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: content. Sometimes this can happen if you hit a token limit; try writing a smaller file"))?;

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

        Ok(Box::new(WriteFileHandle {
            modification,
            tool_use_id: request.tool_use_id.clone(),
            file_manager: self.file_manager.clone(),
        }))
    }
}
