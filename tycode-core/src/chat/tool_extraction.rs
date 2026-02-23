use crate::ai::types::{Content, ToolUseData};

use super::json_tool_parser::parse_json_tool_calls;

pub struct ToolExtractionResult {
    pub tool_calls: Vec<ToolUseData>,
    pub display_text: String,
    pub json_parse_error: Option<String>,
}

/// Extract all tool calls from AI response content using two strategies:
/// 1. Native tool use blocks from the API response
/// 2. JSON tool_use structures parsed from text
pub fn extract_all_tool_calls(content: &Content) -> ToolExtractionResult {
    let native_tool_calls: Vec<_> = content.tool_uses().iter().map(|t| (*t).clone()).collect();

    let (json_tool_calls, display_text, json_parse_error) =
        match parse_json_tool_calls(&content.text()) {
            Ok((calls, remaining)) => (calls, remaining, None),
            Err(e) => (vec![], content.text(), Some(format!("{e:?}"))),
        };

    let mut tool_calls = native_tool_calls;
    tool_calls.extend(json_tool_calls);

    ToolExtractionResult {
        tool_calls,
        display_text,
        json_parse_error,
    }
}
