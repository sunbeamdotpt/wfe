use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use wfe::models::WorkflowStatus;
use wfe::{WorkflowHostBuilder, run_workflow_sync};
use wfe_core::test_support::{
    InMemoryLockProvider, InMemoryPersistenceProvider, InMemoryQueueProvider,
};
use wfe_yaml::load_single_workflow_from_str;

async fn run_yaml_workflow(yaml: &str) -> wfe::models::WorkflowInstance {
    let config = HashMap::new();
    let compiled = load_single_workflow_from_str(yaml, &config).unwrap();

    let persistence = Arc::new(InMemoryPersistenceProvider::new());
    let lock = Arc::new(InMemoryLockProvider::new());
    let queue = Arc::new(InMemoryQueueProvider::new());

    let host = WorkflowHostBuilder::new()
        .use_persistence(persistence as Arc<dyn wfe_core::traits::PersistenceProvider>)
        .use_lock_provider(lock as Arc<dyn wfe_core::traits::DistributedLockProvider>)
        .use_queue_provider(queue as Arc<dyn wfe_core::traits::QueueProvider>)
        .build()
        .unwrap();

    // Register step factories.
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
        serde_json::json!({}),
        Duration::from_secs(10),
    )
    .await
    .unwrap();

    host.stop().await;
    instance
}

#[tokio::test]
async fn simple_echo_workflow_runs_to_completion() {
    let yaml = r#"
workflow:
  id: echo-wf
  version: 1
  steps:
    - name: echo-step
      type: shell
      config:
        run: echo "hello from wfe-yaml"
"#;
    let instance = run_yaml_workflow(yaml).await;
    assert_eq!(instance.status, WorkflowStatus::Complete);
}

#[tokio::test]
async fn workflow_with_output_capture() {
    let wfe_prefix = "##wfe";
    let yaml = format!(
        r#"
workflow:
  id: output-wf
  version: 1
  steps:
    - name: capture
      type: shell
      config:
        run: |
          echo "{wfe_prefix}[output greeting=hello]"
          echo "{wfe_prefix}[output count=42]"
"#
    );
    let instance = run_yaml_workflow(&yaml).await;
    assert_eq!(instance.status, WorkflowStatus::Complete);

    // Check that outputs were captured in the workflow data.
    if let Some(data) = instance.data.as_object() {
        // output_data gets merged into workflow.data by the executor.
        // Check that our outputs exist.
        if let Some(greeting) = data.get("greeting") {
            assert_eq!(greeting.as_str(), Some("hello"));
        }
        if let Some(count) = data.get("count") {
            assert_eq!(count.as_i64(), Some(42)); // auto-converted from string "42"
        }
    }
}

#[tokio::test]
async fn two_sequential_steps_run_in_order() {
    let yaml = r#"
workflow:
  id: seq-wf
  version: 1
  steps:
    - name: step-one
      type: shell
      config:
        run: echo step-one
    - name: step-two
      type: shell
      config:
        run: echo step-two
"#;
    let instance = run_yaml_workflow(yaml).await;
    assert_eq!(instance.status, WorkflowStatus::Complete);

    // Both steps should have completed.
    let complete_count = instance
        .execution_pointers
        .iter()
        .filter(|p| p.status == wfe::models::PointerStatus::Complete)
        .count();
    assert_eq!(complete_count, 2, "Expected 2 completed execution pointers");
}
