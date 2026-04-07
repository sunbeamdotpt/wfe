use wfe_yaml::schema::{YamlCondition, YamlWorkflow, YamlWorkflowFile};

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
    assert_eq!(parsed.workflow.steps[0].step_type.as_deref(), Some("shell"));
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

// --- Multi-workflow file tests ---

#[test]
fn parse_single_workflow_file() {
    let yaml = r#"
workflow:
  id: single
  version: 1
  steps:
    - name: step1
      type: shell
      config:
        run: echo hello
"#;
    let parsed: YamlWorkflowFile = serde_yaml::from_str(yaml).unwrap();
    assert!(parsed.workflow.is_some());
    assert!(parsed.workflows.is_none());
    assert_eq!(parsed.workflow.unwrap().id, "single");
}

#[test]
fn parse_multi_workflow_file() {
    let yaml = r#"
workflows:
  - id: build-wf
    version: 1
    steps:
      - name: build
        type: shell
        config:
          run: cargo build
  - id: test-wf
    version: 1
    steps:
      - name: test
        type: shell
        config:
          run: cargo test
"#;
    let parsed: YamlWorkflowFile = serde_yaml::from_str(yaml).unwrap();
    assert!(parsed.workflow.is_none());
    assert!(parsed.workflows.is_some());
    let workflows = parsed.workflows.unwrap();
    assert_eq!(workflows.len(), 2);
    assert_eq!(workflows[0].id, "build-wf");
    assert_eq!(workflows[1].id, "test-wf");
}

#[test]
fn parse_workflow_with_input_output_schemas() {
    let yaml = r#"
workflow:
  id: typed-wf
  version: 1
  inputs:
    repo_url: string
    tags: "list<string>"
    verbose: bool?
  outputs:
    artifact_path: string
    exit_code: integer
  steps:
    - name: step1
      type: shell
      config:
        run: echo hello
"#;
    let parsed: YamlWorkflow = serde_yaml::from_str(yaml).unwrap();
    assert_eq!(parsed.workflow.inputs.len(), 3);
    assert_eq!(parsed.workflow.inputs.get("repo_url").unwrap(), "string");
    assert_eq!(parsed.workflow.inputs.get("tags").unwrap(), "list<string>");
    assert_eq!(parsed.workflow.inputs.get("verbose").unwrap(), "bool?");
    assert_eq!(parsed.workflow.outputs.len(), 2);
    assert_eq!(
        parsed.workflow.outputs.get("artifact_path").unwrap(),
        "string"
    );
    assert_eq!(parsed.workflow.outputs.get("exit_code").unwrap(), "integer");
}

#[test]
fn parse_step_with_workflow_type() {
    let yaml = r#"
workflow:
  id: parent-wf
  version: 1
  steps:
    - name: run-child
      type: workflow
      config:
        workflow: child-wf
        workflow_version: 2
      inputs:
        - name: repo_url
          path: data.repo
      outputs:
        - name: result
"#;
    let parsed: YamlWorkflow = serde_yaml::from_str(yaml).unwrap();
    let step = &parsed.workflow.steps[0];
    assert_eq!(step.step_type.as_deref(), Some("workflow"));
    let config = step.config.as_ref().unwrap();
    assert_eq!(config.child_workflow.as_deref(), Some("child-wf"));
    assert_eq!(config.child_version, Some(2));
    assert_eq!(step.inputs.len(), 1);
    assert_eq!(step.outputs.len(), 1);
}

#[test]
fn parse_workflow_step_version_defaults() {
    let yaml = r#"
workflow:
  id: parent-wf
  version: 1
  steps:
    - name: run-child
      type: workflow
      config:
        workflow: child-wf
"#;
    let parsed: YamlWorkflow = serde_yaml::from_str(yaml).unwrap();
    let config = parsed.workflow.steps[0].config.as_ref().unwrap();
    assert_eq!(config.child_workflow.as_deref(), Some("child-wf"));
    // version not specified, should be None in schema (compiler defaults to 1).
    assert_eq!(config.child_version, None);
}

#[test]
fn parse_empty_inputs_outputs_default() {
    let yaml = r#"
workflow:
  id: no-schema-wf
  version: 1
  steps:
    - name: step1
      type: shell
      config:
        run: echo hello
"#;
    let parsed: YamlWorkflow = serde_yaml::from_str(yaml).unwrap();
    assert!(parsed.workflow.inputs.is_empty());
    assert!(parsed.workflow.outputs.is_empty());
}

// --- Condition schema tests ---

#[test]
fn parse_step_with_simple_when_condition() {
    let yaml = r#"
workflow:
  id: cond-wf
  version: 1
  steps:
    - name: deploy
      type: shell
      config:
        run: deploy.sh
      when:
        field: .inputs.enabled
        equals: true
"#;
    let parsed: YamlWorkflow = serde_yaml::from_str(yaml).unwrap();
    let step = &parsed.workflow.steps[0];
    assert!(step.when.is_some());
    match step.when.as_ref().unwrap() {
        YamlCondition::Comparison(cmp) => {
            assert_eq!(cmp.field, ".inputs.enabled");
            assert!(cmp.equals.is_some());
        }
        _ => panic!("Expected Comparison variant"),
    }
}

#[test]
fn parse_step_with_nested_combinator_conditions() {
    let yaml = r#"
workflow:
  id: nested-cond-wf
  version: 1
  steps:
    - name: deploy
      type: shell
      config:
        run: deploy.sh
      when:
        all:
          - field: .inputs.count
            gt: 5
          - any:
              - field: .inputs.env
                equals: prod
              - field: .inputs.env
                equals: staging
"#;
    let parsed: YamlWorkflow = serde_yaml::from_str(yaml).unwrap();
    let step = &parsed.workflow.steps[0];
    assert!(step.when.is_some());
    match step.when.as_ref().unwrap() {
        YamlCondition::Combinator(c) => {
            assert!(c.all.is_some());
            let children = c.all.as_ref().unwrap();
            assert_eq!(children.len(), 2);
        }
        _ => panic!("Expected Combinator variant"),
    }
}

#[test]
fn parse_step_with_not_condition() {
    let yaml = r#"
workflow:
  id: not-cond-wf
  version: 1
  steps:
    - name: deploy
      type: shell
      config:
        run: deploy.sh
      when:
        not:
          field: .inputs.skip
          equals: true
"#;
    let parsed: YamlWorkflow = serde_yaml::from_str(yaml).unwrap();
    let step = &parsed.workflow.steps[0];
    match step.when.as_ref().unwrap() {
        YamlCondition::Combinator(c) => {
            assert!(c.not.is_some());
        }
        _ => panic!("Expected Combinator with not"),
    }
}

#[test]
fn parse_step_with_none_condition() {
    let yaml = r#"
workflow:
  id: none-cond-wf
  version: 1
  steps:
    - name: deploy
      type: shell
      config:
        run: deploy.sh
      when:
        none:
          - field: .inputs.skip
            equals: true
          - field: .inputs.disabled
            equals: true
"#;
    let parsed: YamlWorkflow = serde_yaml::from_str(yaml).unwrap();
    let step = &parsed.workflow.steps[0];
    match step.when.as_ref().unwrap() {
        YamlCondition::Combinator(c) => {
            assert!(c.none.is_some());
            assert_eq!(c.none.as_ref().unwrap().len(), 2);
        }
        _ => panic!("Expected Combinator with none"),
    }
}

#[test]
fn parse_step_with_one_of_condition() {
    let yaml = r#"
workflow:
  id: one-of-wf
  version: 1
  steps:
    - name: deploy
      type: shell
      config:
        run: deploy.sh
      when:
        one_of:
          - field: .inputs.mode
            equals: fast
          - field: .inputs.mode
            equals: slow
"#;
    let parsed: YamlWorkflow = serde_yaml::from_str(yaml).unwrap();
    let step = &parsed.workflow.steps[0];
    match step.when.as_ref().unwrap() {
        YamlCondition::Combinator(c) => {
            assert!(c.one_of.is_some());
            assert_eq!(c.one_of.as_ref().unwrap().len(), 2);
        }
        _ => panic!("Expected Combinator with one_of"),
    }
}

#[test]
fn parse_comparison_with_each_operator() {
    // Test that each operator variant deserializes correctly.
    let operators = vec![
        ("equals: 42", "equals"),
        ("not_equals: foo", "not_equals"),
        ("gt: 10", "gt"),
        ("gte: 10", "gte"),
        ("lt: 100", "lt"),
        ("lte: 100", "lte"),
        ("contains: needle", "contains"),
        ("is_null: true", "is_null"),
        ("is_not_null: true", "is_not_null"),
    ];

    for (op_yaml, op_name) in operators {
        let yaml = format!(
            r#"
workflow:
  id: op-{op_name}
  version: 1
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
      when:
        field: .inputs.x
        {op_yaml}
"#
        );
        let parsed: YamlWorkflow = serde_yaml::from_str(&yaml)
            .unwrap_or_else(|e| panic!("Failed to parse operator {op_name}: {e}"));
        let step = &parsed.workflow.steps[0];
        assert!(
            step.when.is_some(),
            "Step should have when condition for operator {op_name}"
        );
        match step.when.as_ref().unwrap() {
            YamlCondition::Comparison(_) => {}
            _ => panic!("Expected Comparison for operator {op_name}"),
        }
    }
}

#[test]
fn parse_step_without_when_has_none() {
    let yaml = r#"
workflow:
  id: no-when-wf
  version: 1
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
"#;
    let parsed: YamlWorkflow = serde_yaml::from_str(yaml).unwrap();
    assert!(parsed.workflow.steps[0].when.is_none());
}
