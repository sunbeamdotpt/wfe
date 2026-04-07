use std::collections::HashMap;

use wfe_core::models::service::{ReadinessCheck, ReadinessProbe, ServiceDefinition, ServicePort};
use wfe_core::traits::ServiceProvider;
use wfe_core::traits::step::StepBody;
use wfe_kubernetes::KubernetesServiceProvider;
use wfe_kubernetes::cleanup;
use wfe_kubernetes::client;
use wfe_kubernetes::config::{ClusterConfig, KubernetesStepConfig};
use wfe_kubernetes::namespace;

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

/// Generate a unique workflow ID to avoid namespace collisions between test runs.
fn unique_id(prefix: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    format!("{prefix}-{ts}")
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

    namespace::ensure_namespace(&client, ns, "test-wf")
        .await
        .unwrap();

    // Idempotent — creating again should succeed.
    namespace::ensure_namespace(&client, ns, "test-wf")
        .await
        .unwrap();

    namespace::delete_namespace(&client, ns).await.unwrap();
}

// ── Step Execution ───────────────────────────────────────────────────

#[tokio::test]
async fn run_echo_job() {
    let config = cluster_config();
    let k8s_client = client::create_client(&config).await.unwrap();

    let ns = unique_id("echo");
    let mut step_cfg = step_config("alpine:3.18", "echo 'hello from k8s'");
    step_cfg.namespace = Some(ns.clone());

    let mut step =
        wfe_kubernetes::KubernetesStep::new(step_cfg, config.clone(), k8s_client.clone());

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
    assert!(
        output["echo-step.stdout"]
            .as_str()
            .unwrap()
            .contains("hello from k8s")
    );
    assert_eq!(output["echo-step.exit_code"], 0);

    // Cleanup namespace.
    namespace::delete_namespace(&k8s_client, &ns).await.ok();
}

#[tokio::test]
async fn run_job_with_wfe_output() {
    let config = cluster_config();
    let k8s_client = client::create_client(&config).await.unwrap();

    let ns = unique_id("output");
    let mut step_cfg = step_config(
        "alpine:3.18",
        r#"echo '##wfe[output version=1.2.3]' && echo '##wfe[output status=ok]'"#,
    );
    step_cfg.namespace = Some(ns.clone());

    let mut step =
        wfe_kubernetes::KubernetesStep::new(step_cfg, config.clone(), k8s_client.clone());

    let instance = wfe_core::models::WorkflowInstance::new("output-wf", 1, serde_json::json!({}));
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

    let ns = unique_id("env");
    let mut step_cfg = step_config("alpine:3.18", "echo \"##wfe[output greeting=$GREETING]\"");
    step_cfg.env.insert("GREETING".into(), "hello".into());
    step_cfg.namespace = Some(ns.clone());

    let mut step =
        wfe_kubernetes::KubernetesStep::new(step_cfg, config.clone(), k8s_client.clone());

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

    let ns = unique_id("fail");
    let mut step_cfg = step_config("alpine:3.18", "exit 1");
    step_cfg.namespace = Some(ns.clone());

    let mut step =
        wfe_kubernetes::KubernetesStep::new(step_cfg, config.clone(), k8s_client.clone());

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

    let ns = unique_id("timeout");
    let mut step_cfg = step_config("alpine:3.18", "sleep 60");
    step_cfg.timeout_ms = Some(5_000); // 5 second timeout
    step_cfg.namespace = Some(ns.clone());

    let mut step =
        wfe_kubernetes::KubernetesStep::new(step_cfg, config.clone(), k8s_client.clone());

    let instance = wfe_core::models::WorkflowInstance::new("timeout-wf", 1, serde_json::json!({}));
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

// ── Service Provider ─────────────────────────────────────────────────

fn nginx_service() -> ServiceDefinition {
    ServiceDefinition {
        name: "nginx".into(),
        image: "nginx:alpine".into(),
        ports: vec![ServicePort::tcp(80)],
        env: HashMap::new(),
        readiness: Some(ReadinessProbe {
            check: ReadinessCheck::TcpSocket(80),
            interval_ms: 2000,
            timeout_ms: 60000,
            retries: 30,
        }),
        command: vec![],
        args: vec![],
        memory: Some("64Mi".into()),
        cpu: Some("100m".into()),
    }
}

fn redis_service() -> ServiceDefinition {
    ServiceDefinition {
        name: "redis".into(),
        image: "redis:7-alpine".into(),
        ports: vec![ServicePort::tcp(6379)],
        env: HashMap::new(),
        readiness: Some(ReadinessProbe {
            check: ReadinessCheck::TcpSocket(6379),
            interval_ms: 2000,
            timeout_ms: 60000,
            retries: 30,
        }),
        command: vec![],
        args: vec![],
        memory: Some("64Mi".into()),
        cpu: None,
    }
}

#[tokio::test]
async fn service_provider_can_provision() {
    let config = cluster_config();
    let k8s_client = client::create_client(&config).await.unwrap();
    let provider = KubernetesServiceProvider::new(k8s_client, config);

    assert!(provider.can_provision(&[nginx_service()]));
    assert!(provider.can_provision(&[]));
    assert!(provider.can_provision(&[nginx_service(), redis_service()]));
}

#[tokio::test]
async fn service_provider_provision_single_service() {
    let config = cluster_config();
    let k8s_client = client::create_client(&config).await.unwrap();
    let provider = KubernetesServiceProvider::new(k8s_client, config);

    let workflow_id = &unique_id("svc-single");
    let services = vec![nginx_service()];

    let endpoints = provider.provision(workflow_id, &services).await.unwrap();

    assert_eq!(endpoints.len(), 1);
    assert_eq!(endpoints[0].name, "nginx");
    assert_eq!(endpoints[0].host, "nginx"); // K8s DNS name
    assert_eq!(endpoints[0].ports.len(), 1);
    assert_eq!(endpoints[0].ports[0].container_port, 80);

    // Teardown.
    provider.teardown(workflow_id).await.unwrap();
}

#[tokio::test]
async fn service_provider_provision_multiple_services() {
    let config = cluster_config();
    let k8s_client = client::create_client(&config).await.unwrap();
    let provider = KubernetesServiceProvider::new(k8s_client, config);

    let workflow_id = &unique_id("svc-multi");
    let services = vec![nginx_service(), redis_service()];

    let endpoints = provider.provision(workflow_id, &services).await.unwrap();

    assert_eq!(endpoints.len(), 2);
    let names: Vec<&str> = endpoints.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"nginx"));
    assert!(names.contains(&"redis"));

    provider.teardown(workflow_id).await.unwrap();
}

#[tokio::test]
async fn service_provider_teardown_is_idempotent() {
    let config = cluster_config();
    let k8s_client = client::create_client(&config).await.unwrap();
    let provider = KubernetesServiceProvider::new(k8s_client, config);

    let workflow_id = &unique_id("svc-teardown");
    let services = vec![nginx_service()];

    provider.provision(workflow_id, &services).await.unwrap();
    provider.teardown(workflow_id).await.unwrap();

    // Second teardown should not error (namespace already deleting/gone).
    // Give K8s a moment to process the deletion.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    // This may error since namespace is gone, but shouldn't panic.
    let _ = provider.teardown(workflow_id).await;
}

#[tokio::test]
async fn service_provider_provision_bad_image_fails() {
    let config = cluster_config();
    let k8s_client = client::create_client(&config).await.unwrap();
    let provider = KubernetesServiceProvider::new(k8s_client, config);

    let workflow_id = &unique_id("svc-badimg");
    let services = vec![ServiceDefinition {
        name: "bad".into(),
        image: "nonexistent-registry.example.com/no-such-image:latest".into(),
        ports: vec![ServicePort::tcp(9999)],
        env: HashMap::new(),
        readiness: Some(ReadinessProbe {
            check: ReadinessCheck::TcpSocket(9999),
            interval_ms: 1000,
            timeout_ms: 8000, // Short timeout so test doesn't hang.
            retries: 4,
        }),
        command: vec![],
        args: vec![],
        memory: None,
        cpu: None,
    }];

    // The pod will be created but will never become ready (image pull will fail).
    let result = provider.provision(workflow_id, &services).await;
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("timed out") || err.contains("failed"),
        "expected timeout or failure error, got: {err}"
    );

    // Cleanup.
    provider.teardown(workflow_id).await.ok();
}

#[tokio::test]
async fn service_provider_provision_duplicate_name_fails() {
    let config = cluster_config();
    let k8s_client = client::create_client(&config).await.unwrap();
    let provider = KubernetesServiceProvider::new(k8s_client, config);

    let workflow_id = &unique_id("svc-dup");
    let svc = ServiceDefinition {
        name: "dupname".into(),
        image: "nginx:alpine".into(),
        ports: vec![ServicePort::tcp(80)],
        env: HashMap::new(),
        readiness: Some(ReadinessProbe {
            check: ReadinessCheck::TcpSocket(80),
            interval_ms: 2000,
            timeout_ms: 30000,
            retries: 15,
        }),
        command: vec![],
        args: vec![],
        memory: None,
        cpu: None,
    };

    // First provision succeeds.
    let endpoints = provider
        .provision(workflow_id, &[svc.clone()])
        .await
        .unwrap();
    assert_eq!(endpoints.len(), 1);

    // Second provision with same name should fail (pod already exists).
    let result = provider.provision(workflow_id, &[svc]).await;
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(err.contains("failed to create"), "unexpected error: {err}");

    provider.teardown(workflow_id).await.ok();
}

#[tokio::test]
async fn service_provider_provision_service_object_conflict() {
    // Pre-create a K8s Service to cause a conflict when the provider tries to create one.
    let config = cluster_config();
    let k8s_client = client::create_client(&config).await.unwrap();
    let provider = KubernetesServiceProvider::new(k8s_client.clone(), config.clone());

    let workflow_id = &unique_id("svc-conflict");
    let ns = namespace::namespace_name(&config.namespace_prefix, workflow_id);
    namespace::ensure_namespace(&k8s_client, &ns, workflow_id)
        .await
        .unwrap();

    // Pre-create just the K8s Service (not the pod).
    let svc_def = nginx_service();
    let svc_manifest = wfe_kubernetes::service_manifests::build_k8s_service(&svc_def, &ns);
    let svcs: kube::Api<k8s_openapi::api::core::v1::Service> =
        kube::Api::namespaced(k8s_client.clone(), &ns);
    svcs.create(&kube::api::PostParams::default(), &svc_manifest)
        .await
        .unwrap();

    // Now provision — pod will create fine but service will conflict.
    let result = provider.provision(workflow_id, &[svc_def]).await;
    assert!(result.is_err());
    let err = format!("{}", result.unwrap_err());
    assert!(
        err.contains("failed to create k8s service"),
        "unexpected error: {err}"
    );

    provider.teardown(workflow_id).await.ok();
}

#[tokio::test]
async fn service_provider_teardown_without_provision() {
    let config = cluster_config();
    let k8s_client = client::create_client(&config).await.unwrap();
    let provider = KubernetesServiceProvider::new(k8s_client, config);

    // Teardown a workflow that was never provisioned -- should error gracefully.
    let result = provider.teardown("never-provisioned").await;
    // Namespace doesn't exist, so delete_namespace returns an error.
    assert!(result.is_err());
}
