//! End-to-end integration tests for the Rust toolchain steps running inside
//! containerd containers.
//!
//! These tests start from a bare Debian image (no Rust installed) and exercise
//! the full Rust CI pipeline: install Rust, install external tools, create a
//! test project, and run every cargo operation.
//!
//! Requirements:
//!   - A running containerd daemon (Lima/colima or native)
//!   - Set `WFE_CONTAINERD_ADDR` to point to the socket
//!
//! These tests are gated behind `rustlang` + `containerd` features and are
//! marked `#[ignore]` so they don't run in normal CI. Run them explicitly:
//!   cargo test -p wfe-yaml --features rustlang,containerd --test rustlang_containerd -- --ignored

#![cfg(all(feature = "rustlang", feature = "containerd"))]

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use wfe::models::WorkflowStatus;
use wfe::{WorkflowHostBuilder, run_workflow_sync};
use wfe_core::test_support::{
    InMemoryLockProvider, InMemoryPersistenceProvider, InMemoryQueueProvider,
};
use wfe_yaml::load_single_workflow_from_str;

/// Returns the containerd address if available, or None.
/// Supports both Unix sockets (`unix:///path`) and TCP (`http://host:port`).
fn containerd_addr() -> Option<String> {
    let addr = std::env::var("WFE_CONTAINERD_ADDR").unwrap_or_else(|_| {
        // Default: TCP proxy on the Lima VM (socat forwarding containerd socket)
        "http://127.0.0.1:2500".to_string()
    });

    // For TCP addresses, assume reachable (the test will fail fast if not).
    if addr.starts_with("http://") || addr.starts_with("tcp://") {
        return Some(addr);
    }

    // For Unix sockets, check the file exists.
    let socket_path = addr.strip_prefix("unix://").unwrap_or(addr.as_str());
    if Path::new(socket_path).exists() {
        Some(addr)
    } else {
        None
    }
}

async fn run_yaml_workflow_with_config(
    yaml: &str,
    config: &HashMap<String, serde_json::Value>,
) -> wfe::models::WorkflowInstance {
    let compiled = load_single_workflow_from_str(yaml, config).unwrap();
    for step in &compiled.definition.steps {
        eprintln!("  step: {:?} type={} config={:?}", step.name, step.step_type, step.step_config);
    }
    eprintln!("  factories: {:?}", compiled.step_factories.iter().map(|(k, _)| k.clone()).collect::<Vec<_>>());

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
        serde_json::json!({}),
        Duration::from_secs(1800),
    )
    .await
    .unwrap();

    host.stop().await;
    instance
}

/// Shared env block and volume template for containerd steps.
/// Uses format! to avoid Rust 2024 reserved `##` token in raw strings.
fn containerd_step_yaml(
    name: &str,
    network: &str,
    pull: &str,
    timeout: &str,
    working_dir: Option<&str>,
    mount_workspace: bool,
    run_script: &str,
) -> String {
    let wfe = "##wfe";
    let wd = working_dir
        .map(|d| format!("        working_dir: {d}"))
        .unwrap_or_default();
    let ws_volume = if mount_workspace {
        "          - source: ((workspace))\n            target: /workspace"
    } else {
        ""
    };

    format!(
        r#"    - name: {name}
      type: containerd
      config:
        image: docker.io/library/debian:bookworm-slim
        containerd_addr: ((containerd_addr))
        user: "0:0"
        network: {network}
        pull: {pull}
        timeout: {timeout}
{wd}
        env:
          CARGO_HOME: /cargo
          RUSTUP_HOME: /rustup
          PATH: /cargo/bin:/rustup/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
        volumes:
          - source: ((cargo_home))
            target: /cargo
          - source: ((rustup_home))
            target: /rustup
{ws_volume}
        run: |
{run_script}
          echo "{wfe}[output {name}.status=ok]"
"#
    )
}

/// Base directory for shared state between host and containerd VM.
/// Must be inside the virtiofs mount defined in test/lima/wfe-test.yaml.
fn shared_dir() -> std::path::PathBuf {
    let base = std::env::var("WFE_IO_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/tmp/wfe-io"));
    std::fs::create_dir_all(&base).unwrap();
    base
}

/// Create a temporary directory inside the shared mount so containerd can see it.
fn shared_tempdir(name: &str) -> std::path::PathBuf {
    let id = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = shared_dir().join(format!("{name}-{id}"));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn make_config(
    addr: &str,
    cargo_home: &Path,
    rustup_home: &Path,
    workspace: Option<&Path>,
) -> HashMap<String, serde_json::Value> {
    let mut config = HashMap::new();
    config.insert(
        "containerd_addr".to_string(),
        serde_json::Value::String(addr.to_string()),
    );
    config.insert(
        "cargo_home".to_string(),
        serde_json::Value::String(cargo_home.to_str().unwrap().to_string()),
    );
    config.insert(
        "rustup_home".to_string(),
        serde_json::Value::String(rustup_home.to_str().unwrap().to_string()),
    );
    if let Some(ws) = workspace {
        config.insert(
            "workspace".to_string(),
            serde_json::Value::String(ws.to_str().unwrap().to_string()),
        );
    }
    config
}

// ---------------------------------------------------------------------------
// Minimal: just echo hello in a containerd step through the workflow engine
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires containerd daemon"]
async fn minimal_echo_in_containerd_via_workflow() {
    let _ = tracing_subscriber::fmt().with_env_filter("wfe_containerd=debug,wfe_core::executor=debug").try_init();
    let Some(addr) = containerd_addr() else {
        eprintln!("SKIP: containerd not available");
        return;
    };

    let mut config = HashMap::new();
    config.insert(
        "containerd_addr".to_string(),
        serde_json::Value::String(addr),
    );

    let wfe = "##wfe";
    let yaml = format!(
        r#"workflow:
  id: minimal-containerd
  version: 1
  error_behavior:
    type: terminate
  steps:
    - name: echo
      type: containerd
      config:
        image: docker.io/library/alpine:3.18
        containerd_addr: ((containerd_addr))
        user: "0:0"
        network: none
        pull: if-not-present
        timeout: 30s
        run: |
          echo hello-from-workflow
          echo "{wfe}[output echo.status=ok]"
"#
    );

    let instance = run_yaml_workflow_with_config(&yaml, &config).await;

    eprintln!("Status: {:?}, Data: {:?}", instance.status, instance.data);
    assert_eq!(instance.status, WorkflowStatus::Complete);
    let data = instance.data.as_object().unwrap();
    assert_eq!(
        data.get("echo.status").and_then(|v| v.as_str()),
        Some("ok"),
    );
}

// ---------------------------------------------------------------------------
// Full Rust CI pipeline in a container: install → build → test → lint → cover
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires containerd daemon"]
async fn full_rust_pipeline_in_container() {
    let Some(addr) = containerd_addr() else {
        eprintln!("SKIP: containerd socket not available");
        return;
    };

    let cargo_home = shared_tempdir("cargo");
    let rustup_home = shared_tempdir("rustup");
    let workspace = shared_tempdir("workspace");

    let config = make_config(
        &addr,
        &cargo_home,
        &rustup_home,
        Some(&workspace),
    );

    let steps = [
        containerd_step_yaml(
            "install-rust", "host", "if-not-present", "10m", None, false,
            "          apt-get update && apt-get install -y curl gcc pkg-config libssl-dev\n\
             \x20         curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable",
        ),
        containerd_step_yaml(
            "install-tools", "host", "never", "10m", None, false,
            "          rustup component add clippy rustfmt llvm-tools-preview\n\
             \x20         cargo install cargo-audit cargo-deny cargo-nextest cargo-llvm-cov",
        ),
        containerd_step_yaml(
            "create-project", "host", "never", "2m", None, true,
            "          cargo init /workspace/test-crate --name test-crate\n\
             \x20         cd /workspace/test-crate\n\
             \x20         echo '#[cfg(test)] mod tests { #[test] fn it_works() { assert_eq!(2+2,4); } }' >> src/main.rs",
        ),
        containerd_step_yaml(
            "cargo-fmt", "none", "never", "2m",
            Some("/workspace/test-crate"), true,
            "          cargo fmt -- --check || cargo fmt",
        ),
        containerd_step_yaml(
            "cargo-check", "none", "never", "5m",
            Some("/workspace/test-crate"), true,
            "          cargo check",
        ),
        containerd_step_yaml(
            "cargo-clippy", "none", "never", "5m",
            Some("/workspace/test-crate"), true,
            "          cargo clippy -- -D warnings",
        ),
        containerd_step_yaml(
            "cargo-test", "none", "never", "5m",
            Some("/workspace/test-crate"), true,
            "          cargo test",
        ),
        containerd_step_yaml(
            "cargo-build", "none", "never", "5m",
            Some("/workspace/test-crate"), true,
            "          cargo build --release",
        ),
        containerd_step_yaml(
            "cargo-nextest", "none", "never", "5m",
            Some("/workspace/test-crate"), true,
            "          cargo nextest run",
        ),
        containerd_step_yaml(
            "cargo-llvm-cov", "none", "never", "5m",
            Some("/workspace/test-crate"), true,
            "          cargo llvm-cov --summary-only",
        ),
        containerd_step_yaml(
            "cargo-audit", "host", "never", "5m",
            Some("/workspace/test-crate"), true,
            "          cargo audit || true",
        ),
        containerd_step_yaml(
            "cargo-deny", "none", "never", "5m",
            Some("/workspace/test-crate"), true,
            "          cargo deny init\n\
             \x20         cargo deny check || true",
        ),
        containerd_step_yaml(
            "cargo-doc", "none", "never", "5m",
            Some("/workspace/test-crate"), true,
            "          cargo doc --no-deps",
        ),
    ];

    let yaml = format!(
        "workflow:\n  id: rust-container-pipeline\n  version: 1\n  error_behavior:\n    type: terminate\n  steps:\n{}",
        steps.join("\n")
    );

    let instance = run_yaml_workflow_with_config(&yaml, &config).await;

    assert_eq!(
        instance.status,
        WorkflowStatus::Complete,
        "workflow should complete successfully, data: {:?}",
        instance.data
    );

    let data = instance.data.as_object().unwrap();

    for key in [
        "install-rust.status",
        "install-tools.status",
        "create-project.status",
        "cargo-fmt.status",
        "cargo-check.status",
        "cargo-clippy.status",
        "cargo-test.status",
        "cargo-build.status",
        "cargo-nextest.status",
        "cargo-llvm-cov.status",
        "cargo-audit.status",
        "cargo-deny.status",
        "cargo-doc.status",
    ] {
        assert_eq!(
            data.get(key).and_then(|v| v.as_str()),
            Some("ok"),
            "step output '{key}' should be 'ok', got: {:?}",
            data.get(key)
        );
    }
}

// ---------------------------------------------------------------------------
// Focused test: just rust-install in a bare container
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore = "requires containerd daemon"]
async fn rust_install_in_bare_container() {
    let Some(addr) = containerd_addr() else {
        eprintln!("SKIP: containerd socket not available");
        return;
    };

    let cargo_home = shared_tempdir("cargo");
    let rustup_home = shared_tempdir("rustup");

    let config = make_config(&addr, &cargo_home, &rustup_home, None);

    let wfe = "##wfe";
    let yaml = format!(
        r#"workflow:
  id: rust-install-container
  version: 1
  error_behavior:
    type: terminate
  steps:
    - name: install
      type: containerd
      config:
        image: docker.io/library/debian:bookworm-slim
        containerd_addr: ((containerd_addr))
        user: "0:0"
        network: host
        pull: if-not-present
        timeout: 10m
        env:
          CARGO_HOME: /cargo
          RUSTUP_HOME: /rustup
          PATH: /cargo/bin:/rustup/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
        volumes:
          - source: ((cargo_home))
            target: /cargo
          - source: ((rustup_home))
            target: /rustup
        run: |
          apt-get update && apt-get install -y curl
          curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable
          rustc --version
          cargo --version
          echo "{wfe}[output rustc_installed=true]"

    - name: verify
      type: containerd
      config:
        image: docker.io/library/debian:bookworm-slim
        containerd_addr: ((containerd_addr))
        user: "0:0"
        network: none
        pull: if-not-present
        timeout: 2m
        env:
          CARGO_HOME: /cargo
          RUSTUP_HOME: /rustup
          PATH: /cargo/bin:/rustup/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
        volumes:
          - source: ((cargo_home))
            target: /cargo
          - source: ((rustup_home))
            target: /rustup
        run: |
          rustc --version
          cargo --version
          echo "{wfe}[output verify.status=ok]"
"#
    );

    let instance = run_yaml_workflow_with_config(&yaml, &config).await;

    assert_eq!(
        instance.status,
        WorkflowStatus::Complete,
        "install workflow should complete, data: {:?}",
        instance.data
    );

    let data = instance.data.as_object().unwrap();
    eprintln!("Workflow data: {:?}", instance.data);
    assert!(
        data.get("rustc_installed").is_some(),
        "rustc_installed should be set, got data: {:?}",
        data
    );
    assert_eq!(
        data.get("verify.status").and_then(|v| v.as_str()),
        Some("ok"),
    );
}
