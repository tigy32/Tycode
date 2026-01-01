use regex::Regex;
use serde_json::{json, Value};
use std::collections::VecDeque;

use crate::{
    chat::events::{ToolExecutionResult, ToolRequest as ToolRequestEvent, ToolRequestType},
    file::access::FileAccessManager,
    tools::r#trait::{
        ContinuationPreference, ToolCallHandle, ToolCategory, ToolExecutor, ToolOutput, ToolRequest,
    },
    tools::ToolName,
};

#[derive(Clone)]
pub struct SearchFilesTool {
    file_manager: FileAccessManager,
}

impl SearchFilesTool {
    pub fn tool_name() -> ToolName {
        ToolName::new("search_files")
    }

    pub fn new(file_manager: FileAccessManager) -> Self {
        Self { file_manager }
    }
}

struct SearchFilesHandle {
    file_manager: FileAccessManager,
    directory_path: String,
    pattern: String,
    file_pattern: Option<String>,
    max_results: usize,
    include_context: bool,
    context_lines: usize,
    tool_use_id: String,
}

#[async_trait::async_trait(?Send)]
impl ToolCallHandle for SearchFilesHandle {
    fn tool_request(&self) -> ToolRequestEvent {
        ToolRequestEvent {
            tool_call_id: self.tool_use_id.clone(),
            tool_name: "search_files".to_string(),
            tool_type: ToolRequestType::Other {
                args: json!({
                    "directory_path": self.directory_path,
                    "pattern": self.pattern,
                    "file_pattern": self.file_pattern,
                    "max_results": self.max_results,
                    "include_context": self.include_context,
                    "context_lines": self.context_lines,
                }),
            },
        }
    }

    async fn execute(self: Box<Self>) -> ToolOutput {
        let result = search_files(
            &self.file_manager,
            &self.directory_path,
            &self.pattern,
            self.file_pattern.as_deref(),
            self.max_results,
            self.include_context,
            self.context_lines,
        )
        .await;

        match result {
            Ok((results, truncated)) => {
                let mut json_results = Vec::new();
                for result in results {
                    let mut result_obj = json!({
                        "path": result.path,
                        "line_number": result.line_number,
                        "line": result.line_content,
                    });

                    if self.include_context {
                        if let Some(before) = result.context_before {
                            result_obj["context_before"] = json!(before);
                        }
                        if let Some(after) = result.context_after {
                            result_obj["context_after"] = json!(after);
                        }
                    }

                    json_results.push(result_obj);
                }

                let mut response = json!({
                    "results": json_results,
                    "count": json_results.len(),
                });

                if truncated {
                    response["truncated"] = json!(true);
                    response["message"] = json!("Results truncated to limit");
                }

                ToolOutput::Result {
                    content: response.to_string(),
                    is_error: false,
                    continuation: ContinuationPreference::Continue,
                    ui_result: ToolExecutionResult::Other { result: response },
                }
            }
            Err(e) => ToolOutput::Result {
                content: format!("Search failed: {e:?}"),
                is_error: true,
                continuation: ContinuationPreference::Continue,
                ui_result: ToolExecutionResult::Error {
                    short_message: "Search failed".to_string(),
                    detailed_message: format!("{e:?}"),
                },
            },
        }
    }
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for SearchFilesTool {
    fn name(&self) -> &'static str {
        "search_files"
    }

    fn description(&self) -> &'static str {
        "Search for text patterns in files"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "directory_path": {
                    "type": "string",
                    "description": "Directory to search in"
                },
                "pattern": {
                    "type": "string",
                    "description": "Regular expression pattern to search for"
                },
                "file_pattern": {
                    "type": "string",
                    "description": "File name pattern (e.g. '*.rs'). Empty string matches all files."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of results to return. Default is 100."
                },
                "include_context": {
                    "type": "boolean",
                    "description": "Include context lines before/after matches. Default is false."
                },
                "context_lines": {
                    "type": "integer",
                    "description": "Number of context lines to include when include_context is true. Default is 2."
                }
            },
            "required": ["directory_path", "pattern", "file_pattern", "max_results", "include_context", "context_lines"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Execution
    }

    async fn process(&self, request: &ToolRequest) -> anyhow::Result<Box<dyn ToolCallHandle>> {
        let directory_path = request
            .arguments
            .get("directory_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: directory_path"))?;

        let pattern = request
            .arguments
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: pattern"))?;

        let file_pattern = request
            .arguments
            .get("file_pattern")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let max_results = request
            .arguments
            .get("max_results")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(100);

        let include_context = request
            .arguments
            .get("include_context")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let context_lines = request
            .arguments
            .get("context_lines")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(2);

        Ok(Box::new(SearchFilesHandle {
            file_manager: self.file_manager.clone(),
            directory_path: directory_path.to_string(),
            pattern: pattern.to_string(),
            file_pattern,
            max_results,
            include_context,
            context_lines,
            tool_use_id: request.tool_use_id.clone(),
        }))
    }
}

#[derive(Debug)]
struct SearchResult {
    path: String, // virtual path
    line_number: usize,
    line_content: String,
    context_before: Option<Vec<String>>,
    context_after: Option<Vec<String>>,
}

// search_files: Recursively searches for a regex pattern in files within a directory.
// Uses virtual paths to abstract real filesystem.
// Filters files based on optional file_pattern (simple wildcard).
// Returns results up to max_results, with truncated flag if more.
// Includes context lines if requested.
// Ensures all operations respect ignore rules via FileAccessManager.
async fn search_files(
    file_manager: &FileAccessManager,
    directory_path: &str,
    pattern: &str,
    file_pattern: Option<&str>,
    max_results: usize,
    include_context: bool,
    context_lines: usize,
) -> anyhow::Result<(Vec<SearchResult>, bool)> {
    let regex = Regex::new(pattern)?;
    let file_regex = file_pattern
        .map(|fp| Regex::new(&wildcard_to_regex(fp)))
        .transpose()?;

    let mut all_files = Vec::new();
    let mut queue = VecDeque::from([directory_path.to_string()]);
    let mut seen = std::collections::HashSet::new();

    while let Some(current_dir) = queue.pop_front() {
        if seen.contains(&current_dir) {
            continue;
        }
        seen.insert(current_dir.clone());

        match file_manager.list_directory(&current_dir).await {
            Ok(entries) => {
                for entry in entries {
                    if entry.is_dir() {
                        queue.push_back(entry.to_string_lossy().into_owned());
                    } else {
                        // Check file pattern if provided
                        let entry = entry.to_string_lossy().to_string();
                        if let Some(ref r) = file_regex {
                            let file_name = entry.split('/').next_back().unwrap_or("");
                            if !r.is_match(file_name) {
                                continue;
                            }
                        }
                        all_files.push(entry);
                    }
                }
            }
            Err(_) => continue, // Skip inaccessible dirs, surface error was surface in manager
        }
    }

    let mut results = Vec::new();
    let mut truncated = false;

    for file_path in all_files {
        if results.len() >= max_results {
            truncated = true;
            break;
        }

        match file_manager.read_file(&file_path).await {
            Ok(content) => {
                let lines: Vec<&str> = content.lines().collect();
                for (i, line) in lines.iter().enumerate() {
                    if regex.is_match(line) {
                        let line_number = i + 1;
                        let line_content = line.to_string();

                        let context_before = if include_context {
                            let start = i.saturating_sub(context_lines);
                            Some(lines[start..i].iter().map(|s| s.to_string()).collect())
                        } else {
                            None
                        };

                        let context_after = if include_context {
                            let end = (i + 1 + context_lines).min(lines.len());
                            Some(lines[i + 1..end].iter().map(|s| s.to_string()).collect())
                        } else {
                            None
                        };

                        results.push(SearchResult {
                            path: file_path.to_string(),
                            line_number,
                            line_content,
                            context_before,
                            context_after,
                        });

                        if results.len() >= max_results {
                            truncated = true;
                            break;
                        }
                    }
                }
            }
            Err(_) => continue, // Skip unreadable files
        }
    }

    Ok((results, truncated))
}

// Convert simple wildcard to regex (e.g., *.rs -> .*\\.rs)
fn wildcard_to_regex(pattern: &str) -> String {
    pattern
        .replace('.', r"\.")
        .replace('*', ".*")
        .replace('?', ".")
}
