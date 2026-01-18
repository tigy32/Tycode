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

/// Tool for applying patches to files
#[derive(Clone)]
pub struct ApplyPatchTool {
    file_manager: FileAccessManager,
}

impl ApplyPatchTool {
    pub fn new(workspace_roots: Vec<PathBuf>) -> anyhow::Result<Self> {
        let file_manager = FileAccessManager::new(workspace_roots)?;
        Ok(Self { file_manager })
    }

    /// Apply a patch to content
    fn apply_patch(&self, content: &str, patch: &str) -> Result<String> {
        let lines: Vec<&str> = content.lines().collect();
        let mut result = Vec::new();
        let mut line_idx = 0;

        let patch_lines: Vec<&str> = patch.lines().collect();
        let mut patch_idx = 0;

        while patch_idx < patch_lines.len() {
            let patch_line = patch_lines[patch_idx];

            if patch_line.starts_with("@@") {
                // Parse the hunk header
                let parts: Vec<&str> = patch_line.split_whitespace().collect();
                if parts.len() < 3 {
                    return Err(anyhow::anyhow!("Invalid patch hunk header: {}", patch_line));
                }

                // Extract line numbers from the format: @@ -old_start,old_count +new_start,new_count @@
                let old_info = parts[1].trim_start_matches('-');
                let old_parts: Vec<&str> = old_info.split(',').collect();
                let old_start: usize = old_parts[0].parse::<usize>()?.saturating_sub(1); // Convert to 0-indexed

                // Skip to the target line
                while line_idx < old_start && line_idx < lines.len() {
                    result.push(lines[line_idx].to_string());
                    line_idx += 1;
                }

                patch_idx += 1;

                // Process the hunk
                while patch_idx < patch_lines.len() && !patch_lines[patch_idx].starts_with("@@") {
                    let patch_line = patch_lines[patch_idx];

                    if patch_line.starts_with('-') {
                        // Remove line - skip it in the original
                        line_idx += 1;
                    } else if let Some(rest) = patch_line.strip_prefix('+') {
                        // Add line
                        result.push(rest.to_string());
                    } else if patch_line.starts_with(' ') {
                        // Context line - copy from original
                        if line_idx < lines.len() {
                            result.push(lines[line_idx].to_string());
                            line_idx += 1;
                        }
                    }

                    patch_idx += 1;
                }
            } else {
                patch_idx += 1;
            }
        }

        // Add any remaining lines from the original
        while line_idx < lines.len() {
            result.push(lines[line_idx].to_string());
            line_idx += 1;
        }

        Ok(result.join("\n"))
    }
}

struct ApplyPatchHandle {
    modification: FileModification,
    tool_use_id: String,
    file_manager: FileAccessManager,
}

#[async_trait::async_trait(?Send)]
impl ToolCallHandle for ApplyPatchHandle {
    fn tool_request(&self) -> ToolRequestEvent {
        ToolRequestEvent {
            tool_call_id: self.tool_use_id.clone(),
            tool_name: "modify_file".to_string(),
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
            Err(e) => ToolOutput::Result {
                content: format!("Failed to apply patch: {e:?}"),
                is_error: true,
                continuation: ContinuationPreference::Continue,
                ui_result: ToolExecutionResult::Error {
                    short_message: "Patch failed".to_string(),
                    detailed_message: format!("{e:?}"),
                },
            },
        }
    }
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for ApplyPatchTool {
    fn name(&self) -> &'static str {
        "modify_file"
    }

    fn description(&self) -> &'static str {
        "Apply a patch/diff to a file"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Path to the file to patch"
                },
                "patch": {
                    "type": "string",
                    "description": "The patch/diff to apply in unified diff format"
                }
            },
            "required": ["file_path", "patch"]
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

        let patch = request
            .arguments
            .get("patch")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: patch"))?;

        let original_content: String = self.file_manager.read_file(file_path).await?;
        let patched_content = self.apply_patch(&original_content, patch)?;

        let modification = FileModification {
            path: PathBuf::from(file_path),
            operation: FileOperation::Update,
            original_content: Some(original_content),
            new_content: Some(patched_content),
            warning: None,
        };

        Ok(Box::new(ApplyPatchHandle {
            modification,
            tool_use_id: request.tool_use_id.clone(),
            file_manager: self.file_manager.clone(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_apply_patch() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().join("test");
        fs::create_dir(&root).unwrap();
        let tool = ApplyPatchTool::new(vec![root.clone()]).unwrap();

        let file_manager = FileAccessManager::new(vec![root.clone()]).unwrap();
        let original_content = "line 1\nline 2\nline 3\nline 4\nline 5";
        file_manager
            .write_file("/test/test.txt", original_content)
            .await
            .unwrap();

        let patch = r#"@@ -2,1 +2,1 @@
-line 2
+line 2 modified"#;

        let request = ToolRequest::new(
            json!({
                "file_path": "/test/test.txt",
                "patch": patch
            }),
            "test_id".to_string(),
        );
        let handle = tool.process(&request).await.unwrap();
        let request_event = handle.tool_request();

        assert_eq!(request_event.tool_name, "modify_file");
        if let ToolRequestType::ModifyFile {
            file_path,
            before,
            after,
        } = request_event.tool_type
        {
            assert_eq!(file_path, "/test/test.txt");
            assert_eq!(before, original_content);
            let expected_new = "line 1\nline 2 modified\nline 3\nline 4\nline 5";
            assert_eq!(after, expected_new);
        } else {
            panic!("Expected ModifyFile request type");
        }
    }
}
