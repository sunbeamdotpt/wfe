use std::collections::HashMap;

use wfe_core::traits::step::StepBody;
use wfe_kubernetes::config::{ClusterConfig, KubernetesStepConfig};
use wfe_kubernetes::namespace;
use wfe_kubernetes::cleanup;
use wfe_kubernetes::client;

/// Path to the Lima sunbeam VM kubeconfig.
fn kubeconfig_path() -> String {
    let home = std::env::var("HOME").unwrap();
    format!("{home}/.lima/sunbeam/copied-from-guest/kubeconfig.yaml")
}

fn cluster_config() -> ClusterConfig {
    ClusterConfig {
        kubeconfig: Some(kubeconfig_path()),
        namespace_prefix: "wfe-test-".into(),
        ..Default::default()
    }
}

fn step_config(image: &str, run: &str) -> KubernetesStepConfig {
    KubernetesStepConfig {
        image: image.into(),
        command: None,
        run: Some(run.into()),
        env: HashMap::new(),
        working_dir: None,
        memory: None,
        cpu: None,
        timeout_ms: None,
        pull_policy: None,
        namespace: None,
    }
}

// ── Client ───────────────────────────────────────────────────────────

#[tokio::test]
async fn client_connects_to_cluster() {
    let config = cluster_config();
    let _client = client::create_client(&config).await.unwrap();
}

// ── Namespace ────────────────────────────────────────────────────────

#[tokio::test]
async fn namespace_create_and_delete() {
    let config = cluster_config();
    let client = client::create_client(&config).await.unwrap();
    let ns = "wfe-test-ns-lifecycle";

    namespace::ensure_namespace(&client, ns, "test-wf").await.unwrap();

    // Idempotent — creating again should succeed.
    namespace::ensure_namespace(&client, ns, "test-wf").await.unwrap();

    namespace::delete_namespace(&client, ns).await.unwrap();
}

// ── Step Execution ───────────────────────────────────────────────────

#[tokio::test]
async fn run_echo_job() {
    let config = cluster_config();
    let k8s_client = client::create_client(&config).await.unwrap();

    let step_cfg = step_config("alpine:3.18", "echo 'hello from k8s'");

    let mut step =
        wfe_kubernetes::KubernetesStep::new(step_cfg, config.clone(), k8s_client.clone());

    let ns = namespace::namespace_name(&config.namespace_prefix, "echo-test");

    // Build a minimal StepExecutionContext.
    let instance = wfe_core::models::WorkflowInstance::new("echo-wf", 1, serde_json::json!({}));
    let mut ws = wfe_core::models::WorkflowStep::new(0, "alpine-echo");
    ws.name = Some("echo-step".into());
    let pointer = wfe_core::models::ExecutionPointer::new(0);

    let ctx = wfe_core::traits::step::StepExecutionContext {
        item: None,
        execution_pointer: &pointer,
        persistence_data: None,
        step: &ws,
        workflow: &instance,
        cancellation_token: tokio_util::sync::CancellationToken::new(),
        host_context: None,
        log_sink: None,
    };

    let result = step.run(&ctx).await.unwrap();
    assert!(result.proceed);

    let output = result.output_data.unwrap();
    assert!(output["echo-step.stdout"]
        .as_str()
        .unwrap()
        .contains("hello from k8s"));
    assert_eq!(output["echo-step.exit_code"], 0);

    // Cleanup namespace.
    namespace::delete_namespace(&k8s_client, &ns).await.ok();
}

#[tokio::test]
async fn run_job_with_wfe_output() {
    let config = cluster_config();
    let k8s_client = client::create_client(&config).await.unwrap();

    let step_cfg = step_config(
        "alpine:3.18",
        r#"echo '##wfe[output version=1.2.3]' && echo '##wfe[output status=ok]'"#,
    );

    let mut step =
        wfe_kubernetes::KubernetesStep::new(step_cfg, config.clone(), k8s_client.clone());

    let ns = namespace::namespace_name(&config.namespace_prefix, "output-test");

    let instance =
        wfe_core::models::WorkflowInstance::new("output-wf", 1, serde_json::json!({}));
    let mut ws = wfe_core::models::WorkflowStep::new(0, "alpine-output");
    ws.name = Some("output-step".into());
    let pointer = wfe_core::models::ExecutionPointer::new(0);

    let ctx = wfe_core::traits::step::StepExecutionContext {
        item: None,
        execution_pointer: &pointer,
        persistence_data: None,
        step: &ws,
        workflow: &instance,
        cancellation_token: tokio_util::sync::CancellationToken::new(),
        host_context: None,
        log_sink: None,
    };

    let result = step.run(&ctx).await.unwrap();
    assert!(result.proceed);

    let output = result.output_data.unwrap();
    assert_eq!(output["version"], "1.2.3");
    assert_eq!(output["status"], "ok");

    namespace::delete_namespace(&k8s_client, &ns).await.ok();
}

#[tokio::test]
async fn run_job_with_env_vars() {
    let config = cluster_config();
    let k8s_client = client::create_client(&config).await.unwrap();

    let mut step_cfg = step_config("alpine:3.18", "echo \"##wfe[output greeting=$GREETING]\"");
    step_cfg.env.insert("GREETING".into(), "hello".into());

    let mut step =
        wfe_kubernetes::KubernetesStep::new(step_cfg, config.clone(), k8s_client.clone());

    let ns = namespace::namespace_name(&config.namespace_prefix, "env-test");

    let instance = wfe_core::models::WorkflowInstance::new(
        "env-wf",
        1,
        serde_json::json!({"app_name": "myapp"}),
    );
    let mut ws = wfe_core::models::WorkflowStep::new(0, "alpine-env");
    ws.name = Some("env-step".into());
    let pointer = wfe_core::models::ExecutionPointer::new(0);

    let ctx = wfe_core::traits::step::StepExecutionContext {
        item: None,
        execution_pointer: &pointer,
        persistence_data: None,
        step: &ws,
        workflow: &instance,
        cancellation_token: tokio_util::sync::CancellationToken::new(),
        host_context: None,
        log_sink: None,
    };

    let result = step.run(&ctx).await.unwrap();
    assert!(result.proceed);

    let output = result.output_data.unwrap();
    assert_eq!(output["greeting"], "hello");

    namespace::delete_namespace(&k8s_client, &ns).await.ok();
}

#[tokio::test]
async fn run_job_nonzero_exit_fails() {
    let config = cluster_config();
    let k8s_client = client::create_client(&config).await.unwrap();

    let step_cfg = step_config("alpine:3.18", "exit 1");

    let mut step =
        wfe_kubernetes::KubernetesStep::new(step_cfg, config.clone(), k8s_client.clone());

    let ns = namespace::namespace_name(&config.namespace_prefix, "fail-test");

    let instance = wfe_core::models::WorkflowInstance::new("fail-wf", 1, serde_json::json!({}));
    let mut ws = wfe_core::models::WorkflowStep::new(0, "alpine-fail");
    ws.name = Some("fail-step".into());
    let pointer = wfe_core::models::ExecutionPointer::new(0);

    let ctx = wfe_core::traits::step::StepExecutionContext {
        item: None,
        execution_pointer: &pointer,
        persistence_data: None,
        step: &ws,
        workflow: &instance,
        cancellation_token: tokio_util::sync::CancellationToken::new(),
        host_context: None,
        log_sink: None,
    };

    let result = step.run(&ctx).await;
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("exited with code"));

    namespace::delete_namespace(&k8s_client, &ns).await.ok();
}

#[tokio::test]
async fn run_job_with_timeout() {
    let config = cluster_config();
    let k8s_client = client::create_client(&config).await.unwrap();

    let mut step_cfg = step_config("alpine:3.18", "sleep 60");
    step_cfg.timeout_ms = Some(5_000); // 5 second timeout

    let mut step =
        wfe_kubernetes::KubernetesStep::new(step_cfg, config.clone(), k8s_client.clone());

    let ns = namespace::namespace_name(&config.namespace_prefix, "timeout-test");

    let instance =
        wfe_core::models::WorkflowInstance::new("timeout-wf", 1, serde_json::json!({}));
    let mut ws = wfe_core::models::WorkflowStep::new(0, "alpine-timeout");
    ws.name = Some("timeout-step".into());
    let pointer = wfe_core::models::ExecutionPointer::new(0);

    let ctx = wfe_core::traits::step::StepExecutionContext {
        item: None,
        execution_pointer: &pointer,
        persistence_data: None,
        step: &ws,
        workflow: &instance,
        cancellation_token: tokio_util::sync::CancellationToken::new(),
        host_context: None,
        log_sink: None,
    };

    let result = step.run(&ctx).await;
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("timed out"));

    namespace::delete_namespace(&k8s_client, &ns).await.ok();
}

// ── Cleanup ──────────────────────────────────────────────────────────

#[tokio::test]
async fn cleanup_stale_namespaces() {
    let config = cluster_config();
    let k8s_client = client::create_client(&config).await.unwrap();

    // Create a namespace that should survive (0 seconds old).
    let ns = "wfe-test-cleanup-survivor";
    namespace::ensure_namespace(&k8s_client, ns, "cleanup-test")
        .await
        .unwrap();

    // Attempt cleanup with a very long threshold — nothing should be deleted.
    let deleted = cleanup::cleanup_stale_namespaces(
        &k8s_client,
        "wfe-test-",
        std::time::Duration::from_secs(86400),
    )
    .await
    .unwrap();
    assert_eq!(deleted, 0);

    namespace::delete_namespace(&k8s_client, ns).await.ok();
}

#[tokio::test]
async fn delete_nonexistent_job_is_ok() {
    let config = cluster_config();
    let k8s_client = client::create_client(&config).await.unwrap();

    // Deleting a job that doesn't exist should succeed (404 is OK).
    cleanup::delete_job(&k8s_client, "default", "wfe-nonexistent-job-12345")
        .await
        .unwrap();
}
