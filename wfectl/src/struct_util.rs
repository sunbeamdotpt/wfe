//! Conversions between `serde_json::Value` and `prost_types::Struct`.

use prost_types::value::Kind;
use prost_types::{ListValue, Struct, Value};

/// Convert a `serde_json::Value` to a `prost_types::Value`.
pub fn json_to_prost(json: &serde_json::Value) -> Value {
    let kind = match json {
        serde_json::Value::Null => Kind::NullValue(0),
        serde_json::Value::Bool(b) => Kind::BoolValue(*b),
        serde_json::Value::Number(n) => Kind::NumberValue(n.as_f64().unwrap_or(0.0)),
        serde_json::Value::String(s) => Kind::StringValue(s.clone()),
        serde_json::Value::Array(arr) => Kind::ListValue(ListValue {
            values: arr.iter().map(json_to_prost).collect(),
        }),
        serde_json::Value::Object(obj) => Kind::StructValue(Struct {
            fields: obj
                .iter()
                .map(|(k, v)| (k.clone(), json_to_prost(v)))
                .collect(),
        }),
    };
    Value { kind: Some(kind) }
}

/// Convert a top-level JSON object into a `prost_types::Struct`.
pub fn json_object_to_struct(json: &serde_json::Value) -> Struct {
    match json {
        serde_json::Value::Object(map) => Struct {
            fields: map
                .iter()
                .map(|(k, v)| (k.clone(), json_to_prost(v)))
                .collect(),
        },
        _ => Struct::default(),
    }
}

/// Convert a `prost_types::Value` back to `serde_json::Value`.
pub fn prost_to_json(value: &Value) -> serde_json::Value {
    match &value.kind {
        Some(Kind::NullValue(_)) | None => serde_json::Value::Null,
        Some(Kind::BoolValue(b)) => serde_json::Value::Bool(*b),
        Some(Kind::NumberValue(n)) => serde_json::Number::from_f64(*n)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Some(Kind::StringValue(s)) => serde_json::Value::String(s.clone()),
        Some(Kind::ListValue(list)) => {
            serde_json::Value::Array(list.values.iter().map(prost_to_json).collect())
        }
        Some(Kind::StructValue(s)) => prost_struct_to_json(s),
    }
}

/// Convert a `prost_types::Struct` to a `serde_json::Value::Object`.
pub fn prost_struct_to_json(s: &Struct) -> serde_json::Value {
    serde_json::Value::Object(
        s.fields
            .iter()
            .map(|(k, v)| (k.clone(), prost_to_json(v)))
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn round_trip_object() {
        let original = json!({
            "name": "test",
            "count": 42,
            "active": true,
            "tags": ["a", "b"],
            "nested": {"key": "value"}
        });
        let s = json_object_to_struct(&original);
        let back = prost_struct_to_json(&s);
        assert_eq!(back["name"], "test");
        assert_eq!(back["count"], 42.0); // numbers become f64
        assert_eq!(back["active"], true);
        assert_eq!(back["tags"][0], "a");
        assert_eq!(back["nested"]["key"], "value");
    }

    #[test]
    fn json_to_prost_null() {
        let v = json_to_prost(&serde_json::Value::Null);
        assert!(matches!(v.kind, Some(Kind::NullValue(_))));
    }

    #[test]
    fn json_object_to_struct_non_object_returns_empty() {
        let s = json_object_to_struct(&json!("not an object"));
        assert!(s.fields.is_empty());
    }
}
