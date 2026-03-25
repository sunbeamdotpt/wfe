use wfe_yaml::schema::YamlWorkflow;

#[test]
fn parse_minimal_yaml() {
    let yaml = r#"
workflow:
  id: minimal
  version: 1
  steps:
    - name: hello
      type: shell
      config:
        run: echo hello
"#;
    let parsed: YamlWorkflow = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(parsed.workflow.id, "minimal");
    assert_eq!(parsed.workflow.version, 1);
    assert_eq!(parsed.workflow.steps.len(), 1);
    assert_eq!(parsed.workflow.steps[0].name, "hello");
    assert_eq!(
        parsed.workflow.steps[0].step_type.as_deref(),
        Some("shell")
    );
}

#[test]
fn parse_with_parallel_block() {
    let yaml = r#"
workflow:
  id: parallel-wf
  version: 1
  steps:
    - name: parallel-group
      parallel:
        - name: task-a
          type: shell
          config:
            run: echo a
        - name: task-b
          type: shell
          config:
            run: echo b
"#;
    let parsed: YamlWorkflow = serde_yaml::from_str(yaml).unwrap();
    let step = &parsed.workflow.steps[0];
    assert!(step.parallel.is_some());
    let children = step.parallel.as_ref().unwrap();
    assert_eq!(children.len(), 2);
    assert_eq!(children[0].name, "task-a");
    assert_eq!(children[1].name, "task-b");
}

#[test]
fn parse_with_hooks() {
    let yaml = r#"
workflow:
  id: hooks-wf
  version: 1
  steps:
    - name: deploy
      type: shell
      config:
        run: deploy.sh
      on_failure:
        name: rollback
        type: shell
        config:
          run: rollback.sh
      ensure:
        name: cleanup
        type: shell
        config:
          run: cleanup.sh
"#;
    let parsed: YamlWorkflow = serde_yaml::from_str(yaml).unwrap();
    let step = &parsed.workflow.steps[0];
    assert!(step.on_failure.is_some());
    assert_eq!(step.on_failure.as_ref().unwrap().name, "rollback");
    assert!(step.ensure.is_some());
    assert_eq!(step.ensure.as_ref().unwrap().name, "cleanup");
}

#[test]
fn parse_with_error_behavior() {
    let yaml = r#"
workflow:
  id: retry-wf
  version: 1
  error_behavior:
    type: retry
    interval: 5s
    max_retries: 5
  steps:
    - name: flaky
      type: shell
      config:
        run: flaky-task.sh
      error_behavior:
        type: terminate
"#;
    let parsed: YamlWorkflow = serde_yaml::from_str(yaml).unwrap();
    let eb = parsed.workflow.error_behavior.as_ref().unwrap();
    assert_eq!(eb.behavior_type, "retry");
    assert_eq!(eb.interval.as_deref(), Some("5s"));
    assert_eq!(eb.max_retries, Some(5));

    let step_eb = parsed.workflow.steps[0].error_behavior.as_ref().unwrap();
    assert_eq!(step_eb.behavior_type, "terminate");
}

#[test]
fn invalid_yaml_returns_error() {
    let yaml = "this is not valid yaml: [";
    let result: Result<YamlWorkflow, _> = serde_yaml::from_str(yaml);
    assert!(result.is_err());
}

#[test]
fn parse_with_yaml_anchors_and_aliases() {
    // Direct anchor/alias: reuse entire config block via *alias.
    let yaml = r#"
workflow:
  id: test-anchors
  version: 1
  steps:
    - name: build
      type: shell
      config: &default_config
        shell: bash
        timeout: 5m
        run: cargo build

    - name: test
      type: shell
      config: *default_config
"#;
    let parsed: YamlWorkflow = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(parsed.workflow.steps.len(), 2);

    // The build step has the original config.
    let build = &parsed.workflow.steps[0];
    let build_config = build.config.as_ref().unwrap();
    assert_eq!(build_config.shell.as_deref(), Some("bash"));
    assert_eq!(build_config.timeout.as_deref(), Some("5m"));
    assert_eq!(build_config.run.as_deref(), Some("cargo build"));

    // The test step gets the same config via alias.
    let test = &parsed.workflow.steps[1];
    let test_config = test.config.as_ref().unwrap();
    assert_eq!(test_config.shell.as_deref(), Some("bash"));
    assert_eq!(test_config.timeout.as_deref(), Some("5m"));
    assert_eq!(test_config.run.as_deref(), Some("cargo build"));
}

#[test]
fn parse_with_scalar_anchors() {
    // Anchors on scalar values.
    let yaml = r#"
workflow:
  id: scalar-anchors
  version: 1
  steps:
    - name: step1
      type: &step_type shell
      config:
        run: echo hi
    - name: step2
      type: *step_type
      config:
        run: echo bye
"#;
    let parsed: YamlWorkflow = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(parsed.workflow.steps[0].step_type.as_deref(), Some("shell"));
    assert_eq!(parsed.workflow.steps[1].step_type.as_deref(), Some("shell"));
}

#[test]
fn parse_with_extra_keys_for_templates() {
    let yaml = r#"
workflow:
  id: template-wf
  version: 1
  _templates:
    default_shell: bash
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
"#;
    let parsed: YamlWorkflow = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(parsed.workflow.id, "template-wf");
    assert_eq!(parsed.workflow.steps.len(), 1);
}
