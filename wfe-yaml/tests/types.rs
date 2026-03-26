use wfe_yaml::types::{parse_type_string, SchemaType};

#[test]
fn parse_all_primitives() {
    assert_eq!(parse_type_string("string").unwrap(), SchemaType::String);
    assert_eq!(parse_type_string("number").unwrap(), SchemaType::Number);
    assert_eq!(parse_type_string("integer").unwrap(), SchemaType::Integer);
    assert_eq!(parse_type_string("bool").unwrap(), SchemaType::Bool);
    assert_eq!(parse_type_string("any").unwrap(), SchemaType::Any);
}

#[test]
fn parse_optional_types() {
    assert_eq!(
        parse_type_string("string?").unwrap(),
        SchemaType::Optional(Box::new(SchemaType::String))
    );
    assert_eq!(
        parse_type_string("integer?").unwrap(),
        SchemaType::Optional(Box::new(SchemaType::Integer))
    );
}

#[test]
fn parse_list_types() {
    assert_eq!(
        parse_type_string("list<string>").unwrap(),
        SchemaType::List(Box::new(SchemaType::String))
    );
    assert_eq!(
        parse_type_string("list<number>").unwrap(),
        SchemaType::List(Box::new(SchemaType::Number))
    );
}

#[test]
fn parse_map_types() {
    assert_eq!(
        parse_type_string("map<string>").unwrap(),
        SchemaType::Map(Box::new(SchemaType::String))
    );
    assert_eq!(
        parse_type_string("map<any>").unwrap(),
        SchemaType::Map(Box::new(SchemaType::Any))
    );
}

#[test]
fn parse_nested_generics() {
    assert_eq!(
        parse_type_string("list<list<string>>").unwrap(),
        SchemaType::List(Box::new(SchemaType::List(Box::new(SchemaType::String))))
    );
    assert_eq!(
        parse_type_string("map<list<integer>>").unwrap(),
        SchemaType::Map(Box::new(SchemaType::List(Box::new(SchemaType::Integer))))
    );
}

#[test]
fn parse_optional_generic() {
    assert_eq!(
        parse_type_string("list<string>?").unwrap(),
        SchemaType::Optional(Box::new(SchemaType::List(Box::new(SchemaType::String))))
    );
}

#[test]
fn parse_unknown_type_returns_error() {
    let err = parse_type_string("foobar").unwrap_err();
    assert!(err.contains("Unknown type"), "Got: {err}");
}

#[test]
fn parse_unknown_generic_container_returns_error() {
    let err = parse_type_string("set<string>").unwrap_err();
    assert!(err.contains("Unknown generic type"), "Got: {err}");
}

#[test]
fn parse_empty_returns_error() {
    let err = parse_type_string("").unwrap_err();
    assert!(err.contains("Empty"), "Got: {err}");
}

#[test]
fn parse_malformed_generic_returns_error() {
    let err = parse_type_string("list<string").unwrap_err();
    assert!(err.contains("Malformed"), "Got: {err}");
}

#[test]
fn display_roundtrip() {
    for s in &[
        "string",
        "number",
        "integer",
        "bool",
        "any",
        "list<string>",
        "map<number>",
        "list<list<string>>",
    ] {
        let parsed = parse_type_string(s).unwrap();
        assert_eq!(parsed.to_string(), *s, "Roundtrip failed for {s}");
    }
}
