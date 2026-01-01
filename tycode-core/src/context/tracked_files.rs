use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use anyhow::{bail, Result};
use serde_json::{json, Value};
use tracing::warn;

use crate::context::{ContextComponent, ContextComponentId};

pub const ID: ContextComponentId = ContextComponentId("tracked_files");
use crate::chat::events::{ToolExecutionResult, ToolRequest as ToolRequestEvent, ToolRequestType};
use crate::file::access::FileAccessManager;
use crate::tools::r#trait::{
    ContinuationPreference, ToolCallHandle, ToolCategory, ToolExecutor, ToolOutput, ToolRequest,
};
use crate::tools::ToolName;

/// Manages tracked files state and provides both context rendering and tool execution.
pub struct TrackedFilesManager {
    tracked_files: Arc<RwLock<BTreeSet<PathBuf>>>,
    file_manager: FileAccessManager,
}

impl TrackedFilesManager {
    pub fn tool_name() -> ToolName {
        ToolName::new("set_tracked_files")
    }

    pub fn new(workspace_roots: Vec<PathBuf>) -> Result<Self> {
        let file_manager = FileAccessManager::new(workspace_roots)?;
        Ok(Self {
            tracked_files: Arc::new(RwLock::new(BTreeSet::new())),
            file_manager,
        })
    }

    /// Get the current set of tracked file paths.
    pub fn get_tracked_files(&self) -> Vec<PathBuf> {
        self.tracked_files
            .read()
            .expect("lock poisoned")
            .iter()
            .cloned()
            .collect()
    }

    /// Clear all tracked files.
    pub fn clear(&self) {
        self.tracked_files.write().expect("lock poisoned").clear();
    }

    /// Set tracked files directly (for testing or programmatic use).
    pub fn set_files(&self, files: Vec<PathBuf>) {
        let mut tracked = self.tracked_files.write().expect("lock poisoned");
        tracked.clear();
        tracked.extend(files);
    }

    async fn read_file_contents(&self) -> Vec<(PathBuf, String)> {
        let tracked = self.tracked_files.read().expect("lock poisoned").clone();
        let mut results = Vec::new();

        for path in tracked {
            let path_str = path.to_string_lossy();
            match self.file_manager.read_file(&path_str).await {
                Ok(content) => results.push((path, content)),
                Err(e) => warn!(?e, "Failed to read tracked file: {:?}", path),
            }
        }

        results
    }
}

#[async_trait::async_trait(?Send)]
impl ContextComponent for TrackedFilesManager {
    fn id(&self) -> ContextComponentId {
        ID
    }

    async fn build_context_section(&self) -> Option<String> {
        let contents = self.read_file_contents().await;
        if contents.is_empty() {
            return None;
        }

        let mut output = String::from("Tracked Files:\n");
        for (path, content) in contents {
            output.push_str(&format!("\n=== {} ===\n{}", path.display(), content));
        }
        Some(output)
    }
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for TrackedFilesManager {
    fn name(&self) -> &str {
        "set_tracked_files"
    }

    fn description(&self) -> &str {
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

    async fn process(&self, request: &ToolRequest) -> Result<Box<dyn ToolCallHandle>> {
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

        let mut valid_paths = Vec::new();
        let mut invalid_files = Vec::new();

        for path_str in file_paths_arr {
            if self.file_manager.file_exists(&path_str).await? {
                valid_paths.push(PathBuf::from(&path_str));
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

        Ok(Box::new(SetTrackedFilesHandle {
            file_paths: valid_paths,
            tool_use_id: request.tool_use_id.clone(),
            tracked_files: self.tracked_files.clone(),
        }))
    }
}

struct SetTrackedFilesHandle {
    file_paths: Vec<PathBuf>,
    tool_use_id: String,
    tracked_files: Arc<RwLock<BTreeSet<PathBuf>>>,
}

#[async_trait::async_trait(?Send)]
impl ToolCallHandle for SetTrackedFilesHandle {
    fn tool_request(&self) -> ToolRequestEvent {
        let file_path_strings: Vec<String> = self
            .file_paths
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        ToolRequestEvent {
            tool_call_id: self.tool_use_id.clone(),
            tool_name: "set_tracked_files".to_string(),
            tool_type: ToolRequestType::ReadFiles {
                file_paths: file_path_strings,
            },
        }
    }

    async fn execute(self: Box<Self>) -> ToolOutput {
        {
            let mut tracked = self.tracked_files.write().expect("lock poisoned");
            tracked.clear();
            tracked.extend(self.file_paths.clone());
        }

        let file_path_strings: Vec<String> = self
            .file_paths
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();

        ToolOutput::Result {
            content: json!({
                "action": "set_tracked_files",
                "tracked_files": file_path_strings
            })
            .to_string(),
            is_error: false,
            continuation: ContinuationPreference::Continue,
            ui_result: ToolExecutionResult::Other {
                result: json!({
                    "action": "set_tracked_files",
                    "tracked_files": file_path_strings
                }),
            },
        }
    }
}
