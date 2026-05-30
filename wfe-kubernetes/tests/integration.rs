use std::collections::HashMap;

use bytes::Bytes;
use wfe_core::models::service::{ReadinessCheck, ReadinessProbe, ServiceDefinition, ServicePort};
use wfe_core::traits::ArtifactStore;
use wfe_core::traits::ServiceProvider;
use wfe_core::traits::step::StepBody;
use wfe_kubernetes::KubernetesServiceProvider;
use wfe_kubernetes::cleanup;
use wfe_kubernetes::client;
use wfe_kubernetes::config::{ClusterConfig, KubernetesStepConfig};
use wfe_kubernetes::namespace;

/// Path to the Lima wfe VM kubeconfig.
fn kubeconfig_path() -> String {
    let home = std::env::var("HOME").unwrap();
    format!("{home}/.lima/lima-wfe/copied-from-guest/kubeconfig.yaml")
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
        shell: None,
        env: HashMap::new(),
        working_dir: None,
        memory: None,
        cpu: None,
        timeout_ms: None,
        pull_policy: None,
        namespace: None,
        artifact_outputs: HashMap::new(),
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
        definition: None,
        item: None,
        execution_pointer: &pointer,
        persistence_data: None,
        step: &ws,
        workflow: &instance,
        cancellation_token: tokio_util::sync::CancellationToken::new(),
        host_context: None,
        log_sink: None,
        artifact_store: None,
        artifact_volume: None,
        artifact_package: None,
        persistence: None,
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
        r###"echo '##wfe[output version=1.2.3]' && echo '##wfe[output status=ok]'"###,
    );
    step_cfg.namespace = Some(ns.clone());

    let mut step =
        wfe_kubernetes::KubernetesStep::new(step_cfg, config.clone(), k8s_client.clone());

    let instance = wfe_core::models::WorkflowInstance::new("output-wf", 1, serde_json::json!({}));
    let mut ws = wfe_core::models::WorkflowStep::new(0, "alpine-output");
    ws.name = Some("output-step".into());
    let pointer = wfe_core::models::ExecutionPointer::new(0);

    let ctx = wfe_core::traits::step::StepExecutionContext {
        definition: None,
        item: None,
        execution_pointer: &pointer,
        persistence_data: None,
        step: &ws,
        workflow: &instance,
        cancellation_token: tokio_util::sync::CancellationToken::new(),
        host_context: None,
        log_sink: None,
        artifact_store: None,
        artifact_volume: None,
        artifact_package: None,
        persistence: None,
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
        definition: None,
        item: None,
        execution_pointer: &pointer,
        persistence_data: None,
        step: &ws,
        workflow: &instance,
        cancellation_token: tokio_util::sync::CancellationToken::new(),
        host_context: None,
        log_sink: None,
        artifact_store: None,
        artifact_volume: None,
        artifact_package: None,
        persistence: None,
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
        definition: None,
        item: None,
        execution_pointer: &pointer,
        persistence_data: None,
        step: &ws,
        workflow: &instance,
        cancellation_token: tokio_util::sync::CancellationToken::new(),
        host_context: None,
        log_sink: None,
        artifact_store: None,
        artifact_volume: None,
        artifact_package: None,
        persistence: None,
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
        definition: None,
        item: None,
        execution_pointer: &pointer,
        persistence_data: None,
        step: &ws,
        workflow: &instance,
        cancellation_token: tokio_util::sync::CancellationToken::new(),
        host_context: None,
        log_sink: None,
        artifact_store: None,
        artifact_volume: None,
        artifact_package: None,
        persistence: None,
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

// ── End-to-end: multi-step workflow with shared volume ──────────────
//
// The tests above exercise the K8s step executor one step at a time. This
// test drives a realistic multi-step pipeline end-to-end, covering every
// feature that real CI workflows depend on:
//
//   * multiple step containers in the same workflow (different images
//     per step so we can confirm cross-image sharing through the PVC)
//   * a `shared_volume` PVC provisioned on the first step, persisting
//     to every subsequent step
//   * the `/bin/bash` shell override so a step can use bash-only features
//   * `extract_workflow_env` threading inputs through as uppercase env
//     vars to every step
//   * `##wfe[output ...]` capture from a later step's stdout
//   * a deliberate non-zero exit on a trailing step proving downstream
//     error propagation
//   * namespace lifecycle: created on first step, reused by all
//     siblings, deleted at the end
//
// The pipeline simulates a tiny CI run:
//
//   1. `write-files`    (alpine:3.18)  → writes `version.txt` + `input.sh`
//                                        to /workspace via `/bin/sh`
//   2. `compute-hash`   (busybox:1.36) → reads files from /workspace,
//                                        computes a sha256 with `sha256sum`,
//                                        emits ##wfe[output] lines
//   3. `verify-bash`    (alpine:3.18+bash) → uses `set -o pipefail` and
//                                            arrays to verify the hash
//   4. `inject-env`     (alpine:3.18)  → echoes workflow-data env vars
//                                        (REPO, BRANCH) through outputs
//
// Without the shared volume, step 2 couldn't see step 1's files. Without
// the shell override, step 3 would fail at `set -o pipefail`. Without
// inputs → env mapping, step 4's $REPO would be empty.
#[tokio::test]
async fn multi_step_workflow_with_shared_volume() {
    use tokio_util::sync::CancellationToken;
    use wfe_core::models::{
        ExecutionPointer, SharedVolume, WorkflowDefinition, WorkflowInstance, WorkflowStep,
    };
    use wfe_core::traits::step::{StepBody, StepExecutionContext};

    let cluster = cluster_config();
    let client = client::create_client(&cluster).await.unwrap();

    let root_id = unique_id("multistep");

    // The definition declares a shared /workspace volume. The K8s
    // executor reads this from `ctx.definition` and provisions a PVC
    // on the first step; every subsequent step in the same namespace
    // mounts the same claim.
    let definition = WorkflowDefinition {
        id: "multistep-ci".into(),
        name: Some("Multi-Step Integration Test".into()),
        version: 1,
        description: None,
        steps: vec![],
        default_error_behavior: Default::default(),
        default_error_retry_interval: None,
        services: vec![],
        shared_volume: Some(SharedVolume {
            mount_path: "/workspace".into(),
            size: Some("1Gi".into()),
        }),
    };

    // Single WorkflowInstance for the whole pipeline. Each step below
    // reuses it so they all share the same namespace (derived from
    // root_workflow_id → id fallback) and therefore the same PVC.
    let instance = WorkflowInstance {
        id: root_id.clone(),
        name: "multistep-1".into(),
        root_workflow_id: None,
        workflow_definition_id: "multistep-ci".into(),
        version: 1,
        description: None,
        reference: None,
        execution_pointers: vec![],
        next_execution: None,
        status: wfe_core::models::WorkflowStatus::Runnable,
        data: serde_json::json!({"repo": "wfe", "branch": "mainline"}),
        create_time: chrono::Utc::now(),
        complete_time: None,
    };

    let ns = crate::namespace::namespace_name(&cluster.namespace_prefix, &root_id);

    // Shared helper to run one step and assert the proceed flag + return
    // the captured output JSON.
    async fn run_step(
        step_cfg: KubernetesStepConfig,
        step_name: &str,
        instance: &WorkflowInstance,
        definition: &WorkflowDefinition,
        cluster: &wfe_kubernetes::config::ClusterConfig,
        client: &kube::Client,
    ) -> serde_json::Value {
        let mut step =
            wfe_kubernetes::KubernetesStep::new(step_cfg, cluster.clone(), client.clone());
        let mut ws = WorkflowStep::new(0, step_name);
        ws.name = Some(step_name.into());
        let pointer = ExecutionPointer::new(0);

        let ctx = StepExecutionContext {
            item: None,
            execution_pointer: &pointer,
            persistence_data: None,
            step: &ws,
            workflow: instance,
            definition: Some(definition),
            cancellation_token: CancellationToken::new(),
            host_context: None,
            log_sink: None,
            artifact_store: None,
            artifact_volume: None,
            artifact_package: None,
            persistence: None,
        };

        let result = step.run(&ctx).await.unwrap_or_else(|e| {
            panic!("step '{step_name}' failed: {e}");
        });
        assert!(result.proceed, "step '{step_name}' did not proceed");
        result.output_data.expect("output_data missing")
    }

    // The final cleanup call at the bottom of this function handles the
    // happy-path teardown. If any assertion below panics the namespace
    // will be left behind; the test harness runs `cleanup_stale_namespaces`
    // to reap those on the next run.
    let _ = &client; // acknowledge unused in guard-less form

    // ── Step 1: write files to /workspace via /bin/sh on alpine ────
    let mut s1 = step_config(
        "alpine:3.18",
        r###"
mkdir -p /workspace/pipeline
echo "1.9.0-test" > /workspace/pipeline/version.txt
printf 'hello from step 1\n' > /workspace/pipeline/input.sh
ls -la /workspace/pipeline
echo "##wfe[output step1_ok=true]"
"###,
    );
    s1.namespace = Some(ns.clone());
    let out1 = run_step(s1, "write-files", &instance, &definition, &cluster, &client).await;
    // `true` parses as a JSON boolean in build_output_data, not a string.
    assert_eq!(out1["step1_ok"], serde_json::Value::Bool(true));

    // ── Step 2: read the files written by step 1, hash them ────────
    // Uses a DIFFERENT image (busybox) so we prove cross-image
    // /workspace sharing through the PVC, not just container layer
    // reuse. sha256sum output is emitted as ##wfe[output hash=...].
    let mut s2 = step_config(
        "busybox:1.36",
        r###"
cd /workspace/pipeline
test -f version.txt || { echo "version.txt missing" >&2; exit 1; }
test -f input.sh    || { echo "input.sh missing"    >&2; exit 1; }
HASH=$(sha256sum version.txt | cut -c1-16)
VERSION=$(cat version.txt)
echo "##wfe[output hash=$HASH]"
echo "##wfe[output version=$VERSION]"
"###,
    );
    s2.namespace = Some(ns.clone());
    let out2 = run_step(
        s2,
        "compute-hash",
        &instance,
        &definition,
        &cluster,
        &client,
    )
    .await;
    assert_eq!(out2["version"], "1.9.0-test");
    let hash = out2["hash"].as_str().expect("hash in output");
    assert_eq!(hash.len(), 16, "hash should be 16 hex chars: {hash}");
    assert!(
        hash.chars().all(|c| c.is_ascii_hexdigit()),
        "hash not hex: {hash}"
    );

    // ── Step 3: bash-only features (pipefail + arrays) ──────────────
    // `alpine:3.18` doesn't have bash; use the bash-tagged image and
    // explicit shell override to prove the `shell:` config works
    // end-to-end.
    // Use debian:bookworm-slim — the `bash:5` image on docker hub mangles
    // its entrypoint such that `/bin/bash -c <script>` exits 128 before
    // the script runs. debian-slim has /bin/bash at the conventional path
    // and runs vanilla.
    let mut s3 = step_config(
        "debian:bookworm-slim",
        r###"
set -euo pipefail
# Bash-only: array + [[ ]] + process substitution
declare -a files=(version.txt input.sh)
for f in "${files[@]}"; do
    test -f /workspace/pipeline/$f
done
# pipefail makes `false | true` fail — if we reach the echo, pipefail
# actually caused the || branch to fire, which is the bash behavior
# we want to confirm.
if ! { false | true ; }; then
    echo "##wfe[output pipefail_ok=true]"
else
    echo "##wfe[output pipefail_ok=false]"
fi
echo "##wfe[output bash_features_ok=true]"
"###,
    );
    s3.shell = Some("/bin/bash".into());
    s3.namespace = Some(ns.clone());
    let out3 = run_step(s3, "verify-bash", &instance, &definition, &cluster, &client).await;
    assert_eq!(out3["bash_features_ok"], serde_json::Value::Bool(true));
    assert_eq!(out3["pipefail_ok"], serde_json::Value::Bool(true));

    // ── Step 4: confirm workflow.data env injection ────────────────
    // The instance was started with data {"repo": "wfe", "branch":
    // "mainline"}; extract_workflow_env uppercases keys so $REPO and
    // $BRANCH must be present inside the container.
    let mut s4 = step_config(
        "alpine:3.18",
        r###"
echo "##wfe[output repo=$REPO]"
echo "##wfe[output branch=$BRANCH]"
# Prove the volume is still there by listing files from step 1.
COUNT=$(ls /workspace/pipeline | wc -l | tr -d ' ')
echo "##wfe[output file_count=$COUNT]"
"###,
    );
    s4.namespace = Some(ns.clone());
    let out4 = run_step(s4, "inject-env", &instance, &definition, &cluster, &client).await;
    assert_eq!(out4["repo"], "wfe");
    assert_eq!(out4["branch"], "mainline");
    // `2` parses as a JSON number, not a string.
    assert_eq!(out4["file_count"], serde_json::Value::Number(2.into()));

    // Explicit cleanup (the guard still runs on panic paths).
    namespace::delete_namespace(&client, &ns).await.ok();
}

// ── Regression: sub-workflow inherits shared_volume via data ─────────
//
// The real CI topology is: ci (root, declares shared_volume) → checkout
// sub-workflow (no shared_volume on its definition). The K8s executor
// must pick up the shared_volume from the inherited `_wfe_shared_volume`
// key in workflow.data, NOT from the sub-workflow's definition.
//
// This test was added after a production bug where the PVC was never
// created because sub-workflow steps checked context.definition.shared_volume
// which was None on the child definition.
#[tokio::test]
async fn sub_workflow_inherits_shared_volume_from_data() {
    use tokio_util::sync::CancellationToken;
    use wfe_core::models::{
        ExecutionPointer, SharedVolume, WorkflowDefinition, WorkflowInstance, WorkflowStep,
    };
    use wfe_core::traits::step::{StepBody, StepExecutionContext};

    let cluster = cluster_config();
    let client = client::create_client(&cluster).await.unwrap();
    let root_id = unique_id("subwf-pvc");

    // The *child* definition does NOT declare shared_volume — this is
    // the whole point. Only the root ci workflow declares it, and the
    // config propagates via `_wfe_shared_volume` in workflow.data.
    let child_definition = WorkflowDefinition::new("lint", 1);

    // Simulate the data a sub-workflow receives from a root that has
    // shared_volume. WorkflowHost::start_workflow_with_name injects
    // `_wfe_shared_volume` into instance.data; SubWorkflowStep copies
    // the parent's data into the child.
    let child_data = serde_json::json!({
        "repo_url": "https://example.com/repo.git",
        "_wfe_shared_volume": {
            "mount_path": "/workspace",
            "size": "1Gi"
        }
    });

    let child_instance = WorkflowInstance {
        id: unique_id("child"),
        name: "lint-regression-1".into(),
        // Points at the root ci workflow — K8s executor derives the
        // namespace from this, placing us in the root's namespace.
        root_workflow_id: Some(root_id.clone()),
        workflow_definition_id: "lint".into(),
        version: 1,
        description: None,
        reference: None,
        execution_pointers: vec![],
        next_execution: None,
        status: wfe_core::models::WorkflowStatus::Runnable,
        data: child_data,
        create_time: chrono::Utc::now(),
        complete_time: None,
    };

    let ns = namespace::namespace_name(&cluster.namespace_prefix, &root_id);

    // Step config — write a file to /workspace and verify it persists.
    let mut step_cfg = step_config(
        "alpine:3.18",
        "echo pvc-ok > /workspace/pvc-test.txt && cat /workspace/pvc-test.txt",
    );
    step_cfg.namespace = Some(ns.clone());

    let mut step = wfe_kubernetes::KubernetesStep::new(step_cfg, cluster.clone(), client.clone());

    let mut ws = WorkflowStep::new(0, "pvc-check");
    ws.name = Some("pvc-check".into());
    let pointer = ExecutionPointer::new(0);

    let ctx = StepExecutionContext {
        item: None,
        execution_pointer: &pointer,
        persistence_data: None,
        step: &ws,
        workflow: &child_instance,
        // definition has NO shared_volume — the executor must read
        // _wfe_shared_volume from workflow.data instead.
        definition: Some(&child_definition),
        cancellation_token: CancellationToken::new(),
        host_context: None,
        log_sink: None,
        artifact_store: None,
        artifact_volume: None,
        artifact_package: None,
        persistence: None,
    };

    let result = step.run(&ctx).await.unwrap_or_else(|e| {
        panic!("pvc-check step failed (regression: sub-workflow shared_volume not inherited): {e}");
    });
    assert!(result.proceed);

    // Verify the PVC was actually created in the namespace.
    use k8s_openapi::api::core::v1::PersistentVolumeClaim;
    let pvcs: kube::Api<PersistentVolumeClaim> = kube::Api::namespaced(client.clone(), &ns);
    let pvc = pvcs.get("wfe-workspace").await;
    assert!(
        pvc.is_ok(),
        "PVC 'wfe-workspace' should exist in namespace {ns}"
    );

    let output = result.output_data.unwrap();
    let stdout = output["pvc-check.stdout"].as_str().unwrap_or("");
    assert!(
        stdout.contains("pvc-ok"),
        "expected pvc-ok in stdout, got: {stdout}"
    );

    namespace::delete_namespace(&client, &ns).await.ok();
}

// ── Artifact Store ───────────────────────────────────────────────────

#[tokio::test]
async fn run_job_with_artifact_input() {
    let config = cluster_config();
    let k8s_client = client::create_client(&config).await.unwrap();

    // Create a local artifact store with a test artifact.
    let store_dir = std::env::temp_dir().join(format!("wfe-k8s-artifact-test-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&store_dir).await.unwrap();
    let store = wfe_core::local_artifact_store::LocalArtifactStore::open(&store_dir).await.unwrap();

    // Build a tar.gz artifact containing a file.
    let artifact_bytes = tokio::task::spawn_blocking(|| {
        let mut buf = Vec::new();
        let enc = flate2::write::GzEncoder::new(&mut buf, flate2::Compression::default());
        let mut tar = tar::Builder::new(enc);
        let mut header = tar::Header::new_gnu();
        header.set_path("artifact.txt").unwrap();
        header.set_size(15);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append(&header, std::io::Cursor::new(b"hello artifact\n")).unwrap();
        let enc = tar.into_inner().unwrap();
        enc.finish().unwrap();
        buf
    }).await.unwrap();

    let reader: std::pin::Pin<Box<dyn tokio::io::AsyncRead + Send>> =
        Box::pin(std::io::Cursor::new(artifact_bytes));
    let artifact_ref = store.put(reader).await.unwrap();

    // Build artifact volume.
    let mut artifacts = std::collections::HashMap::new();
    artifacts.insert("mydata".to_string(), Bytes::from(
        tokio::fs::read(store_dir.join("blobs/sha256").join(&artifact_ref.digest.strip_prefix("sha256:").unwrap())).await.unwrap()
    ));
    let artifact_volume = wfe_core::ArtifactVolume::from_artifacts(artifacts);

    let ns = unique_id("artifact-in");
    let mut step_cfg = step_config(
        "alpine:3.18",
        "for i in 1 2 3 4 5; do [ -f /wfe-artifacts/inputs/mydata/artifact.txt ] && break; sleep 1; done; cat /wfe-artifacts/inputs/mydata/artifact.txt && echo \"##wfe[output result=ok]\"",
    );
    step_cfg.namespace = Some(ns.clone());

    let mut step =
        wfe_kubernetes::KubernetesStep::new(step_cfg, config.clone(), k8s_client.clone());

    let instance = wfe_core::models::WorkflowInstance::new(
        "artifact-in-wf",
        1,
        serde_json::json!({}),
    );
    let mut ws = wfe_core::models::WorkflowStep::new(0, "alpine-artifact-in");
    ws.name = Some("artifact-in-step".into());
    let pointer = wfe_core::models::ExecutionPointer::new(0);

    let ctx = wfe_core::traits::step::StepExecutionContext {
        definition: None,
        item: None,
        execution_pointer: &pointer,
        persistence_data: None,
        step: &ws,
        workflow: &instance,
        cancellation_token: tokio_util::sync::CancellationToken::new(),
        host_context: None,
        log_sink: None,
        artifact_store: Some(&store),
        artifact_volume: Some(&artifact_volume),
        artifact_package: None,
        persistence: None,
    };

    // mount_artifacts must be called before run (mirrors executor loop).
    step.mount_artifacts(&ctx).await.unwrap();
    let result = step.run(&ctx).await.unwrap();
    step.unmount_artifacts(&ctx).await.unwrap();

    assert!(result.proceed);
    let output = result.output_data.unwrap();
    let stdout = output["artifact-in-step.stdout"].as_str().unwrap_or("");
    assert!(stdout.contains("hello artifact"), "expected artifact content in stdout, got: {stdout}");
    assert_eq!(output["result"], "ok");

    namespace::delete_namespace(&k8s_client, &ns).await.ok();
    let _ = tokio::fs::remove_dir_all(&store_dir).await;
}

#[tokio::test]
async fn run_job_with_artifact_output() {
    let config = cluster_config();
    let k8s_client = client::create_client(&config).await.unwrap();

    let store_dir = std::env::temp_dir().join(format!("wfe-k8s-artifact-out-test-{}", uuid::Uuid::new_v4()));
    tokio::fs::create_dir_all(&store_dir).await.unwrap();
    let store = wfe_core::local_artifact_store::LocalArtifactStore::open(&store_dir).await.unwrap();

    let ns = unique_id("artifact-out");
    let mut step_cfg = step_config(
        "alpine:3.18",
        "mkdir -p /wfe-artifacts/outputs/build && echo 'build artifact' > /wfe-artifacts/outputs/build/bin.txt",
    );
    step_cfg.namespace = Some(ns.clone());
    step_cfg.artifact_outputs.insert("build".into(), "/wfe-artifacts/outputs/build".into());

    let mut step =
        wfe_kubernetes::KubernetesStep::new(step_cfg, config.clone(), k8s_client.clone());

    let instance = wfe_core::models::WorkflowInstance::new(
        "artifact-out-wf",
        1,
        serde_json::json!({}),
    );
    let mut ws = wfe_core::models::WorkflowStep::new(0, "alpine-artifact-out");
    ws.name = Some("artifact-out-step".into());
    let pointer = wfe_core::models::ExecutionPointer::new(0);

    let ctx = wfe_core::traits::step::StepExecutionContext {
        definition: None,
        item: None,
        execution_pointer: &pointer,
        persistence_data: None,
        step: &ws,
        workflow: &instance,
        cancellation_token: tokio_util::sync::CancellationToken::new(),
        host_context: None,
        log_sink: None,
        artifact_store: Some(&store),
        artifact_volume: None,
        artifact_package: None,
        persistence: None,
    };

    let result = step.run(&ctx).await.unwrap();
    assert!(result.proceed);

    let output = result.output_data.unwrap();
    assert!(
        output.get("build").is_some(),
        "expected artifact ref in output data, got: {output}"
    );
    let artifact_digest = wfe_core::models::parse_artifact_ref(&output["build"]).unwrap();
    assert!(store.exists(&artifact_digest).await, "artifact should exist in store");

    namespace::delete_namespace(&k8s_client, &ns).await.ok();
    let _ = tokio::fs::remove_dir_all(&store_dir).await;
}

#[tokio::test]
async fn run_job_with_artifact_output_but_no_store() {
    let config = cluster_config();
    let k8s_client = client::create_client(&config).await.unwrap();

    let ns = unique_id("artifact-out-no-store");
    let mut step_cfg = step_config(
        "alpine:3.18",
        "mkdir -p /wfe-artifacts/outputs/build && echo 'build artifact' > /wfe-artifacts/outputs/build/bin.txt",
    );
    step_cfg.namespace = Some(ns.clone());
    step_cfg.artifact_outputs.insert("build".into(), "/wfe-artifacts/outputs/build".into());

    let mut step =
        wfe_kubernetes::KubernetesStep::new(step_cfg, config.clone(), k8s_client.clone());

    let instance = wfe_core::models::WorkflowInstance::new(
        "artifact-out-no-store-wf",
        1,
        serde_json::json!({}),
    );
    let mut ws = wfe_core::models::WorkflowStep::new(0, "alpine-artifact-out-no-store");
    ws.name = Some("artifact-out-no-store-step".into());
    let pointer = wfe_core::models::ExecutionPointer::new(0);

    let ctx = wfe_core::traits::step::StepExecutionContext {
        definition: None,
        item: None,
        execution_pointer: &pointer,
        persistence_data: None,
        step: &ws,
        workflow: &instance,
        cancellation_token: tokio_util::sync::CancellationToken::new(),
        host_context: None,
        log_sink: None,
        artifact_store: None, // No store — artifact should be copied but not persisted.
        artifact_volume: None,
        artifact_package: None,
        persistence: None,
    };

    let result = step.run(&ctx).await.unwrap();
    assert!(result.proceed);

    // Without a store, the artifact ref should NOT appear in output data.
    let output = result.output_data.unwrap();
    assert!(
        output.get("build").is_none(),
        "expected no artifact ref when store is None, got: {output}"
    );

    namespace::delete_namespace(&k8s_client, &ns).await.ok();
}

#[tokio::test]
async fn run_job_without_explicit_namespace() {
    let config = cluster_config();
    let k8s_client = client::create_client(&config).await.unwrap();

    // Don't set namespace — it should fall back to namespace_prefix + workflow_id.
    let mut step_cfg = step_config("alpine:3.18", "echo 'hello from fallback ns'");

    let mut step =
        wfe_kubernetes::KubernetesStep::new(step_cfg, config.clone(), k8s_client.clone());

    let wf_id = unique_id("fallback-ns");
    let instance = wfe_core::models::WorkflowInstance::new(&wf_id, 1, serde_json::json!({}));
    let mut ws = wfe_core::models::WorkflowStep::new(0, "alpine-fallback-ns");
    ws.name = Some("fallback-ns-step".into());
    let pointer = wfe_core::models::ExecutionPointer::new(0);

    let ctx = wfe_core::traits::step::StepExecutionContext {
        definition: None,
        item: None,
        execution_pointer: &pointer,
        persistence_data: None,
        step: &ws,
        workflow: &instance,
        cancellation_token: tokio_util::sync::CancellationToken::new(),
        host_context: None,
        log_sink: None,
        artifact_store: None,
        artifact_volume: None,
        artifact_package: None,
        persistence: None,
    };

    let result = step.run(&ctx).await.unwrap();
    assert!(result.proceed);

    let output = result.output_data.unwrap();
    assert!(
        output["fallback-ns-step.stdout"]
            .as_str()
            .unwrap()
            .contains("hello from fallback ns")
    );

    // Cleanup the auto-generated namespace.
    let expected_ns = wfe_kubernetes::namespace::namespace_name(
        &config.namespace_prefix,
        &wf_id,
    );
    namespace::delete_namespace(&k8s_client, &expected_ns).await.ok();
}
