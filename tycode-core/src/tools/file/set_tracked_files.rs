use crate::file::access::FileAccessManager;
use crate::security::types::RiskLevel;
use crate::tools::r#trait::{ToolExecutor, ToolRequest, ToolResult};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Clone)]
pub struct SetTrackedFilesTool {
    file_manager: FileAccessManager,
}

impl SetTrackedFilesTool {
    pub fn new(workspace_roots: Vec<PathBuf>) -> Self {
        let file_manager = FileAccessManager::new(workspace_roots);
        Self { file_manager }
    }
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for SetTrackedFilesTool {
    fn name(&self) -> &'static str {
        "set_tracked_files"
    }

    fn description(&self) -> &'static str {
        "Set the complete list of files to track for inclusion in all future messages. This replaces any previously tracked files. Minimize tracked files to conserve context. Pass an empty array to clear all tracked files."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_paths": {
                    "type": "array",
                    "items": {
                        "type": "string"
                    },
                    "description": "Array of file paths to track. Empty array clears all tracked files."
                }
            },
            "required": ["file_paths"]
        })
    }

    fn evaluate_risk(&self, _arguments: &Value) -> RiskLevel {
        RiskLevel::ReadOnly
    }

    async fn validate(&self, request: &ToolRequest) -> Result<ToolResult> {
        // Handle file_paths as either array or string to support qwen3-coder,
        // which tends to provide strings that are JSON arrays or single paths.
        // We don't advertise this as a supported capability to models, but if
        // get a malformed request we do our best to figure out what they meant
        let mut file_paths_value = request
            .arguments
            .get("file_paths")
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: file_paths"))?
            .clone();

        let file_paths_arr: Vec<String> = loop {
            match file_paths_value {
                Value::Array(arr) => {
                    break arr
                        .into_iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect()
                }
                Value::String(s) => match serde_json::from_str::<Value>(&s) {
                    Ok(value) => file_paths_value = value,
                    Err(_) => bail!("file_paths must be an array of strings"),
                },
                _ => bail!("file_paths must be an array of strings"),
            }
        };

        let mut new_paths = Vec::new();
        let mut invalid_files = Vec::new();

        // Validate all files exist
        for path_str in &file_paths_arr {
            if self.file_manager.file_exists(path_str).await? {
                new_paths.push(PathBuf::from(path_str));
            } else {
                invalid_files.push(path_str.to_string());
            }
        }

        if !invalid_files.is_empty() {
            return Err(anyhow::anyhow!(
                "The following files do not exist: {:?}",
                invalid_files
            ));
        }

        // Calculate total context size
        let mut total_size = 0usize;
        for path in &new_paths {
            let path_str = path.to_string_lossy();
            if let Ok(content) = self.file_manager.read_file(&path_str).await {
                total_size += content.len();
            }
        }

        // Return the files to track in the result
        // The actor will handle actually updating the tracked files state
        Ok(ToolResult::with_ui(
            json!({
                "action": "set_tracked_files",
                "tracked_files": new_paths.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
                "tracked_files_count": new_paths.len(),
                "total_context_size_bytes": total_size,
                "message": if new_paths.is_empty() {
                    "Cleared all tracked files".to_string()
                } else {
                    format!("Now tracking {} file(s). Context size: {} bytes", new_paths.len(), total_size)
                }
            }),
            json!({
                "success": true,
                "tracked_files": new_paths.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
                "tracked_files_count": new_paths.len(),
                "total_context_size_bytes": total_size,
                "message": if new_paths.is_empty() {
                    "Cleared all tracked files".to_string()
                } else {
                    format!("Now tracking {} file(s). Context size: {} bytes", new_paths.len(), total_size)
                }
            }),
        ))
    }
}
