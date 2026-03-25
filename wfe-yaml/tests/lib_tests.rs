use std::collections::HashMap;
use std::io::Write;

use wfe_yaml::{load_workflow, load_workflow_from_str};

#[test]
fn load_workflow_from_file() {
    let yaml = r#"
workflow:
  id: file-load-wf
  version: 1
  steps:
    - name: hello
      type: shell
      config:
        run: echo hello
"#;
    // Write to a temp file manually.
    let path = std::path::Path::new("/tmp/wfe_test_load_workflow.yaml");
    let mut file = std::fs::File::create(path).unwrap();
    file.write_all(yaml.as_bytes()).unwrap();
    file.flush().unwrap();

    let compiled = load_workflow(path, &HashMap::new()).unwrap();
    assert_eq!(compiled.definition.id, "file-load-wf");
    assert_eq!(compiled.definition.version, 1);
    assert_eq!(compiled.definition.steps.len(), 1);

    // Clean up.
    let _ = std::fs::remove_file(path);
}

#[test]
fn load_workflow_from_nonexistent_file_returns_error() {
    let path = std::path::Path::new("/tmp/nonexistent_wfe_test_file_12345.yaml");
    let result = load_workflow(path, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("IO error") || err.contains("No such file"),
        "Expected IO error, got: {err}"
    );
}

#[test]
fn load_workflow_from_str_with_invalid_yaml_returns_error() {
    let yaml = "this is not valid yaml: [[[";
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("YAML parse error"),
        "Expected YAML parse error, got: {err}"
    );
}

#[test]
fn load_workflow_from_str_with_interpolation() {
    let yaml = r#"
workflow:
  id: interp-wf
  version: 1
  steps:
    - name: greet
      type: shell
      config:
        run: echo ((message))
"#;
    let mut config = HashMap::new();
    config.insert("message".to_string(), serde_json::json!("hello world"));

    let compiled = load_workflow_from_str(yaml, &config).unwrap();
    let step = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("greet"))
        .unwrap();

    let shell_config: wfe_yaml::executors::shell::ShellConfig =
        serde_json::from_value(step.step_config.clone().unwrap()).unwrap();
    assert_eq!(shell_config.run, "echo hello world");
}

#[test]
fn load_workflow_from_str_with_unresolved_variable_returns_error() {
    let yaml = r#"
workflow:
  id: unresolved-wf
  version: 1
  steps:
    - name: step1
      type: shell
      config:
        run: echo ((missing))
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("missing"),
        "Expected unresolved variable error, got: {err}"
    );
}
