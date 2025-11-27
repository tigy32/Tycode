use crate::file::access::FileAccessManager;
use crate::tools::r#trait::{ToolCategory, ToolExecutor, ToolRequest, ValidatedToolCall};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Clone)]
pub struct SetTrackedFilesTool {
    file_manager: FileAccessManager,
}

impl SetTrackedFilesTool {
    pub fn new(workspace_roots: Vec<PathBuf>) -> anyhow::Result<Self> {
        let file_manager = FileAccessManager::new(workspace_roots)?;
        Ok(Self { file_manager })
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

    fn category(&self) -> ToolCategory {
        ToolCategory::Execution
    }

    async fn validate(&self, request: &ToolRequest) -> Result<ValidatedToolCall> {
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

        let mut file_paths = Vec::new();
        let mut invalid_files = Vec::new();

        // Validate all files exist
        for path_str in file_paths_arr {
            if self.file_manager.file_exists(&path_str).await? {
                file_paths.push(path_str);
            } else {
                invalid_files.push(path_str);
            }
        }

        if !invalid_files.is_empty() {
            return Err(anyhow::anyhow!(
                "The following files do not exist: {:?}",
                invalid_files
            ));
        }

        Ok(ValidatedToolCall::SetTrackedFiles { file_paths })
    }
}
