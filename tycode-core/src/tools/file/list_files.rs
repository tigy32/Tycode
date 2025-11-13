use crate::file::access::FileAccessManager;
use crate::tools::r#trait::{ToolCategory, ToolExecutor, ToolRequest, ValidatedToolCall};
use anyhow::Result;
use serde_json::{json, Value};
use std::path::PathBuf;

#[derive(Clone)]
pub struct ListFilesTool {
    workspace_roots: Vec<PathBuf>,
    file_manager: FileAccessManager,
}

impl ListFilesTool {
    pub fn new(workspace_roots: Vec<PathBuf>) -> Self {
        let file_manager = FileAccessManager::new(workspace_roots.clone());
        Self {
            workspace_roots,
            file_manager,
        }
    }
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for ListFilesTool {
    fn name(&self) -> &'static str {
        "list_files"
    }

    fn description(&self) -> &'static str {
        "List files and directories in a directory"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "directory_path": {
                    "type": "string",
                    "description": "Path to directory to list. Use empty string or '.' to list workspace root(s)."
                },
            },
            "required": ["directory_path"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Execution
    }

    async fn validate(&self, request: &ToolRequest) -> Result<ValidatedToolCall> {
        let directory_path = request
            .arguments
            .get("directory_path")
            .and_then(|v| v.as_str());

        let mut all_entries = Vec::new();
        let display_path;

        if let Some(dir_path) = directory_path {
            // List specific directory
            let paths = self.file_manager.list_directory(dir_path).await?;
            display_path = dir_path.to_string();

            for path in paths {
                let is_dir = self
                    .file_manager
                    .list_directory(&path.to_string_lossy())
                    .await
                    .is_ok();

                all_entries.push(json!({
                    "name": path.file_name().unwrap_or_default().to_string_lossy(),
                    "path": path.to_string_lossy(),
                    "type": if is_dir { "directory" } else { "file" },
                }));
            }
        } else {
            // No directory specified - list all workspace roots
            display_path = if self.workspace_roots.len() == 1 {
                self.workspace_roots[0].to_string_lossy().to_string()
            } else {
                "all workspace roots".to_string()
            };

            for root in &self.workspace_roots {
                let root_str = root.to_string_lossy().to_string();
                let paths = self.file_manager.list_directory(&root_str).await?;

                for path in paths {
                    let relative_path = path
                        .strip_prefix(root)
                        .ok()
                        .map(|rel| rel.to_string_lossy().to_string())
                        .unwrap_or_else(|| path.to_string_lossy().to_string());

                    let is_dir = self
                        .file_manager
                        .list_directory(&relative_path)
                        .await
                        .is_ok();

                    all_entries.push(json!({
                        "name": path.file_name().unwrap_or_default().to_string_lossy(),
                        "path": relative_path,
                        "type": if is_dir { "directory" } else { "file" },
                        "workspace": root.file_name().unwrap_or_default().to_string_lossy(),
                    }));
                }
            }
        }

        all_entries.sort_by(|a, b| {
            let a_type = a["type"].as_str().unwrap_or("");
            let b_type = b["type"].as_str().unwrap_or("");
            let a_name = a["name"].as_str().unwrap_or("");
            let b_name = b["name"].as_str().unwrap_or("");

            match (a_type, b_type) {
                ("directory", "file") => std::cmp::Ordering::Less,
                ("file", "directory") => std::cmp::Ordering::Greater,
                _ => a_name.cmp(b_name),
            }
        });

        Ok(ValidatedToolCall::context_only(json!({
            "entries": all_entries,
            "path": display_path,
        })))
    }
}
