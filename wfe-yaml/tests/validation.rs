use std::collections::HashMap;

use wfe_yaml::{load_single_workflow_from_str, load_workflow_from_str};

#[test]
fn empty_steps_returns_validation_error() {
    let yaml = r#"
workflow:
  id: empty-wf
  version: 1
  steps: []
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
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
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
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
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
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
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
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
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
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
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
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
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
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
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
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
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
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
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
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
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
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
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
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
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
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
        let result = load_single_workflow_from_str(&yaml, &HashMap::new());
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
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("Duplicate step name") && err.contains("task-a"),
        "Expected duplicate name in parallel children, got: {err}"
    );
}

// --- Workflow step validation tests ---

#[test]
fn workflow_step_missing_config_returns_error() {
    let yaml = r#"
workflow:
  id: wf-missing-config
  version: 1
  steps:
    - name: run-child
      type: workflow
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("config"),
        "Expected error about missing config, got: {err}"
    );
}

#[test]
fn workflow_step_missing_workflow_field_returns_error() {
    let yaml = r#"
workflow:
  id: wf-missing-field
  version: 1
  steps:
    - name: run-child
      type: workflow
      config:
        run: echo oops
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("config.workflow"),
        "Expected error about missing config.workflow, got: {err}"
    );
}

#[test]
fn valid_workflow_step_passes_validation() {
    let yaml = r#"
workflow:
  id: parent
  version: 1
  steps:
    - name: run-child
      type: workflow
      config:
        workflow: child-wf
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_ok(), "Valid workflow step should pass, got: {:?}", result.err());
}

// --- Multi-workflow validation tests ---

#[test]
fn multi_workflow_valid_passes() {
    let yaml = r#"
workflows:
  - id: build
    version: 1
    steps:
      - name: compile
        type: shell
        config:
          run: cargo build
  - id: test
    version: 1
    steps:
      - name: run-tests
        type: shell
        config:
          run: cargo test
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_ok(), "Valid multi-workflow should pass, got: {:?}", result.err());
    assert_eq!(result.unwrap().len(), 2);
}

#[test]
fn multi_workflow_duplicate_ids_returns_error() {
    let yaml = r#"
workflows:
  - id: my-wf
    version: 1
    steps:
      - name: step1
        type: shell
        config:
          run: echo a
  - id: my-wf
    version: 2
    steps:
      - name: step2
        type: shell
        config:
          run: echo b
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("Duplicate workflow ID"),
        "Expected duplicate workflow ID error, got: {err}"
    );
}

#[test]
fn both_workflow_and_workflows_returns_error() {
    let yaml = r#"
workflow:
  id: single
  version: 1
  steps:
    - name: s1
      type: shell
      config:
        run: echo hi
workflows:
  - id: multi
    version: 1
    steps:
      - name: s2
        type: shell
        config:
          run: echo bye
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("Cannot specify both"),
        "Expected error about both workflow and workflows, got: {err}"
    );
}

#[test]
fn neither_workflow_nor_workflows_returns_error() {
    let yaml = r#"
something_else:
  id: nothing
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("Must specify either"),
        "Expected error about missing workflow/workflows, got: {err}"
    );
}

#[test]
fn empty_workflows_list_returns_error() {
    let yaml = r#"
workflows: []
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("empty"),
        "Expected error about empty workflows, got: {err}"
    );
}

// --- Circular reference detection tests ---

#[test]
fn circular_reference_detected() {
    let yaml = r#"
workflows:
  - id: wf-a
    version: 1
    steps:
      - name: call-b
        type: workflow
        config:
          workflow: wf-b
  - id: wf-b
    version: 1
    steps:
      - name: call-a
        type: workflow
        config:
          workflow: wf-a
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("Circular workflow reference"),
        "Expected circular reference error, got: {err}"
    );
}

#[test]
fn self_referencing_workflow_detected() {
    let yaml = r#"
workflows:
  - id: self-ref
    version: 1
    steps:
      - name: call-self
        type: workflow
        config:
          workflow: self-ref
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("Circular workflow reference"),
        "Expected circular reference error, got: {err}"
    );
}

#[test]
fn valid_workflow_reference_passes() {
    let yaml = r#"
workflows:
  - id: parent
    version: 1
    steps:
      - name: call-child
        type: workflow
        config:
          workflow: child
  - id: child
    version: 1
    steps:
      - name: do-work
        type: shell
        config:
          run: echo working
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_ok(), "Valid workflow reference should pass, got: {:?}", result.err());
}

#[test]
fn external_workflow_reference_does_not_error() {
    // Referencing a workflow not in this file is allowed (it may be registered separately).
    let yaml = r#"
workflow:
  id: caller
  version: 1
  steps:
    - name: call-external
      type: workflow
      config:
        workflow: some-external-wf
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_ok(), "External workflow ref should not error, got: {:?}", result.err());
}

#[test]
fn load_single_workflow_from_multi_file_returns_error() {
    let yaml = r#"
workflows:
  - id: wf-a
    version: 1
    steps:
      - name: step1
        type: shell
        config:
          run: echo a
  - id: wf-b
    version: 1
    steps:
      - name: step2
        type: shell
        config:
          run: echo b
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("Expected single workflow"),
        "Expected single workflow error, got: {err}"
    );
}

// --- validate_multi edge cases ---

#[test]
fn multi_workflow_single_workflow_passes() {
    let yaml = r#"
workflows:
  - id: only-one
    version: 1
    steps:
      - name: step1
        type: shell
        config:
          run: echo hello
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_ok(), "Single workflow in multi-mode should pass, got: {:?}", result.err());
    assert_eq!(result.unwrap().len(), 1);
}

#[test]
fn multi_workflow_no_cross_references() {
    let yaml = r#"
workflows:
  - id: alpha
    version: 1
    steps:
      - name: a-step
        type: shell
        config:
          run: echo alpha
  - id: beta
    version: 1
    steps:
      - name: b-step
        type: shell
        config:
          run: echo beta
  - id: gamma
    version: 1
    steps:
      - name: g-step
        type: shell
        config:
          run: echo gamma
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_ok(), "Multiple independent workflows should pass, got: {:?}", result.err());
    assert_eq!(result.unwrap().len(), 3);
}

#[test]
fn multi_workflow_with_valid_cross_reference() {
    let yaml = r#"
workflows:
  - id: parent
    version: 1
    steps:
      - name: call-child
        type: workflow
        config:
          workflow: child
  - id: child
    version: 1
    steps:
      - name: do-work
        type: shell
        config:
          run: echo working
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_ok(), "Cross-referenced workflows should pass, got: {:?}", result.err());
}

// --- Cycle detection edge cases ---

#[test]
fn three_node_cycle_detected() {
    let yaml = r#"
workflows:
  - id: wf-a
    version: 1
    steps:
      - name: call-b
        type: workflow
        config:
          workflow: wf-b
  - id: wf-b
    version: 1
    steps:
      - name: call-c
        type: workflow
        config:
          workflow: wf-c
  - id: wf-c
    version: 1
    steps:
      - name: call-a
        type: workflow
        config:
          workflow: wf-a
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("Circular workflow reference"),
        "Expected circular reference error for 3-node cycle, got: {err}"
    );
}

#[test]
fn chain_no_cycle_passes() {
    let yaml = r#"
workflows:
  - id: wf-a
    version: 1
    steps:
      - name: call-b
        type: workflow
        config:
          workflow: wf-b
  - id: wf-b
    version: 1
    steps:
      - name: call-c
        type: workflow
        config:
          workflow: wf-c
  - id: wf-c
    version: 1
    steps:
      - name: leaf
        type: shell
        config:
          run: echo done
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_ok(), "Linear chain should not be a cycle, got: {:?}", result.err());
}

#[test]
fn diamond_dependency_no_cycle_passes() {
    let yaml = r#"
workflows:
  - id: wf-a
    version: 1
    steps:
      - name: call-b
        type: workflow
        config:
          workflow: wf-b
      - name: call-c
        type: workflow
        config:
          workflow: wf-c
  - id: wf-b
    version: 1
    steps:
      - name: call-d
        type: workflow
        config:
          workflow: wf-d
  - id: wf-c
    version: 1
    steps:
      - name: call-d-too
        type: workflow
        config:
          workflow: wf-d
  - id: wf-d
    version: 1
    steps:
      - name: leaf
        type: shell
        config:
          run: echo done
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_ok(), "Diamond dependency should not be a cycle, got: {:?}", result.err());
}

// --- Deno step validation ---

#[test]
fn deno_step_missing_config_returns_error() {
    let yaml = r#"
workflow:
  id: deno-no-config
  version: 1
  steps:
    - name: bad-deno
      type: deno
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("Deno") && err.contains("config"),
        "Expected Deno config error, got: {err}"
    );
}

#[test]
fn deno_step_missing_script_and_file_returns_error() {
    let yaml = r#"
workflow:
  id: deno-no-script
  version: 1
  steps:
    - name: bad-deno
      type: deno
      config:
        env:
          FOO: bar
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("Deno") && (err.contains("script") || err.contains("file")),
        "Expected Deno script/file error, got: {err}"
    );
}

// --- BuildKit step validation ---

#[test]
fn buildkit_step_missing_config_returns_error() {
    let yaml = r#"
workflow:
  id: bk-no-config
  version: 1
  steps:
    - name: bad-bk
      type: buildkit
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("BuildKit") && err.contains("config"),
        "Expected BuildKit config error, got: {err}"
    );
}

#[test]
fn buildkit_step_missing_dockerfile_returns_error() {
    let yaml = r#"
workflow:
  id: bk-no-dockerfile
  version: 1
  steps:
    - name: bad-bk
      type: buildkit
      config:
        context: .
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("dockerfile"),
        "Expected dockerfile error, got: {err}"
    );
}

#[test]
fn buildkit_step_missing_context_returns_error() {
    let yaml = r#"
workflow:
  id: bk-no-context
  version: 1
  steps:
    - name: bad-bk
      type: buildkit
      config:
        dockerfile: Dockerfile
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("context"),
        "Expected context error, got: {err}"
    );
}

#[test]
fn buildkit_step_push_without_tags_returns_error() {
    let yaml = r#"
workflow:
  id: bk-push-no-tags
  version: 1
  steps:
    - name: bad-bk
      type: buildkit
      config:
        dockerfile: Dockerfile
        context: .
        push: true
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("push") && err.contains("tags"),
        "Expected push/tags error, got: {err}"
    );
}

#[test]
fn buildkit_step_valid_passes() {
    let yaml = r#"
workflow:
  id: bk-valid
  version: 1
  steps:
    - name: build-image
      type: buildkit
      config:
        dockerfile: Dockerfile
        context: .
        tags:
          - myimg:latest
        push: true
"#;
    // Validation passes even without the buildkit feature (validation is not feature-gated).
    // Compilation will fail without the feature, but validation should succeed.
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    // This may fail at compilation if buildkit feature is not enabled, which is fine.
    // We're testing validation, not compilation. If it errors, check it's not a validation error.
    if let Err(ref e) = result {
        let err = e.to_string();
        assert!(
            !err.contains("Validation error"),
            "BuildKit validation should pass for valid config, got: {err}"
        );
    }
}

// --- Containerd step validation ---

#[test]
fn containerd_step_missing_config_returns_error() {
    let yaml = r#"
workflow:
  id: ctd-no-config
  version: 1
  steps:
    - name: bad-ctd
      type: containerd
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("Containerd") && err.contains("config"),
        "Expected Containerd config error, got: {err}"
    );
}

#[test]
fn containerd_step_missing_image_returns_error() {
    let yaml = r#"
workflow:
  id: ctd-no-image
  version: 1
  steps:
    - name: bad-ctd
      type: containerd
      config:
        run: echo hello
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("image"),
        "Expected image error, got: {err}"
    );
}

#[test]
fn containerd_step_missing_run_and_command_returns_error() {
    let yaml = r#"
workflow:
  id: ctd-no-run
  version: 1
  steps:
    - name: bad-ctd
      type: containerd
      config:
        image: alpine:latest
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("run") || err.contains("command"),
        "Expected run/command error, got: {err}"
    );
}

#[test]
fn containerd_step_both_run_and_command_returns_error() {
    let yaml = r#"
workflow:
  id: ctd-both
  version: 1
  steps:
    - name: bad-ctd
      type: containerd
      config:
        image: alpine:latest
        run: echo hello
        command:
          - echo
          - hello
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("cannot have both"),
        "Expected 'cannot have both' error, got: {err}"
    );
}

#[test]
fn containerd_step_invalid_network_returns_error() {
    let yaml = r#"
workflow:
  id: ctd-bad-net
  version: 1
  steps:
    - name: bad-ctd
      type: containerd
      config:
        image: alpine:latest
        run: echo hello
        network: overlay
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("network") && err.contains("overlay"),
        "Expected invalid network error, got: {err}"
    );
}

#[test]
fn containerd_step_valid_networks_pass() {
    for net in &["none", "host", "bridge"] {
        let yaml = format!(
            r#"
workflow:
  id: ctd-net-{net}
  version: 1
  steps:
    - name: step1
      type: containerd
      config:
        image: alpine:latest
        run: echo hello
        network: {net}
"#
        );
        let result = load_single_workflow_from_str(&yaml, &HashMap::new());
        if let Err(ref e) = result {
            let err = e.to_string();
            assert!(
                !err.contains("network"),
                "Network '{net}' should be valid, got: {err}"
            );
        }
    }
}

#[test]
fn containerd_step_invalid_pull_policy_returns_error() {
    let yaml = r#"
workflow:
  id: ctd-bad-pull
  version: 1
  steps:
    - name: bad-ctd
      type: containerd
      config:
        image: alpine:latest
        run: echo hello
        pull: aggressive
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("pull") && err.contains("aggressive"),
        "Expected invalid pull policy error, got: {err}"
    );
}

#[test]
fn containerd_step_valid_pull_policies_pass() {
    for pull in &["always", "if-not-present", "never"] {
        let yaml = format!(
            r#"
workflow:
  id: ctd-pull-{pull}
  version: 1
  steps:
    - name: step1
      type: containerd
      config:
        image: alpine:latest
        run: echo hello
        pull: {pull}
"#
        );
        let result = load_single_workflow_from_str(&yaml, &HashMap::new());
        if let Err(ref e) = result {
            let err = e.to_string();
            assert!(
                !err.contains("pull policy"),
                "Pull policy '{pull}' should be valid, got: {err}"
            );
        }
    }
}

#[test]
fn containerd_step_with_command_only_passes_validation() {
    let yaml = r#"
workflow:
  id: ctd-cmd
  version: 1
  steps:
    - name: ctd-step
      type: containerd
      config:
        image: alpine:latest
        command:
          - echo
          - hello
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    if let Err(ref e) = result {
        let err = e.to_string();
        assert!(
            !err.contains("Validation error"),
            "Containerd step with command only should pass validation, got: {err}"
        );
    }
}

// --- Hook validation edge cases ---

#[test]
fn on_failure_hook_with_invalid_step_returns_error() {
    let yaml = r#"
workflow:
  id: hook-invalid-wf
  version: 1
  steps:
    - name: deploy
      type: shell
      config:
        run: deploy.sh
      on_failure:
        name: rollback
        type: shell
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("config"),
        "Expected config error for invalid hook, got: {err}"
    );
}

#[test]
fn on_success_hook_with_invalid_step_returns_error() {
    let yaml = r#"
workflow:
  id: hook-invalid-success
  version: 1
  steps:
    - name: deploy
      type: shell
      config:
        run: deploy.sh
      on_success:
        name: notify
        type: shell
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("config"),
        "Expected config error for invalid on_success hook, got: {err}"
    );
}

#[test]
fn ensure_hook_with_invalid_step_returns_error() {
    let yaml = r#"
workflow:
  id: hook-invalid-ensure
  version: 1
  steps:
    - name: deploy
      type: shell
      config:
        run: deploy.sh
      ensure:
        name: cleanup
        type: shell
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("config"),
        "Expected config error for invalid ensure hook, got: {err}"
    );
}

#[test]
fn parallel_with_nested_invalid_child_returns_error() {
    let yaml = r#"
workflow:
  id: nested-invalid-wf
  version: 1
  steps:
    - name: outer
      parallel:
        - name: inner
          parallel:
            - name: deep
              type: shell
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("config"),
        "Expected config error for deeply nested invalid step, got: {err}"
    );
}

// --- Workflow reference collection from hooks and parallel ---

#[test]
fn workflow_ref_in_on_success_hook_detected_for_cycles() {
    let yaml = r#"
workflows:
  - id: wf-a
    version: 1
    steps:
      - name: step1
        type: shell
        config:
          run: echo hi
        on_success:
          name: hook
          type: workflow
          config:
            workflow: wf-b
  - id: wf-b
    version: 1
    steps:
      - name: step2
        type: shell
        config:
          run: echo hi
        on_success:
          name: hook2
          type: workflow
          config:
            workflow: wf-a
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("Circular workflow reference"),
        "Expected circular reference from hooks, got: {err}"
    );
}

#[test]
fn workflow_ref_in_ensure_hook_detected_for_cycles() {
    let yaml = r#"
workflows:
  - id: wf-x
    version: 1
    steps:
      - name: step1
        type: shell
        config:
          run: echo hi
        ensure:
          name: ensure-hook
          type: workflow
          config:
            workflow: wf-y
  - id: wf-y
    version: 1
    steps:
      - name: step2
        type: shell
        config:
          run: echo hi
        ensure:
          name: ensure-hook2
          type: workflow
          config:
            workflow: wf-x
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("Circular workflow reference"),
        "Expected circular reference from ensure hooks, got: {err}"
    );
}

#[test]
fn workflow_ref_in_parallel_block_detected_for_cycles() {
    let yaml = r#"
workflows:
  - id: wf-p
    version: 1
    steps:
      - name: par
        parallel:
          - name: call-q
            type: workflow
            config:
              workflow: wf-q
  - id: wf-q
    version: 1
    steps:
      - name: par2
        parallel:
          - name: call-p
            type: workflow
            config:
              workflow: wf-p
"#;
    let result = load_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("Circular workflow reference"),
        "Expected circular reference from parallel blocks, got: {err}"
    );
}

// --- Compiler error paths ---

#[test]
fn unknown_step_type_returns_compilation_error() {
    let yaml = r#"
workflow:
  id: unknown-type-wf
  version: 1
  steps:
    - name: bad-step
      type: terraform
      config:
        run: plan
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("Unknown step type") && err.contains("terraform"),
        "Expected unknown step type error, got: {err}"
    );
}

// --- lib.rs error paths ---

#[test]
fn load_workflow_from_nonexistent_file_returns_io_error() {
    let path = std::path::Path::new("/tmp/nonexistent_wfe_test_file.yaml");
    let result = wfe_yaml::load_workflow(path, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("IO error") || err.contains("No such file"),
        "Expected IO error, got: {err}"
    );
}
