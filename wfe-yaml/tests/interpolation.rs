use std::collections::HashMap;

use wfe_yaml::interpolation::interpolate;

#[test]
fn simple_var_replacement() {
    let mut config = HashMap::new();
    config.insert("name".to_string(), serde_json::json!("world"));

    let result = interpolate("hello ((name))", &config).unwrap();
    assert_eq!(result, "hello world");
}

#[test]
fn nested_path_replacement() {
    let mut config = HashMap::new();
    config.insert(
        "config".to_string(),
        serde_json::json!({
            "database": {
                "host": "localhost",
                "port": 5432
            }
        }),
    );

    let result = interpolate("host: ((config.database.host))", &config).unwrap();
    assert_eq!(result, "host: localhost");

    let result = interpolate("port: ((config.database.port))", &config).unwrap();
    assert_eq!(result, "port: 5432");
}

#[test]
fn unresolved_var_returns_error() {
    let config = HashMap::new();
    let result = interpolate("hello ((missing_var))", &config);
    assert!(result.is_err());
    let err = result.unwrap_err();
    assert!(err.to_string().contains("missing_var"));
}

#[test]
fn no_vars_passes_through_unchanged() {
    let config = HashMap::new();
    let input = "no variables here";
    let result = interpolate(input, &config).unwrap();
    assert_eq!(result, input);
}

#[test]
fn multiple_vars_in_one_string() {
    let mut config = HashMap::new();
    config.insert("first".to_string(), serde_json::json!("hello"));
    config.insert("second".to_string(), serde_json::json!("world"));

    let result = interpolate("((first)) ((second))!", &config).unwrap();
    assert_eq!(result, "hello world!");
}

#[test]
fn interpolation_does_not_break_yaml_anchors() {
    let mut config = HashMap::new();
    config.insert("version".to_string(), serde_json::json!("1.0"));

    // YAML anchor syntax should not be confused with ((var)) syntax.
    let yaml = r#"
default: &default
  version: ((version))
merged:
  <<: *default
"#;
    let result = interpolate(yaml, &config).unwrap();
    assert!(result.contains("version: 1.0"));
    assert!(result.contains("&default"));
    assert!(result.contains("*default"));
}
