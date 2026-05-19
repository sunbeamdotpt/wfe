use std::collections::HashMap;
use std::time::Duration;

use wfe_core::models::error_behavior::ErrorBehavior;
use wfe_yaml::{load_single_workflow_from_str, load_workflow_from_str};

#[test]
fn single_step_produces_one_workflow_step() {
    let yaml = r#"
workflow:
  id: single
  version: 1
  steps:
    - name: hello
      type: shell
      config:
        run: echo hello
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    // The definition should have exactly 1 main step.
    let main_steps: Vec<_> = compiled
        .definition
        .steps
        .iter()
        .filter(|s| s.name.as_deref() == Some("hello"))
        .collect();
    assert_eq!(main_steps.len(), 1);
    assert_eq!(main_steps[0].id, 0);
}

#[test]
fn two_sequential_steps_wired_correctly() {
    let yaml = r#"
workflow:
  id: sequential
  version: 1
  steps:
    - name: step-a
      type: shell
      config:
        run: echo a
    - name: step-b
      type: shell
      config:
        run: echo b
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();

    let step_a = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("step-a"))
        .unwrap();
    let step_b = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("step-b"))
        .unwrap();

    // step-a should have an outcome pointing to step-b.
    assert_eq!(step_a.outcomes.len(), 1);
    assert_eq!(step_a.outcomes[0].next_step, step_b.id);
}

#[test]
fn parallel_block_produces_container_with_children() {
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
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();

    let container = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("parallel-group"))
        .unwrap();

    assert!(
        container.step_type.contains("SequenceStep"),
        "Container should be a SequenceStep, got: {}",
        container.step_type
    );
    assert_eq!(container.children.len(), 2);
}

#[test]
fn on_failure_creates_compensation_step() {
    let yaml = r#"
workflow:
  id: compensation-wf
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
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();

    let deploy = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("deploy"))
        .unwrap();

    assert!(deploy.compensation_step_id.is_some());
    assert_eq!(deploy.error_behavior, Some(ErrorBehavior::Compensate));

    let rollback = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("rollback"))
        .unwrap();

    assert_eq!(deploy.compensation_step_id, Some(rollback.id));
}

#[test]
fn error_behavior_maps_correctly() {
    let yaml = r#"
workflow:
  id: retry-wf
  version: 1
  error_behavior:
    type: retry
    interval: 5s
    max_retries: 10
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
      error_behavior:
        type: suspend
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();

    assert_eq!(
        compiled.definition.default_error_behavior,
        ErrorBehavior::Retry {
            interval: Duration::from_secs(5),
            max_retries: 10,
        }
    );

    let step = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("step1"))
        .unwrap();
    assert_eq!(step.error_behavior, Some(ErrorBehavior::Suspend));
}

#[test]
fn anchors_compile_correctly() {
    let yaml = r#"
workflow:
  id: anchor-wf
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
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();

    // Should have 2 main steps + factories.
    let build_step = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("build"))
        .unwrap();
    let test_step = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("test"))
        .unwrap();

    // Both should have step_config.
    assert!(build_step.step_config.is_some());
    assert!(test_step.step_config.is_some());

    // Build should wire to test.
    assert_eq!(build_step.outcomes.len(), 1);
    assert_eq!(build_step.outcomes[0].next_step, test_step.id);

    // Test uses the same config via alias - shell should be bash.
    let test_config: wfe_yaml::executors::shell::ShellConfig =
        serde_json::from_value(test_step.step_config.clone().unwrap()).unwrap();
    assert_eq!(test_config.run, "cargo build");
    assert_eq!(
        test_config.shell, "bash",
        "shell should be inherited from YAML anchor alias"
    );
}

#[test]
fn on_success_creates_step_wired_after_main() {
    let yaml = r#"
workflow:
  id: success-hook-wf
  version: 1
  steps:
    - name: build
      type: shell
      config:
        run: cargo build
      on_success:
        name: notify
        type: shell
        config:
          run: echo "build succeeded"
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();

    let build = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("build"))
        .unwrap();
    let notify = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("notify"))
        .unwrap();

    // build should have an outcome pointing to notify with label "success".
    assert_eq!(build.outcomes.len(), 1);
    assert_eq!(build.outcomes[0].next_step, notify.id);
    assert_eq!(build.outcomes[0].label.as_deref(), Some("success"));
}

#[test]
fn ensure_creates_step_wired_after_main() {
    let yaml = r#"
workflow:
  id: ensure-wf
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
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();

    let deploy = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("deploy"))
        .unwrap();
    let cleanup = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("cleanup"))
        .unwrap();

    // deploy should have an outcome pointing to cleanup with label "ensure".
    assert_eq!(deploy.outcomes.len(), 1);
    assert_eq!(deploy.outcomes[0].next_step, cleanup.id);
    assert_eq!(deploy.outcomes[0].label.as_deref(), Some("ensure"));
}

#[test]
fn ensure_not_wired_when_on_success_present() {
    let yaml = r#"
workflow:
  id: both-hooks-wf
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
      ensure:
        name: cleanup
        type: shell
        config:
          run: cleanup.sh
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();

    let deploy = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("deploy"))
        .unwrap();

    // When on_success is present, ensure should NOT add another outcome to the main step.
    // Only the on_success outcome should be there.
    assert_eq!(deploy.outcomes.len(), 1);
    assert_eq!(deploy.outcomes[0].label.as_deref(), Some("success"));
}

#[test]
fn error_behavior_terminate_maps_correctly() {
    let yaml = r#"
workflow:
  id: terminate-wf
  version: 1
  error_behavior:
    type: terminate
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert_eq!(
        compiled.definition.default_error_behavior,
        ErrorBehavior::Terminate
    );
}

#[test]
fn error_behavior_compensate_maps_correctly() {
    let yaml = r#"
workflow:
  id: compensate-wf
  version: 1
  error_behavior:
    type: compensate
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert_eq!(
        compiled.definition.default_error_behavior,
        ErrorBehavior::Compensate
    );
}

#[test]
fn error_behavior_retry_defaults() {
    // retry with no interval or max_retries should use defaults.
    let yaml = r#"
workflow:
  id: retry-defaults-wf
  version: 1
  error_behavior:
    type: retry
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert_eq!(
        compiled.definition.default_error_behavior,
        ErrorBehavior::Retry {
            interval: Duration::from_secs(60),
            max_retries: 3,
        }
    );
}

#[test]
fn error_behavior_retry_with_minute_interval() {
    let yaml = r#"
workflow:
  id: retry-min-wf
  version: 1
  error_behavior:
    type: retry
    interval: 2m
    max_retries: 5
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert_eq!(
        compiled.definition.default_error_behavior,
        ErrorBehavior::Retry {
            interval: Duration::from_millis(2 * 60 * 1000),
            max_retries: 5,
        }
    );
}

#[test]
fn error_behavior_retry_with_raw_number_interval() {
    // When interval is a raw number (no suffix), it is treated as milliseconds.
    let yaml = r#"
workflow:
  id: retry-raw-wf
  version: 1
  error_behavior:
    type: retry
    interval: "500"
    max_retries: 2
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert_eq!(
        compiled.definition.default_error_behavior,
        ErrorBehavior::Retry {
            interval: Duration::from_millis(500),
            max_retries: 2,
        }
    );
}

#[test]
fn unknown_error_behavior_returns_error() {
    let yaml = r#"
workflow:
  id: bad-eb-wf
  version: 1
  error_behavior:
    type: explode
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result {
        Err(e) => e.to_string(),
        Ok(_) => panic!("expected error"),
    };
    assert!(
        err.contains("explode"),
        "Error should mention the invalid type, got: {err}"
    );
}

#[test]
fn parallel_block_children_have_step_configs() {
    let yaml = r#"
workflow:
  id: parallel-config-wf
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
        - name: task-c
          type: shell
          config:
            run: echo c
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();

    let container = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("parallel-group"))
        .unwrap();
    assert_eq!(container.children.len(), 3);

    // Each child should have a step_config.
    for child_name in &["task-a", "task-b", "task-c"] {
        let child = compiled
            .definition
            .steps
            .iter()
            .find(|s| s.name.as_deref() == Some(*child_name))
            .unwrap();
        assert!(
            child.step_config.is_some(),
            "Child {child_name} should have step_config"
        );
    }

    // Factories should include entries for all 3 children.
    assert!(compiled.step_factories.len() >= 3);
}

#[test]
fn step_config_serializes_shell_config() {
    let yaml = r#"
workflow:
  id: config-ser-wf
  version: 1
  steps:
    - name: build
      type: shell
      config:
        run: cargo build
        shell: bash
        timeout: 30s
        env:
          RUST_LOG: debug
        working_dir: /tmp
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();

    let step = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("build"))
        .unwrap();

    let config: wfe_yaml::executors::shell::ShellConfig =
        serde_json::from_value(step.step_config.clone().unwrap()).unwrap();
    assert_eq!(config.run, "cargo build");
    assert_eq!(config.shell, "bash");
    assert_eq!(config.timeout_ms, Some(30_000));
    assert_eq!(config.env.get("RUST_LOG").unwrap(), "debug");
    assert_eq!(config.working_dir.as_deref(), Some("/tmp"));
}

#[test]
fn config_file_field_generates_run_command() {
    let yaml = r#"
workflow:
  id: file-wf
  version: 1
  steps:
    - name: run-script
      type: shell
      config:
        file: my_script.sh
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    let step = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("run-script"))
        .unwrap();

    let config: wfe_yaml::executors::shell::ShellConfig =
        serde_json::from_value(step.step_config.clone().unwrap()).unwrap();
    assert_eq!(config.run, "sh my_script.sh");
}

#[test]
fn default_shell_is_sh() {
    let yaml = r#"
workflow:
  id: default-shell-wf
  version: 1
  steps:
    - name: step1
      type: shell
      config:
        run: echo hello
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    let step = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("step1"))
        .unwrap();

    let config: wfe_yaml::executors::shell::ShellConfig =
        serde_json::from_value(step.step_config.clone().unwrap()).unwrap();
    assert_eq!(config.shell, "sh", "Default shell should be 'sh'");
}

#[test]
fn on_failure_factory_is_registered() {
    let yaml = r#"
workflow:
  id: factory-wf
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
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();

    // Should have factories for both deploy and rollback.
    let has_deploy = compiled
        .step_factories
        .iter()
        .any(|(key, _)| key.contains("deploy"));
    let has_rollback = compiled
        .step_factories
        .iter()
        .any(|(key, _)| key.contains("rollback"));
    assert!(has_deploy, "Should have factory for deploy step");
    assert!(has_rollback, "Should have factory for rollback step");
}

#[test]
fn on_success_factory_is_registered() {
    let yaml = r#"
workflow:
  id: success-factory-wf
  version: 1
  steps:
    - name: build
      type: shell
      config:
        run: cargo build
      on_success:
        name: notify
        type: shell
        config:
          run: echo ok
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();

    let has_notify = compiled
        .step_factories
        .iter()
        .any(|(key, _)| key.contains("notify"));
    assert!(has_notify, "Should have factory for on_success step");
}

#[test]
fn ensure_factory_is_registered() {
    let yaml = r#"
workflow:
  id: ensure-factory-wf
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
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();

    let has_cleanup = compiled
        .step_factories
        .iter()
        .any(|(key, _)| key.contains("cleanup"));
    assert!(has_cleanup, "Should have factory for ensure step");
}

#[test]
fn parallel_with_error_behavior_on_container() {
    let yaml = r#"
workflow:
  id: parallel-eb-wf
  version: 1
  steps:
    - name: parallel-group
      error_behavior:
        type: terminate
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
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();

    let container = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("parallel-group"))
        .unwrap();
    assert_eq!(container.error_behavior, Some(ErrorBehavior::Terminate));
}

#[test]
fn sequential_wiring_with_hooks() {
    // When step A has on_success hook, the hook should wire to step B.
    let yaml = r#"
workflow:
  id: hook-wiring-wf
  version: 1
  steps:
    - name: step-a
      type: shell
      config:
        run: echo a
      on_success:
        name: hook-a
        type: shell
        config:
          run: echo hook
    - name: step-b
      type: shell
      config:
        run: echo b
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();

    let step_a = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("step-a"))
        .unwrap();
    let hook_a = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("hook-a"))
        .unwrap();
    let step_b = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("step-b"))
        .unwrap();

    // step-a -> hook-a (via on_success outcome).
    assert_eq!(step_a.outcomes[0].next_step, hook_a.id);
    // hook-a -> step-b (sequential wiring).
    assert_eq!(hook_a.outcomes.len(), 1);
    assert_eq!(hook_a.outcomes[0].next_step, step_b.id);
}

#[test]
fn workflow_description_is_set() {
    let yaml = r#"
workflow:
  id: desc-wf
  version: 1
  description: "A test workflow"
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert_eq!(
        compiled.definition.description.as_deref(),
        Some("A test workflow")
    );
}

#[test]
fn missing_config_section_returns_error() {
    let yaml = r#"
workflow:
  id: no-config-wf
  version: 1
  steps:
    - name: bad-step
      type: shell
"#;
    let result = load_single_workflow_from_str(yaml, &HashMap::new());
    assert!(result.is_err());
    let err = match result {
        Err(e) => e.to_string(),
        Ok(_) => panic!("expected error"),
    };
    assert!(
        err.contains("config"),
        "Error should mention missing config, got: {err}"
    );
}

// --- Workflow step compilation tests ---

#[test]
fn workflow_step_compiles_correctly() {
    let yaml = r#"
workflow:
  id: parent-wf
  version: 1
  steps:
    - name: run-child
      type: workflow
      config:
        workflow: child-wf
        workflow_version: 3
      outputs:
        - name: result
        - name: status
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();

    let step = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("run-child"))
        .unwrap();

    assert!(step.step_type.contains("workflow"));
    assert!(step.step_config.is_some());

    // Verify the serialized config contains the workflow_id and version.
    let config: serde_json::Value = step.step_config.clone().unwrap();
    assert_eq!(config["workflow_id"].as_str(), Some("child-wf"));
    assert_eq!(config["version"].as_u64(), Some(3));
    assert_eq!(config["output_keys"].as_array().unwrap().len(), 2);
}

#[test]
fn workflow_step_version_defaults_to_1() {
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
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();

    let step = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("run-child"))
        .unwrap();

    let config: serde_json::Value = step.step_config.clone().unwrap();
    assert_eq!(config["version"].as_u64(), Some(1));
}

#[test]
fn workflow_step_factory_is_registered() {
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
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();

    let has_workflow_factory = compiled
        .step_factories
        .iter()
        .any(|(key, _)| key.contains("workflow") && key.contains("run-child"));
    assert!(
        has_workflow_factory,
        "Should have factory for workflow step"
    );
}

#[test]
fn compile_multi_workflow_file() {
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
    let workflows = load_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert_eq!(workflows.len(), 2);
    assert_eq!(workflows[0].definition.id, "build");
    assert_eq!(workflows[1].definition.id, "test");
}

#[test]
fn compile_multi_workflow_with_cross_references() {
    let yaml = r#"
workflows:
  - id: pipeline
    version: 1
    steps:
      - name: run-build
        type: workflow
        config:
          workflow: build
  - id: build
    version: 1
    steps:
      - name: compile
        type: shell
        config:
          run: cargo build
"#;
    let workflows = load_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert_eq!(workflows.len(), 2);

    // The pipeline workflow should have a workflow step.
    let pipeline = &workflows[0];
    let step = pipeline
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("run-build"))
        .unwrap();
    assert!(step.step_type.contains("workflow"));
}

#[test]
fn workflow_step_with_mixed_steps() {
    let yaml = r#"
workflow:
  id: mixed-wf
  version: 1
  steps:
    - name: setup
      type: shell
      config:
        run: echo setup
    - name: run-child
      type: workflow
      config:
        workflow: child-wf
    - name: cleanup
      type: shell
      config:
        run: echo cleanup
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();

    // Should have 3 main steps.
    let step_names: Vec<_> = compiled
        .definition
        .steps
        .iter()
        .filter_map(|s| s.name.as_deref())
        .collect();
    assert!(step_names.contains(&"setup"));
    assert!(step_names.contains(&"run-child"));
    assert!(step_names.contains(&"cleanup"));

    // setup -> run-child -> cleanup wiring.
    let setup = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("setup"))
        .unwrap();
    let run_child = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("run-child"))
        .unwrap();
    let cleanup = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("cleanup"))
        .unwrap();

    assert_eq!(setup.outcomes[0].next_step, run_child.id);
    assert_eq!(run_child.outcomes[0].next_step, cleanup.id);
}

/// Regression test: SubWorkflowStep must actually wait for child completion,
/// not return next() immediately. The compiled factory must produce a real
/// SubWorkflowStep (from wfe-core), not a placeholder.
#[tokio::test]
async fn workflow_step_factory_produces_real_sub_workflow_step() {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Mutex;
    use wfe_core::models::{ExecutionPointer, WorkflowInstance, WorkflowStep as WfStep};
    use wfe_core::traits::step::{HostContext, StepExecutionContext};

    let yaml = r#"
workflows:
  - id: child
    version: 1
    steps:
      - name: do-work
        type: shell
        config:
          run: echo done

  - id: parent
    version: 1
    steps:
      - name: run-child
        type: workflow
        config:
          workflow: child
"#;
    let config = HashMap::new();
    let workflows = load_workflow_from_str(yaml, &config).unwrap();

    // Find the parent workflow's factory for the "run-child" step
    let parent = workflows
        .iter()
        .find(|w| w.definition.id == "parent")
        .unwrap();
    let factory_key = parent
        .step_factories
        .iter()
        .find(|(k, _)| k.contains("run-child"))
        .map(|(k, _)| k.clone())
        .expect("run-child factory should exist");

    // Create a step from the factory
    let factory = &parent
        .step_factories
        .iter()
        .find(|(k, _)| *k == factory_key)
        .unwrap()
        .1;
    let mut step = factory();

    // Mock host context that records the start_workflow call
    struct MockHost {
        called: Mutex<bool>,
    }
    impl HostContext for MockHost {
        fn start_workflow(
            &self,
            _def: &str,
            _ver: u32,
            _data: serde_json::Value,
            _parent_root_workflow_id: Option<String>,
        ) -> Pin<Box<dyn Future<Output = wfe_core::Result<String>> + Send + '_>> {
            *self.called.lock().unwrap() = true;
            Box::pin(async { Ok("child-instance-id".to_string()) })
        }
    }

    let host = MockHost {
        called: Mutex::new(false),
    };
    let pointer = ExecutionPointer::new(0);
    let wf_step = WfStep::new(0, &factory_key);
    let workflow = WorkflowInstance::new("parent", 1, serde_json::json!({}));
    let ctx = StepExecutionContext {
        definition: None,
        item: None,
        execution_pointer: &pointer,
        persistence_data: None,
        step: &wf_step,
        workflow: &workflow,
        cancellation_token: tokio_util::sync::CancellationToken::new(),
        host_context: Some(&host),
        log_sink: None,
        artifact_store: None,
        artifact_volume: None,
        artifact_package: None,
    };

    let result = step.run(&ctx).await.unwrap();

    // THE KEY ASSERTION: must NOT proceed immediately.
    // It must return wait_for_event so the parent waits for the child.
    assert!(
        !result.proceed,
        "SubWorkflowStep must NOT proceed immediately — it should wait for child completion"
    );
    assert_eq!(
        result.event_name.as_deref(),
        Some("wfe.workflow.completed"),
        "SubWorkflowStep must wait for wfe.workflow.completed event"
    );
    assert!(
        *host.called.lock().unwrap(),
        "SubWorkflowStep must call host_context.start_workflow()"
    );
}

// --- Condition compilation tests ---

#[test]
fn compile_simple_condition_into_step_condition() {
    let yaml = r#"
workflow:
  id: cond-compile
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
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    let step = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("deploy"))
        .unwrap();

    assert!(step.when.is_some(), "Step should have a when condition");
    match step.when.as_ref().unwrap() {
        wfe_core::models::StepCondition::Comparison(cmp) => {
            assert_eq!(cmp.field, ".inputs.enabled");
            assert_eq!(cmp.operator, wfe_core::models::ComparisonOp::Equals);
            assert_eq!(cmp.value, Some(serde_json::json!(true)));
        }
        other => panic!("Expected Comparison, got: {other:?}"),
    }
}

#[test]
fn compile_nested_condition() {
    let yaml = r#"
workflow:
  id: nested-cond-compile
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
          - not:
              field: .inputs.skip
              equals: true
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    let step = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("deploy"))
        .unwrap();

    assert!(step.when.is_some());
    match step.when.as_ref().unwrap() {
        wfe_core::models::StepCondition::All(children) => {
            assert_eq!(children.len(), 2);
            // First child: comparison
            match &children[0] {
                wfe_core::models::StepCondition::Comparison(cmp) => {
                    assert_eq!(cmp.field, ".inputs.count");
                    assert_eq!(cmp.operator, wfe_core::models::ComparisonOp::Gt);
                    assert_eq!(cmp.value, Some(serde_json::json!(5)));
                }
                other => panic!("Expected Comparison, got: {other:?}"),
            }
            // Second child: not
            match &children[1] {
                wfe_core::models::StepCondition::Not(inner) => match inner.as_ref() {
                    wfe_core::models::StepCondition::Comparison(cmp) => {
                        assert_eq!(cmp.field, ".inputs.skip");
                        assert_eq!(cmp.operator, wfe_core::models::ComparisonOp::Equals);
                    }
                    other => panic!("Expected Comparison inside Not, got: {other:?}"),
                },
                other => panic!("Expected Not, got: {other:?}"),
            }
        }
        other => panic!("Expected All, got: {other:?}"),
    }
}

#[test]
fn step_without_when_has_none_condition() {
    let yaml = r#"
workflow:
  id: no-when-compile
  version: 1
  steps:
    - name: step1
      type: shell
      config:
        run: echo hi
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    let step = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("step1"))
        .unwrap();
    assert!(step.when.is_none());
}

#[test]
fn compile_all_comparison_operators() {
    use wfe_core::models::ComparisonOp;

    let ops = vec![
        ("equals: 42", ComparisonOp::Equals),
        ("not_equals: foo", ComparisonOp::NotEquals),
        ("gt: 10", ComparisonOp::Gt),
        ("gte: 10", ComparisonOp::Gte),
        ("lt: 100", ComparisonOp::Lt),
        ("lte: 100", ComparisonOp::Lte),
        ("contains: needle", ComparisonOp::Contains),
        ("is_null: true", ComparisonOp::IsNull),
        ("is_not_null: true", ComparisonOp::IsNotNull),
    ];

    for (op_yaml, expected_op) in ops {
        let yaml = format!(
            r#"
workflow:
  id: op-test
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
        let compiled = load_single_workflow_from_str(&yaml, &HashMap::new())
            .unwrap_or_else(|e| panic!("Failed to compile with {op_yaml}: {e}"));
        let step = compiled
            .definition
            .steps
            .iter()
            .find(|s| s.name.as_deref() == Some("step1"))
            .unwrap();

        match step.when.as_ref().unwrap() {
            wfe_core::models::StepCondition::Comparison(cmp) => {
                assert_eq!(cmp.operator, expected_op, "Operator mismatch for {op_yaml}");
            }
            other => panic!("Expected Comparison for {op_yaml}, got: {other:?}"),
        }
    }
}

#[test]
fn compile_condition_on_parallel_container() {
    let yaml = r#"
workflow:
  id: parallel-cond
  version: 1
  steps:
    - name: parallel-group
      when:
        field: .inputs.run_parallel
        equals: true
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
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    let container = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("parallel-group"))
        .unwrap();

    assert!(
        container.when.is_some(),
        "Parallel container should have when condition"
    );
}


#[test]
#[cfg(all(feature = "buildkit", feature = "containerd", feature = "kubernetes"))]
fn compile_proxy_workflow() {
    let yaml = std::fs::read_to_string("../../proxy/workflows.yaml").unwrap();
    let config = std::collections::HashMap::new();
    let workflows = wfe_yaml::load_workflow_from_str(&yaml, &config).unwrap();
    assert_eq!(workflows.len(), 1);
    let compiled = &workflows[0];
    assert_eq!(compiled.definition.id, "proxy-ci");
    assert_eq!(compiled.definition.steps.len(), 4);
    assert_eq!(compiled.step_factories.len(), 4);
}
