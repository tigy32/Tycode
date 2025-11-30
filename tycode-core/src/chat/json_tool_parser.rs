use crate::ai::ToolUseData;
use anyhow::Result;
use serde_json::Value;
use tracing::debug;
use uuid::Uuid;

/// Manual brace matching handles nested structures and escaped quotes that regex cannot reliably parse.
fn find_json_end(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if start >= bytes.len() {
        return None;
    }

    let opener = bytes[start];
    let closer = match opener {
        b'{' => b'}',
        b'[' => b']',
        _ => return None,
    };

    let mut depth = 0;
    let mut in_string = false;
    let mut escape_next = false;
    let mut i = start;

    while i < bytes.len() {
        let ch = bytes[i];

        if escape_next {
            escape_next = false;
            i += 1;
            continue;
        }

        if ch == b'\\' && in_string {
            escape_next = true;
            i += 1;
            continue;
        }

        if ch == b'"' {
            in_string = !in_string;
            i += 1;
            continue;
        }

        if in_string {
            i += 1;
            continue;
        }

        if ch == opener {
            depth += 1;
        } else if ch == closer {
            depth -= 1;
            if depth == 0 {
                return Some(i + 1);
            }
        }

        i += 1;
    }

    None
}

/// Recursion handles nested content arrays in Anthropic's message format.
fn extract_tool_uses(value: &Value) -> Vec<ToolUseData> {
    let mut results = Vec::new();

    if let Some(obj) = value.as_object() {
        if obj.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
            if let (Some(name), Some(input)) =
                (obj.get("name").and_then(|v| v.as_str()), obj.get("input"))
            {
                let id = obj
                    .get("id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| Uuid::new_v4().to_string());

                results.push(ToolUseData {
                    id,
                    name: name.to_string(),
                    arguments: input.clone(),
                });
            }
        }

        if let Some(content) = obj.get("content") {
            results.extend(extract_tool_uses(content));
        }
    }

    if let Some(arr) = value.as_array() {
        for item in arr {
            results.extend(extract_tool_uses(item));
        }
    }

    results
}

/// Linear scan from text start required because JSON string state depends on all preceding characters.
fn is_inside_json_string(text: &str, pos: usize) -> bool {
    let bytes = text.as_bytes();
    let mut in_string = false;
    let mut escape_next = false;

    for (i, &ch) in bytes.iter().enumerate() {
        if i >= pos {
            return in_string;
        }

        if escape_next {
            escape_next = false;
            continue;
        }

        if ch == b'\\' && in_string {
            escape_next = true;
            continue;
        }

        if ch == b'"' {
            in_string = !in_string;
        }
    }

    in_string
}

/// Prevents leaving partial JSON in remaining text by finding complete outermost structure.
fn find_outermost_json_containing(
    text: &str,
    search_start: usize,
    marker_pos: usize,
) -> Option<(usize, usize, Value)> {
    let search_region = &text[search_start..marker_pos];

    for (offset, _) in search_region.match_indices('{') {
        let json_start = search_start + offset;
        let Some(json_end) = find_json_end(text, json_start) else {
            continue;
        };

        if json_end <= marker_pos {
            continue;
        }

        let json_str = &text[json_start..json_end];
        if let Ok(parsed) = serde_json::from_str::<Value>(json_str) {
            let extracted = extract_tool_uses(&parsed);
            if !extracted.is_empty() {
                return Some((json_start, json_end, parsed));
            }
        }
    }

    None
}

/// Parse JSON tool calls from text containing `"type":"tool_use"` structures.
///
/// This parser extracts tool calls that match the Anthropic/Claude format.
/// Tool calls can appear as:
/// - Standalone JSON objects: `{"type":"tool_use","id":"toolu_xxx","name":"tool_name","input":{...}}`
/// - Inside a message object's content array: `{"content":[{"type":"tool_use",...}],...}`
/// - Multiple tool calls mixed with regular text
///
/// Returns extracted tool calls and remaining text with JSON tool calls removed.
pub fn parse_json_tool_calls(text: &str) -> Result<(Vec<ToolUseData>, String)> {
    let mut tool_calls = Vec::new();
    let mut remaining_text = String::new();
    let mut last_end = 0;

    let tool_use_marker = "\"type\":\"tool_use\"";
    let mut search_pos = 0;

    while let Some(marker_pos) = text[search_pos..].find(tool_use_marker) {
        let abs_marker_pos = search_pos + marker_pos;

        // Prevents false extraction when tool call syntax appears in string parameter values
        if is_inside_json_string(text, abs_marker_pos) {
            search_pos = abs_marker_pos + tool_use_marker.len();
            continue;
        }

        let Some((json_start, json_end, parsed)) =
            find_outermost_json_containing(text, last_end, abs_marker_pos)
        else {
            search_pos = abs_marker_pos + tool_use_marker.len();
            continue;
        };

        let extracted = extract_tool_uses(&parsed);

        remaining_text.push_str(&text[last_end..json_start]);
        tool_calls.extend(extracted);
        last_end = json_end;
        search_pos = json_end;
    }

    remaining_text.push_str(&text[last_end..]);

    debug!(
        tool_count = tool_calls.len(),
        remaining_len = remaining_text.len(),
        "Parsed JSON tool calls"
    );

    Ok((tool_calls, remaining_text.trim().to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_single_standalone_tool_call() {
        let input = r#"{"type":"tool_use","id":"toolu_123","name":"test_tool","input":{"param1":"value1"}}"#;

        let (calls, remaining) = parse_json_tool_calls(input).unwrap();

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "toolu_123");
        assert_eq!(calls[0].name, "test_tool");
        assert_eq!(calls[0].arguments["param1"], "value1");
        assert!(remaining.is_empty());
    }

    #[test]
    fn test_tool_calls_in_content_array() {
        let input = r#"{"id":"msg_01","type":"message","role":"assistant","content":[{"type":"tool_use","id":"toolu_01K","name":"manage_task_list","input":{"title":"Test","tasks":[]}},{"type":"tool_use","id":"toolu_01L","name":"set_tracked_files","input":{"file_paths":[]}}],"model":"claude-opus-4-5-20251101"}"#;

        let (calls, remaining) = parse_json_tool_calls(input).unwrap();

        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].id, "toolu_01K");
        assert_eq!(calls[0].name, "manage_task_list");
        assert_eq!(calls[1].id, "toolu_01L");
        assert_eq!(calls[1].name, "set_tracked_files");
        assert!(remaining.is_empty());
    }

    #[test]
    fn test_tool_calls_mixed_with_text() {
        let input = r#"Here is some text before.
{"type":"tool_use","id":"toolu_abc","name":"my_tool","input":{"key":"value"}}
And some text after."#;

        let (calls, remaining) = parse_json_tool_calls(input).unwrap();

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "my_tool");
        assert!(remaining.contains("Here is some text before."));
        assert!(remaining.contains("And some text after."));
    }

    #[test]
    fn test_no_tool_calls() {
        let input = "Just regular text without any tool calls";
        let (calls, remaining) = parse_json_tool_calls(input).unwrap();

        assert!(calls.is_empty());
        assert_eq!(remaining, input);
    }

    #[test]
    fn test_invalid_json_gracefully_skipped() {
        let input = r#"{"type":"tool_use","id":"incomplete"#;
        let (calls, remaining) = parse_json_tool_calls(input).unwrap();

        assert!(calls.is_empty());
        assert_eq!(remaining, input);
    }

    #[test]
    fn test_missing_id_generates_uuid() {
        let input = r#"{"type":"tool_use","name":"no_id_tool","input":{"a":1}}"#;

        let (calls, _) = parse_json_tool_calls(input).unwrap();

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "no_id_tool");
        assert!(!calls[0].id.is_empty());
    }

    #[test]
    fn test_nested_json_in_input() {
        let input = r#"{"type":"tool_use","id":"toolu_nested","name":"complex_tool","input":{"nested":{"deep":{"value":42}},"array":[1,2,3]}}"#;

        let (calls, _) = parse_json_tool_calls(input).unwrap();

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["nested"]["deep"]["value"], 42);
        assert_eq!(calls[0].arguments["array"][1], 2);
    }

    #[test]
    fn test_multiple_separate_tool_calls() {
        let input = r#"First: {"type":"tool_use","id":"t1","name":"tool1","input":{}}
Second: {"type":"tool_use","id":"t2","name":"tool2","input":{}}"#;

        let (calls, remaining) = parse_json_tool_calls(input).unwrap();

        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "tool1");
        assert_eq!(calls[1].name, "tool2");
        assert!(remaining.contains("First:"));
        assert!(remaining.contains("Second:"));
    }

    #[test]
    fn test_string_with_escaped_quotes() {
        let input = r#"{"type":"tool_use","id":"t1","name":"test","input":{"message":"He said \"hello\""}}"#;

        let (calls, _) = parse_json_tool_calls(input).unwrap();

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].arguments["message"], "He said \"hello\"");
    }

    #[test]
    fn test_nested_tool_call_in_string_parameter() {
        // A write_file tool call that contains JSON resembling a tool call in its content string
        // The inner "tool call" should NOT be extracted - it's just string content
        let input = r#"{"type":"tool_use","id":"outer","name":"write_file","input":{"content":"{\"type\":\"tool_use\",\"id\":\"inner\",\"name\":\"should_not_extract\",\"input\":{}}"}}"#;

        let (calls, remaining) = parse_json_tool_calls(input).unwrap();

        // Should only extract the outer tool call
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].id, "outer");
        assert_eq!(calls[0].name, "write_file");
        // The inner JSON should remain as string content, not extracted
        let content = calls[0].arguments["content"].as_str().unwrap();
        assert!(content.contains("should_not_extract"));
        assert!(remaining.is_empty());
    }

    #[test]
    fn test_real_world_example() {
        let input = r#"{"id":"msg_01FE5LdhP7dTZCT5E9jFz6X9","type":"message","role":"assistant","content":[{"type":"tool_use","id":"toolu_01K2BLo5hGK86Q9NSkw4kbPv","name":"manage_task_list","input":{"title":"Tool test complete","tasks":[{"description":"Await user request","status":"completed"},{"description":"Understand/Explore the code base and propose a comprehensive plan","status":"completed"}]}},{"type":"tool_use","id":"toolu_01LxYAHu8HLb7MJtD5WC73Ur","name":"set_tracked_files","input":{"file_paths":[]}}],"model":"claude-opus-4-5-20251101","stop_reason":"tool_use","stop_sequence":null,"usage":{"input_tokens":4038,"cache_creation_input_tokens":0,"cache_read_input_tokens":0,"output_tokens":185,"thinking_tokens":252}}"#;

        let (calls, _) = parse_json_tool_calls(input).unwrap();

        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "manage_task_list");
        assert_eq!(calls[0].id, "toolu_01K2BLo5hGK86Q9NSkw4kbPv");
        assert_eq!(calls[1].name, "set_tracked_files");
        assert_eq!(calls[1].id, "toolu_01LxYAHu8HLb7MJtD5WC73Ur");
    }
}
