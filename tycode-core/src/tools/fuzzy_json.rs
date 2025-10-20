use anyhow::{Context, Result};
use serde_json::Value;

/// Attempts to coerce a JSON value to match the expected schema.
/// Handles common model mistakes like numeric values as strings,
/// JSON arrays encoded as strings, etc.
pub fn coerce_to_schema(value: &Value, schema: &Value) -> Result<Value> {
    let schema_type = schema
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("object");

    match schema_type {
        "object" => coerce_object(value, schema),
        "array" => coerce_array(value, schema),
        "string" => coerce_string(value),
        "integer" | "number" => coerce_number(value, schema_type),
        "boolean" => coerce_boolean(value),
        _ => Ok(value.clone()),
    }
}

fn coerce_object(value: &Value, schema: &Value) -> Result<Value> {
    match value {
        Value::Object(map) => {
            let properties = schema.get("properties");
            if properties.is_none() {
                return Ok(value.clone());
            }

            let properties = properties.unwrap().as_object();
            if properties.is_none() {
                return Ok(value.clone());
            }

            let properties = properties.unwrap();
            let mut coerced = serde_json::Map::new();

            for (key, val) in map {
                if let Some(prop_schema) = properties.get(key) {
                    let coerced_val = coerce_to_schema(val, prop_schema)
                        .with_context(|| format!("Failed to coerce property '{key}'"))?;
                    coerced.insert(key.clone(), coerced_val);
                } else {
                    coerced.insert(key.clone(), val.clone());
                }
            }

            Ok(Value::Object(coerced))
        }
        _ => Ok(value.clone()),
    }
}

fn coerce_array(value: &Value, schema: &Value) -> Result<Value> {
    match value {
        Value::Array(arr) => {
            if let Some(items_schema) = schema.get("items") {
                let coerced: Result<Vec<Value>> = arr
                    .iter()
                    .map(|item| coerce_to_schema(item, items_schema))
                    .collect();
                Ok(Value::Array(coerced?))
            } else {
                Ok(value.clone())
            }
        }
        Value::String(s) => match serde_json::from_str::<Value>(s) {
            Ok(Value::Array(arr)) => {
                if let Some(items_schema) = schema.get("items") {
                    let coerced: Result<Vec<Value>> = arr
                        .iter()
                        .map(|item| coerce_to_schema(item, items_schema))
                        .collect();
                    Ok(Value::Array(coerced?))
                } else {
                    Ok(Value::Array(arr))
                }
            }
            Ok(other) => {
                if let Some(items_schema) = schema.get("items") {
                    coerce_to_schema(&other, items_schema)
                } else {
                    Ok(other)
                }
            }
            Err(_) => Ok(value.clone()),
        },
        _ => Ok(value.clone()),
    }
}

fn coerce_string(value: &Value) -> Result<Value> {
    match value {
        Value::String(_) => Ok(value.clone()),
        Value::Number(n) => Ok(Value::String(n.to_string())),
        Value::Bool(b) => Ok(Value::String(b.to_string())),
        _ => Ok(value.clone()),
    }
}

fn coerce_number(value: &Value, schema_type: &str) -> Result<Value> {
    match value {
        Value::Number(_) => Ok(value.clone()),
        Value::String(s) => {
            let trimmed = s.trim();
            if schema_type == "integer" {
                if let Ok(n) = trimmed.parse::<i64>() {
                    return Ok(Value::Number(n.into()));
                }
            }
            if let Ok(n) = trimmed.parse::<f64>() {
                if let Some(num) = serde_json::Number::from_f64(n) {
                    return Ok(Value::Number(num));
                }
            }
            Ok(value.clone())
        }
        _ => Ok(value.clone()),
    }
}

fn coerce_boolean(value: &Value) -> Result<Value> {
    match value {
        Value::Bool(_) => Ok(value.clone()),
        Value::String(s) => {
            let trimmed = s.trim().to_lowercase();
            match trimmed.as_str() {
                "true" | "1" | "yes" => Ok(Value::Bool(true)),
                "false" | "0" | "no" => Ok(Value::Bool(false)),
                _ => Ok(value.clone()),
            }
        }
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(Value::Bool(i != 0))
            } else {
                Ok(value.clone())
            }
        }
        _ => Ok(value.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_coerce_string_to_integer() {
        let value = json!("120");
        let schema = json!({"type": "integer"});
        let result = coerce_to_schema(&value, &schema).unwrap();
        assert_eq!(result, json!(120));
    }

    #[test]
    fn test_coerce_string_to_number() {
        let value = json!("3.14");
        let schema = json!({"type": "number"});
        let result = coerce_to_schema(&value, &schema).unwrap();
        assert_eq!(result, json!(3.14));
    }

    #[test]
    fn test_coerce_string_to_boolean() {
        let value = json!("true");
        let schema = json!({"type": "boolean"});
        let result = coerce_to_schema(&value, &schema).unwrap();
        assert_eq!(result, json!(true));
    }

    #[test]
    fn test_coerce_object_with_numeric_string() {
        let value = json!({
            "timeout_seconds": "120",
            "command": "cargo build"
        });
        let schema = json!({
            "type": "object",
            "properties": {
                "timeout_seconds": {"type": "integer"},
                "command": {"type": "string"}
            }
        });
        let result = coerce_to_schema(&value, &schema).unwrap();
        assert_eq!(
            result,
            json!({
                "timeout_seconds": 120,
                "command": "cargo build"
            })
        );
    }

    #[test]
    fn test_coerce_stringified_array() {
        let value = json!("[1, 2, 3]");
        let schema = json!({
            "type": "array",
            "items": {"type": "integer"}
        });
        let result = coerce_to_schema(&value, &schema).unwrap();
        assert_eq!(result, json!([1, 2, 3]));
    }

    #[test]
    fn test_preserve_valid_values() {
        let value = json!({"count": 42, "name": "test"});
        let schema = json!({
            "type": "object",
            "properties": {
                "count": {"type": "integer"},
                "name": {"type": "string"}
            }
        });
        let result = coerce_to_schema(&value, &schema).unwrap();
        assert_eq!(result, value);
    }

    #[test]
    fn test_malformed_stringified_array_returns_original() {
        let malformed_json = json!("[{\"op\": \"replace\", \"path\": \"/foo\"}");
        let schema = json!({
            "type": "array",
            "items": {"type": "object"}
        });
        let result = coerce_to_schema(&malformed_json, &schema).unwrap();
        assert_eq!(result, malformed_json);
    }
}
