use crate::file::access::FileAccessManager;
use crate::file::find::{self, find_closest_match};
use crate::tools::r#trait::{
    FileModification, FileOperation, ToolExecutor, ToolRequest, ValidatedToolCall,
};
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
    pub fn new(workspace_roots: Vec<PathBuf>) -> Self {
        let file_manager = FileAccessManager::new(workspace_roots);
        Self { file_manager }
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
                        Some(closest) => closest.get_correction_feedback().unwrap(),
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

    async fn validate(&self, request: &ToolRequest) -> Result<ValidatedToolCall> {
        let file_path = request
            .arguments
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: file_path"))?;

        let diff_value = request
            .arguments
            .get("diff")
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: diff"))?;

        // Handle diff as either array or string to support qwen3-coder,
        // which tends to provide strings that are JSON arrays or diffs.
        // We don't advertise this as a supported capability to models, but if
        // we get a malformed request we do our best to figure out what they meant
        let mut diff_value_parsed = diff_value.clone();
        let diff_arr: Vec<Value> = loop {
            match diff_value_parsed {
                Value::Array(arr) => {
                    break arr;
                }
                Value::String(s) => match serde_json::from_str::<Value>(&s) {
                    Ok(value) => diff_value_parsed = value,
                    Err(_) => bail!("diff must be an array of search and replace blocks"),
                },
                _ => bail!("diff must be an array of search and replace blocks"),
            }
        };

        // Read the current content using FileAccessManager
        let original_content: String = self.file_manager.read_file(file_path).await?;

        // Parse and apply the diff
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
            original_content: Some(original_content.to_string()),
            new_content: Some(new_content),
        };

        Ok(ValidatedToolCall::FileModification(modification))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_replacements_fails_on_multiple_occurrences() {
        let tool = ReplaceInFileTool::new(vec![]);
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
        let tool = ReplaceInFileTool::new(vec![]);
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
        let tool = ReplaceInFileTool::new(vec![]);
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
