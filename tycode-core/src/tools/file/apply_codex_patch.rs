use crate::file::access::FileAccessManager;
use crate::file::find::find_closest_match;
use crate::tools::r#trait::{
    FileModification, FileOperation, ToolCategory, ToolExecutor, ToolRequest, ValidatedToolCall,
};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::path::PathBuf;

/// Tool for applying codex-style patches without line numbers
#[derive(Clone)]
pub struct ApplyCodexPatchTool {
    file_manager: FileAccessManager,
}

#[derive(Debug, Clone, PartialEq)]
enum CodexHunkLine {
    Context(String),
    Removal(String),
    Addition(String),
}

impl CodexHunkLine {
    pub fn patch(&self) -> String {
        match self {
            CodexHunkLine::Context(s) => format!(" {s}"),
            CodexHunkLine::Removal(s) => format!("-{s}"),
            CodexHunkLine::Addition(s) => format!("+{s}"),
        }
    }
}

#[derive(Debug)]
struct CodexHunk {
    lines: Vec<CodexHunkLine>,
}

impl CodexHunk {
    pub fn patch(&self) -> String {
        let mut result = String::new();
        for line in &self.lines {
            result = format!("{result}{}\n", line.patch());
        }
        result
    }
}

impl ApplyCodexPatchTool {
    pub fn new(workspace_roots: Vec<PathBuf>) -> Self {
        let file_manager = FileAccessManager::new(workspace_roots);
        Self { file_manager }
    }

    /// Strip leading and trailing @@ markers from a hunk string.
    fn strip_leading_trailing_markers(&self, hunk_str: &str) -> String {
        let lines: Vec<&str> = hunk_str.lines().collect();
        let mut start = 0;
        let mut end = lines.len();

        while start < end && lines[start].trim_start().starts_with("@@") {
            start += 1;
        }

        while end > start && lines[end - 1].trim_start().starts_with("@@") {
            end -= 1;
        }

        lines[start..end].join("\n")
    }

    /// AI models sometimes incorrectly concatenate multiple hunks into a single string.
    /// This silently fixes such errors to improve usability without advertising the capability.
    fn split_hunks_on_markers(&self, hunks: &[String]) -> Vec<String> {
        hunks
            .iter()
            .flat_map(|hunk| self.split_single_hunk(hunk))
            .collect()
    }

    fn split_single_hunk(&self, hunk: &str) -> Vec<String> {
        let lines: Vec<&str> = hunk.lines().collect();
        let mut result = Vec::new();
        let mut current_hunk_lines = Vec::new();
        let mut seen_content = false;

        for line in lines {
            let is_marker = line.trim_start().starts_with("@@");

            if is_marker && !seen_content {
                continue;
            }

            if is_marker && seen_content {
                if !current_hunk_lines.is_empty() {
                    result.push(current_hunk_lines.join("\n"));
                    current_hunk_lines.clear();
                }
                seen_content = false;
                continue;
            }

            current_hunk_lines.push(line);
            seen_content = true;
        }

        if !current_hunk_lines.is_empty() {
            result.push(current_hunk_lines.join("\n"));
        }
        result
    }

    /// Parse a single hunk from a string.
    fn parse_single_hunk(&self, hunk_str: &str) -> Result<CodexHunk> {
        let cleaned = self.strip_leading_trailing_markers(hunk_str);
        let lines: Vec<&str> = cleaned.lines().collect();
        let mut hunk_lines = Vec::new();

        for line in lines {
            if line.starts_with("-") {
                hunk_lines.push(CodexHunkLine::Removal(line[1..].to_string()));
            } else if line.starts_with("+") {
                hunk_lines.push(CodexHunkLine::Addition(line[1..].to_string()));
            } else if line.starts_with(" ") {
                hunk_lines.push(CodexHunkLine::Context(line[1..].to_string()));
            } else if line.is_empty() {
                hunk_lines.push(CodexHunkLine::Context(String::new()));
            } else {
                hunk_lines.push(CodexHunkLine::Context(line.to_string()));
            }
        }

        while let Some(CodexHunkLine::Context(content)) = hunk_lines.first() {
            if !content.trim().is_empty() {
                break;
            }
            hunk_lines.remove(0);
        }

        while let Some(CodexHunkLine::Context(content)) = hunk_lines.last() {
            if !content.trim().is_empty() {
                break;
            }
            hunk_lines.pop();
        }

        let has_changes = hunk_lines
            .iter()
            .any(|line| matches!(line, CodexHunkLine::Removal(_) | CodexHunkLine::Addition(_)));

        if !has_changes {
            bail!("Hunk must contain at least one addition (+ line) or removal (- line)");
        }

        Ok(CodexHunk { lines: hunk_lines })
    }

    /// Find the position where a hunk should be applied
    fn find_hunk_position(&self, file_lines: &[String], hunk: &CodexHunk) -> Result<usize> {
        let expected_original: Vec<String> = hunk
            .lines
            .iter()
            .filter_map(|line| match line {
                CodexHunkLine::Context(content) => Some(content.clone()),
                CodexHunkLine::Removal(content) => Some(content.clone()),
                CodexHunkLine::Addition(_) => None,
            })
            .collect();

        if expected_original.is_empty() {
            bail!(
                "Hunk must contain some original content to match: \n{}",
                hunk.patch()
            );
        }

        let mut matches = Vec::new();
        for start_idx in 0..=file_lines.len().saturating_sub(expected_original.len()) {
            if self.hunk_matches_at(file_lines, start_idx, &expected_original) {
                matches.push(start_idx);
            }
        }

        match matches.len() {
            0 => {
                let closest_match =
                    find_closest_match(file_lines.to_vec(), expected_original.clone());

                if let Some(closest) = closest_match {
                    bail!(
                        "Could not find matching content for hunk in file. {}\n\nTip: ensure you are tracking the file (set_tracked_files tool) to give see the latest contents of the file.",
                        closest.get_correction_feedback().unwrap(),
                    );
                }

                bail!("Could not find matching content for hunk in file. The original content expected by this patch does not match any location in the file.\n\nOriginal content being searched for:\n{}\n\nTip: Check that the file content matches what the patch expects.",
                    hunk.patch()
                );
            }
            1 => Ok(matches[0]),
            _ => {
                bail!("Found {} possible locations for hunk matching: \n{}.\n\nTip: Use more lines of context to make the location unique",
                    matches.len(),
                    hunk.patch()
                );
            }
        }
    }

    /// Check if two lines match, tolerating whitespace differences
    fn lines_match_tolerant(&self, file_line: &str, expected_line: &str) -> bool {
        if file_line == expected_line {
            return true;
        }

        if file_line.trim().is_empty() && expected_line.trim().is_empty() {
            return true;
        }

        if file_line.trim_end() == expected_line.trim_end() {
            return true;
        }

        if file_line.starts_with(' ') && &file_line[1..] == expected_line {
            return true;
        }

        if expected_line.starts_with(' ') && &expected_line[1..] == file_line {
            return true;
        }

        false
    }

    /// Check if expected hunk lines match file content at given position.
    /// Uses tolerant matching to accommodate whitespace variations from models.
    fn hunk_matches_at(
        &self,
        file_lines: &[String],
        start_idx: usize,
        expected_lines: &[String],
    ) -> bool {
        expected_lines.iter().enumerate().all(|(i, expected_line)| {
            file_lines
                .get(start_idx + i)
                .map(|file_line| self.lines_match_tolerant(file_line, expected_line))
                .unwrap_or(false)
        })
    }

    /// Apply a single hunk to the file lines
    fn apply_hunk(&self, file_lines: &mut Vec<String>, hunk: &CodexHunk) -> Result<usize> {
        let position = self.find_hunk_position(file_lines, hunk)?;
        let mut file_pos = position;
        let mut hunk_line_idx = 0;

        while hunk_line_idx < hunk.lines.len() {
            let line = &hunk.lines[hunk_line_idx];
            match line {
                CodexHunkLine::Context(content) => {
                    let file_line = file_lines.get(file_pos).ok_or_else(|| {
                        anyhow::anyhow!("Context line {} does not exist", file_pos + 1)
                    })?;
                    if !self.lines_match_tolerant(file_line, content) {
                        bail!(
                            "Context mismatch at line {}: expected '{}' but found '{}'",
                            file_pos + 1,
                            content,
                            file_line
                        );
                    }
                    file_pos += 1;
                }
                CodexHunkLine::Removal(content) => {
                    let file_line = file_lines.get(file_pos).ok_or_else(|| {
                        anyhow::anyhow!("Cannot remove line {} - does not exist", file_pos + 1)
                    })?;
                    if !self.lines_match_tolerant(file_line, content) {
                        bail!(
                            "Removal mismatch at line {}: expected '{}' but found '{}'",
                            file_pos + 1,
                            content,
                            file_line
                        );
                    }
                    file_lines.remove(file_pos);
                }
                CodexHunkLine::Addition(content) => {
                    file_lines.insert(file_pos, content.clone());
                    file_pos += 1;
                }
            }
            hunk_line_idx += 1;
        }

        Ok(position)
    }

    /// Apply multiple hunks individually, collecting success/failure info.
    /// Returns success if ANY hunk was applied successfully.
    /// Logs warnings about failed hunks with full hunk content.
    fn apply_hunks(
        &self,
        content: &str,
        hunk_strings: &[String],
    ) -> Result<(String, Option<String>)> {
        let mut file_lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        let mut successes = Vec::new();
        let mut failures: Vec<(usize, String, String)> = Vec::new();

        // Phase 1: Parse hunks individually, collect parse failures
        let mut parsed_hunks = Vec::new();
        for (idx, hunk_str) in hunk_strings.iter().enumerate() {
            match self.parse_single_hunk(hunk_str) {
                Ok(hunk) => parsed_hunks.push((idx, hunk, hunk_str.clone())),
                Err(e) => failures.push((idx, format!("{}", e), hunk_str.clone())),
            }
        }

        // Phase 2: Find positions for hunks individually, collect position failures
        let mut positioned_hunks = Vec::new();
        for (idx, hunk, hunk_str) in parsed_hunks {
            match self.find_hunk_position(&file_lines, &hunk) {
                Ok(pos) => positioned_hunks.push((idx, pos, hunk, hunk_str)),
                Err(e) => failures.push((idx, format!("{}", e), hunk_str)),
            }
        }

        // Sort by position descending (bottom to top) to avoid line number shifts
        positioned_hunks.sort_by_key(|(_, pos, _, _)| std::cmp::Reverse(*pos));

        // Phase 3: Apply each hunk individually, collect application failures
        for (idx, _pos, hunk, hunk_str) in positioned_hunks {
            match self.apply_hunk(&mut file_lines, &hunk) {
                Ok(_) => successes.push(idx),
                Err(e) => failures.push((idx, format!("{}", e), hunk_str)),
            }
        }

        // If all hunks failed, return error with details about all failures
        if successes.is_empty() {
            let mut error_msg = format!("All {} hunk(s) failed:\n\n", hunk_strings.len());
            for (idx, error, content) in &failures {
                error_msg.push_str(&format!("Hunk {} failed:\n", idx));
                error_msg.push_str(&format!("Error: {}\n", error));
                error_msg.push_str(&format!("Hunk content:\n{}\n\n", content));
            }
            return Err(anyhow::anyhow!(error_msg));
        }

        // If some failed but others succeeded, log warnings and return success with modified content
        if !failures.is_empty() {
            let mut warning_msg = format!(
                "Applied {}/{} hunks. {} failed and were skipped:\n\n",
                successes.len(),
                hunk_strings.len(),
                failures.len()
            );
            for (idx, error, content) in &failures {
                warning_msg.push_str(&format!("Hunk {} failed:\n", idx));
                warning_msg.push_str(&format!("Error: {}\n", error));
                warning_msg.push_str(&format!("Hunk content:\n{}\n\n", content));
            }
            return Ok((file_lines.join("\n"), Some(warning_msg)));
        }

        Ok((file_lines.join("\n"), None))
    }
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for ApplyCodexPatchTool {
    fn name(&self) -> &'static str {
        "modify_file"
    }

    fn description(&self) -> &'static str {
        "Modify a file by applying multiple hunks in a single call (no line numbers required). Each hunk independently specifies a location and changes to apply."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to patch"
                },
                "hunks": {
                    "type": "string",
                    "description": r#"One or more diffs to apply to the file. Multiple independent changes can be applied in a single call by separating hunks with @@ markers.

Each hunk shows which lines to keep (context), remove, or add:
- Lines starting with ' ' (space) = context - existing lines that help locate where to make changes
- Lines starting with '-' = remove this line
- Lines starting with '+' = add this line

The tool finds the right location by matching the context lines, then applies the additions and removals.

Example - to change 'line 3' to 'line 3 modified':
 line 2
-line 3
+line 3 modified
 line 4

Example - multiple changes in one call:
 line 2
-line 3
+line 3 modified
 line 4
@@
 line 10
-line 11
+line 11 updated
 line 12

Use enough context lines to uniquely identify each location."#
                }
            },
            "required": ["file_path", "hunks"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Execution
    }

    async fn validate(&self, request: &ToolRequest) -> Result<ValidatedToolCall> {
        let file_path = request
            .arguments
            .get("file_path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: file_path"))?;

        let hunks_string = request
            .arguments
            .get("hunks")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                anyhow::anyhow!("Missing required parameter: hunks (must be a string)")
            })?;

        if hunks_string.trim().is_empty() {
            bail!("hunks string must not be empty");
        }

        let hunk_strings = self.split_hunks_on_markers(&[hunks_string.to_string()]);

        let original_content: String = self.file_manager.read_file(file_path).await?;

        let (patched_content, warning) = self.apply_hunks(&original_content, &hunk_strings)?;

        let modification = FileModification {
            path: PathBuf::from(file_path),
            operation: FileOperation::Update,
            original_content: Some(original_content),
            new_content: Some(patched_content),
            warning,
        };

        Ok(ValidatedToolCall::FileModification(modification))
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_apply_codex_patch_simple() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().join("test");
        fs::create_dir(&root).unwrap();
        let tool = ApplyCodexPatchTool::new(vec![root.clone()]);

        let file_manager = FileAccessManager::new(vec![root.clone()]);
        let original_content = "line 1\nline 2\nline 3\nline 4\nline 5";
        file_manager
            .write_file("/test/test.txt", original_content)
            .await
            .unwrap();

        let hunks = r#" line 2
-line 3
+line 3 modified
 line 4"#;

        let request = ToolRequest::new(
            json!({
                "file_path": "/test/test.txt",
                "hunks": hunks
            }),
            "test_id".to_string(),
        );
        let result = tool.validate(&request).await.unwrap();

        match result {
            ValidatedToolCall::FileModification(modification) => {
                assert_eq!(modification.path.to_str().unwrap(), "/test/test.txt");
                assert_eq!(modification.operation, FileOperation::Update);
                let expected_new = "line 1\nline 2\nline 3 modified\nline 4\nline 5";
                assert_eq!(modification.new_content.as_ref().unwrap(), expected_new);
                assert_eq!(
                    modification.original_content.as_ref().unwrap(),
                    original_content
                );
            }
            _ => panic!("Expected FileModification variant"),
        }
    }

    #[tokio::test]
    async fn test_apply_codex_patch_unprefixed_context() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().join("test");
        fs::create_dir(&root).unwrap();
        let tool = ApplyCodexPatchTool::new(vec![root.clone()]);

        let file_manager = FileAccessManager::new(vec![root.clone()]);
        let original_content = "line 1\nline 2\nline 3\nline 4\nline 5";
        file_manager
            .write_file("/test/test.txt", original_content)
            .await
            .unwrap();

        let hunks = r#"line 2
-line 3
+line 3 modified
line 4"#;

        let request = ToolRequest::new(
            json!({
                "file_path": "/test/test.txt",
                "hunks": hunks
            }),
            "test_id".to_string(),
        );
        let result = tool.validate(&request).await.unwrap();

        match result {
            ValidatedToolCall::FileModification(modification) => {
                let expected_new = "line 1\nline 2\nline 3 modified\nline 4\nline 5";
                assert_eq!(modification.new_content.as_ref().unwrap(), expected_new);
            }
            _ => panic!("Expected FileModification variant"),
        }
    }

    #[tokio::test]
    async fn test_apply_codex_patch_whitespace_tolerant() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().join("test");
        fs::create_dir(&root).unwrap();
        let tool = ApplyCodexPatchTool::new(vec![root.clone()]);

        let file_manager = FileAccessManager::new(vec![root.clone()]);
        let original_content = "line 1\nline 2\n line 3\nline 4";
        file_manager
            .write_file("/test/test.txt", original_content)
            .await
            .unwrap();

        let hunks = r#" line 2
 line 3
-line 4
+line 5"#;

        let request = ToolRequest::new(
            json!({
                "file_path": "/test/test.txt",
                "hunks": hunks
            }),
            "test_id".to_string(),
        );
        let result = tool.validate(&request).await.unwrap();

        match result {
            ValidatedToolCall::FileModification(modification) => {
                let expected_new = "line 1\nline 2\n line 3\nline 5";
                assert_eq!(modification.new_content.as_ref().unwrap(), expected_new);
            }
            _ => panic!("Expected FileModification variant"),
        }
    }

    #[tokio::test]
    async fn test_apply_codex_patch_add_only() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().join("test");
        fs::create_dir(&root).unwrap();
        let tool = ApplyCodexPatchTool::new(vec![root.clone()]);

        let file_manager = FileAccessManager::new(vec![root.clone()]);
        let original_content = "line 1\nline 2\nline 3";
        file_manager
            .write_file("/test/test.txt", original_content)
            .await
            .unwrap();

        let hunks = r#" line 1
+ added line
 line 2"#;

        let request = ToolRequest::new(
            json!({
                "file_path": "/test/test.txt",
                "hunks": hunks
            }),
            "test_id".to_string(),
        );
        let result = tool.validate(&request).await.unwrap();

        match result {
            ValidatedToolCall::FileModification(modification) => {
                let expected_new = "line 1\n added line\nline 2\nline 3";
                assert_eq!(modification.new_content.as_ref().unwrap(), expected_new);
            }
            _ => panic!("Expected FileModification variant"),
        }
    }

    #[tokio::test]
    async fn test_apply_codex_patch_invalid_format() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().join("test");
        fs::create_dir(&root).unwrap();
        let tool = ApplyCodexPatchTool::new(vec![root.clone()]);

        let file_manager = FileAccessManager::new(vec![root.clone()]);
        let original_content = "line 1\nline 2";
        file_manager
            .write_file("/test/test.txt", original_content)
            .await
            .unwrap();

        let hunks = " line 1\n line 2";

        let request = ToolRequest::new(
            json!({
                "file_path": "/test/test.txt",
                "hunks": hunks
            }),
            "test_id".to_string(),
        );
        let result = tool.validate(&request).await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must contain at least one addition"));
    }

    #[tokio::test]
    async fn test_apply_codex_patch_multiple_hunks() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().join("test");
        fs::create_dir(&root).unwrap();
        let tool = ApplyCodexPatchTool::new(vec![root.clone()]);

        let file_manager = FileAccessManager::new(vec![root.clone()]);
        let original_content = "line 1\nline 2\nline 3\nline 4\nline 5\nline 6\nline 7";
        file_manager
            .write_file("/test/test.txt", original_content)
            .await
            .unwrap();

        let hunks = r#" line 2
-line 3
+line 3 modified
 line 4
@@
 line 6
-line 7
+line 7 updated"#;

        let request = ToolRequest::new(
            json!({
                "file_path": "/test/test.txt",
                "hunks": hunks
            }),
            "test_id".to_string(),
        );
        let result = tool.validate(&request).await.unwrap();

        match result {
            ValidatedToolCall::FileModification(modification) => {
                let expected_new =
                    "line 1\nline 2\nline 3 modified\nline 4\nline 5\nline 6\nline 7 updated";
                assert_eq!(modification.new_content.as_ref().unwrap(), expected_new);
                assert_eq!(
                    modification.original_content.as_ref().unwrap(),
                    original_content
                );
            }
            _ => panic!("Expected FileModification variant"),
        }
    }

    #[tokio::test]
    async fn test_apply_codex_patch_interleaved_changes() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().join("test");
        fs::create_dir(&root).unwrap();
        let tool = ApplyCodexPatchTool::new(vec![root.clone()]);

        let file_manager = FileAccessManager::new(vec![root.clone()]);
        let original_content = "some context\nsome line to remove\nsome other context\nanother to remove\nfinal context";
        file_manager
            .write_file("/test/test.txt", original_content)
            .await
            .unwrap();

        let hunks = r#" some context
+ insert A
-some line to remove
 some other context
+ insert B
-another to remove
 final context"#;

        let request = ToolRequest::new(
            json!({
                "file_path": "/test/test.txt",
                "hunks": hunks
            }),
            "test_id".to_string(),
        );
        let result = tool.validate(&request).await.unwrap();

        match result {
            ValidatedToolCall::FileModification(modification) => {
                let expected_new =
                    "some context\n insert A\nsome other context\n insert B\nfinal context";
                assert_eq!(modification.new_content.as_ref().unwrap(), expected_new);
                assert_eq!(
                    modification.original_content.as_ref().unwrap(),
                    original_content
                );
            }
            _ => panic!("Expected FileModification variant"),
        }
    }

    #[tokio::test]
    async fn test_strip_leading_trailing_markers() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().join("test");
        fs::create_dir(&root).unwrap();
        let tool = ApplyCodexPatchTool::new(vec![root.clone()]);

        let file_manager = FileAccessManager::new(vec![root.clone()]);
        let original_content = "line 1\nline 2\nline 3";
        file_manager
            .write_file("/test/test.txt", original_content)
            .await
            .unwrap();

        let hunks = r#"@@
 line 1
-line 2
+line 2 modified
 line 3
@@"#;

        let request = ToolRequest::new(
            json!({
                "file_path": "/test/test.txt",
                "hunks": hunks
            }),
            "test_id".to_string(),
        );
        let result = tool.validate(&request).await.unwrap();

        match result {
            ValidatedToolCall::FileModification(modification) => {
                let expected_new = "line 1\nline 2 modified\nline 3";
                assert_eq!(modification.new_content.as_ref().unwrap(), expected_new);
            }
            _ => panic!("Expected FileModification variant"),
        }
    }

    #[tokio::test]
    async fn test_apply_codex_patch_partial_failure() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().join("test");
        fs::create_dir(&root).unwrap();
        let tool = ApplyCodexPatchTool::new(vec![root.clone()]);

        let file_manager = FileAccessManager::new(vec![root.clone()]);
        let original_content = "line 1\nline 2\nline 3\nline 4\nline 5";
        file_manager
            .write_file("/test/test.txt", original_content)
            .await
            .unwrap();

        // First hunk succeeds, second hunk fails (no match)
        let hunks = r#" line 1
-line 2
+line 2 modified
 line 3
@@
 nonexistent
-line should fail
+replacement"#;

        let request = ToolRequest::new(
            json!({
                "file_path": "/test/test.txt",
                "hunks": hunks
            }),
            "test_id".to_string(),
        );
        let result = tool.validate(&request).await.unwrap();

        match result {
            ValidatedToolCall::FileModification(modification) => {
                // First hunk should be applied, second skipped
                let expected_new = "line 1\nline 2 modified\nline 3\nline 4\nline 5";
                assert_eq!(modification.new_content.as_ref().unwrap(), expected_new);
            }
            _ => panic!("Expected FileModification variant"),
        }
    }

    #[tokio::test]
    async fn test_merge_conflict_resolution() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().join("test");
        fs::create_dir(&root).unwrap();
        let tool = ApplyCodexPatchTool::new(vec![root.clone()]);

        let file_manager = FileAccessManager::new(vec![root.clone()]);
        let original_content = r#"fn sum_numbers(numbers: Vec<i32>) -> i32 {
    let mut total = 0;
    for num in numbers {
        total += num;
        <<<<<<< HEAD
        // Old debug output
        println!("Adding {} to total", num);
        =======
        // New debug output with emoji! 🎯
        println!("🔢 Adding {} to total 📊", num);
        >>>>>>> branch-feature-emoji-logs
    }
    return total;
}"#;
        file_manager
            .write_file("/test/conflict.rs", original_content)
            .await
            .unwrap();

        let hunks = r#"    for num in numbers {
        total += num;
-        <<<<<<< HEAD
-        // Old debug output
-        println!("Adding {} to total", num);
-        =======
-        // New debug output with emoji! 🎯
         println!("🔢 Adding {} to total 📊", num);
-        >>>>>>> branch-feature-emoji-logs
    }
    return total;"#;

        let request = ToolRequest::new(
            json!({
                "file_path": "/test/conflict.rs",
                "hunks": hunks
            }),
            "test_id".to_string(),
        );
        let result = tool.validate(&request).await;

        match result {
            Ok(ValidatedToolCall::FileModification(modification)) => {
                let expected_new = r#"fn sum_numbers(numbers: Vec<i32>) -> i32 {
    let mut total = 0;
    for num in numbers {
        total += num;
        println!("🔢 Adding {} to total 📊", num);
    }
    return total;
}"#;
                assert_eq!(modification.new_content.as_ref().unwrap(), expected_new);
            }
            Err(e) => {
                panic!("Merge conflict resolution failed: {}", e);
            }
            _ => panic!("Expected FileModification variant"),
        }
    }

    #[tokio::test]
    async fn test_whitespace_mismatch_in_context() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().join("test");
        fs::create_dir(&root).unwrap();
        let tool = ApplyCodexPatchTool::new(vec![root.clone()]);

        let file_manager = FileAccessManager::new(vec![root.clone()]);
        let original_content = "    line with 4 spaces\n        line with 8 spaces\n    back to 4";
        file_manager
            .write_file("/test/whitespace.txt", original_content)
            .await
            .unwrap();

        let hunks = r#"    line with 4 spaces
-        line with 8 spaces
+         line with 9 spaces
    back to 4"#;

        let request = ToolRequest::new(
            json!({
                "file_path": "/test/whitespace.txt",
                "hunks": hunks
            }),
            "test_id".to_string(),
        );
        let result = tool.validate(&request).await.unwrap();

        match result {
            ValidatedToolCall::FileModification(modification) => {
                let expected_new =
                    "    line with 4 spaces\n         line with 9 spaces\n    back to 4";
                assert_eq!(modification.new_content.as_ref().unwrap(), expected_new);
            }
            _ => panic!("Expected FileModification variant"),
        }
    }

    #[tokio::test]
    async fn test_hunk_with_extra_leading_space_in_context() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().join("test");
        fs::create_dir(&root).unwrap();
        let tool = ApplyCodexPatchTool::new(vec![root.clone()]);

        let file_manager = FileAccessManager::new(vec![root.clone()]);
        let original_content = "line 1\n        line 2 with 8 spaces\nline 3";
        file_manager
            .write_file("/test/test.txt", original_content)
            .await
            .unwrap();

        let hunks = r#" line 1
         line 2 with 8 spaces
-line 3
+line 3 modified"#;

        let request = ToolRequest::new(
            json!({
                "file_path": "/test/test.txt",
                "hunks": hunks
            }),
            "test_id".to_string(),
        );
        let result = tool.validate(&request).await;

        match result {
            Ok(ValidatedToolCall::FileModification(modification)) => {
                let expected_new = "line 1\n        line 2 with 8 spaces\nline 3 modified";
                assert_eq!(modification.new_content.as_ref().unwrap(), expected_new);
            }
            Err(e) => {
                println!("Error (this reveals the bug): {}", e);
                panic!("Hunk matching failed due to whitespace handling bug: {}", e);
            }
            _ => panic!("Expected FileModification variant"),
        }
    }

    #[test]
    fn test_lines_match_tolerant_asymmetry() {
        let tool = ApplyCodexPatchTool::new(vec![]);

        assert!(tool.lines_match_tolerant("line content", "line content"));

        assert!(tool.lines_match_tolerant(" line content", "line content"));

        let result = tool.lines_match_tolerant("line content", " line content");
        assert!(
            result,
            "Bug: lines_match_tolerant is asymmetric. It tolerates file having extra space but not expected having extra space."
        );
    }
}
