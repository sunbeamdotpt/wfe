//! Integration tests for wfe-buildkit using a real BuildKit daemon.
//!
//! These tests require a running BuildKit daemon. The socket path is read
//! from `WFE_BUILDKIT_ADDR`, falling back to
//! `unix:///Users/sienna/.lima/wfe-test/sock/buildkitd.sock`.
//!
//! If the daemon is not available, the tests are skipped gracefully.

use std::collections::HashMap;
use std::path::Path;

use wfe_buildkit::config::{BuildkitConfig, TlsConfig};
use wfe_buildkit::BuildkitStep;

use wfe_core::models::{ExecutionPointer, WorkflowInstance, WorkflowStep};
use wfe_core::traits::step::{StepBody, StepExecutionContext};

/// Get the BuildKit daemon address from the environment or use the default.
fn buildkit_addr() -> String {
    std::env::var("WFE_BUILDKIT_ADDR").unwrap_or_else(|_| {
        "unix:///Users/sienna/.lima/wfe-test/sock/buildkitd.sock".to_string()
    })
}

/// Check whether the BuildKit daemon socket is reachable.
fn buildkitd_available() -> bool {
    let addr = buildkit_addr();
    if let Some(path) = addr.strip_prefix("unix://") {
        Path::new(path).exists()
    } else {
        // For TCP endpoints, optimistically assume available.
        true
    }
}

fn make_test_context(
    step_name: &str,
) -> (
    WorkflowStep,
    ExecutionPointer,
    WorkflowInstance,
) {
    let mut step = WorkflowStep::new(0, "buildkit");
    step.name = Some(step_name.to_string());
    let pointer = ExecutionPointer::new(0);
    let instance = WorkflowInstance::new("test-wf", 1, serde_json::json!({}));
    (step, pointer, instance)
}

#[tokio::test]
async fn build_simple_dockerfile_via_grpc() {
    if !buildkitd_available() {
        eprintln!(
            "SKIP: BuildKit daemon not available at {}",
            buildkit_addr()
        );
        return;
    }

    // Create a temp directory with a trivial Dockerfile.
    let tmp = tempfile::tempdir().unwrap();
    let dockerfile = tmp.path().join("Dockerfile");
    std::fs::write(
        &dockerfile,
        "FROM alpine:latest\nRUN echo built\n",
    )
    .unwrap();

    let config = BuildkitConfig {
        dockerfile: "Dockerfile".to_string(),
        context: tmp.path().to_string_lossy().to_string(),
        target: None,
        tags: vec![],
        build_args: HashMap::new(),
        cache_from: vec![],
        cache_to: vec![],
        push: false,
        output_type: None,
        buildkit_addr: buildkit_addr(),
        tls: TlsConfig::default(),
        registry_auth: HashMap::new(),
        timeout_ms: Some(120_000), // 2 minutes
    };

    let mut step = BuildkitStep::new(config);

    let (ws, pointer, instance) = make_test_context("integration-build");
    let cancel = tokio_util::sync::CancellationToken::new();
    let ctx = StepExecutionContext {
        item: None,
        execution_pointer: &pointer,
        persistence_data: None,
        step: &ws,
        workflow: &instance,
        cancellation_token: cancel,
        host_context: None,
    };

    let result = step.run(&ctx).await.expect("build should succeed");

    assert!(result.proceed);

    let data = result.output_data.expect("should have output_data");
    let obj = data.as_object().expect("output_data should be an object");

    // Without tags/push, BuildKit does not produce a digest in the exporter
    // response. The build succeeds but the digest is absent.
    assert!(
        obj.contains_key("integration-build.stdout"),
        "expected stdout key, got: {:?}",
        obj.keys().collect::<Vec<_>>()
    );
    assert!(
        obj.contains_key("integration-build.stderr"),
        "expected stderr key, got: {:?}",
        obj.keys().collect::<Vec<_>>()
    );

    // If a digest IS present (e.g., newer buildkitd versions), validate its format.
    if let Some(digest_val) = obj.get("integration-build.digest") {
        let digest = digest_val.as_str().unwrap();
        assert!(
            digest.starts_with("sha256:"),
            "digest should start with sha256:, got: {digest}"
        );
        assert_eq!(
            digest.len(),
            7 + 64,
            "digest should be sha256:<64hex>, got: {digest}"
        );
    }
}

#[tokio::test]
async fn build_with_build_args() {
    if !buildkitd_available() {
        eprintln!(
            "SKIP: BuildKit daemon not available at {}",
            buildkit_addr()
        );
        return;
    }

    let tmp = tempfile::tempdir().unwrap();
    let dockerfile = tmp.path().join("Dockerfile");
    std::fs::write(
        &dockerfile,
        "FROM alpine:latest\nARG MY_VAR=default\nRUN echo \"value=$MY_VAR\"\n",
    )
    .unwrap();

    let mut build_args = HashMap::new();
    build_args.insert("MY_VAR".to_string(), "custom_value".to_string());

    let config = BuildkitConfig {
        dockerfile: "Dockerfile".to_string(),
        context: tmp.path().to_string_lossy().to_string(),
        target: None,
        tags: vec![],
        build_args,
        cache_from: vec![],
        cache_to: vec![],
        push: false,
        output_type: None,
        buildkit_addr: buildkit_addr(),
        tls: TlsConfig::default(),
        registry_auth: HashMap::new(),
        timeout_ms: Some(120_000),
    };

    let mut step = BuildkitStep::new(config);

    let (ws, pointer, instance) = make_test_context("build-args-test");
    let cancel = tokio_util::sync::CancellationToken::new();
    let ctx = StepExecutionContext {
        item: None,
        execution_pointer: &pointer,
        persistence_data: None,
        step: &ws,
        workflow: &instance,
        cancellation_token: cancel,
        host_context: None,
    };

    let result = step.run(&ctx).await.expect("build with args should succeed");
    assert!(result.proceed);

    let data = result.output_data.expect("should have output_data");
    let obj = data.as_object().unwrap();

    // Build should complete and produce output data entries.
    assert!(
        obj.contains_key("build-args-test.stdout"),
        "expected stdout key, got: {:?}",
        obj.keys().collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn connect_to_unavailable_daemon_returns_error() {
    // Use a deliberately wrong address to test error handling.
    let config = BuildkitConfig {
        dockerfile: "Dockerfile".to_string(),
        context: ".".to_string(),
        target: None,
        tags: vec![],
        build_args: HashMap::new(),
        cache_from: vec![],
        cache_to: vec![],
        push: false,
        output_type: None,
        buildkit_addr: "unix:///tmp/nonexistent-buildkitd.sock".to_string(),
        tls: TlsConfig::default(),
        registry_auth: HashMap::new(),
        timeout_ms: Some(5_000),
    };

    let mut step = BuildkitStep::new(config);

    let (ws, pointer, instance) = make_test_context("error-test");
    let cancel = tokio_util::sync::CancellationToken::new();
    let ctx = StepExecutionContext {
        item: None,
        execution_pointer: &pointer,
        persistence_data: None,
        step: &ws,
        workflow: &instance,
        cancellation_token: cancel,
        host_context: None,
    };

    let err = step.run(&ctx).await;
    assert!(err.is_err(), "should fail when daemon is unavailable");
}
