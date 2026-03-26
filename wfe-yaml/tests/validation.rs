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

// --- Condition validation tests ---

#[test]
fn condition_field_exists_in_inputs_ok() {
    let yaml = r#"
workflow:
  id: cond-input-ok
  version: 1
  inputs:
    enabled: bool
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
      when:
        field: .inputs.enabled
        equals: true
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_ok(), "Field path to known input should pass, got: {:?}", result.err());
}

#[test]
fn condition_field_exists_in_outputs_ok() {
    let yaml = r#"
workflow:
  id: cond-output-ok
  version: 1
  outputs:
    result: string
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
      outputs:
        - name: result
      when:
        field: .outputs.result
        equals: success
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_ok(), "Field path to known output should pass, got: {:?}", result.err());
}

#[test]
fn condition_field_missing_input_returns_error() {
    let yaml = r#"
workflow:
  id: cond-bad-input
  version: 1
  inputs:
    name: string
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
      when:
        field: .inputs.nonexistent
        equals: foo
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("unknown input field") && err.contains("nonexistent"),
        "Expected unknown input field error, got: {err}"
    );
}

#[test]
fn condition_field_missing_output_returns_error() {
    let yaml = r#"
workflow:
  id: cond-bad-output
  version: 1
  outputs:
    result: string
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
      outputs:
        - name: result
      when:
        field: .outputs.missing
        equals: bar
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("unknown output field") && err.contains("missing"),
        "Expected unknown output field error, got: {err}"
    );
}

#[test]
fn condition_gt_on_string_returns_type_error() {
    let yaml = r#"
workflow:
  id: cond-type-err
  version: 1
  inputs:
    name: string
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
      when:
        field: .inputs.name
        gt: 5
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("gt/gte/lt/lte") && err.contains("number/integer"),
        "Expected type mismatch error, got: {err}"
    );
}

#[test]
fn condition_gt_on_number_passes() {
    let yaml = r#"
workflow:
  id: cond-gt-num
  version: 1
  inputs:
    count: number
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
      when:
        field: .inputs.count
        gt: 5
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_ok(), "gt on number should pass, got: {:?}", result.err());
}

#[test]
fn condition_contains_on_bool_returns_type_error() {
    let yaml = r#"
workflow:
  id: cond-contains-bool
  version: 1
  inputs:
    active: bool
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
      when:
        field: .inputs.active
        contains: true
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("contains") && err.contains("string/list"),
        "Expected type mismatch error for contains, got: {err}"
    );
}

#[test]
fn condition_contains_on_string_passes() {
    let yaml = r#"
workflow:
  id: cond-contains-str
  version: 1
  inputs:
    name: string
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
      when:
        field: .inputs.name
        contains: needle
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_ok(), "contains on string should pass, got: {:?}", result.err());
}

#[test]
fn condition_contains_on_list_passes() {
    let yaml = r#"
workflow:
  id: cond-contains-list
  version: 1
  inputs:
    tags: "list<string>"
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
      when:
        field: .inputs.tags
        contains: release
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_ok(), "contains on list should pass, got: {:?}", result.err());
}

#[test]
fn condition_is_null_on_non_optional_returns_error() {
    let yaml = r#"
workflow:
  id: cond-null-nonopt
  version: 1
  inputs:
    name: string
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
      when:
        field: .inputs.name
        is_null: true
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("is_null/is_not_null") && err.contains("optional"),
        "Expected type mismatch error for is_null, got: {err}"
    );
}

#[test]
fn condition_is_null_on_optional_passes() {
    let yaml = r#"
workflow:
  id: cond-null-opt
  version: 1
  inputs:
    name: string?
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
      when:
        field: .inputs.name
        is_null: true
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_ok(), "is_null on optional should pass, got: {:?}", result.err());
}

#[test]
fn unused_output_field_returns_error() {
    let yaml = r#"
workflow:
  id: unused-output
  version: 1
  outputs:
    result: string
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
        err.contains("never produced") && err.contains("result"),
        "Expected unused output error, got: {err}"
    );
}

#[test]
fn output_produced_by_step_passes() {
    let yaml = r#"
workflow:
  id: used-output
  version: 1
  outputs:
    result: string
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
      outputs:
        - name: result
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_ok(), "Output produced by step should pass, got: {:?}", result.err());
}

#[test]
fn no_outputs_schema_no_error() {
    // When there are no declared outputs, no "unused output" error should fire.
    let yaml = r#"
workflow:
  id: no-outputs
  version: 1
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_ok(), "No outputs schema should not cause error, got: {:?}", result.err());
}

#[test]
fn condition_on_schemaless_workflow_skips_field_validation() {
    // When no inputs/outputs are declared, field paths are not validated.
    let yaml = r#"
workflow:
  id: schemaless
  version: 1
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
      when:
        field: .inputs.anything
        equals: whatever
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(
        result.is_ok(),
        "Schemaless workflow should skip field validation, got: {:?}",
        result.err()
    );
}

#[test]
fn condition_invalid_field_path_segment_returns_error() {
    let yaml = r#"
workflow:
  id: bad-path
  version: 1
  inputs:
    x: string
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
      when:
        field: .data.x
        equals: foo
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("inputs") && err.contains("outputs"),
        "Expected error about invalid path segment, got: {err}"
    );
}

// --- Task file includes tests ---

#[test]
fn include_single_file() {
    let dir = tempfile::tempdir().unwrap();
    let child_path = dir.path().join("child.yaml");
    std::fs::write(
        &child_path,
        r#"
workflow:
  id: child-wf
  version: 1
  steps:
    - name: child-step
      type: shell
      config:
        run: echo child
"#,
    )
    .unwrap();

    let main_yaml = format!(
        r#"
include:
  - child.yaml
workflow:
  id: main-wf
  version: 1
  steps:
    - name: main-step
      type: shell
      config:
        run: echo main
"#
    );

    let main_path = dir.path().join("main.yaml");
    std::fs::write(&main_path, &main_yaml).unwrap();

    let result =
        wfe_yaml::load_workflow_with_includes(&main_yaml, &main_path, &HashMap::new());
    assert!(result.is_ok(), "Include single file should work, got: {:?}", result.err());
    let workflows = result.unwrap();
    assert_eq!(workflows.len(), 2);
    let ids: Vec<&str> = workflows.iter().map(|w| w.definition.id.as_str()).collect();
    assert!(ids.contains(&"main-wf"));
    assert!(ids.contains(&"child-wf"));
}

#[test]
fn include_multiple_files() {
    let dir = tempfile::tempdir().unwrap();

    std::fs::write(
        dir.path().join("a.yaml"),
        r#"
workflow:
  id: wf-a
  version: 1
  steps:
    - name: a-step
      type: shell
      config:
        run: echo a
"#,
    )
    .unwrap();

    std::fs::write(
        dir.path().join("b.yaml"),
        r#"
workflow:
  id: wf-b
  version: 1
  steps:
    - name: b-step
      type: shell
      config:
        run: echo b
"#,
    )
    .unwrap();

    let main_yaml = r#"
include:
  - a.yaml
  - b.yaml
workflow:
  id: main-wf
  version: 1
  steps:
    - name: main-step
      type: shell
      config:
        run: echo main
"#;

    let main_path = dir.path().join("main.yaml");
    std::fs::write(&main_path, main_yaml).unwrap();

    let result =
        wfe_yaml::load_workflow_with_includes(main_yaml, &main_path, &HashMap::new());
    assert!(result.is_ok(), "Include multiple files should work, got: {:?}", result.err());
    let workflows = result.unwrap();
    assert_eq!(workflows.len(), 3);
}

#[test]
fn include_with_override_main_takes_precedence() {
    let dir = tempfile::tempdir().unwrap();

    // Child defines wf with id "shared"
    std::fs::write(
        dir.path().join("child.yaml"),
        r#"
workflow:
  id: shared
  version: 1
  steps:
    - name: child-step
      type: shell
      config:
        run: echo child
"#,
    )
    .unwrap();

    // Main also defines wf with id "shared" — main should win
    let main_yaml = r#"
include:
  - child.yaml
workflow:
  id: shared
  version: 1
  steps:
    - name: main-step
      type: shell
      config:
        run: echo main
"#;

    let main_path = dir.path().join("main.yaml");
    std::fs::write(&main_path, main_yaml).unwrap();

    let result =
        wfe_yaml::load_workflow_with_includes(main_yaml, &main_path, &HashMap::new());
    assert!(result.is_ok(), "Override should work, got: {:?}", result.err());
    let workflows = result.unwrap();
    // Only 1 workflow since main takes precedence over included
    assert_eq!(workflows.len(), 1);
    assert_eq!(workflows[0].definition.id, "shared");
    // Verify it's the main's version
    let step_names: Vec<_> = workflows[0]
        .definition
        .steps
        .iter()
        .filter_map(|s| s.name.as_deref())
        .collect();
    assert!(
        step_names.contains(&"main-step"),
        "Main file should take precedence, got steps: {:?}",
        step_names
    );
}

#[test]
fn include_missing_file_returns_error() {
    let dir = tempfile::tempdir().unwrap();

    let main_yaml = r#"
include:
  - nonexistent.yaml
workflow:
  id: main-wf
  version: 1
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
"#;

    let main_path = dir.path().join("main.yaml");
    std::fs::write(&main_path, main_yaml).unwrap();

    let result =
        wfe_yaml::load_workflow_with_includes(main_yaml, &main_path, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("nonexistent") || err.contains("not found") || err.contains("No such file"),
        "Expected file not found error, got: {err}"
    );
}

#[test]
fn include_cycle_detection() {
    let dir = tempfile::tempdir().unwrap();

    // A includes B, B includes A
    let a_yaml = r#"
include:
  - b.yaml
workflow:
  id: wf-a
  version: 1
  steps:
    - name: a-step
      type: shell
      config:
        run: echo a
"#;

    let b_yaml = r#"
include:
  - a.yaml
workflow:
  id: wf-b
  version: 1
  steps:
    - name: b-step
      type: shell
      config:
        run: echo b
"#;

    std::fs::write(dir.path().join("a.yaml"), a_yaml).unwrap();
    std::fs::write(dir.path().join("b.yaml"), b_yaml).unwrap();

    let a_path = dir.path().join("a.yaml");

    let result =
        wfe_yaml::load_workflow_with_includes(a_yaml, &a_path, &HashMap::new());
    assert!(result.is_err());
    let err = match result { Err(e) => e.to_string(), Ok(_) => panic!("expected error") };
    assert!(
        err.contains("Circular include"),
        "Expected circular include error, got: {err}"
    );
}

#[test]
fn condition_equals_on_any_type_passes() {
    let yaml = r#"
workflow:
  id: cond-any-type
  version: 1
  inputs:
    name: string
    count: integer
    active: bool
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
      when:
        all:
          - field: .inputs.name
            equals: foo
          - field: .inputs.count
            equals: 42
          - field: .inputs.active
            equals: true
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_ok(), "equals should work on all types, got: {:?}", result.err());
}

#[test]
fn condition_gt_on_integer_passes() {
    let yaml = r#"
workflow:
  id: cond-gt-int
  version: 1
  inputs:
    count: integer
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
      when:
        field: .inputs.count
        gte: 10
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_ok(), "gte on integer should pass, got: {:?}", result.err());
}

#[test]
fn condition_on_any_type_field_allows_all_operators() {
    // 'any' type should pass all operator checks
    let yaml = r#"
workflow:
  id: cond-any-ops
  version: 1
  inputs:
    data: any
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
      when:
        field: .inputs.data
        gt: 5
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_ok(), "any type should allow gt, got: {:?}", result.err());
}
