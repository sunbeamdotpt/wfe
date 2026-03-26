//! Integration tests for the containerd gRPC-based runner.
//!
//! These tests require a live containerd daemon. They are skipped when the
//! socket is not available. Set `WFE_CONTAINERD_ADDR` to point to a custom
//! socket, or use the default `~/.lima/wfe-test/sock/containerd.sock`.
//!
//! Before running, ensure the test image is pre-pulled:
//!   ctr -n default image pull docker.io/library/alpine:3.18

use std::collections::HashMap;
use std::path::Path;

use wfe_containerd::config::{ContainerdConfig, TlsConfig};
use wfe_containerd::ContainerdStep;
use wfe_core::models::{ExecutionPointer, WorkflowInstance, WorkflowStep};
use wfe_core::traits::step::{StepBody, StepExecutionContext};

/// Returns the containerd socket address if available, or None.
fn containerd_addr() -> Option<String> {
    let addr = std::env::var("WFE_CONTAINERD_ADDR").unwrap_or_else(|_| {
        format!(
            "unix://{}/.lima/wfe-test/sock/containerd.sock",
            std::env::var("HOME").unwrap_or_else(|_| "/root".to_string())
        )
    });

    let socket_path = addr.strip_prefix("unix://").unwrap_or(addr.as_str());

    if Path::new(socket_path).exists() {
        Some(addr)
    } else {
        None
    }
}

fn minimal_config(addr: &str) -> ContainerdConfig {
    ContainerdConfig {
        image: "docker.io/library/alpine:3.18".to_string(),
        command: None,
        run: Some("echo hello".to_string()),
        env: HashMap::new(),
        volumes: vec![],
        working_dir: None,
        user: "0:0".to_string(),
        network: "none".to_string(),
        memory: None,
        cpu: None,
        pull: "never".to_string(),
        containerd_addr: addr.to_string(),
        cli: "nerdctl".to_string(),
        tls: TlsConfig::default(),
        registry_auth: HashMap::new(),
        timeout_ms: None,
    }
}

fn make_context<'a>(
    step: &'a WorkflowStep,
    workflow: &'a WorkflowInstance,
    pointer: &'a ExecutionPointer,
) -> StepExecutionContext<'a> {
    StepExecutionContext {
        item: None,
        execution_pointer: pointer,
        persistence_data: None,
        step,
        workflow,
        cancellation_token: tokio_util::sync::CancellationToken::new(),
    }
}

// ── Connection error for missing socket ──────────────────────────────

#[tokio::test]
async fn connect_error_for_missing_socket() {
    let config = minimal_config("/tmp/nonexistent-wfe-containerd-integ.sock");
    let mut step = ContainerdStep::new(config);

    let wf_step = WorkflowStep::new(0, "containerd");
    let workflow = WorkflowInstance::new("test-wf", 1, serde_json::json!({}));
    let pointer = ExecutionPointer::new(0);
    let ctx = make_context(&wf_step, &workflow, &pointer);

    let result = step.run(&ctx).await;
    let err = result.expect_err("should fail with socket not found");
    let msg = format!("{err}");
    assert!(
        msg.contains("socket not found"),
        "expected 'socket not found' error, got: {msg}"
    );
}

// ── Image check failure for non-existent image ──────────────────────

#[tokio::test]
async fn image_not_found_error() {
    let Some(addr) = containerd_addr() else {
        eprintln!("SKIP: containerd socket not available");
        return;
    };

    let mut config = minimal_config(&addr);
    config.image = "nonexistent-image-wfe-test:latest".to_string();
    config.pull = "if-not-present".to_string();
    let mut step = ContainerdStep::new(config);

    let wf_step = WorkflowStep::new(0, "containerd");
    let workflow = WorkflowInstance::new("test-wf", 1, serde_json::json!({}));
    let pointer = ExecutionPointer::new(0);
    let ctx = make_context(&wf_step, &workflow, &pointer);

    let result = step.run(&ctx).await;
    let err = result.expect_err("should fail with image not found");
    let msg = format!("{err}");
    assert!(
        msg.contains("not found"),
        "expected 'not found' error, got: {msg}"
    );
}

// ── pull=never skips image check ─────────────────────────────────────

#[tokio::test]
async fn skip_image_check_when_pull_never() {
    let Some(addr) = containerd_addr() else {
        eprintln!("SKIP: containerd socket not available");
        return;
    };

    // Using a non-existent image but pull=never should skip the check.
    // The step will fail later at container creation, but the image check is skipped.
    let mut config = minimal_config(&addr);
    config.image = "nonexistent-image-wfe-test-never:latest".to_string();
    config.pull = "never".to_string();
    let mut step = ContainerdStep::new(config);

    let wf_step = WorkflowStep::new(0, "containerd");
    let workflow = WorkflowInstance::new("test-wf", 1, serde_json::json!({}));
    let pointer = ExecutionPointer::new(0);
    let ctx = make_context(&wf_step, &workflow, &pointer);

    let result = step.run(&ctx).await;
    // It should fail, but NOT with "not found in containerd" (image check).
    // It should fail later (container creation, snapshot, etc.).
    let err = result.expect_err("should fail at container or task creation");
    let msg = format!("{err}");
    assert!(
        !msg.contains("Pre-pull it with"),
        "image check should have been skipped for pull=never, got: {msg}"
    );
}

// ── Step name defaults to "unknown" when None ────────────────────────

#[tokio::test]
async fn unnamed_step_uses_unknown_in_output_keys() {
    // This test only verifies build_output_data behavior — no socket needed.
    let parsed = HashMap::from([("result".to_string(), "ok".to_string())]);
    let data = ContainerdStep::build_output_data("unknown", "out", "err", 0, &parsed);
    let obj = data.as_object().unwrap();
    assert!(obj.contains_key("unknown.stdout"));
    assert!(obj.contains_key("unknown.stderr"));
    assert!(obj.contains_key("unknown.exit_code"));
    assert_eq!(obj.get("result").unwrap(), "ok");
}
