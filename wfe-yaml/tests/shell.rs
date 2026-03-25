use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use wfe::models::WorkflowStatus;
use wfe::{WorkflowHostBuilder, run_workflow_sync};
use wfe_core::test_support::{
    InMemoryLockProvider, InMemoryPersistenceProvider, InMemoryQueueProvider,
};
use wfe_yaml::load_workflow_from_str;

async fn run_yaml_workflow_with_data(
    yaml: &str,
    data: serde_json::Value,
) -> wfe::models::WorkflowInstance {
    let config = HashMap::new();
    let compiled = load_workflow_from_str(yaml, &config).unwrap();

    let persistence = Arc::new(InMemoryPersistenceProvider::new());
    let lock = Arc::new(InMemoryLockProvider::new());
    let queue = Arc::new(InMemoryQueueProvider::new());

    let host = WorkflowHostBuilder::new()
        .use_persistence(persistence as Arc<dyn wfe_core::traits::PersistenceProvider>)
        .use_lock_provider(lock as Arc<dyn wfe_core::traits::DistributedLockProvider>)
        .use_queue_provider(queue as Arc<dyn wfe_core::traits::QueueProvider>)
        .build()
        .unwrap();

    for (key, factory) in compiled.step_factories {
        host.register_step_factory(&key, factory).await;
    }

    host.register_workflow_definition(compiled.definition.clone())
        .await;
    host.start().await.unwrap();

    let instance = run_workflow_sync(
        &host,
        &compiled.definition.id,
        compiled.definition.version,
        data,
        Duration::from_secs(10),
    )
    .await
    .unwrap();

    host.stop().await;
    instance
}

async fn run_yaml_workflow(yaml: &str) -> wfe::models::WorkflowInstance {
    run_yaml_workflow_with_data(yaml, serde_json::json!({})).await
}

#[tokio::test]
async fn simple_echo_captures_stdout() {
    let yaml = r#"
workflow:
  id: echo-capture-wf
  version: 1
  steps:
    - name: echo-step
      type: shell
      config:
        run: echo "hello world"
"#;
    let instance = run_yaml_workflow(yaml).await;
    assert_eq!(instance.status, WorkflowStatus::Complete);

    // stdout should be captured in workflow data.
    if let Some(data) = instance.data.as_object() {
        if let Some(stdout) = data.get("echo-step.stdout") {
            assert!(
                stdout.as_str().unwrap().contains("hello world"),
                "stdout should contain 'hello world', got: {}",
                stdout
            );
        }
    }
}

#[tokio::test]
async fn wfe_output_parsing() {
    let wfe_prefix = "##wfe";
    let yaml = format!(
        r#"
workflow:
  id: output-parse-wf
  version: 1
  steps:
    - name: output-step
      type: shell
      config:
        run: |
          echo "{wfe_prefix}[output greeting=hello]"
          echo "{wfe_prefix}[output count=42]"
          echo "{wfe_prefix}[output path=/usr/local/bin]"
"#
    );
    let instance = run_yaml_workflow(&yaml).await;
    assert_eq!(instance.status, WorkflowStatus::Complete);

    if let Some(data) = instance.data.as_object() {
        if let Some(greeting) = data.get("greeting") {
            assert_eq!(greeting.as_str(), Some("hello"));
        }
        if let Some(count) = data.get("count") {
            assert_eq!(count.as_str(), Some("42"));
        }
        if let Some(path) = data.get("path") {
            assert_eq!(path.as_str(), Some("/usr/local/bin"));
        }
    }
}

#[tokio::test]
async fn nonzero_exit_code_causes_failure() {
    let yaml = r#"
workflow:
  id: fail-wf
  version: 1
  error_behavior:
    type: terminate
  steps:
    - name: fail-step
      type: shell
      config:
        run: exit 1
"#;
    let instance = run_yaml_workflow(yaml).await;
    assert_eq!(
        instance.status,
        WorkflowStatus::Terminated,
        "Workflow should terminate on non-zero exit code"
    );
}

#[tokio::test]
async fn env_vars_from_config_injected() {
    let wfe_prefix = "##wfe";
    let yaml = format!(
        r#"
workflow:
  id: env-wf
  version: 1
  steps:
    - name: env-step
      type: shell
      config:
        run: echo "{wfe_prefix}[output my_var=$MY_VAR]"
        env:
          MY_VAR: custom_value
"#
    );
    let instance = run_yaml_workflow(&yaml).await;
    assert_eq!(instance.status, WorkflowStatus::Complete);

    if let Some(data) = instance.data.as_object() {
        if let Some(my_var) = data.get("my_var") {
            assert_eq!(my_var.as_str(), Some("custom_value"));
        }
    }
}

#[tokio::test]
async fn workflow_data_injected_as_env_vars() {
    let wfe_prefix = "##wfe";
    let yaml = format!(
        r#"
workflow:
  id: data-env-wf
  version: 1
  steps:
    - name: data-step
      type: shell
      config:
        run: echo "{wfe_prefix}[output result=$GREETING]"
"#
    );
    let instance = run_yaml_workflow_with_data(
        &yaml,
        serde_json::json!({"greeting": "hi there"}),
    )
    .await;
    assert_eq!(instance.status, WorkflowStatus::Complete);

    if let Some(data) = instance.data.as_object() {
        if let Some(result) = data.get("result") {
            assert_eq!(result.as_str(), Some("hi there"));
        }
    }
}

#[tokio::test]
async fn working_dir_is_respected() {
    let yaml = r#"
workflow:
  id: workdir-wf
  version: 1
  steps:
    - name: pwd-step
      type: shell
      config:
        run: pwd
        working_dir: /tmp
"#;
    let instance = run_yaml_workflow(yaml).await;
    assert_eq!(instance.status, WorkflowStatus::Complete);

    if let Some(data) = instance.data.as_object() {
        if let Some(stdout) = data.get("pwd-step.stdout") {
            let output = stdout.as_str().unwrap().trim();
            // On macOS, /tmp -> /private/tmp
            assert!(
                output == "/tmp" || output == "/private/tmp",
                "Expected /tmp or /private/tmp, got: {output}"
            );
        }
    }
}

#[tokio::test]
async fn shell_step_with_bash() {
    let yaml = r#"
workflow:
  id: bash-wf
  version: 1
  steps:
    - name: bash-step
      type: shell
      config:
        run: echo "using bash"
        shell: bash
"#;
    let instance = run_yaml_workflow(yaml).await;
    assert_eq!(instance.status, WorkflowStatus::Complete);
}
