use aws_smithy_types::Document;
use serde_json::Value;
use std::collections::BTreeMap;

/// Convert a serde_json::Value to an AWS Document
pub fn to_doc(value: Value) -> Document {
    match value {
        Value::Null => Document::Null,
        Value::Bool(b) => Document::Bool(b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Document::Number(aws_smithy_types::Number::NegInt(i))
            } else if let Some(u) = n.as_u64() {
                Document::Number(aws_smithy_types::Number::PosInt(u))
            } else if let Some(f) = n.as_f64() {
                Document::Number(aws_smithy_types::Number::Float(f))
            } else {
                Document::Null
            }
        }
        Value::String(s) => Document::String(s),
        Value::Array(arr) => Document::Array(arr.into_iter().map(to_doc).collect()),
        Value::Object(obj) => {
            // Sort keys alphabetically to ensure deterministic serialization for Bedrock prompt caching.
            // Cache keys depend on exact byte-for-byte request equality.
            let sorted: BTreeMap<String, Document> =
                obj.into_iter().map(|(k, v)| (k, to_doc(v))).collect();
            Document::Object(sorted.into_iter().collect())
        }
    }
}

/// Convert an AWS Document to a serde_json::Value
pub fn from_doc(doc: Document) -> Value {
    match doc {
        Document::Null => Value::Null,
        Document::Bool(b) => Value::Bool(b),
        Document::Number(n) => match n {
            aws_smithy_types::Number::PosInt(u) => Value::Number(u.into()),
            aws_smithy_types::Number::NegInt(i) => Value::Number(i.into()),
            aws_smithy_types::Number::Float(f) => serde_json::Number::from_f64(f)
                .map(Value::Number)
                .unwrap_or(Value::Null),
        },
        Document::String(s) => Value::String(s),
        Document::Array(arr) => Value::Array(arr.into_iter().map(from_doc).collect()),
        Document::Object(obj) => {
            Value::Object(obj.into_iter().map(|(k, v)| (k, from_doc(v))).collect())
        }
    }
}
