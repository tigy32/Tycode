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
use serde_json::{json, Value};
use std::path::PathBuf;

const TOOL_NAME: &str = "replace_in_file";

#[derive(Debug, Clone)]
struct SearchReplaceBlock {
    search: String,
    replace: String,
}

// Models produce varying delimiter lengths; accepting 3+ handles generation variance
fn is_search_start(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.ends_with("SEARCH") && !trimmed.ends_with("SEARCH>") {
        return false;
    }
    let prefix = trimmed
        .strip_suffix("SEARCH>")
        .or_else(|| trimmed.strip_suffix("SEARCH"))
        .unwrap();
    let prefix = prefix.trim_end();
    prefix.len() >= 3 && prefix.chars().all(|c| c == '-')
}

fn is_search_end(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.len() >= 3 && trimmed.chars().all(|c| c == '=')
}

fn is_replace_end(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.ends_with("REPLACE") && !trimmed.ends_with("REPLACE>") {
        return false;
    }
    let prefix = trimmed
        .strip_suffix("REPLACE>")
        .or_else(|| trimmed.strip_suffix("REPLACE"))
        .unwrap();
    let prefix = prefix.trim_end();
    prefix.len() >= 3 && prefix.chars().all(|c| c == '+')
}

/// Models frequently introduce whitespace inconsistencies that exact matching fails on
fn line_trimmed_fallback_match(original: &str, search: &str) -> Option<(usize, usize)> {
    let original_lines: Vec<&str> = original.lines().collect();
    let search_lines: Vec<&str> = search.lines().collect();

    if search_lines.is_empty() {
        return None;
    }

    for i in 0..=original_lines.len().saturating_sub(search_lines.len()) {
        let mut matches = true;
        for j in 0..search_lines.len() {
            if original_lines[i + j].trim() != search_lines[j].trim() {
                matches = false;
                break;
            }
        }

        if matches {
            let match_start: usize = original_lines[..i].iter().map(|l| l.len() + 1).sum();
            let matched_content: String = original_lines[i..i + search_lines.len()].join("\n");
            return Some((match_start, match_start + matched_content.len()));
        }
    }
    None
}

/// Models reliably generate correct first/last lines but may hallucinate middle content
fn block_anchor_fallback_match(original: &str, search: &str) -> Option<(usize, usize)> {
    let original_lines: Vec<&str> = original.lines().collect();
    let search_lines: Vec<&str> = search.lines().collect();

    if search_lines.len() < 3 {
        return None;
    }

    let first_search = search_lines[0].trim();
    let last_search = search_lines[search_lines.len() - 1].trim();
    let block_size = search_lines.len();

    for i in 0..=original_lines.len().saturating_sub(block_size) {
        if original_lines[i].trim() == first_search
            && original_lines[i + block_size - 1].trim() == last_search
        {
            let match_start: usize = original_lines[..i].iter().map(|l| l.len() + 1).sum();
            let matched_content: String = original_lines[i..i + block_size].join("\n");
            return Some((match_start, match_start + matched_content.len()));
        }
    }
    None
}

#[derive(Clone)]
pub struct ClineReplaceInFileTool {
    file_manager: FileAccessManager,
}

impl ClineReplaceInFileTool {
    pub fn tool_name() -> ToolName {
        ToolName::new(TOOL_NAME)
    }

    pub fn new(workspace_roots: Vec<PathBuf>) -> anyhow::Result<Self> {
        let file_manager = FileAccessManager::new(workspace_roots)?;
        Ok(Self { file_manager })
    }

    fn parse_diff_blocks(diff: &str) -> Result<Vec<SearchReplaceBlock>> {
        let mut blocks = Vec::new();
        let lines: Vec<&str> = diff.lines().collect();
        let mut i = 0;

        while i < lines.len() {
            if !is_search_start(lines[i]) {
                i += 1;
                continue;
            }
            i += 1;

            let mut search_lines = Vec::new();
            while i < lines.len() && !is_search_end(lines[i]) {
                search_lines.push(lines[i]);
                i += 1;
            }

            if i >= lines.len() {
                bail!("Missing ======= separator after SEARCH block");
            }
            i += 1;

            let mut replace_lines = Vec::new();
            while i < lines.len() && !is_replace_end(lines[i]) {
                replace_lines.push(lines[i]);
                i += 1;
            }

            if i >= lines.len() {
                bail!("Missing +++++++ REPLACE marker after ======= separator");
            }
            i += 1;

            blocks.push(SearchReplaceBlock {
                search: search_lines.join("\n"),
                replace: replace_lines.join("\n"),
            });
        }

        if blocks.is_empty() {
            bail!("No valid SEARCH/REPLACE blocks found in diff. Expected format:\n------- SEARCH\n[content to find]\n=======\n[replacement content]\n+++++++ REPLACE");
        }

        Ok(blocks)
    }

    fn apply_replacements(
        &self,
        content: &str,
        replacements: Vec<SearchReplaceBlock>,
    ) -> Result<String> {
        let mut result = content.to_string();

        for block in replacements {
            let search = match search_content(&result, &block.search) {
                MatchResult::Multiple { matches, .. } => {
                    bail!(
                        "The following search pattern appears more than once in the file (found {} times). Use unique context to match exactly one occurrence.\n\nSearch pattern:\n{}\n\nTip: Include more surrounding context to make this search pattern unique.",
                        matches,
                        block.search
                    );
                }
                MatchResult::Guess { closest, .. } => {
                    let message = closest
                        .and_then(|c| c.get_correction_feedback())
                        .unwrap_or_else(|| {
                            "Reread the file to see the actual content.".to_string()
                        });
                    bail!("Exact match not found. {message}");
                }
                MatchResult::Exact(search) => search,
                MatchResult::Fuzzy { matched_content } => matched_content,
            };

            if search == block.replace {
                bail!(
                    "Search and replace contents are identical. No changes would be made.\n\nContent:\n{}",
                    block.replace
                );
            }

            result = result.replacen(&search, &block.replace, 1);
        }

        Ok(result)
    }
}

enum MatchResult {
    Multiple {
        matches: usize,
    },
    /// Exact match found - contains the matched content from the original
    Exact(String),
    /// Fuzzy match found via fallback - contains (start_idx, end_idx) and matched content
    Fuzzy {
        matched_content: String,
    },
    /// No match found
    Guess {
        closest: Option<find::MatchResult>,
    },
}

fn search_content(source: &str, search: &str) -> MatchResult {
    // Strategy 1: Exact substring match
    let matches = source.matches(search).count();
    if matches > 1 {
        return MatchResult::Multiple { matches };
    }
    if matches == 1 {
        return MatchResult::Exact(search.to_string());
    }

    // Strategy 2: Line-trimmed fallback (ignore leading/trailing whitespace per line)
    if let Some((start, end)) = line_trimmed_fallback_match(source, search) {
        let matched_content = source[start..end].to_string();
        return MatchResult::Fuzzy { matched_content };
    }

    // Strategy 3: Block-anchor fallback (for 3+ line blocks, match first/last lines)
    if let Some((start, end)) = block_anchor_fallback_match(source, search) {
        let matched_content = source[start..end].to_string();
        return MatchResult::Fuzzy { matched_content };
    }

    // No match found - provide fuzzy suggestion for error message
    let best_match = find_closest_match(
        source.lines().map(str::to_string).collect(),
        search.lines().map(str::to_string).collect(),
    );

    MatchResult::Guess {
        closest: best_match,
    }
}

struct ClineReplaceInFileHandle {
    modification: FileModification,
    tool_use_id: String,
    file_manager: FileAccessManager,
}

#[async_trait::async_trait(?Send)]
impl ToolCallHandle for ClineReplaceInFileHandle {
    fn tool_request(&self) -> ToolRequestEvent {
        ToolRequestEvent {
            tool_call_id: self.tool_use_id.clone(),
            tool_name: TOOL_NAME.to_string(),
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

const DIFF_INSTRUCTIONS: &str = r#"One or more SEARCH/REPLACE blocks following this exact format:
  ```
  ------- SEARCH
  [exact content to find]
  =======
  [new content to replace with]
  +++++++ REPLACE
  ```
  Critical rules:
  1. SEARCH content must match the associated file section to find EXACTLY:
     * Match character-for-character including whitespace, indentation, line endings
     * Include all comments, docstrings, etc.
  2. SEARCH/REPLACE blocks will ONLY replace the first match occurrence.
     * Including multiple unique SEARCH/REPLACE blocks if you need to make multiple changes.
     * Include *just* enough lines in each SEARCH section to uniquely match each set of lines that need to change.
     * When using multiple SEARCH/REPLACE blocks, list them in the order they appear in the file.
  3. Keep SEARCH/REPLACE blocks concise:
     * Break large SEARCH/REPLACE blocks into a series of smaller blocks that each change a small portion of the file.
     * Include just the changing lines, and a few surrounding lines if needed for uniqueness.
     * Do not include long runs of unchanging lines in SEARCH/REPLACE blocks.
     * Each line must be complete. Never truncate lines mid-way through as this can cause matching failures.
  4. Special operations:
     * To move code: Use two SEARCH/REPLACE blocks (one to delete from original + one to insert at new location)
     * To delete code: Use empty REPLACE section"#;

#[async_trait::async_trait(?Send)]
impl ToolExecutor for ClineReplaceInFileTool {
    fn name(&self) -> &'static str {
        TOOL_NAME
    }

    fn description(&self) -> &'static str {
        "Request to replace sections of content in an existing file using SEARCH/REPLACE blocks that define exact changes to specific parts of the file. This tool should be used when you need to make targeted changes to specific parts of a file."
    }

    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "The path of the file to modify (relative to the current working directory)"
                },
                "diff": {
                    "type": "string",
                    "description": DIFF_INSTRUCTIONS
                }
            },
            "required": ["path", "diff"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::Execution
    }

    async fn process(&self, request: &ToolRequest) -> Result<Box<dyn ToolCallHandle>> {
        let file_path = request
            .arguments
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: path"))?;

        let diff = request
            .arguments
            .get("diff")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: diff"))?;

        let replacements = Self::parse_diff_blocks(diff)?;
        let original_content: String = self.file_manager.read_file(file_path).await?;
        let new_content = self.apply_replacements(&original_content, replacements)?;

        let modification = FileModification {
            path: PathBuf::from(file_path),
            operation: FileOperation::Update,
            original_content: Some(original_content),
            new_content: Some(new_content),
            warning: None,
        };

        Ok(Box::new(ClineReplaceInFileHandle {
            modification,
            tool_use_id: request.tool_use_id.clone(),
            file_manager: self.file_manager.clone(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // === Flexible Marker Parsing Tests ===

    #[test]
    fn test_flexible_markers_three_dashes() {
        let diff = r#"--- SEARCH
old
===
new
+++ REPLACE"#;

        let blocks = ClineReplaceInFileTool::parse_diff_blocks(diff).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].search, "old");
        assert_eq!(blocks[0].replace, "new");
    }

    #[test]
    fn test_flexible_markers_ten_dashes() {
        let diff = r#"---------- SEARCH
old
==========
new
++++++++++ REPLACE"#;

        let blocks = ClineReplaceInFileTool::parse_diff_blocks(diff).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].search, "old");
        assert_eq!(blocks[0].replace, "new");
    }

    #[test]
    fn test_flexible_markers_with_legacy_angle_bracket() {
        let diff = r#"------- SEARCH>
old
=======
new
+++++++ REPLACE>"#;

        let blocks = ClineReplaceInFileTool::parse_diff_blocks(diff).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].search, "old");
        assert_eq!(blocks[0].replace, "new");
    }

    #[test]
    fn test_is_search_start_rejects_two_dashes() {
        assert!(!is_search_start("-- SEARCH"));
    }

    #[test]
    fn test_is_search_end_rejects_two_equals() {
        assert!(!is_search_end("=="));
    }

    #[test]
    fn test_is_replace_end_rejects_two_plus() {
        assert!(!is_replace_end("++ REPLACE"));
    }

    // === Line-Trimmed Fallback Tests ===

    #[test]
    fn test_line_trimmed_fallback_whitespace_variations() {
        // Original has different indentation than search
        let original = "    fn foo() {\n        println!(\"hello\");\n    }";
        let search = "fn foo() {\nprintln!(\"hello\");\n}";

        let result = line_trimmed_fallback_match(original, search);
        assert!(result.is_some());
        let (start, end) = result.unwrap();
        assert_eq!(&original[start..end], original);
    }

    #[test]
    fn test_line_trimmed_fallback_leading_whitespace() {
        let original = "line1\n  indented\nline3";
        let search = "indented"; // No leading whitespace

        let result = line_trimmed_fallback_match(original, search);
        assert!(result.is_some());
    }

    #[test]
    fn test_line_trimmed_fallback_no_match() {
        let original = "line1\nline2\nline3";
        let search = "completely different";

        let result = line_trimmed_fallback_match(original, search);
        assert!(result.is_none());
    }

    // === Block-Anchor Fallback Tests ===

    #[test]
    fn test_block_anchor_fallback_middle_differs() {
        // First and last lines match, middle differs
        let original = "fn start() {\n    // original comment\n}";
        let search = "fn start() {\n    // different comment\n}";

        let result = block_anchor_fallback_match(original, search);
        assert!(result.is_some());
        let (start, end) = result.unwrap();
        assert_eq!(&original[start..end], original);
    }

    #[test]
    fn test_block_anchor_fallback_requires_three_lines() {
        let original = "line1\nline2";
        let search = "line1\nline2";

        // Should return None for < 3 lines
        let result = block_anchor_fallback_match(original, search);
        assert!(result.is_none());
    }

    #[test]
    fn test_block_anchor_fallback_with_whitespace() {
        let original = "  fn start() {\n    body\n  }";
        let search = "fn start() {\nbody\n}"; // No leading whitespace

        let result = block_anchor_fallback_match(original, search);
        assert!(result.is_some());
    }

    // === Integration: search_content fallback chain ===

    #[test]
    fn test_search_content_exact_match() {
        let source = "fn foo() {}";
        let search = "fn foo() {}";

        match search_content(source, search) {
            MatchResult::Exact(s) => assert_eq!(s, search),
            _ => panic!("Expected exact match"),
        }
    }

    #[test]
    fn test_search_content_line_trimmed_fallback() {
        let source = "    fn foo() {\n        body\n    }";
        let search = "fn foo() {\nbody\n}"; // Different whitespace

        match search_content(source, search) {
            MatchResult::Fuzzy { matched_content } => {
                assert_eq!(matched_content, source);
            }
            _ => panic!("Expected fuzzy match via line-trimmed fallback"),
        }
    }

    #[test]
    fn test_search_content_block_anchor_fallback() {
        let source = "fn start() {\n    original middle\n}";
        // Same anchors, different middle - line-trimmed won't match, block-anchor will
        let search = "fn start() {\n    completely different middle\n}";

        match search_content(source, search) {
            MatchResult::Fuzzy { matched_content } => {
                assert_eq!(matched_content, source);
            }
            _ => panic!("Expected fuzzy match via block-anchor fallback"),
        }
    }

    #[test]
    fn test_search_content_no_match_returns_guess() {
        let source = "fn foo() {}";
        let search = "completely unrelated content that won't match";

        match search_content(source, search) {
            MatchResult::Guess { .. } => {}
            _ => panic!("Expected Guess for no match"),
        }
    }

    #[test]
    fn test_search_content_multiple_matches() {
        let source = "duplicate\nother\nduplicate";
        let search = "duplicate";

        match search_content(source, search) {
            MatchResult::Multiple { matches } => assert_eq!(matches, 2),
            _ => panic!("Expected Multiple match result"),
        }
    }

    // === Original Tests ===

    #[test]
    fn test_parse_single_block() {
        let diff = r#"------- SEARCH
old content
=======
new content
+++++++ REPLACE"#;

        let blocks = ClineReplaceInFileTool::parse_diff_blocks(diff).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].search, "old content");
        assert_eq!(blocks[0].replace, "new content");
    }

    #[test]
    fn test_parse_multiple_blocks() {
        let diff = r#"------- SEARCH
first old
=======
first new
+++++++ REPLACE

------- SEARCH
second old
=======
second new
+++++++ REPLACE"#;

        let blocks = ClineReplaceInFileTool::parse_diff_blocks(diff).unwrap();
        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].search, "first old");
        assert_eq!(blocks[0].replace, "first new");
        assert_eq!(blocks[1].search, "second old");
        assert_eq!(blocks[1].replace, "second new");
    }

    #[test]
    fn test_parse_multiline_content() {
        let diff = r#"------- SEARCH
fn old_function() {
    println!("old");
}
=======
fn new_function() {
    println!("new");
}
+++++++ REPLACE"#;

        let blocks = ClineReplaceInFileTool::parse_diff_blocks(diff).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(
            blocks[0].search,
            "fn old_function() {\n    println!(\"old\");\n}"
        );
        assert_eq!(
            blocks[0].replace,
            "fn new_function() {\n    println!(\"new\");\n}"
        );
    }

    #[test]
    fn test_parse_empty_replace_for_delete() {
        let diff = r#"------- SEARCH
code to delete
=======
+++++++ REPLACE"#;

        let blocks = ClineReplaceInFileTool::parse_diff_blocks(diff).unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0].search, "code to delete");
        assert_eq!(blocks[0].replace, "");
    }

    #[test]
    fn test_parse_missing_separator_fails() {
        let diff = r#"------- SEARCH
old content
+++++++ REPLACE"#;

        let result = ClineReplaceInFileTool::parse_diff_blocks(diff);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_no_blocks_fails() {
        let diff = "just some random text";
        let result = ClineReplaceInFileTool::parse_diff_blocks(diff);
        assert!(result.is_err());
    }

    #[test]
    fn test_apply_replacements_single() {
        let tool = ClineReplaceInFileTool::new(vec![]).unwrap();
        let content = "line1\nold content\nline3";
        let replacements = vec![SearchReplaceBlock {
            search: "old content".to_string(),
            replace: "new content".to_string(),
        }];

        let result = tool.apply_replacements(content, replacements).unwrap();
        assert_eq!(result, "line1\nnew content\nline3");
    }

    #[test]
    fn test_apply_replacements_multiple_occurrences_fails() {
        let tool = ClineReplaceInFileTool::new(vec![]).unwrap();
        let content = "duplicate\nother\nduplicate";
        let replacements = vec![SearchReplaceBlock {
            search: "duplicate".to_string(),
            replace: "replaced".to_string(),
        }];

        let result = tool.apply_replacements(content, replacements);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("more than once"));
    }
}
