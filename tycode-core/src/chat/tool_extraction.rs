use crate::ai::types::{Content, ToolUseData};

use super::json_tool_parser::parse_json_tool_calls;
use super::xml_tool_parser::parse_xml_tool_calls;

/// Result of extracting tool calls from AI response content.
/// Combines native API tool calls with XML and JSON parsed calls from text.
pub struct ToolExtractionResult {
    /// All extracted tool calls (native + XML + JSON)
    pub tool_calls: Vec<ToolUseData>,
    /// Remaining display text after removing parsed tool call blocks
    pub display_text: String,
    /// Error from XML parsing, if any
    pub xml_parse_error: Option<String>,
    /// Error from JSON parsing, if any
    pub json_parse_error: Option<String>,
}

/// Extract all tool calls from AI response content using three strategies:
/// 1. Native tool use blocks from the API response
/// 2. XML function_calls blocks parsed from text
/// 3. JSON tool_use structures parsed from remaining text
pub fn extract_all_tool_calls(content: &Content) -> ToolExtractionResult {
    let native_tool_calls: Vec<_> = content.tool_uses().iter().map(|t| (*t).clone()).collect();

    let (xml_tool_calls, text_after_xml, xml_parse_error) =
        match parse_xml_tool_calls(&content.text()) {
            Ok((calls, remaining)) => (calls, remaining, None),
            Err(e) => (vec![], content.text(), Some(format!("{e:?}"))),
        };

    let (json_tool_calls, display_text, json_parse_error) =
        match parse_json_tool_calls(&text_after_xml) {
            Ok((calls, remaining)) => (calls, remaining, None),
            Err(e) => (vec![], text_after_xml, Some(format!("{e:?}"))),
        };

    let mut tool_calls = native_tool_calls;
    tool_calls.extend(xml_tool_calls);
    tool_calls.extend(json_tool_calls);

    ToolExtractionResult {
        tool_calls,
        display_text,
        xml_parse_error,
        json_parse_error,
    }
}
