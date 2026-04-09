//! Integration tests for the containerd gRPC-based runner.
//!
//! These tests require a live containerd daemon. They are skipped when the
//! socket is not available. Set `WFE_CONTAINERD_ADDR` to point to a custom
//! socket, or use the default `~/.lima/wfe-test/containerd.sock`.
//!
//! Before running, ensure the test image is pre-pulled:
//!   ctr -n default image pull docker.io/library/alpine:3.18

use std::collections::HashMap;
use std::path::Path;

use wfe_containerd::ContainerdStep;
use wfe_containerd::config::{ContainerdConfig, TlsConfig};
use wfe_core::models::{ExecutionPointer, WorkflowInstance, WorkflowStep};
use wfe_core::traits::step::{StepBody, StepExecutionContext};

/// Returns the containerd address if available, or None.
/// Set `WFE_CONTAINERD_ADDR` to a TCP address (http://host:port) or
/// Unix socket path (unix:///path). Defaults to the Lima wfe-test
/// TCP proxy at http://127.0.0.1:2500.
fn containerd_addr() -> Option<String> {
    if let Ok(addr) = std::env::var("WFE_CONTAINERD_ADDR") {
        if addr.starts_with("http://") || addr.starts_with("tcp://") {
            return Some(addr);
        }
        let socket_path = addr.strip_prefix("unix://").unwrap_or(addr.as_str());
        if Path::new(socket_path).exists() {
            return Some(addr);
        }
        return None;
    }

    // Default: check if the Lima wfe-test socket exists (for lightweight tests).
    let home = std::env::var("HOME").unwrap_or_else(|_| "/root".to_string());
    let socket = format!("{home}/.lima/wfe-test/containerd.sock");
    if Path::new(&socket).exists() {
        Some(format!("unix://{socket}"))
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
        definition: None,
        item: None,
        execution_pointer: pointer,
        persistence_data: None,
        step,
        workflow,
        cancellation_token: tokio_util::sync::CancellationToken::new(),
        host_context: None,
        log_sink: None,
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

// ── Run a real container end-to-end ──────────────────────────────────

#[tokio::test]
async fn run_echo_hello_in_container() {
    let Some(addr) = containerd_addr() else {
        eprintln!("SKIP: containerd socket not available");
        return;
    };

    let mut config = minimal_config(&addr);
    config.image = "docker.io/library/alpine:3.18".to_string();
    config.run = Some("echo hello-from-container".to_string());
    config.pull = "if-not-present".to_string();
    config.user = "0:0".to_string();
    config.timeout_ms = Some(30_000);
    let mut step = ContainerdStep::new(config);

    let mut wf_step = WorkflowStep::new(0, "containerd");
    wf_step.name = Some("echo-test".to_string());
    let workflow = WorkflowInstance::new("test-wf", 1, serde_json::json!({}));
    let pointer = ExecutionPointer::new(0);
    let ctx = make_context(&wf_step, &workflow, &pointer);

    let result = step.run(&ctx).await;
    match &result {
        Ok(r) => {
            eprintln!("SUCCESS: {:?}", r.output_data);
            let data = r.output_data.as_ref().unwrap().as_object().unwrap();
            let stdout = data.get("echo-test.stdout").unwrap().as_str().unwrap();
            assert!(stdout.contains("hello-from-container"), "stdout: {stdout}");
        }
        Err(e) => panic!("container step failed: {e}"),
    }
}

// ── Run a container with a volume mount ──────────────────────────────

#[tokio::test]
async fn run_container_with_volume_mount() {
    let Some(addr) = containerd_addr() else {
        eprintln!("SKIP: containerd socket not available");
        return;
    };

    let shared_dir = std::env::var("WFE_IO_DIR").unwrap_or_else(|_| "/tmp/wfe-io".to_string());
    let vol_dir = format!("{shared_dir}/test-vol");
    std::fs::create_dir_all(&vol_dir).unwrap();

    let mut config = minimal_config(&addr);
    config.image = "docker.io/library/alpine:3.18".to_string();
    config.run = Some("echo hello > /mnt/test/output.txt && cat /mnt/test/output.txt".to_string());
    config.pull = "if-not-present".to_string();
    config.user = "0:0".to_string();
    config.timeout_ms = Some(30_000);
    config.volumes = vec![wfe_containerd::VolumeMountConfig {
        source: vol_dir.clone(),
        target: "/mnt/test".to_string(),
        readonly: false,
    }];
    let mut step = ContainerdStep::new(config);

    let mut wf_step = WorkflowStep::new(0, "containerd");
    wf_step.name = Some("vol-test".to_string());
    let workflow = WorkflowInstance::new("test-wf", 1, serde_json::json!({}));
    let pointer = ExecutionPointer::new(0);
    let ctx = make_context(&wf_step, &workflow, &pointer);

    match step.run(&ctx).await {
        Ok(r) => {
            let data = r.output_data.as_ref().unwrap().as_object().unwrap();
            let stdout = data.get("vol-test.stdout").unwrap().as_str().unwrap();
            assert!(stdout.contains("hello"), "stdout: {stdout}");
        }
        Err(e) => panic!("container step with volume failed: {e}"),
    }

    std::fs::remove_dir_all(&vol_dir).ok();
}

// ── Run a container with volume mount and network (simulates install step) ──

#[tokio::test]
async fn run_debian_with_volume_and_network() {
    let Some(addr) = containerd_addr() else {
        eprintln!("SKIP: containerd socket not available");
        return;
    };

    let shared_dir = std::env::var("WFE_IO_DIR").unwrap_or_else(|_| "/tmp/wfe-io".to_string());
    let cargo_dir = format!("{shared_dir}/test-cargo");
    let rustup_dir = format!("{shared_dir}/test-rustup");
    std::fs::create_dir_all(&cargo_dir).unwrap();
    std::fs::create_dir_all(&rustup_dir).unwrap();

    let mut config = minimal_config(&addr);
    config.image = "docker.io/library/debian:bookworm-slim".to_string();
    config.run = Some("echo hello && ls /cargo && ls /rustup".to_string());
    config.pull = "if-not-present".to_string();
    config.user = "0:0".to_string();
    config.network = "host".to_string();
    config.timeout_ms = Some(30_000);
    config
        .env
        .insert("CARGO_HOME".to_string(), "/cargo".to_string());
    config
        .env
        .insert("RUSTUP_HOME".to_string(), "/rustup".to_string());
    config.volumes = vec![
        wfe_containerd::VolumeMountConfig {
            source: cargo_dir.clone(),
            target: "/cargo".to_string(),
            readonly: false,
        },
        wfe_containerd::VolumeMountConfig {
            source: rustup_dir.clone(),
            target: "/rustup".to_string(),
            readonly: false,
        },
    ];
    let mut step = ContainerdStep::new(config);

    let mut wf_step = WorkflowStep::new(0, "containerd");
    wf_step.name = Some("debian-test".to_string());
    let workflow = WorkflowInstance::new("test-wf", 1, serde_json::json!({}));
    let pointer = ExecutionPointer::new(0);
    let ctx = make_context(&wf_step, &workflow, &pointer);

    match step.run(&ctx).await {
        Ok(r) => {
            eprintln!("SUCCESS: {:?}", r.output_data);
        }
        Err(e) => panic!("debian container with volumes failed: {e}"),
    }

    std::fs::remove_dir_all(&cargo_dir).ok();
    std::fs::remove_dir_all(&rustup_dir).ok();
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
