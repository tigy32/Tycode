use aws_smithy_types::Document;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

/// This is a massive hack trying to work around a bug in bedrock prompt cache.
/// When smithy-rs is convering a hashmap to json string its using hashmap
/// iteration order, which thanks to rust's DDOS resistent hashing scheme is
/// effectively random. This causes bedrock to not use the cache since it
/// detects contents have changed.
///
/// The rust SDK needs to own the hashmap (which is probably another bug) so I
/// can't cache values. So this is the best I got 🤷‍♂️ Perhaps if we retry until
/// we happen to get sorted order (and hopefully iteration order is stable until
/// mutation) we will get better prompt caching.
///
/// In practice this seems to work. The largest tools have 3 parameters so on
/// average we'll take 6 attempts to get sorted order.
fn create_sorted_hashmap(sorted: BTreeMap<String, Document>) -> HashMap<String, Document> {
    const MAX_RETRIES: usize = 100;

    for _ in 0..MAX_RETRIES {
        let mut map = HashMap::new();
        for (k, v) in &sorted {
            map.insert(k.clone(), v.clone());
        }

        let keys: Vec<_> = map.keys().collect();
        let sorted_keys: Vec<_> = sorted.keys().collect();

        if keys == sorted_keys {
            return map;
        }
    }

    let mut map = HashMap::new();
    for (k, v) in sorted {
        map.insert(k, v);
    }
    map
}

/// Convert a serde_json::Value to an AWS Document
pub fn to_doc(value: Value) -> Document {
    match value {
        Value::Null => Document::Null,
        Value::Bool(b) => Document::Bool(b),
        Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                Document::Number(aws_smithy_types::Number::PosInt(u))
            } else if let Some(i) = n.as_i64() {
                Document::Number(aws_smithy_types::Number::NegInt(i))
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
            // We retry HashMap creation until iteration order matches sorted order (works for small maps).
            let sorted: BTreeMap<String, Document> =
                obj.into_iter().map(|(k, v)| (k, to_doc(v))).collect();
            Document::Object(create_sorted_hashmap(sorted))
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
