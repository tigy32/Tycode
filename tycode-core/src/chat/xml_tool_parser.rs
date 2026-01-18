use crate::ai::ToolUseData;
use anyhow::{bail, Result};
use serde_json::Value;
use tracing::debug;
use uuid::Uuid;

fn find_opening_tag(text: &str, base_name: &str) -> Option<(usize, usize)> {
    let mut pos = 0;
    while pos < text.len() {
        let lt_pos = text[pos..].find('<')?;
        let abs_lt = pos + lt_pos;

        let gt_pos = text[abs_lt..].find('>')?;
        let tag_content = &text[abs_lt + 1..abs_lt + gt_pos];

        // Check if tag matches base_name or prefix:base_name
        let tag_name = tag_content.split_whitespace().next().unwrap_or("");
        if tag_name == base_name || tag_name.ends_with(&format!(":{}", base_name)) {
            return Some((abs_lt, abs_lt + gt_pos + 1));
        }

        pos = abs_lt + 1;
    }
    None
}

fn find_closing_tag(text: &str, base_name: &str) -> Option<(usize, usize)> {
    find_closing_tag_with_nesting(text, base_name, 1)
}

fn find_closing_tag_with_nesting(
    text: &str,
    base_name: &str,
    initial_depth: usize,
) -> Option<(usize, usize)> {
    let mut depth = initial_depth;
    let mut pos = 0;

    while pos < text.len() {
        let open_pos = find_opening_tag(&text[pos..], base_name);
        let close_pos = find_first_closing_tag(&text[pos..], base_name);

        if let (Some((o_start, _)), Some((c_start, _))) = (open_pos, close_pos) {
            if o_start < c_start {
                depth += 1;
                pos += o_start + 1;
                continue;
            }
        }

        if let Some((c_start, c_end)) = close_pos {
            depth -= 1;
            if depth == 0 {
                return Some((pos + c_start, pos + c_end));
            }
            pos += c_end;
            continue;
        }

        if let Some((o_start, _)) = open_pos {
            depth += 1;
            pos += o_start + 1;
            continue;
        }

        return None;
    }
    None
}

fn find_first_closing_tag(text: &str, base_name: &str) -> Option<(usize, usize)> {
    let mut pos = 0;
    while pos < text.len() {
        let lt_pos = text[pos..].find("</")?;
        let abs_lt = pos + lt_pos;

        let gt_pos = text[abs_lt..].find('>')?;
        let tag_name = text[abs_lt + 2..abs_lt + gt_pos].trim();

        if tag_name == base_name || tag_name.ends_with(&format!(":{}", base_name)) {
            return Some((abs_lt, abs_lt + gt_pos + 1));
        }

        pos = abs_lt + 2;
    }
    None
}

fn find_named_opening_tag<'a>(text: &'a str, base_name: &str) -> Option<(usize, usize, &'a str)> {
    let mut pos = 0;
    while pos < text.len() {
        let lt_pos = text[pos..].find('<')?;
        let abs_lt = pos + lt_pos;

        let gt_pos = text[abs_lt..].find('>')?;
        let tag_content = &text[abs_lt + 1..abs_lt + gt_pos];

        // Extract tag name (first word)
        let tag_name = tag_content.split_whitespace().next().unwrap_or("");
        if tag_name == base_name || tag_name.ends_with(&format!(":{}", base_name)) {
            // Extract name attribute value
            if let Some(name_start) = tag_content.find("name=\"") {
                let value_start = name_start + 6;
                if let Some(value_end) = tag_content[value_start..].find('"') {
                    let name = &tag_content[value_start..value_start + value_end];
                    return Some((abs_lt, abs_lt + gt_pos + 1, name));
                }
            }
        }

        pos = abs_lt + 1;
    }
    None
}

/// Permissive matching allows any XML prefix to handle variation in model outputs.
pub fn parse_xml_tool_calls(text: &str) -> Result<(Vec<ToolUseData>, String)> {
    let mut tool_calls = Vec::new();
    let mut remaining_text = String::new();
    let mut last_end = 0;

    let mut search_start = 0;
    while let Some((open_start, open_end)) =
        find_opening_tag(&text[search_start..], "function_calls")
    {
        let abs_open_start = search_start + open_start;
        let abs_open_end = search_start + open_end;

        let Some((close_start, close_end)) =
            find_closing_tag(&text[abs_open_end..], "function_calls")
        else {
            bail!("Unclosed function_calls tag at position {}", abs_open_start);
        };
        let abs_close_start = abs_open_end + close_start;
        let abs_close_end = abs_open_end + close_end;

        remaining_text.push_str(&text[last_end..abs_open_start]);

        let block_content = &text[abs_open_end..abs_close_start];
        let parsed = parse_invoke_blocks(block_content)?;
        tool_calls.extend(parsed);

        last_end = abs_close_end;
        search_start = abs_close_end;
    }

    remaining_text.push_str(&text[last_end..]);

    debug!(
        tool_count = tool_calls.len(),
        remaining_len = remaining_text.len(),
        "Parsed XML tool calls"
    );

    Ok((tool_calls, remaining_text.trim().to_string()))
}

fn parse_invoke_blocks(content: &str) -> Result<Vec<ToolUseData>> {
    let mut tool_calls = Vec::new();

    let mut search_start = 0;
    while let Some((_open_start, open_end, name)) =
        find_named_opening_tag(&content[search_start..], "invoke")
    {
        let abs_open_end = search_start + open_end;

        let Some((close_start, close_end)) = find_closing_tag(&content[abs_open_end..], "invoke")
        else {
            bail!("Unclosed invoke tag for tool '{}'", name);
        };
        let abs_close_start = abs_open_end + close_start;
        let abs_close_end = abs_open_end + close_end;

        let invoke_content = &content[abs_open_end..abs_close_start];
        let parameters = parse_parameters(invoke_content)?;

        tool_calls.push(ToolUseData {
            id: Uuid::new_v4().to_string(),
            name: name.to_string(),
            arguments: parameters,
        });

        search_start = abs_close_end;
    }

    Ok(tool_calls)
}

fn parse_parameters(content: &str) -> Result<Value> {
    let mut params = serde_json::Map::new();

    let mut search_start = 0;
    while let Some((_open_start, open_end, name)) =
        find_named_opening_tag(&content[search_start..], "parameter")
    {
        let abs_open_end = search_start + open_end;

        let Some((close_start, close_end)) =
            find_closing_tag(&content[abs_open_end..], "parameter")
        else {
            bail!("Unclosed parameter tag for '{}'", name);
        };
        let abs_close_start = abs_open_end + close_start;
        let abs_close_end = abs_open_end + close_end;

        let value_str = &content[abs_open_end..abs_close_start];

        // Specification requires arrays/objects as JSON, scalars as strings
        let value = match serde_json::from_str(value_str) {
            Ok(v) => v,
            Err(_) => Value::String(value_str.to_string()),
        };

        params.insert(name.to_string(), value);
        search_start = abs_close_end;
    }

    Ok(Value::Object(params))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_single_tool_call() {
        let input = r#"Some text before
<function_calls>
<invoke name="test_tool">
<parameter name="param1">value1</parameter>
<parameter name="param2">42</parameter>
</invoke>
</function_calls>
Some text after"#;

        let (calls, remaining) = parse_xml_tool_calls(input).unwrap();

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "test_tool");
        assert_eq!(calls[0].arguments["param1"], "value1");
        assert_eq!(calls[0].arguments["param2"], 42);
        assert!(remaining.contains("Some text before"));
        assert!(remaining.contains("Some text after"));
    }

    #[test]
    fn test_parse_multiple_tool_calls() {
        let input = r#"<function_calls>
<invoke name="tool1">
<parameter name="a">1</parameter>
</invoke>
<invoke name="tool2">
<parameter name="b">2</parameter>
</invoke>
</function_calls>"#;

        let (calls, _) = parse_xml_tool_calls(input).unwrap();

        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "tool1");
        assert_eq!(calls[1].name, "tool2");
    }

    #[test]
    fn test_parse_json_parameter() {
        let input = r#"<function_calls>
<invoke name="test">
<parameter name="arr">["a", "b", "c"]</parameter>
<parameter name="obj">{"key": "value"}</parameter>
</invoke>
</function_calls>"#;

        let (calls, _) = parse_xml_tool_calls(input).unwrap();

        assert_eq!(calls.len(), 1);
        assert!(calls[0].arguments["arr"].is_array());
        assert!(calls[0].arguments["obj"].is_object());
    }

    #[test]
    fn test_no_tool_calls() {
        let input = "Just regular text without any tool calls";
        let (calls, remaining) = parse_xml_tool_calls(input).unwrap();

        assert!(calls.is_empty());
        assert_eq!(remaining, input);
    }

    #[test]
    fn test_parse_with_xml_prefix() {
        // Parser accepts any XML prefix on tags
        let prefix = "antml";
        let input = format!(
            "<{}:function_calls>\n<{}:invoke name=\"prefixed_tool\">\n<{}:parameter name=\"key\">value</{}:parameter>\n</{}:invoke>\n</{}:function_calls>",
            prefix, prefix, prefix, prefix, prefix, prefix
        );

        let (calls, _) = parse_xml_tool_calls(&input).unwrap();

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "prefixed_tool");
        assert_eq!(calls[0].arguments["key"], "value");
    }

    #[test]
    fn test_parse_with_mixed_prefixes() {
        // Parser accepts different prefixes on different tags
        let input = "<abc:function_calls>\n<xyz:invoke name=\"mixed\">\n<foo:parameter name=\"p\">val</bar:parameter>\n</qux:invoke>\n</def:function_calls>";

        let (calls, _) = parse_xml_tool_calls(input).unwrap();

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "mixed");
        assert_eq!(calls[0].arguments["p"], "val");
    }

    #[test]
    fn test_nested_tool_call_in_parameter() {
        // Scenario: AI writes a file containing an XML tool call example
        let lt = '<';
        let inner = format!(
            "{}function_calls>\n{}invoke name=\"nested_example\">\n{}parameter name=\"k\">v{}/parameter>\n{}/invoke>\n{}/function_calls>",
            lt, lt, lt, lt, lt, lt
        );
        let input = format!(
            "{}function_calls>\n{}invoke name=\"write_file\">\n{}parameter name=\"path\">x.md{}/parameter>\n{}parameter name=\"content\">{}{}/parameter>\n{}/invoke>\n{}/function_calls>",
            lt, lt, lt, lt, lt, inner, lt, lt, lt
        );

        let (calls, remaining) = parse_xml_tool_calls(&input).unwrap();

        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].name, "write_file");
        assert_eq!(calls[0].arguments["path"], "x.md");

        let content = calls[0].arguments.get("content");
        assert!(content.is_some());
        assert!(remaining.is_empty());
    }
}
