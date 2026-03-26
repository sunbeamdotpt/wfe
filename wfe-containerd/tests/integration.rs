use std::collections::HashMap;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;

use tempfile::TempDir;
use wfe_containerd::config::{ContainerdConfig, RegistryAuth, TlsConfig};
use wfe_containerd::ContainerdStep;
use wfe_core::models::{ExecutionPointer, WorkflowInstance, WorkflowStep};
use wfe_core::traits::step::{StepBody, StepExecutionContext};

fn minimal_config() -> ContainerdConfig {
    ContainerdConfig {
        image: "alpine:3.18".to_string(),
        command: None,
        run: Some("echo hello".to_string()),
        env: HashMap::new(),
        volumes: vec![],
        working_dir: None,
        user: "65534:65534".to_string(),
        network: "none".to_string(),
        memory: None,
        cpu: None,
        pull: "never".to_string(),
        containerd_addr: "/run/containerd/containerd.sock".to_string(),
        cli: "nerdctl".to_string(),
        tls: TlsConfig::default(),
        registry_auth: HashMap::new(),
        timeout_ms: None,
    }
}

/// Create a fake nerdctl script in a temp dir.
fn create_fake_nerdctl(dir: &TempDir, script: &str) -> String {
    let nerdctl_path = dir.path().join("nerdctl");
    let mut file = std::fs::File::create(&nerdctl_path).unwrap();
    file.write_all(script.as_bytes()).unwrap();
    std::fs::set_permissions(&nerdctl_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    dir.path().to_string_lossy().to_string()
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

/// Wrapper for set_var that handles the Rust 2024 unsafe requirement.
fn set_path(value: &str) {
    // SAFETY: These tests run sequentially (nextest runs each test in its own process)
    // so concurrent mutation of environment variables is not a concern.
    unsafe {
        std::env::set_var("PATH", value);
    }
}

// ── Happy-path: run succeeds with output parsing ───────────────────

#[tokio::test]
async fn run_success_with_outputs() {
    let tmp = TempDir::new().unwrap();
    let wfe_marker = "##wfe[output";
    let script = format!(
        "#!/bin/sh\n\
         while [ $# -gt 0 ]; do\n\
         case \"$1\" in\n\
         run) echo 'hello from container'\n\
         echo '{wfe_marker} result=success]'\n\
         echo '{wfe_marker} version=1.0.0]'\n\
         exit 0;;\n\
         pull) echo 'Pulling...'; exit 0;;\n\
         login) exit 0;;\n\
         --*) shift; shift;;\n\
         *) shift;;\n\
         esac\n\
         done\n\
         exit 1\n"
    );
    let bin_dir = create_fake_nerdctl(&tmp, &script);

    let mut config = minimal_config();
    config.pull = "always".to_string();
    let mut step = ContainerdStep::new(config);

    let mut named_step = WorkflowStep::new(0, "containerd");
    named_step.name = Some("my_container".to_string());
    let workflow = WorkflowInstance::new("test-wf", 1, serde_json::json!({}));
    let pointer = ExecutionPointer::new(0);
    let ctx = make_context(&named_step, &workflow, &pointer);

    let original_path = std::env::var("PATH").unwrap_or_default();
    set_path(&format!("{bin_dir}:{original_path}"));

    let result = step.run(&ctx).await;

    set_path(&original_path);

    let result = result.expect("run should succeed");
    assert!(result.proceed);
    let output = result.output_data.unwrap();
    let obj = output.as_object().unwrap();
    assert_eq!(obj.get("result").unwrap(), "success");
    assert_eq!(obj.get("version").unwrap(), "1.0.0");
    assert!(
        obj.get("my_container.stdout")
            .unwrap()
            .as_str()
            .unwrap()
            .contains("hello from container")
    );
    assert_eq!(obj.get("my_container.exit_code").unwrap(), 0);
}

// ── Non-zero exit code ─────────────────────────────────────────────

#[tokio::test]
async fn run_container_failure() {
    let tmp = TempDir::new().unwrap();
    let script = "#!/bin/sh\n\
         while [ $# -gt 0 ]; do\n\
         case \"$1\" in\n\
         run) echo 'something went wrong' >&2; exit 1;;\n\
         pull) exit 0;;\n\
         --*) shift; shift;;\n\
         *) shift;;\n\
         esac\n\
         done\n\
         exit 1\n";
    let bin_dir = create_fake_nerdctl(&tmp, script);

    let config = minimal_config();
    let mut step = ContainerdStep::new(config);

    let wf_step = WorkflowStep::new(0, "containerd");
    let workflow = WorkflowInstance::new("test-wf", 1, serde_json::json!({}));
    let pointer = ExecutionPointer::new(0);
    let ctx = make_context(&wf_step, &workflow, &pointer);

    let original_path = std::env::var("PATH").unwrap_or_default();
    set_path(&format!("{bin_dir}:{original_path}"));

    let result = step.run(&ctx).await;

    set_path(&original_path);

    let err = result.expect_err("should fail with non-zero exit");
    let msg = format!("{err}");
    assert!(msg.contains("Container exited with code 1"), "got: {msg}");
    assert!(msg.contains("something went wrong"), "got: {msg}");
}

// ── Pull failure ───────────────────────────────────────────────────

#[tokio::test]
async fn run_pull_failure() {
    let tmp = TempDir::new().unwrap();
    let script = "#!/bin/sh\n\
         while [ $# -gt 0 ]; do\n\
         case \"$1\" in\n\
         pull) echo 'image not found' >&2; exit 1;;\n\
         --*) shift; shift;;\n\
         *) shift;;\n\
         esac\n\
         done\n\
         exit 1\n";
    let bin_dir = create_fake_nerdctl(&tmp, script);

    let mut config = minimal_config();
    config.pull = "always".to_string();
    let mut step = ContainerdStep::new(config);

    let wf_step = WorkflowStep::new(0, "containerd");
    let workflow = WorkflowInstance::new("test-wf", 1, serde_json::json!({}));
    let pointer = ExecutionPointer::new(0);
    let ctx = make_context(&wf_step, &workflow, &pointer);

    let original_path = std::env::var("PATH").unwrap_or_default();
    set_path(&format!("{bin_dir}:{original_path}"));

    let result = step.run(&ctx).await;

    set_path(&original_path);

    let err = result.expect_err("should fail on pull");
    let msg = format!("{err}");
    assert!(msg.contains("Image pull failed"), "got: {msg}");
}

// ── Timeout ────────────────────────────────────────────────────────

#[tokio::test]
async fn run_timeout() {
    let tmp = TempDir::new().unwrap();
    let script = "#!/bin/sh\n\
         while [ $# -gt 0 ]; do\n\
         case \"$1\" in\n\
         run) sleep 30; exit 0;;\n\
         pull) exit 0;;\n\
         --*) shift; shift;;\n\
         *) shift;;\n\
         esac\n\
         done\n\
         exit 1\n";
    let bin_dir = create_fake_nerdctl(&tmp, script);

    let mut config = minimal_config();
    config.timeout_ms = Some(100);
    let mut step = ContainerdStep::new(config);

    let wf_step = WorkflowStep::new(0, "containerd");
    let workflow = WorkflowInstance::new("test-wf", 1, serde_json::json!({}));
    let pointer = ExecutionPointer::new(0);
    let ctx = make_context(&wf_step, &workflow, &pointer);

    let original_path = std::env::var("PATH").unwrap_or_default();
    set_path(&format!("{bin_dir}:{original_path}"));

    let result = step.run(&ctx).await;

    set_path(&original_path);

    let err = result.expect_err("should timeout");
    let msg = format!("{err}");
    assert!(msg.contains("timed out"), "got: {msg}");
}

// ── Missing nerdctl binary ─────────────────────────────────────────

#[tokio::test]
async fn run_missing_nerdctl() {
    let tmp = TempDir::new().unwrap();
    let bin_dir = tmp.path().to_string_lossy().to_string();

    let config = minimal_config();
    let mut step = ContainerdStep::new(config);

    let wf_step = WorkflowStep::new(0, "containerd");
    let workflow = WorkflowInstance::new("test-wf", 1, serde_json::json!({}));
    let pointer = ExecutionPointer::new(0);
    let ctx = make_context(&wf_step, &workflow, &pointer);

    let original_path = std::env::var("PATH").unwrap_or_default();
    set_path(&bin_dir);

    let result = step.run(&ctx).await;

    set_path(&original_path);

    let err = result.expect_err("should fail with missing binary");
    let msg = format!("{err}");
    assert!(
        msg.contains("Failed to spawn nerdctl run") || msg.contains("Failed to pull image")
            || msg.contains("Failed to spawn docker run"),
        "got: {msg}"
    );
}

// ── pull=never skips pull ──────────────────────────────────────────

#[tokio::test]
async fn run_skip_pull_when_never() {
    let tmp = TempDir::new().unwrap();
    let script = "#!/bin/sh\n\
         while [ $# -gt 0 ]; do\n\
         case \"$1\" in\n\
         run) echo ran; exit 0;;\n\
         pull) echo 'pull should not be called' >&2; exit 1;;\n\
         --*) shift; shift;;\n\
         *) shift;;\n\
         esac\n\
         done\n\
         exit 1\n";
    let bin_dir = create_fake_nerdctl(&tmp, script);

    let mut config = minimal_config();
    config.pull = "never".to_string();
    let mut step = ContainerdStep::new(config);

    let wf_step = WorkflowStep::new(0, "containerd");
    let workflow = WorkflowInstance::new("test-wf", 1, serde_json::json!({}));
    let pointer = ExecutionPointer::new(0);
    let ctx = make_context(&wf_step, &workflow, &pointer);

    let original_path = std::env::var("PATH").unwrap_or_default();
    set_path(&format!("{bin_dir}:{original_path}"));

    let result = step.run(&ctx).await;

    set_path(&original_path);

    result.expect("should succeed without pulling");
}

// ── Workflow data is injected as env vars ──────────────────────────

#[tokio::test]
async fn run_injects_workflow_data_as_env() {
    let tmp = TempDir::new().unwrap();
    let script = "#!/bin/sh\n\
         while [ $# -gt 0 ]; do\n\
         case \"$1\" in\n\
         run) shift\n\
         for arg in \"$@\"; do echo \"ARG:$arg\"; done\n\
         exit 0;;\n\
         pull) exit 0;;\n\
         --*) shift; shift;;\n\
         *) shift;;\n\
         esac\n\
         done\n\
         exit 1\n";
    let bin_dir = create_fake_nerdctl(&tmp, script);

    let mut config = minimal_config();
    config.pull = "never".to_string();
    config.run = None;
    config.command = Some(vec!["true".to_string()]);
    let mut step = ContainerdStep::new(config);

    let mut wf_step = WorkflowStep::new(0, "containerd");
    wf_step.name = Some("env_test".to_string());
    let workflow = WorkflowInstance::new(
        "test-wf",
        1,
        serde_json::json!({"my_key": "my_value", "count": 42}),
    );
    let pointer = ExecutionPointer::new(0);
    let ctx = make_context(&wf_step, &workflow, &pointer);

    let original_path = std::env::var("PATH").unwrap_or_default();
    set_path(&format!("{bin_dir}:{original_path}"));

    let result = step.run(&ctx).await;

    set_path(&original_path);

    let result = result.expect("should succeed");
    let stdout = result
        .output_data
        .unwrap()
        .as_object()
        .unwrap()
        .get("env_test.stdout")
        .unwrap()
        .as_str()
        .unwrap()
        .to_string();

    // The workflow data should have been injected as uppercase env vars
    assert!(stdout.contains("MY_KEY=my_value"), "stdout: {stdout}");
    assert!(stdout.contains("COUNT=42"), "stdout: {stdout}");
}

// ── Step name defaults to "unknown" when None ──────────────────────

#[tokio::test]
async fn run_unnamed_step_uses_unknown() {
    let tmp = TempDir::new().unwrap();
    let script = "#!/bin/sh\n\
         while [ $# -gt 0 ]; do\n\
         case \"$1\" in\n\
         run) echo output; exit 0;;\n\
         --*) shift; shift;;\n\
         *) shift;;\n\
         esac\n\
         done\n\
         exit 1\n";
    let bin_dir = create_fake_nerdctl(&tmp, script);

    let config = minimal_config();
    let mut step = ContainerdStep::new(config);

    let wf_step = WorkflowStep::new(0, "containerd");
    let workflow = WorkflowInstance::new("test-wf", 1, serde_json::json!({}));
    let pointer = ExecutionPointer::new(0);
    let ctx = make_context(&wf_step, &workflow, &pointer);

    let original_path = std::env::var("PATH").unwrap_or_default();
    set_path(&format!("{bin_dir}:{original_path}"));

    let result = step.run(&ctx).await;

    set_path(&original_path);

    let result = result.expect("should succeed");
    let output = result.output_data.unwrap();
    let obj = output.as_object().unwrap();
    assert!(obj.contains_key("unknown.stdout"));
    assert!(obj.contains_key("unknown.stderr"));
    assert!(obj.contains_key("unknown.exit_code"));
}

// ── Registry login failure ─────────────────────────────────────────

#[tokio::test]
async fn run_login_failure() {
    let tmp = TempDir::new().unwrap();
    let script = "#!/bin/sh\n\
         while [ $# -gt 0 ]; do\n\
         case \"$1\" in\n\
         login) echo unauthorized >&2; exit 1;;\n\
         --*) shift; shift;;\n\
         *) shift;;\n\
         esac\n\
         done\n\
         exit 0\n";
    let bin_dir = create_fake_nerdctl(&tmp, script);

    let mut config = minimal_config();
    config.registry_auth = HashMap::from([(
        "registry.example.com".to_string(),
        RegistryAuth {
            username: "user".to_string(),
            password: "wrong".to_string(),
        },
    )]);
    let mut step = ContainerdStep::new(config);

    let wf_step = WorkflowStep::new(0, "containerd");
    let workflow = WorkflowInstance::new("test-wf", 1, serde_json::json!({}));
    let pointer = ExecutionPointer::new(0);
    let ctx = make_context(&wf_step, &workflow, &pointer);

    let original_path = std::env::var("PATH").unwrap_or_default();
    set_path(&format!("{bin_dir}:{original_path}"));

    let result = step.run(&ctx).await;

    set_path(&original_path);

    let err = result.expect_err("should fail on login");
    let msg = format!("{err}");
    assert!(msg.contains("login failed"), "got: {msg}");
}

// ── Successful login with TLS then run ─────────────────────────────

#[tokio::test]
async fn run_login_success_with_tls() {
    let tmp = TempDir::new().unwrap();
    let script = "#!/bin/sh\n\
         while [ $# -gt 0 ]; do\n\
         case \"$1\" in\n\
         run) echo ok; exit 0;;\n\
         pull) exit 0;;\n\
         login) exit 0;;\n\
         --*) shift; shift;;\n\
         *) shift;;\n\
         esac\n\
         done\n\
         exit 1\n";
    let bin_dir = create_fake_nerdctl(&tmp, script);

    let mut config = minimal_config();
    config.pull = "never".to_string();
    config.tls = TlsConfig {
        ca: Some("/ca.pem".to_string()),
        cert: Some("/cert.pem".to_string()),
        key: Some("/key.pem".to_string()),
    };
    config.registry_auth = HashMap::from([(
        "secure.io".to_string(),
        RegistryAuth {
            username: "admin".to_string(),
            password: "secret".to_string(),
        },
    )]);
    let mut step = ContainerdStep::new(config);

    let wf_step = WorkflowStep::new(0, "containerd");
    let workflow = WorkflowInstance::new("test-wf", 1, serde_json::json!({}));
    let pointer = ExecutionPointer::new(0);
    let ctx = make_context(&wf_step, &workflow, &pointer);

    let original_path = std::env::var("PATH").unwrap_or_default();
    set_path(&format!("{bin_dir}:{original_path}"));

    let result = step.run(&ctx).await;

    set_path(&original_path);

    result.expect("should succeed with login + TLS");
}

// ── pull=if-not-present triggers pull ──────────────────────────────

#[tokio::test]
async fn run_pull_if_not_present() {
    let tmp = TempDir::new().unwrap();
    // Track that pull was called via a marker file
    let marker = tmp.path().join("pull_called");
    let marker_str = marker.to_string_lossy();
    let script = format!(
        "#!/bin/sh\n\
         while [ $# -gt 0 ]; do\n\
         case \"$1\" in\n\
         run) echo ran; exit 0;;\n\
         pull) touch {marker_str}; exit 0;;\n\
         --*) shift; shift;;\n\
         *) shift;;\n\
         esac\n\
         done\n\
         exit 1\n"
    );
    let bin_dir = create_fake_nerdctl(&tmp, &script);

    let mut config = minimal_config();
    config.pull = "if-not-present".to_string();
    let mut step = ContainerdStep::new(config);

    let wf_step = WorkflowStep::new(0, "containerd");
    let workflow = WorkflowInstance::new("test-wf", 1, serde_json::json!({}));
    let pointer = ExecutionPointer::new(0);
    let ctx = make_context(&wf_step, &workflow, &pointer);

    let original_path = std::env::var("PATH").unwrap_or_default();
    set_path(&format!("{bin_dir}:{original_path}"));

    let result = step.run(&ctx).await;

    set_path(&original_path);

    result.expect("should succeed");
    assert!(marker.exists(), "pull should have been called for if-not-present");
}
