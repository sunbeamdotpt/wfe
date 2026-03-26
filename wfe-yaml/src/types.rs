/// Parsed type representation for workflow input/output schemas.
///
/// This mirrors what wfe-core's `SchemaType` will provide, but is self-contained
/// so wfe-yaml can parse type strings without depending on wfe-core's schema module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaType {
    String,
    Number,
    Integer,
    Bool,
    Any,
    Optional(Box<SchemaType>),
    List(Box<SchemaType>),
    Map(Box<SchemaType>),
}

impl std::fmt::Display for SchemaType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SchemaType::String => write!(f, "string"),
            SchemaType::Number => write!(f, "number"),
            SchemaType::Integer => write!(f, "integer"),
            SchemaType::Bool => write!(f, "bool"),
            SchemaType::Any => write!(f, "any"),
            SchemaType::Optional(inner) => write!(f, "{inner}?"),
            SchemaType::List(inner) => write!(f, "list<{inner}>"),
            SchemaType::Map(inner) => write!(f, "map<{inner}>"),
        }
    }
}

/// Parse a type string like `"string"`, `"string?"`, `"list<number>"`, `"map<string>"`.
///
/// Supports:
/// - Primitives: `"string"`, `"number"`, `"integer"`, `"bool"`, `"any"`
/// - Optional: `"string?"` -> `Optional(String)`
/// - List: `"list<string>"` -> `List(String)`
/// - Map: `"map<number>"` -> `Map(Number)`
/// - Nested generics: `"list<list<string>>"` -> `List(List(String))`
pub fn parse_type_string(s: &str) -> Result<SchemaType, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("Empty type string".to_string());
    }

    // Check for optional suffix (but not inside generics).
    if s.ends_with('?') && !s.ends_with(">?") {
        // Simple optional like "string?"
        let inner = parse_type_string(&s[..s.len() - 1])?;
        return Ok(SchemaType::Optional(Box::new(inner)));
    }

    // Handle optional on generic types like "list<string>?"
    if s.ends_with(">?") {
        let inner = parse_type_string(&s[..s.len() - 1])?;
        return Ok(SchemaType::Optional(Box::new(inner)));
    }

    // Check for generic types: list<...> or map<...>
    if let Some(inner_start) = s.find('<') {
        if !s.ends_with('>') {
            return Err(format!("Malformed generic type: '{s}' (missing closing '>')"));
        }
        let container = &s[..inner_start];
        let inner_str = &s[inner_start + 1..s.len() - 1];

        let inner_type = parse_type_string(inner_str)?;

        match container {
            "list" => Ok(SchemaType::List(Box::new(inner_type))),
            "map" => Ok(SchemaType::Map(Box::new(inner_type))),
            other => Err(format!("Unknown generic type: '{other}'")),
        }
    } else {
        // Primitive types.
        match s {
            "string" => Ok(SchemaType::String),
            "number" => Ok(SchemaType::Number),
            "integer" => Ok(SchemaType::Integer),
            "bool" => Ok(SchemaType::Bool),
            "any" => Ok(SchemaType::Any),
            other => Err(format!("Unknown type: '{other}'")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_primitive_string() {
        assert_eq!(parse_type_string("string").unwrap(), SchemaType::String);
    }

    #[test]
    fn parse_primitive_number() {
        assert_eq!(parse_type_string("number").unwrap(), SchemaType::Number);
    }

    #[test]
    fn parse_primitive_integer() {
        assert_eq!(parse_type_string("integer").unwrap(), SchemaType::Integer);
    }

    #[test]
    fn parse_primitive_bool() {
        assert_eq!(parse_type_string("bool").unwrap(), SchemaType::Bool);
    }

    #[test]
    fn parse_primitive_any() {
        assert_eq!(parse_type_string("any").unwrap(), SchemaType::Any);
    }

    #[test]
    fn parse_optional_string() {
        assert_eq!(
            parse_type_string("string?").unwrap(),
            SchemaType::Optional(Box::new(SchemaType::String))
        );
    }

    #[test]
    fn parse_optional_number() {
        assert_eq!(
            parse_type_string("number?").unwrap(),
            SchemaType::Optional(Box::new(SchemaType::Number))
        );
    }

    #[test]
    fn parse_list_string() {
        assert_eq!(
            parse_type_string("list<string>").unwrap(),
            SchemaType::List(Box::new(SchemaType::String))
        );
    }

    #[test]
    fn parse_map_number() {
        assert_eq!(
            parse_type_string("map<number>").unwrap(),
            SchemaType::Map(Box::new(SchemaType::Number))
        );
    }

    #[test]
    fn parse_nested_list() {
        assert_eq!(
            parse_type_string("list<list<string>>").unwrap(),
            SchemaType::List(Box::new(SchemaType::List(Box::new(SchemaType::String))))
        );
    }

    #[test]
    fn parse_nested_map_in_list() {
        assert_eq!(
            parse_type_string("list<map<integer>>").unwrap(),
            SchemaType::List(Box::new(SchemaType::Map(Box::new(SchemaType::Integer))))
        );
    }

    #[test]
    fn parse_optional_list() {
        assert_eq!(
            parse_type_string("list<string>?").unwrap(),
            SchemaType::Optional(Box::new(SchemaType::List(Box::new(SchemaType::String))))
        );
    }

    #[test]
    fn parse_unknown_type_error() {
        let result = parse_type_string("foobar");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown type"));
    }

    #[test]
    fn parse_unknown_generic_error() {
        let result = parse_type_string("set<string>");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown generic type"));
    }

    #[test]
    fn parse_empty_string_error() {
        let result = parse_type_string("");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Empty type string"));
    }

    #[test]
    fn parse_malformed_generic_error() {
        let result = parse_type_string("list<string");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Malformed generic type"));
    }

    #[test]
    fn parse_whitespace_trimmed() {
        assert_eq!(parse_type_string("  string  ").unwrap(), SchemaType::String);
    }

    #[test]
    fn parse_deeply_nested() {
        assert_eq!(
            parse_type_string("list<list<list<bool>>>").unwrap(),
            SchemaType::List(Box::new(SchemaType::List(Box::new(SchemaType::List(
                Box::new(SchemaType::Bool)
            )))))
        );
    }

    #[test]
    fn display_roundtrip_primitives() {
        for type_str in &["string", "number", "integer", "bool", "any"] {
            let parsed = parse_type_string(type_str).unwrap();
            assert_eq!(parsed.to_string(), *type_str);
        }
    }

    #[test]
    fn display_roundtrip_generics() {
        for type_str in &["list<string>", "map<number>", "list<list<string>>"] {
            let parsed = parse_type_string(type_str).unwrap();
            assert_eq!(parsed.to_string(), *type_str);
        }
    }

    #[test]
    fn display_optional() {
        let t = SchemaType::Optional(Box::new(SchemaType::String));
        assert_eq!(t.to_string(), "string?");
    }

    #[test]
    fn parse_map_any() {
        assert_eq!(
            parse_type_string("map<any>").unwrap(),
            SchemaType::Map(Box::new(SchemaType::Any))
        );
    }

    #[test]
    fn parse_optional_bool() {
        assert_eq!(
            parse_type_string("bool?").unwrap(),
            SchemaType::Optional(Box::new(SchemaType::Bool))
        );
    }
}
