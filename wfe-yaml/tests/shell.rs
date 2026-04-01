use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use wfe::models::WorkflowStatus;
use wfe::{WorkflowHostBuilder, run_workflow_sync};
use wfe_core::test_support::{
    InMemoryLockProvider, InMemoryPersistenceProvider, InMemoryQueueProvider,
};
use wfe_yaml::load_single_workflow_from_str;

async fn run_yaml_workflow_with_data(
    yaml: &str,
    data: serde_json::Value,
) -> wfe::models::WorkflowInstance {
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

/// A test LogSink that collects all chunks.
struct CollectingLogSink {
    chunks: tokio::sync::Mutex<Vec<wfe_core::traits::LogChunk>>,
}

impl CollectingLogSink {
    fn new() -> Self {
        Self { chunks: tokio::sync::Mutex::new(Vec::new()) }
    }

    async fn chunks(&self) -> Vec<wfe_core::traits::LogChunk> {
        self.chunks.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl wfe_core::traits::LogSink for CollectingLogSink {
    async fn write_chunk(&self, chunk: wfe_core::traits::LogChunk) {
        self.chunks.lock().await.push(chunk);
    }
}

/// Run a workflow with a LogSink to verify log streaming works end-to-end.
async fn run_yaml_workflow_with_log_sink(
    yaml: &str,
    log_sink: Arc<CollectingLogSink>,
) -> wfe::models::WorkflowInstance {
    let config = HashMap::new();
    let compiled = load_single_workflow_from_str(yaml, &config).unwrap();

    let persistence = Arc::new(InMemoryPersistenceProvider::new());
    let lock = Arc::new(InMemoryLockProvider::new());
    let queue = Arc::new(InMemoryQueueProvider::new());

    let host = WorkflowHostBuilder::new()
        .use_persistence(persistence as Arc<dyn wfe_core::traits::PersistenceProvider>)
        .use_lock_provider(lock as Arc<dyn wfe_core::traits::DistributedLockProvider>)
        .use_queue_provider(queue as Arc<dyn wfe_core::traits::QueueProvider>)
        .use_log_sink(log_sink as Arc<dyn wfe_core::traits::LogSink>)
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
        serde_json::json!({}),
        Duration::from_secs(10),
    )
    .await
    .unwrap();

    host.stop().await;
    instance
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
            assert_eq!(count.as_i64(), Some(42)); // auto-converted from string "42"
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

// ── LogSink regression tests ─────────────────────────────────────────

#[tokio::test]
async fn log_sink_receives_stdout_chunks() {
    let log_sink = Arc::new(CollectingLogSink::new());
    let yaml = r#"
workflow:
  id: logsink-stdout-wf
  version: 1
  steps:
    - name: echo-step
      type: shell
      config:
        run: echo "line one" && echo "line two"
"#;
    let instance = run_yaml_workflow_with_log_sink(yaml, log_sink.clone()).await;
    assert_eq!(instance.status, WorkflowStatus::Complete);

    let chunks = log_sink.chunks().await;
    assert!(chunks.len() >= 2, "expected at least 2 stdout chunks, got {}", chunks.len());

    let stdout_chunks: Vec<_> = chunks
        .iter()
        .filter(|c| c.stream == wfe_core::traits::LogStreamType::Stdout)
        .collect();
    assert!(stdout_chunks.len() >= 2, "expected at least 2 stdout chunks");

    let all_data: String = stdout_chunks.iter()
        .map(|c| String::from_utf8_lossy(&c.data).to_string())
        .collect();
    assert!(all_data.contains("line one"), "stdout should contain 'line one', got: {all_data}");
    assert!(all_data.contains("line two"), "stdout should contain 'line two', got: {all_data}");

    // Verify chunk metadata.
    for chunk in &stdout_chunks {
        assert!(!chunk.workflow_id.is_empty());
        assert_eq!(chunk.step_name, "echo-step");
    }
}

#[tokio::test]
async fn log_sink_receives_stderr_chunks() {
    let log_sink = Arc::new(CollectingLogSink::new());
    let yaml = r#"
workflow:
  id: logsink-stderr-wf
  version: 1
  steps:
    - name: err-step
      type: shell
      config:
        run: echo "stderr output" >&2
"#;
    let instance = run_yaml_workflow_with_log_sink(yaml, log_sink.clone()).await;
    assert_eq!(instance.status, WorkflowStatus::Complete);

    let chunks = log_sink.chunks().await;
    let stderr_chunks: Vec<_> = chunks
        .iter()
        .filter(|c| c.stream == wfe_core::traits::LogStreamType::Stderr)
        .collect();
    assert!(!stderr_chunks.is_empty(), "expected stderr chunks");

    let stderr_data: String = stderr_chunks.iter()
        .map(|c| String::from_utf8_lossy(&c.data).to_string())
        .collect();
    assert!(stderr_data.contains("stderr output"), "stderr should contain 'stderr output', got: {stderr_data}");
}

#[tokio::test]
async fn log_sink_captures_multi_step_workflow() {
    let log_sink = Arc::new(CollectingLogSink::new());
    let yaml = r#"
workflow:
  id: logsink-multi-wf
  version: 1
  steps:
    - name: step-a
      type: shell
      config:
        run: echo "from step a"
    - name: step-b
      type: shell
      config:
        run: echo "from step b"
"#;
    let instance = run_yaml_workflow_with_log_sink(yaml, log_sink.clone()).await;
    assert_eq!(instance.status, WorkflowStatus::Complete);

    let chunks = log_sink.chunks().await;
    let step_names: Vec<_> = chunks.iter().map(|c| c.step_name.as_str()).collect();
    assert!(step_names.contains(&"step-a"), "should have chunks from step-a");
    assert!(step_names.contains(&"step-b"), "should have chunks from step-b");
}

#[tokio::test]
async fn log_sink_not_configured_still_works() {
    // Without a log_sink, the buffered path should still work.
    let yaml = r#"
workflow:
  id: no-logsink-wf
  version: 1
  steps:
    - name: echo-step
      type: shell
      config:
        run: echo "no sink"
"#;
    let instance = run_yaml_workflow(yaml).await;
    assert_eq!(instance.status, WorkflowStatus::Complete);
    let data = instance.data.as_object().unwrap();
    assert!(data.get("echo-step.stdout").unwrap().as_str().unwrap().contains("no sink"));
}

// ── Security regression tests ────────────────────────────────────────

#[tokio::test]
async fn security_blocked_env_vars_not_injected() {
    // MEDIUM-22: Workflow data keys like "path" must NOT override PATH.
    let yaml = r#"
workflow:
  id: sec-env-wf
  version: 1
  steps:
    - name: check-path
      type: shell
      config:
        run: echo "$PATH"
"#;
    // Set a workflow data key "path" that would override PATH if not blocked.
    let instance = run_yaml_workflow_with_data(
        yaml,
        serde_json::json!({"path": "/attacker/bin"}),
    )
    .await;
    assert_eq!(instance.status, WorkflowStatus::Complete);

    let data = instance.data.as_object().unwrap();
    let stdout = data.get("check-path.stdout").unwrap().as_str().unwrap();
    // PATH should NOT contain /attacker/bin.
    assert!(
        !stdout.contains("/attacker/bin"),
        "PATH should not be overridden by workflow data, got: {stdout}"
    );
}

#[tokio::test]
async fn security_safe_env_vars_still_injected() {
    // Verify non-blocked keys still work after the security fix.
    let wfe_prefix = "##wfe";
    let yaml = format!(
        r#"
workflow:
  id: sec-safe-env-wf
  version: 1
  steps:
    - name: check-var
      type: shell
      config:
        run: echo "{wfe_prefix}[output val=$MY_CUSTOM_VAR]"
"#
    );
    let instance = run_yaml_workflow_with_data(
        &yaml,
        serde_json::json!({"my_custom_var": "works"}),
    )
    .await;
    assert_eq!(instance.status, WorkflowStatus::Complete);

    let data = instance.data.as_object().unwrap();
    assert_eq!(data.get("val").and_then(|v| v.as_str()), Some("works"));
}
