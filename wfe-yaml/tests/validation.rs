use std::collections::HashMap;

use wfe_yaml::load_workflow_from_str;

#[test]
fn empty_steps_returns_validation_error() {
    let yaml = r#"
workflow:
  id: empty-wf
  version: 1
  steps: []
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("at least one step"),
        "Expected 'at least one step' error, got: {err}"
    );
}

#[test]
fn step_with_no_type_and_no_parallel_returns_error() {
    let yaml = r#"
workflow:
  id: no-type-wf
  version: 1
  steps:
    - name: bad-step
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("type") && err.contains("parallel"),
        "Expected error about missing type or parallel, got: {err}"
    );
}

#[test]
fn step_with_both_type_and_parallel_returns_error() {
    let yaml = r#"
workflow:
  id: both-wf
  version: 1
  steps:
    - name: bad-step
      type: shell
      parallel:
        - name: child
          type: shell
          config:
            run: echo hi
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("cannot have both"),
        "Expected 'cannot have both' error, got: {err}"
    );
}

#[test]
fn duplicate_step_names_returns_error() {
    let yaml = r#"
workflow:
  id: dup-wf
  version: 1
  steps:
    - name: deploy
      type: shell
      config:
        run: echo a
    - name: deploy
      type: shell
      config:
        run: echo b
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("Duplicate step name") && err.contains("deploy"),
        "Expected duplicate name error, got: {err}"
    );
}

#[test]
fn shell_step_missing_run_and_file_returns_error() {
    let yaml = r#"
workflow:
  id: no-run-wf
  version: 1
  steps:
    - name: bad-shell
      type: shell
      config:
        shell: bash
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("config.run") || err.contains("config.file"),
        "Expected error about missing run/file, got: {err}"
    );
}

#[test]
fn shell_step_missing_config_section_returns_error() {
    let yaml = r#"
workflow:
  id: no-config-wf
  version: 1
  steps:
    - name: bad-shell
      type: shell
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("config"),
        "Expected error about missing config, got: {err}"
    );
}

#[test]
fn invalid_error_behavior_type_returns_error() {
    let yaml = r#"
workflow:
  id: bad-eb-wf
  version: 1
  error_behavior:
    type: panic
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("panic"),
        "Expected error mentioning invalid type, got: {err}"
    );
}

#[test]
fn invalid_step_level_error_behavior_returns_error() {
    let yaml = r#"
workflow:
  id: bad-step-eb-wf
  version: 1
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
      error_behavior:
        type: crash
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("crash"),
        "Expected error mentioning invalid type, got: {err}"
    );
}

#[test]
fn valid_minimal_step_passes_validation() {
    let yaml = r#"
workflow:
  id: valid-wf
  version: 1
  steps:
    - name: hello
      type: shell
      config:
        run: echo hello
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_ok(), "Valid workflow should pass, got: {:?}", result.err());
}

#[test]
fn valid_parallel_step_passes_validation() {
    let yaml = r#"
workflow:
  id: valid-parallel-wf
  version: 1
  steps:
    - name: parallel-group
      parallel:
        - name: task-a
          type: shell
          config:
            run: echo a
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_ok(), "Valid parallel workflow should pass, got: {:?}", result.err());
}

#[test]
fn hook_steps_are_also_validated_for_duplicates() {
    let yaml = r#"
workflow:
  id: hook-dup-wf
  version: 1
  steps:
    - name: deploy
      type: shell
      config:
        run: deploy.sh
      on_failure:
        name: deploy
        type: shell
        config:
          run: rollback.sh
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("Duplicate step name"),
        "Expected duplicate name error for hook, got: {err}"
    );
}

#[test]
fn on_success_hook_validated() {
    let yaml = r#"
workflow:
  id: hook-val-wf
  version: 1
  steps:
    - name: deploy
      type: shell
      config:
        run: deploy.sh
      on_success:
        name: notify
        type: shell
        config:
          run: echo ok
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_ok(), "Valid on_success hook should pass, got: {:?}", result.err());
}

#[test]
fn ensure_hook_validated() {
    let yaml = r#"
workflow:
  id: ensure-val-wf
  version: 1
  steps:
    - name: deploy
      type: shell
      config:
        run: deploy.sh
      ensure:
        name: cleanup
        type: shell
        config:
          run: cleanup.sh
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_ok(), "Valid ensure hook should pass, got: {:?}", result.err());
}

#[test]
fn all_valid_error_behavior_types_pass() {
    for eb_type in &["retry", "suspend", "terminate", "compensate"] {
        let yaml = format!(
            r#"
workflow:
  id: eb-{eb_type}-wf
  version: 1
  error_behavior:
    type: {eb_type}
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
"#
        );
        let result = load_workflow_from_str(&yaml, &HashMap::new());
        assert!(
            result.is_ok(),
            "Error behavior type '{eb_type}' should be valid, got: {:?}",
            result.err()
        );
    }
}

#[test]
fn parallel_children_duplicate_names_detected() {
    let yaml = r#"
workflow:
  id: par-dup-wf
  version: 1
  steps:
    - name: parallel-group
      parallel:
        - name: task-a
          type: shell
          config:
            run: echo a
        - name: task-a
          type: shell
          config:
            run: echo b
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("Duplicate step name") && err.contains("task-a"),
        "Expected duplicate name in parallel children, got: {err}"
    );
}
