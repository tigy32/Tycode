use crate::chat::events::{ToolExecutionResult, ToolRequest as ToolRequestEvent, ToolRequestType};
use crate::file::access::FileAccessManager;
use crate::file::find::{self, find_closest_match};
use crate::file::manager::FileModificationManager;
use crate::tools::r#trait::{
    ContinuationPreference, FileModification, FileOperation, ToolCallHandle, ToolCategory,
    ToolExecutor, ToolOutput, ToolRequest,
};
use crate::tools::ToolName;
use anyhow::{bail, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::PathBuf;

/// Tool for replacing sections of content in files
#[derive(Debug, Clone, Deserialize)]
pub struct SearchReplaceBlock {
    pub search: String,
    pub replace: String,
}

#[derive(Clone)]
pub struct ReplaceInFileTool {
    file_manager: FileAccessManager,
}

impl ReplaceInFileTool {
    pub fn tool_name() -> ToolName {
        ToolName::new("modify_file")
    }

    pub fn new(workspace_roots: Vec<PathBuf>) -> anyhow::Result<Self> {
        let file_manager = FileAccessManager::new(workspace_roots)?;
        Ok(Self { file_manager })
    }

    /// Apply replacements to content
    fn apply_replacements(
        &self,
        content: &str,
        replacements: Vec<SearchReplaceBlock>,
    ) -> Result<String> {
        let mut result = content.to_string();

        for block in replacements {
            let search = match search(result.clone(), block.search.clone()) {
                MatchResult::Multiple { matches, .. } => {
                    bail!(
                        "The following search pattern appears more than once in the file (found {} times). Use unique context to match exactly one occurrence.\n\nSearch pattern:\n{}\n\nTip: Include more surrounding context to make this search pattern unique.",
                        matches,
                        block.search
                    );
                }
                MatchResult::Guess { closest, .. } => {
                    let message = match closest {
                        Some(closest) => closest.get_correction_feedback().unwrap_or_else(|| "Found a perfect line-level match, but the exact string search failed. This may be due to whitespace or formatting differences. Reread the file to see the actual content.".to_string()),
                        None => "Reread the file (using the set_tracked_file tool and/or read the file contents from the next context message).".to_string(),
                    };
                    bail!("Exact match not found. {message}");
                }
                MatchResult::Exact(search) => search,
            };

            // Check if search and replace are identical
            if search == block.replace {
                bail!(
                    "Search and replace contents are identical for the following pattern. No changes would be made. Please provide different replacement content.\n\nSearch/Replace pattern:\n{}",
                    block.replace
                );
            }

            // Replace the single occurrence as specified
            result = result.replacen(&search, &block.replace, 1);
        }

        Ok(result)
    }
}

#[allow(dead_code)]
enum MatchResult {
    Multiple {
        requested: String,
        matches: usize,
    },
    Exact(String),
    Guess {
        requested: String,
        closest: Option<find::MatchResult>,
    },
}

fn search(source: String, search: String) -> MatchResult {
    let matches = source.split(&search).count() - 1;
    if matches > 1 {
        return MatchResult::Multiple {
            requested: search,
            matches,
        };
    }

    if matches == 1 {
        return MatchResult::Exact(search);
    }

    let best_match = find_closest_match(
        source.lines().map(str::to_string).collect(),
        search.lines().map(str::to_string).collect(),
    );

    MatchResult::Guess {
        requested: search,
        closest: best_match,
    }
}

struct ReplaceInFileHandle {
    modification: FileModification,
    tool_use_id: String,
    file_manager: FileAccessManager,
}

#[async_trait::async_trait(?Send)]
impl ToolCallHandle for ReplaceInFileHandle {
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
                content: format!("Failed to apply modification: {e:?}"),
                is_error: true,
                continuation: ContinuationPreference::Continue,
                ui_result: ToolExecutionResult::Error {
                    short_message: "Modification failed".to_string(),
                    detailed_message: format!("{e:?}"),
                },
            },
        }
    }
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for ReplaceInFileTool {
    fn name(&self) -> &'static str {
        "modify_file"
    }

    fn description(&self) -> &'static str {
        "Replace sections of content in an existing file"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to modify. Must be tracked using the set_tracked_files tool before being modified. Paths must always be absolute (e.g., starting from the project root like /tycode/...). The search block in diff must exactly match the content of the file to replace from the context."
                },
                "diff": {
                    "type": "array",
                    "description": "Array of search and replace blocks. You can (and should) specify multiple find/replace blocks for the same file to apply multiple changes at once.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "search": {
                                "type": "string",
                                "description": "Exact content to find. The search block must exactly match exactly one string in the source file — do not use it to match multiple instances (e.g., you cannot replace all 'banana' with 'carrot' if there are multiple 'banana'). Include sufficient unique surrounding context to ensure unambiguous, exact matching."
                            },
                            "replace": {
                                "type": "string",
                                "description": "New content to replace with"
                            }
                        },
                        "required": ["search", "replace"],
                        "additionalProperties": false
                    }
                }
            },
            "required": ["file_path", "diff"]
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

        let diff_value = request
            .arguments
            .get("diff")
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: diff"))?;

        let mut diff_value_parsed = diff_value.clone();
        let diff_arr: Vec<Value> = loop {
            match diff_value_parsed {
                Value::Array(arr) => break arr,
                Value::String(s) => match serde_json::from_str::<Value>(&s) {
                    Ok(value) => diff_value_parsed = value,
                    Err(_) => bail!("diff must be an array of search and replace blocks"),
                },
                _ => bail!("diff must be an array of search and replace blocks"),
            }
        };

        let original_content: String = self.file_manager.read_file(file_path).await?;

        let replacements: Vec<SearchReplaceBlock> = diff_arr
            .into_iter()
            .map(|item| {
                serde_json::from_value(item)
                    .map_err(|e| anyhow::anyhow!("Invalid diff entry: {e:?}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let new_content = self.apply_replacements(&original_content, replacements)?;

        let modification = FileModification {
            path: PathBuf::from(file_path),
            operation: FileOperation::Update,
            original_content: Some(original_content),
            new_content: Some(new_content),
            warning: None,
        };

        Ok(Box::new(ReplaceInFileHandle {
            modification,
            tool_use_id: request.tool_use_id.clone(),
            file_manager: self.file_manager.clone(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_replacements_fails_on_multiple_occurrences() {
        let tool = ReplaceInFileTool::new(vec![]).unwrap();
        let content = "line1\nsearch\nline2\nsearch\nline3";
        let replacements = vec![SearchReplaceBlock {
            search: "search".to_string(),
            replace: "replaced".to_string(),
        }];

        let result = tool.apply_replacements(content, replacements);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("The following search pattern appears more than once in the file"));
    }

    #[test]
    fn test_apply_replacements_succeeds_on_single_occurrence() {
        let tool = ReplaceInFileTool::new(vec![]).unwrap();
        let content = "line1\nsearch\nline2";
        let replacements = vec![SearchReplaceBlock {
            search: "search".to_string(),
            replace: "replaced".to_string(),
        }];

        let result = tool.apply_replacements(content, replacements);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "line1\nreplaced\nline2");
    }

    #[test]
    fn test_apply_replacements_fails_on_identical_search_and_replace() {
        let tool = ReplaceInFileTool::new(vec![]).unwrap();
        let content = "line1\nsearch\nline2";
        let replacements = vec![SearchReplaceBlock {
            search: "search".to_string(),
            replace: "search".to_string(), // identical to search
        }];

        let result = tool.apply_replacements(content, replacements);
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Search and replace contents are identical"));
    }
}
