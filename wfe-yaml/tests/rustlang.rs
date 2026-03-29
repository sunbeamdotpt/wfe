#![cfg(feature = "rustlang")]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use wfe::models::WorkflowStatus;
use wfe::{WorkflowHostBuilder, run_workflow_sync};
use wfe_core::test_support::{
    InMemoryLockProvider, InMemoryPersistenceProvider, InMemoryQueueProvider,
};
use wfe_yaml::load_single_workflow_from_str;

fn has_factory(compiled: &wfe_yaml::compiler::CompiledWorkflow, key: &str) -> bool {
    compiled.step_factories.iter().any(|(k, _)| k == key)
}

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
        Duration::from_secs(30),
    )
    .await
    .unwrap();

    host.stop().await;
    instance
}

// ---------------------------------------------------------------------------
// Compiler tests — verify YAML compiles to correct step types and configs
// ---------------------------------------------------------------------------

#[test]
fn compile_cargo_build_step() {
    let yaml = r#"
workflow:
  id: cargo-build-wf
  version: 1
  steps:
    - name: build
      type: cargo-build
      config:
        release: true
        package: my-crate
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    let step = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("build"))
        .unwrap();
    assert_eq!(step.step_type, "wfe_yaml::cargo::build");
    assert!(has_factory(&compiled, "wfe_yaml::cargo::build"));
}

#[test]
fn compile_cargo_test_step() {
    let yaml = r#"
workflow:
  id: cargo-test-wf
  version: 1
  steps:
    - name: test
      type: cargo-test
      config:
        features:
          - feat1
          - feat2
        extra_args:
          - "--"
          - "--nocapture"
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    let step = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("test"))
        .unwrap();
    assert_eq!(step.step_type, "wfe_yaml::cargo::test");
}

#[test]
fn compile_cargo_check_step() {
    let yaml = r#"
workflow:
  id: cargo-check-wf
  version: 1
  steps:
    - name: check
      type: cargo-check
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert!(has_factory(&compiled, "wfe_yaml::cargo::check"));
}

#[test]
fn compile_cargo_clippy_step() {
    let yaml = r#"
workflow:
  id: cargo-clippy-wf
  version: 1
  steps:
    - name: lint
      type: cargo-clippy
      config:
        all_features: true
        extra_args:
          - "--"
          - "-D"
          - "warnings"
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert!(has_factory(&compiled, "wfe_yaml::cargo::lint"));
}

#[test]
fn compile_cargo_fmt_step() {
    let yaml = r#"
workflow:
  id: cargo-fmt-wf
  version: 1
  steps:
    - name: format
      type: cargo-fmt
      config:
        extra_args:
          - "--check"
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert!(has_factory(&compiled, "wfe_yaml::cargo::format"));
}

#[test]
fn compile_cargo_doc_step() {
    let yaml = r#"
workflow:
  id: cargo-doc-wf
  version: 1
  steps:
    - name: docs
      type: cargo-doc
      config:
        extra_args:
          - "--no-deps"
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert!(has_factory(&compiled, "wfe_yaml::cargo::docs"));
}

#[test]
fn compile_cargo_publish_step() {
    let yaml = r#"
workflow:
  id: cargo-publish-wf
  version: 1
  steps:
    - name: publish
      type: cargo-publish
      config:
        extra_args:
          - "--dry-run"
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert!(has_factory(&compiled, "wfe_yaml::cargo::publish"));
}

#[test]
fn compile_cargo_step_with_toolchain() {
    let yaml = r#"
workflow:
  id: nightly-wf
  version: 1
  steps:
    - name: nightly-check
      type: cargo-check
      config:
        toolchain: nightly
        no_default_features: true
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert!(has_factory(&compiled, "wfe_yaml::cargo::nightly-check"));
}

#[test]
fn compile_cargo_step_with_timeout() {
    let yaml = r#"
workflow:
  id: timeout-wf
  version: 1
  steps:
    - name: slow-build
      type: cargo-build
      config:
        timeout: 5m
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert!(has_factory(&compiled, "wfe_yaml::cargo::slow-build"));
}

#[test]
fn compile_cargo_step_without_config() {
    let yaml = r#"
workflow:
  id: bare-wf
  version: 1
  steps:
    - name: bare-check
      type: cargo-check
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert!(has_factory(&compiled, "wfe_yaml::cargo::bare-check"));
}

#[test]
fn compile_cargo_multi_step_pipeline() {
    let yaml = r#"
workflow:
  id: ci-pipeline
  version: 1
  steps:
    - name: fmt
      type: cargo-fmt
      config:
        extra_args: ["--check"]
    - name: check
      type: cargo-check
    - name: clippy
      type: cargo-clippy
      config:
        extra_args: ["--", "-D", "warnings"]
    - name: test
      type: cargo-test
    - name: build
      type: cargo-build
      config:
        release: true
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert!(has_factory(&compiled, "wfe_yaml::cargo::fmt"));
    assert!(has_factory(&compiled, "wfe_yaml::cargo::check"));
    assert!(has_factory(&compiled, "wfe_yaml::cargo::clippy"));
    assert!(has_factory(&compiled, "wfe_yaml::cargo::test"));
    assert!(has_factory(&compiled, "wfe_yaml::cargo::build"));
}

#[test]
fn compile_cargo_step_with_all_shared_flags() {
    let yaml = r#"
workflow:
  id: full-flags-wf
  version: 1
  steps:
    - name: full
      type: cargo-build
      config:
        package: my-crate
        features: [foo, bar]
        all_features: false
        no_default_features: true
        release: true
        toolchain: stable
        profile: release
        extra_args: ["--jobs", "4"]
        working_dir: /tmp/project
        timeout: 30s
        env:
          RUSTFLAGS: "-C target-cpu=native"
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert!(has_factory(&compiled, "wfe_yaml::cargo::full"));
}

#[test]
fn compile_cargo_step_preserves_step_config_json() {
    let yaml = r#"
workflow:
  id: config-json-wf
  version: 1
  steps:
    - name: build
      type: cargo-build
      config:
        release: true
        package: wfe-core
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    let step = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("build"))
        .unwrap();

    let step_config = step.step_config.as_ref().unwrap();
    assert_eq!(step_config["command"], "build");
    assert_eq!(step_config["release"], true);
    assert_eq!(step_config["package"], "wfe-core");
}

// ---------------------------------------------------------------------------
// Integration tests — run actual cargo commands through the workflow engine
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cargo_check_on_self_succeeds() {
    let yaml = r#"
workflow:
  id: self-check
  version: 1
  steps:
    - name: check
      type: cargo-check
      config:
        working_dir: .
        timeout: 120s
"#;
    let instance = run_yaml_workflow(yaml).await;
    assert_eq!(instance.status, WorkflowStatus::Complete);

    let data = instance.data.as_object().unwrap();
    assert!(data.contains_key("check.stdout") || data.contains_key("check.stderr"));
}

#[tokio::test]
async fn cargo_fmt_check_compiles() {
    let yaml = r#"
workflow:
  id: fmt-check
  version: 1
  steps:
    - name: fmt
      type: cargo-fmt
      config:
        working_dir: .
        extra_args: ["--check"]
        timeout: 60s
"#;
    let config = HashMap::new();
    let compiled = load_single_workflow_from_str(yaml, &config).unwrap();
    assert!(has_factory(&compiled, "wfe_yaml::cargo::fmt"));
}

// ---------------------------------------------------------------------------
// Rustup compiler tests
// ---------------------------------------------------------------------------

#[test]
fn compile_rust_install_step() {
    let yaml = r#"
workflow:
  id: rust-install-wf
  version: 1
  steps:
    - name: install-rust
      type: rust-install
      config:
        profile: minimal
        default_toolchain: stable
        timeout: 5m
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    let step = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("install-rust"))
        .unwrap();
    assert_eq!(step.step_type, "wfe_yaml::rustup::install-rust");
    assert!(has_factory(&compiled, "wfe_yaml::rustup::install-rust"));
}

#[test]
fn compile_rustup_toolchain_step() {
    let yaml = r#"
workflow:
  id: tc-install-wf
  version: 1
  steps:
    - name: add-nightly
      type: rustup-toolchain
      config:
        toolchain: nightly-2024-06-01
        profile: minimal
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert!(has_factory(&compiled, "wfe_yaml::rustup::add-nightly"));
}

#[test]
fn compile_rustup_component_step() {
    let yaml = r#"
workflow:
  id: comp-add-wf
  version: 1
  steps:
    - name: add-tools
      type: rustup-component
      config:
        components: [clippy, rustfmt, rust-src]
        toolchain: nightly
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert!(has_factory(&compiled, "wfe_yaml::rustup::add-tools"));
}

#[test]
fn compile_rustup_target_step() {
    let yaml = r#"
workflow:
  id: target-add-wf
  version: 1
  steps:
    - name: add-wasm
      type: rustup-target
      config:
        targets: [wasm32-unknown-unknown]
        toolchain: stable
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert!(has_factory(&compiled, "wfe_yaml::rustup::add-wasm"));
}

#[test]
fn compile_rustup_step_without_config() {
    let yaml = r#"
workflow:
  id: bare-install-wf
  version: 1
  steps:
    - name: install
      type: rust-install
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert!(has_factory(&compiled, "wfe_yaml::rustup::install"));
}

#[test]
fn compile_rustup_step_preserves_config_json() {
    let yaml = r#"
workflow:
  id: config-json-wf
  version: 1
  steps:
    - name: tc
      type: rustup-toolchain
      config:
        toolchain: nightly
        profile: minimal
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    let step = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("tc"))
        .unwrap();

    let step_config = step.step_config.as_ref().unwrap();
    assert_eq!(step_config["command"], "toolchain-install");
    assert_eq!(step_config["toolchain"], "nightly");
    assert_eq!(step_config["profile"], "minimal");
}

#[test]
fn compile_full_rust_ci_pipeline() {
    let yaml = r#"
workflow:
  id: full-rust-ci
  version: 1
  steps:
    - name: install
      type: rust-install
      config:
        profile: minimal
        default_toolchain: stable
    - name: add-nightly
      type: rustup-toolchain
      config:
        toolchain: nightly
    - name: add-components
      type: rustup-component
      config:
        components: [clippy, rustfmt]
    - name: add-wasm
      type: rustup-target
      config:
        targets: [wasm32-unknown-unknown]
    - name: fmt
      type: cargo-fmt
      config:
        extra_args: ["--check"]
    - name: check
      type: cargo-check
    - name: clippy
      type: cargo-clippy
      config:
        extra_args: ["--", "-D", "warnings"]
    - name: test
      type: cargo-test
    - name: build
      type: cargo-build
      config:
        release: true
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert!(has_factory(&compiled, "wfe_yaml::rustup::install"));
    assert!(has_factory(&compiled, "wfe_yaml::rustup::add-nightly"));
    assert!(has_factory(&compiled, "wfe_yaml::rustup::add-components"));
    assert!(has_factory(&compiled, "wfe_yaml::rustup::add-wasm"));
    assert!(has_factory(&compiled, "wfe_yaml::cargo::fmt"));
    assert!(has_factory(&compiled, "wfe_yaml::cargo::check"));
    assert!(has_factory(&compiled, "wfe_yaml::cargo::clippy"));
    assert!(has_factory(&compiled, "wfe_yaml::cargo::test"));
    assert!(has_factory(&compiled, "wfe_yaml::cargo::build"));
}

#[test]
fn compile_rustup_component_with_extra_args() {
    let yaml = r#"
workflow:
  id: comp-extra-wf
  version: 1
  steps:
    - name: add-llvm
      type: rustup-component
      config:
        components: [llvm-tools-preview]
        extra_args: ["--force"]
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert!(has_factory(&compiled, "wfe_yaml::rustup::add-llvm"));
}

#[test]
fn compile_rustup_target_multiple() {
    let yaml = r#"
workflow:
  id: multi-target-wf
  version: 1
  steps:
    - name: cross-targets
      type: rustup-target
      config:
        targets:
          - wasm32-unknown-unknown
          - aarch64-linux-android
          - x86_64-unknown-linux-musl
        toolchain: nightly
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert!(has_factory(&compiled, "wfe_yaml::rustup::cross-targets"));

    let step_config = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("cross-targets"))
        .unwrap()
        .step_config
        .as_ref()
        .unwrap();
    assert_eq!(step_config["command"], "target-add");
    let targets = step_config["targets"].as_array().unwrap();
    assert_eq!(targets.len(), 3);
}

// ---------------------------------------------------------------------------
// External cargo tool step compiler tests
// ---------------------------------------------------------------------------

#[test]
fn compile_cargo_audit_step() {
    let yaml = r#"
workflow:
  id: audit-wf
  version: 1
  steps:
    - name: audit
      type: cargo-audit
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert!(has_factory(&compiled, "wfe_yaml::cargo::audit"));

    let step_config = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("audit"))
        .unwrap()
        .step_config
        .as_ref()
        .unwrap();
    assert_eq!(step_config["command"], "audit");
}

#[test]
fn compile_cargo_deny_step() {
    let yaml = r#"
workflow:
  id: deny-wf
  version: 1
  steps:
    - name: license-check
      type: cargo-deny
      config:
        extra_args: ["check"]
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert!(has_factory(&compiled, "wfe_yaml::cargo::license-check"));
}

#[test]
fn compile_cargo_nextest_step() {
    let yaml = r#"
workflow:
  id: nextest-wf
  version: 1
  steps:
    - name: fast-test
      type: cargo-nextest
      config:
        features: [foo]
        extra_args: ["--no-fail-fast"]
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert!(has_factory(&compiled, "wfe_yaml::cargo::fast-test"));

    let step_config = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("fast-test"))
        .unwrap()
        .step_config
        .as_ref()
        .unwrap();
    assert_eq!(step_config["command"], "nextest");
}

#[test]
fn compile_cargo_llvm_cov_step() {
    let yaml = r#"
workflow:
  id: cov-wf
  version: 1
  steps:
    - name: coverage
      type: cargo-llvm-cov
      config:
        extra_args: ["--html", "--output-dir", "coverage"]
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert!(has_factory(&compiled, "wfe_yaml::cargo::coverage"));

    let step_config = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("coverage"))
        .unwrap()
        .step_config
        .as_ref()
        .unwrap();
    assert_eq!(step_config["command"], "llvm-cov");
}

#[test]
fn compile_full_ci_with_external_tools() {
    let yaml = r#"
workflow:
  id: full-ci-external
  version: 1
  steps:
    - name: audit
      type: cargo-audit
    - name: deny
      type: cargo-deny
      config:
        extra_args: ["check", "licenses"]
    - name: test
      type: cargo-nextest
    - name: coverage
      type: cargo-llvm-cov
      config:
        extra_args: ["--summary-only"]
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert!(has_factory(&compiled, "wfe_yaml::cargo::audit"));
    assert!(has_factory(&compiled, "wfe_yaml::cargo::deny"));
    assert!(has_factory(&compiled, "wfe_yaml::cargo::test"));
    assert!(has_factory(&compiled, "wfe_yaml::cargo::coverage"));
}

#[test]
fn compile_cargo_doc_mdx_step() {
    let yaml = r#"
workflow:
  id: doc-mdx-wf
  version: 1
  steps:
    - name: docs
      type: cargo-doc-mdx
      config:
        package: my-crate
        output_dir: docs/api
        extra_args: ["--no-deps"]
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert!(has_factory(&compiled, "wfe_yaml::cargo::docs"));

    let step_config = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("docs"))
        .unwrap()
        .step_config
        .as_ref()
        .unwrap();
    assert_eq!(step_config["command"], "doc-mdx");
    assert_eq!(step_config["package"], "my-crate");
    assert_eq!(step_config["output_dir"], "docs/api");
}

#[test]
fn compile_cargo_doc_mdx_minimal() {
    let yaml = r#"
workflow:
  id: doc-mdx-minimal-wf
  version: 1
  steps:
    - name: generate-docs
      type: cargo-doc-mdx
"#;
    let compiled = load_single_workflow_from_str(yaml, &HashMap::new()).unwrap();
    assert!(has_factory(&compiled, "wfe_yaml::cargo::generate-docs"));

    let step_config = compiled
        .definition
        .steps
        .iter()
        .find(|s| s.name.as_deref() == Some("generate-docs"))
        .unwrap()
        .step_config
        .as_ref()
        .unwrap();
    assert_eq!(step_config["command"], "doc-mdx");
    assert!(step_config["output_dir"].is_null());
}
