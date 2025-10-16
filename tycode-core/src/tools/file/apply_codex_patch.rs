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
    Context(String),  // Line starting with ' ' or unprefixed
    Removal(String),  // Line starting with '-'
    Addition(String), // Line starting with '+'
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
        let mut result = "".to_string();
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

    /// Parse a codex patch into hunks
    fn parse_patch(&self, patch: &str) -> Result<Vec<CodexHunk>> {
        let mut hunks = Vec::new();
        let lines: Vec<&str> = patch.lines().collect();
        let mut i = 0;

        while i < lines.len() {
            if lines[i].trim_start().starts_with("@@") {
                let hunk = self.parse_single_hunk(&lines, &mut i)?;
                hunks.push(hunk);
            } else {
                i += 1;
            }
        }

        if hunks.is_empty() {
            bail!("No valid codex hunks found in patch. Expected format starting with @@");
        }

        Ok(hunks)
    }

    /// Parse a single hunk starting at index i
    fn parse_single_hunk(&self, lines: &[&str], i: &mut usize) -> Result<CodexHunk> {
        let mut hunk_lines = Vec::new();

        // Skip the @@ line
        *i += 1;

        while *i < lines.len() {
            let line = lines[*i];

            // Stop at next hunk or end of patch
            if line.trim_start().starts_with("@@") {
                *i -= 1; // Back up so outer loop sees the next @@
                break;
            }

            if line.starts_with("-") {
                hunk_lines.push(CodexHunkLine::Removal(line[1..].to_string()));
            } else if line.starts_with("+") {
                hunk_lines.push(CodexHunkLine::Addition(line[1..].to_string()));
            } else if line.starts_with(" ") {
                hunk_lines.push(CodexHunkLine::Context(line[1..].to_string()));
            } else if line.is_empty() {
                hunk_lines.push(CodexHunkLine::Context(String::new()));
            } else if !line.starts_with("@@") {
                hunk_lines.push(CodexHunkLine::Context(line.to_string()));
            } else {
                bail!("Invalid line format in hunk: '{}'. Expected lines starting with '-', '+', or ' '", line);
            }

            *i += 1;
        }

        // Trim leading blank context lines - these are often just separators between hunks
        while let Some(CodexHunkLine::Context(content)) = hunk_lines.first() {
            if !content.trim().is_empty() {
                break;
            }
            hunk_lines.remove(0);
        }

        // Trim trailing blank context lines - these are often just separators between hunks
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
            bail!(
                "Hunk must contain at least one addition (+ line) or removal (- line): {}",
                lines.join("\n")
            );
        }

        Ok(CodexHunk { lines: hunk_lines })
    }

    /// Find the position where a hunk should be applied
    fn find_hunk_position(&self, file_lines: &[String], hunk: &CodexHunk) -> Result<usize> {
        // Build the expected file content by converting the hunk to original form
        let expected_original: Vec<String> = hunk
            .lines
            .iter()
            .filter_map(|line| match line {
                CodexHunkLine::Context(content) => Some(content.clone()),
                CodexHunkLine::Removal(content) => Some(content.clone()),
                CodexHunkLine::Addition(_) => None, // Additions don't exist in original
            })
            .collect();

        if expected_original.is_empty() {
            bail!(
                "Hunk must contain some original content to match: \n{}",
                hunk.patch()
            );
        }

        let mut matches = Vec::new();

        // Try to find the expected original content sequence in the file
        // with tolerant matching for single leading space differences
        for start_idx in 0..=file_lines.len().saturating_sub(expected_original.len()) {
            let matches_sequence =
                expected_original
                    .iter()
                    .enumerate()
                    .all(|(i, expected_line)| {
                        if let Some(file_line) = file_lines.get(start_idx + i) {
                            self.lines_match_tolerant(file_line, expected_line)
                        } else {
                            false
                        }
                    });

            if matches_sequence {
                matches.push(start_idx);
            }
        }

        match matches.len() {
            0 => {
                // No exact matches - try fuzzy matching for helpful error
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
        // Exact match first (fast path)
        if file_line == expected_line {
            return true;
        }

        // Both lines are whitespace-only - treat as matching
        if file_line.trim().is_empty() && expected_line.trim().is_empty() {
            return true;
        }

        // Trim trailing whitespace and compare
        if file_line.trim_end() == expected_line.trim_end() {
            return true;
        }

        // Tolerate single leading space difference.
        // Models sometimes forget to include leading space to indicate context
        // in the diff format.
        if file_line.starts_with(' ') && &file_line[1..] == expected_line {
            return true;
        }

        false
    }

    /// Apply a single hunk to the file lines
    fn apply_hunk(&self, file_lines: &mut Vec<String>, hunk: &CodexHunk) -> Result<usize> {
        let position = self.find_hunk_position(file_lines, hunk)?;

        // Apply the hunk by walking through it and modifying the file
        let mut file_pos = position;
        let mut hunk_line_idx = 0;

        while hunk_line_idx < hunk.lines.len() {
            match &hunk.lines[hunk_line_idx] {
                CodexHunkLine::Context(content) => {
                    // Context should match the file content (with tolerant matching)
                    if let Some(file_line) = file_lines.get(file_pos) {
                        if !self.lines_match_tolerant(file_line, content) {
                            bail!(
                                "Context mismatch at line {}: expected '{}' but found '{}'",
                                file_pos + 1,
                                content,
                                file_line
                            );
                        }
                    } else {
                        bail!("Context line {} does not exist in file", file_pos + 1);
                    }
                    file_pos += 1;
                    hunk_line_idx += 1;
                }
                CodexHunkLine::Removal(content) => {
                    // Remove the line and verify it matches (with tolerant matching)
                    if let Some(file_line) = file_lines.get(file_pos) {
                        if !self.lines_match_tolerant(file_line, content) {
                            bail!("Removal mismatch at line {}: expected to remove '{}' but found '{}'",
                                file_pos + 1, content, file_line);
                        }
                    } else {
                        bail!("Cannot remove line {} - line does not exist", file_pos + 1);
                    }

                    // Remove the line from file
                    file_lines.remove(file_pos);
                    hunk_line_idx += 1;
                    // Don't increment file_pos since we removed a line
                }
                CodexHunkLine::Addition(content) => {
                    // Insert the new line at current position
                    file_lines.insert(file_pos, content.clone());
                    file_pos += 1;
                    hunk_line_idx += 1;
                }
            }
        }

        Ok(position)
    }

    /// Apply the entire patch to content
    fn apply_patch(&self, content: &str, patch: &str) -> Result<String> {
        let mut file_lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
        let hunks = self.parse_patch(patch)?;

        // Apply hunks from bottom to top to avoid line number shifts
        let mut hunk_positions = Vec::new();
        for hunk in &hunks {
            let position = self.find_hunk_position(&file_lines, hunk)?;
            hunk_positions.push((position, hunk));
        }

        // Sort by position in reverse order
        hunk_positions.sort_by_key(|(pos, _)| *pos);
        hunk_positions.reverse();

        // Apply each hunk
        for (_position, hunk) in hunk_positions {
            self.apply_hunk(&mut file_lines, hunk)?;
        }

        Ok(file_lines.join("\n"))
    }
}

#[async_trait::async_trait(?Send)]
impl ToolExecutor for ApplyCodexPatchTool {
    fn name(&self) -> &'static str {
        "modify_file"
    }

    fn description(&self) -> &'static str {
        "Apply a codex-style patch to a file (format with @@ but no line numbers)"
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "file_path": {
                    "type": "string",
                    "description": "Absolute path to the file to patch"
                },
                "patch": {
                    "type": "string",
                    "description": r#"Patch content using hunks. Provide one or more hunks; each hunk must start with a line that contains exactly '@@'. Within a hunk, every line must start with one of:
- ' ' (single space) for a context line, followed by the exact file text. Use a single space line (' ') to represent a blank context line.
- '-' for a removal line, followed by the exact file text to remove.
- '+' for an addition line, followed by the exact file text to insert.

Example:
@@
 line 2
-line 3
+line 3 modified
 line 4

@@
 some context
+ inserted line
 another context"#
                }
            },
            "required": ["file_path", "patch"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Modification
    }

    async fn validate(&self, request: &ToolRequest) -> Result<ValidatedToolCall> {
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

        // Basic patch format validation
        if !patch.contains("@@") {
            bail!("Invalid patch format: must contain at least one '@@' hunk header");
        }

        // Read the current content using FileAccessManager
        let original_content: String = self.file_manager.read_file(file_path).await?;

        // Apply the patch
        let patched_content = self.apply_patch(&original_content, patch)?;

        let modification = FileModification {
            path: PathBuf::from(file_path),
            operation: FileOperation::Update,
            original_content: Some(original_content),
            new_content: Some(patched_content),
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

        let patch = r#"@@
 line 2
-line 3
+line 3 modified
 line 4"#;

        let request = ToolRequest::new(
            json!({
                "file_path": "/test/test.txt",
                "patch": patch
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

        // Patch with unprefixed context lines (no leading space)
        let patch = r#"@@
line 2
-line 3
+line 3 modified
line 4"#;

        let request = ToolRequest::new(
            json!({
                "file_path": "/test/test.txt",
                "patch": patch
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

        // Patch expects "line 3" but file has " line 3" (with leading space)
        let patch = r#"@@
 line 2
 line 3
-line 4
+line 5"#;

        let request = ToolRequest::new(
            json!({
                "file_path": "/test/test.txt",
                "patch": patch
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

        let patch = r#"@@
 line 1
+ added line
 line 2"#;

        let request = ToolRequest::new(
            json!({
                "file_path": "/test/test.txt",
                "patch": patch
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

        // Patch without @@ header
        let patch = r#"- line 2
+ line 2 modified"#;

        let request = ToolRequest::new(
            json!({
                "file_path": "/test/test.txt",
                "patch": patch
            }),
            "test_id".to_string(),
        );
        let result = tool.validate(&request).await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("must contain at least one '@@' hunk header"));
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

        let patch = r#"@@
 line 2
-line 3
+line 3 modified
 line 4

@@
 line 6
-line 7
+line 7 updated
"#;

        let request = ToolRequest::new(
            json!({
                "file_path": "/test/test.txt",
                "patch": patch
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

        // This is the complex interleaved format you mentioned
        let patch = r#"@@
 some context
+ insert A
-some line to remove
 some other context
+ insert B
-another to remove
 final context"#;

        let request = ToolRequest::new(
            json!({
                "file_path": "/test/test.txt",
                "patch": patch
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
}
