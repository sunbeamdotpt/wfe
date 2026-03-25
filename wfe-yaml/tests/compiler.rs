use std::collections::HashMap;
use std::time::Duration;

use wfe_core::models::error_behavior::ErrorBehavior;
use wfe_yaml::load_workflow_from_str;

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
    let compiled = load_workflow_from_str(yaml, &HashMap::new()).unwrap();
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
    let compiled = load_workflow_from_str(yaml, &HashMap::new()).unwrap();

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
    let compiled = load_workflow_from_str(yaml, &HashMap::new()).unwrap();

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
    let compiled = load_workflow_from_str(yaml, &HashMap::new()).unwrap();

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
    let compiled = load_workflow_from_str(yaml, &HashMap::new()).unwrap();

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
    let compiled = load_workflow_from_str(yaml, &HashMap::new()).unwrap();

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
    assert_eq!(test_config.shell, "bash", "shell should be inherited from YAML anchor alias");
}
