use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Describes a single type in the workflow schema type system.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SchemaType {
    String,
    Number,
    Integer,
    Bool,
    Optional(Box<SchemaType>),
    List(Box<SchemaType>),
    Map(Box<SchemaType>),
    Any,
}

/// Defines expected input and output schemas for a workflow.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkflowSchema {
    #[serde(default)]
    pub inputs: HashMap<String, SchemaType>,
    #[serde(default)]
    pub outputs: HashMap<String, SchemaType>,
}

/// Parse a type string into a [`SchemaType`].
///
/// Supported formats:
/// - `"string"`, `"number"`, `"integer"`, `"bool"`, `"any"`
/// - `"string?"` (optional)
/// - `"list<string>"`, `"map<number>"` (generic containers)
/// - Nested: `"list<list<string>>"`
pub fn parse_type(s: &str) -> crate::Result<SchemaType> {
    let s = s.trim();

    // Handle optional suffix.
    if let Some(inner) = s.strip_suffix('?') {
        let inner_type = parse_type(inner)?;
        return Ok(SchemaType::Optional(Box::new(inner_type)));
    }

    // Handle generic containers: list<...> and map<...>.
    if let Some(rest) = s.strip_prefix("list<") {
        let inner = rest
            .strip_suffix('>')
            .ok_or_else(|| crate::WfeError::StepExecution(format!("Invalid type syntax: {s}")))?;
        let inner_type = parse_type(inner)?;
        return Ok(SchemaType::List(Box::new(inner_type)));
    }
    if let Some(rest) = s.strip_prefix("map<") {
        let inner = rest
            .strip_suffix('>')
            .ok_or_else(|| crate::WfeError::StepExecution(format!("Invalid type syntax: {s}")))?;
        let inner_type = parse_type(inner)?;
        return Ok(SchemaType::Map(Box::new(inner_type)));
    }

    // Primitive types.
    match s {
        "string" => Ok(SchemaType::String),
        "number" => Ok(SchemaType::Number),
        "integer" => Ok(SchemaType::Integer),
        "bool" => Ok(SchemaType::Bool),
        "any" => Ok(SchemaType::Any),
        _ => Err(crate::WfeError::StepExecution(format!("Unknown type: {s}"))),
    }
}

/// Validate that a JSON value matches the expected [`SchemaType`].
pub fn validate_value(value: &serde_json::Value, expected: &SchemaType) -> Result<(), String> {
    match expected {
        SchemaType::String => {
            if value.is_string() {
                Ok(())
            } else {
                Err(format!("expected string, got {}", value_type_name(value)))
            }
        }
        SchemaType::Number => {
            if value.is_number() {
                Ok(())
            } else {
                Err(format!("expected number, got {}", value_type_name(value)))
            }
        }
        SchemaType::Integer => {
            if value.is_i64() || value.is_u64() {
                Ok(())
            } else {
                Err(format!("expected integer, got {}", value_type_name(value)))
            }
        }
        SchemaType::Bool => {
            if value.is_boolean() {
                Ok(())
            } else {
                Err(format!("expected bool, got {}", value_type_name(value)))
            }
        }
        SchemaType::Optional(inner) => {
            if value.is_null() {
                Ok(())
            } else {
                validate_value(value, inner)
            }
        }
        SchemaType::List(inner) => {
            if let Some(arr) = value.as_array() {
                for (i, item) in arr.iter().enumerate() {
                    validate_value(item, inner).map_err(|e| format!("list element [{i}]: {e}"))?;
                }
                Ok(())
            } else {
                Err(format!("expected list, got {}", value_type_name(value)))
            }
        }
        SchemaType::Map(inner) => {
            if let Some(obj) = value.as_object() {
                for (key, val) in obj {
                    validate_value(val, inner).map_err(|e| format!("map key \"{key}\": {e}"))?;
                }
                Ok(())
            } else {
                Err(format!("expected map, got {}", value_type_name(value)))
            }
        }
        SchemaType::Any => Ok(()),
    }
}

fn value_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "bool",
        serde_json::Value::Number(_) => "number",
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

impl WorkflowSchema {
    /// Validate that the given data satisfies all input field requirements.
    pub fn validate_inputs(&self, data: &serde_json::Value) -> Result<(), Vec<String>> {
        self.validate_fields(&self.inputs, data)
    }

    /// Validate that the given data satisfies all output field requirements.
    pub fn validate_outputs(&self, data: &serde_json::Value) -> Result<(), Vec<String>> {
        self.validate_fields(&self.outputs, data)
    }

    fn validate_fields(
        &self,
        fields: &HashMap<String, SchemaType>,
        data: &serde_json::Value,
    ) -> Result<(), Vec<String>> {
        let obj = match data.as_object() {
            Some(o) => o,
            None => {
                return Err(vec!["expected an object".to_string()]);
            }
        };

        let mut errors = Vec::new();

        for (name, schema_type) in fields {
            match obj.get(name) {
                Some(value) => {
                    if let Err(e) = validate_value(value, schema_type) {
                        errors.push(format!("field \"{name}\": {e}"));
                    }
                }
                None => {
                    // Missing field is OK for optional types (null is acceptable).
                    if !matches!(schema_type, SchemaType::Optional(_)) {
                        errors.push(format!("missing required field: \"{name}\""));
                    }
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -- parse_type tests --

    #[test]
    fn parse_type_string() {
        assert_eq!(parse_type("string").unwrap(), SchemaType::String);
    }

    #[test]
    fn parse_type_number() {
        assert_eq!(parse_type("number").unwrap(), SchemaType::Number);
    }

    #[test]
    fn parse_type_integer() {
        assert_eq!(parse_type("integer").unwrap(), SchemaType::Integer);
    }

    #[test]
    fn parse_type_bool() {
        assert_eq!(parse_type("bool").unwrap(), SchemaType::Bool);
    }

    #[test]
    fn parse_type_any() {
        assert_eq!(parse_type("any").unwrap(), SchemaType::Any);
    }

    #[test]
    fn parse_type_optional_string() {
        assert_eq!(
            parse_type("string?").unwrap(),
            SchemaType::Optional(Box::new(SchemaType::String))
        );
    }

    #[test]
    fn parse_type_optional_number() {
        assert_eq!(
            parse_type("number?").unwrap(),
            SchemaType::Optional(Box::new(SchemaType::Number))
        );
    }

    #[test]
    fn parse_type_list_string() {
        assert_eq!(
            parse_type("list<string>").unwrap(),
            SchemaType::List(Box::new(SchemaType::String))
        );
    }

    #[test]
    fn parse_type_list_number() {
        assert_eq!(
            parse_type("list<number>").unwrap(),
            SchemaType::List(Box::new(SchemaType::Number))
        );
    }

    #[test]
    fn parse_type_map_string() {
        assert_eq!(
            parse_type("map<string>").unwrap(),
            SchemaType::Map(Box::new(SchemaType::String))
        );
    }

    #[test]
    fn parse_type_map_number() {
        assert_eq!(
            parse_type("map<number>").unwrap(),
            SchemaType::Map(Box::new(SchemaType::Number))
        );
    }

    #[test]
    fn parse_type_nested_list() {
        assert_eq!(
            parse_type("list<list<string>>").unwrap(),
            SchemaType::List(Box::new(SchemaType::List(Box::new(SchemaType::String))))
        );
    }

    #[test]
    fn parse_type_unknown_errors() {
        assert!(parse_type("foobar").is_err());
    }

    #[test]
    fn parse_type_trims_whitespace() {
        assert_eq!(parse_type("  string  ").unwrap(), SchemaType::String);
    }

    // -- validate_value tests --

    #[test]
    fn validate_string_match() {
        assert!(validate_value(&json!("hello"), &SchemaType::String).is_ok());
    }

    #[test]
    fn validate_string_mismatch() {
        assert!(validate_value(&json!(42), &SchemaType::String).is_err());
    }

    #[test]
    fn validate_number_match() {
        assert!(validate_value(&json!(2.78), &SchemaType::Number).is_ok());
    }

    #[test]
    fn validate_number_mismatch() {
        assert!(validate_value(&json!("not a number"), &SchemaType::Number).is_err());
    }

    #[test]
    fn validate_integer_match() {
        assert!(validate_value(&json!(42), &SchemaType::Integer).is_ok());
    }

    #[test]
    fn validate_integer_mismatch_float() {
        assert!(validate_value(&json!(2.78), &SchemaType::Integer).is_err());
    }

    #[test]
    fn validate_bool_match() {
        assert!(validate_value(&json!(true), &SchemaType::Bool).is_ok());
    }

    #[test]
    fn validate_bool_mismatch() {
        assert!(validate_value(&json!(1), &SchemaType::Bool).is_err());
    }

    #[test]
    fn validate_optional_null_passes() {
        let ty = SchemaType::Optional(Box::new(SchemaType::String));
        assert!(validate_value(&json!(null), &ty).is_ok());
    }

    #[test]
    fn validate_optional_correct_inner_passes() {
        let ty = SchemaType::Optional(Box::new(SchemaType::String));
        assert!(validate_value(&json!("hello"), &ty).is_ok());
    }

    #[test]
    fn validate_optional_wrong_inner_fails() {
        let ty = SchemaType::Optional(Box::new(SchemaType::String));
        assert!(validate_value(&json!(42), &ty).is_err());
    }

    #[test]
    fn validate_list_match() {
        let ty = SchemaType::List(Box::new(SchemaType::Number));
        assert!(validate_value(&json!([1, 2, 3]), &ty).is_ok());
    }

    #[test]
    fn validate_list_mismatch_element() {
        let ty = SchemaType::List(Box::new(SchemaType::Number));
        assert!(validate_value(&json!([1, "two", 3]), &ty).is_err());
    }

    #[test]
    fn validate_list_not_array() {
        let ty = SchemaType::List(Box::new(SchemaType::Number));
        assert!(validate_value(&json!("not a list"), &ty).is_err());
    }

    #[test]
    fn validate_map_match() {
        let ty = SchemaType::Map(Box::new(SchemaType::Number));
        assert!(validate_value(&json!({"a": 1, "b": 2}), &ty).is_ok());
    }

    #[test]
    fn validate_map_mismatch_value() {
        let ty = SchemaType::Map(Box::new(SchemaType::Number));
        assert!(validate_value(&json!({"a": 1, "b": "two"}), &ty).is_err());
    }

    #[test]
    fn validate_map_not_object() {
        let ty = SchemaType::Map(Box::new(SchemaType::Number));
        assert!(validate_value(&json!([1, 2]), &ty).is_err());
    }

    #[test]
    fn validate_any_always_passes() {
        assert!(validate_value(&json!(null), &SchemaType::Any).is_ok());
        assert!(validate_value(&json!("str"), &SchemaType::Any).is_ok());
        assert!(validate_value(&json!(42), &SchemaType::Any).is_ok());
        assert!(validate_value(&json!([1, 2]), &SchemaType::Any).is_ok());
    }

    // -- WorkflowSchema validate_inputs / validate_outputs tests --

    #[test]
    fn validate_inputs_all_present() {
        let schema = WorkflowSchema {
            inputs: HashMap::from([
                ("name".into(), SchemaType::String),
                ("age".into(), SchemaType::Integer),
            ]),
            outputs: HashMap::new(),
        };
        let data = json!({"name": "Alice", "age": 30});
        assert!(schema.validate_inputs(&data).is_ok());
    }

    #[test]
    fn validate_inputs_missing_required_field() {
        let schema = WorkflowSchema {
            inputs: HashMap::from([
                ("name".into(), SchemaType::String),
                ("age".into(), SchemaType::Integer),
            ]),
            outputs: HashMap::new(),
        };
        let data = json!({"name": "Alice"});
        let errs = schema.validate_inputs(&data).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("age")));
    }

    #[test]
    fn validate_inputs_wrong_type() {
        let schema = WorkflowSchema {
            inputs: HashMap::from([("count".into(), SchemaType::Integer)]),
            outputs: HashMap::new(),
        };
        let data = json!({"count": "not-a-number"});
        let errs = schema.validate_inputs(&data).unwrap_err();
        assert!(!errs.is_empty());
    }

    #[test]
    fn validate_outputs_missing_field() {
        let schema = WorkflowSchema {
            inputs: HashMap::new(),
            outputs: HashMap::from([("result".into(), SchemaType::String)]),
        };
        let data = json!({});
        let errs = schema.validate_outputs(&data).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("result")));
    }

    #[test]
    fn validate_inputs_optional_field_missing_is_ok() {
        let schema = WorkflowSchema {
            inputs: HashMap::from([(
                "nickname".into(),
                SchemaType::Optional(Box::new(SchemaType::String)),
            )]),
            outputs: HashMap::new(),
        };
        let data = json!({});
        assert!(schema.validate_inputs(&data).is_ok());
    }

    #[test]
    fn validate_not_object_errors() {
        let schema = WorkflowSchema {
            inputs: HashMap::from([("x".into(), SchemaType::String)]),
            outputs: HashMap::new(),
        };
        let errs = schema.validate_inputs(&json!("not an object")).unwrap_err();
        assert!(errs[0].contains("expected an object"));
    }

    #[test]
    fn schema_serde_round_trip() {
        let schema = WorkflowSchema {
            inputs: HashMap::from([("name".into(), SchemaType::String)]),
            outputs: HashMap::from([("result".into(), SchemaType::Bool)]),
        };
        let json_str = serde_json::to_string(&schema).unwrap();
        let deserialized: WorkflowSchema = serde_json::from_str(&json_str).unwrap();
        assert_eq!(deserialized.inputs["name"], SchemaType::String);
        assert_eq!(deserialized.outputs["result"], SchemaType::Bool);
    }
}
